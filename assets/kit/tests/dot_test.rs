//! `dot` layout engine (references/formats.md "layout.json"): DOT source
//! generation, `dot -Tjson` parsing on a fixed fixture (no dot needed) and
//! determinism of the full pipeline (skipped when dot is absent).

mod common;

use common::kit_dir;
use std::collections::BTreeMap;
use std::fs;
use storybook_kit::dot;
use storybook_kit::layout::layout;
use storybook_kit::model::{Cell, Edge, Flows, Terminal, TerminalKind};

fn cell(view: &str, state: &str) -> Cell {
    Cell {
        view: view.to_string(),
        state_id: state.to_string(),
        assignment: BTreeMap::new(),
    }
}

fn edge(from: &str, action: &str, to: &str, label: Option<&str>) -> Edge {
    Edge {
        from: from.to_string(),
        action: action.to_string(),
        to: to.to_string(),
        label: label.map(str::to_string),
    }
}

/// Two views (clusters), a forward chain and a cycle-closing back edge.
fn sample() -> (Vec<String>, Vec<Cell>, Flows, BTreeMap<String, String>) {
    let views = vec!["a".to_string(), "b".to_string()];
    let cells = vec![cell("a", "a/s1"), cell("a", "a/s2"), cell("b", "b/s3")];
    let flows = Flows {
        roots: vec!["a/s1".to_string()],
        edges: vec![
            edge("a/s1", "on_next", "a/s2", Some("next \"step\"")),
            edge("a/s2", "on_go", "b/s3", None),
            edge("b/s3", "on_back", "a/s1", None),
        ],
        terminals: vec![Terminal {
            state: "a/s1".to_string(),
            action: "on_stay".to_string(),
            kind: TerminalKind::Self_,
        }],
        variants: BTreeMap::new(),
    };
    let mut titles = BTreeMap::new();
    titles.insert(
        "a/s1".to_string(),
        "Start \"q\" \\ done".to_string(), // quotes and a backslash
    );
    (views, cells, flows, titles)
}

#[test]
fn build_dot_clusters_labels_and_back_edges() {
    let (views, cells, flows, titles) = sample();
    let placed = layout(&views, &cells, &flows, &titles);
    let src = dot::build_dot(&views, &placed);

    // deterministic header
    assert!(src.starts_with("digraph G {\nrankdir=LR;\n"), "{src}");
    // one cluster per view, labeled with the view id
    assert!(
        src.contains("subgraph \"cluster_a\" {\nlabel=\"a\";\n"),
        "{src}"
    );
    assert!(
        src.contains("subgraph \"cluster_b\" {\nlabel=\"b\";\n"),
        "{src}"
    );
    // node ids quoted, titles escaped (quotes and backslashes doubled)
    assert!(
        src.contains("\"a/s1\" [label=\"Start \\\"q\\\" \\\\ done\"];"),
        "{src}"
    );
    // a node without a title falls back to the state id
    assert!(src.contains("\"a/s2\" [label=\"a/s2\"];"), "{src}");
    // edge labels escaped, forward edge constrained
    assert!(
        src.contains("\"a/s1\" -> \"a/s2\" [label=\"next \\\"step\\\"\"];"),
        "{src}"
    );
    // back edge: dashed and out of the ranking
    assert!(
        src.contains("\"b/s3\" -> \"a/s1\" [constraint=false, style=dashed];"),
        "{src}"
    );
}

#[test]
fn parse_dot_json_flips_y_and_reads_geometry() {
    let text = fs::read_to_string(kit_dir().join("tests/fixtures/dot/sample.json")).unwrap();
    let geo = dot::parse_dot_json(&text).expect("fixture parses");

    // node a/s1: center 135.88,130.83, 3.33x2.22 in = 239.76x159.84 px box;
    // bb top y = 250.83, so the top-left is (16.0, 250.83 - 210.75 = 40.08);
    // the stored rect is the thumbnail: the box minus the title strip.
    let n = &geo.nodes["a/s1"];
    assert!((n.x - 16.0).abs() < 0.01, "{n:?}");
    assert!((n.y - 40.08).abs() < 0.01, "{n:?}");
    assert!((n.w - 239.76).abs() < 0.01, "{n:?}");
    assert!((n.h - (159.84 - dot::TITLE_STRIP)).abs() < 0.01, "{n:?}");

    // cluster_a: bb 8,42.83,598.77,242.83 -> x 8, y 8, w 590.77, h 200
    let c = geo
        .clusters
        .iter()
        .find(|c| c.label == "A")
        .expect("cluster A present");
    assert!((c.rect.x - 8.0).abs() < 0.01, "{c:?}");
    assert!((c.rect.y - 8.0).abs() < 0.01, "{c:?}");
    assert!((c.rect.w - 590.77).abs() < 0.01, "{c:?}");
    assert!((c.rect.h - 200.0).abs() < 0.01, "{c:?}");
    // the cluster owns its two nodes (a/s1, a/s2)
    assert_eq!(c.nodes, vec!["a/s1", "a/s2"]);

    // edges: tail/head names, chain points (start + triples), label, arrow
    assert_eq!(geo.edges.len(), 3);
    let e = geo.edges.iter().find(|e| e.tail == "a/s1").unwrap();
    assert_eq!(e.head, "a/s2");
    assert_eq!(e.points.len(), 4);
    assert!((e.points.len() - 1) % 3 == 0);
    // y flip: dot y 130.83 -> 250.83 - 130.83 = 120.0
    assert!((e.points[0].y - 120.0).abs() < 0.01, "{e:?}");
    let lp = e.label_pos.expect("edge label pos");
    assert!(
        (lp.x - 303.38).abs() < 0.01 && (lp.y - 111.75).abs() < 0.01,
        "{lp:?}"
    );
    let arrow = e.arrow.as_ref().expect("arrowhead polygon");
    assert_eq!(arrow.len(), 3);

    // the back edge is a multi-segment chain (10 points = 3 cubics)
    let back = geo.edges.iter().find(|e| e.tail == "b/s3").unwrap();
    assert_eq!(back.points.len(), 10);
    assert!((back.points.len() - 1) % 3 == 0);
}

#[test]
fn dot_program_resolution_respects_env() {
    // explicit "off" disables; an explicit path wins verbatim; unset
    // falls back to searching PATH for `dot`
    assert_eq!(dot::resolve_dot(Some("off")), None);
    assert_eq!(
        dot::resolve_dot(Some("/nonexistent/dot-for-sbk-test")),
        Some("/nonexistent/dot-for-sbk-test".into())
    );
    if !common::has_dot_on_path() {
        assert_eq!(dot::resolve_dot(None), None);
    }
}

#[test]
fn layout_with_dot_is_deterministic_and_attaches_geometry() {
    let Some(program) = dot::dot_program() else {
        eprintln!("skipping: dot not on PATH");
        return;
    };
    let (views, cells, flows, titles) = sample();
    let first = dot::layout_with_dot(&program, &views, &cells, &flows, &titles)
        .expect("dot layout succeeds");
    let second = dot::layout_with_dot(&program, &views, &cells, &flows, &titles)
        .expect("dot layout succeeds");

    assert_eq!(
        serde_json::to_string_pretty(&first).unwrap(),
        serde_json::to_string_pretty(&second).unwrap(),
        "same input must give byte-identical layout.json"
    );
    assert_eq!(first.engine, "dot");
    assert!(first.dot_version.as_deref().unwrap_or("").contains('.'));
    assert!(first.nodes.iter().all(|n| n.rect.is_some()));
    // the back edge keeps its flag and gains a Bezier chain
    let back = first.edges.iter().find(|e| e.back).unwrap();
    let pts = back.points.as_ref().unwrap();
    assert!((pts.len() - 1) % 3 == 0);
    // clusters: one per view that has nodes, labeled with the view id
    assert_eq!(first.clusters.len(), 2);
    assert!(first.clusters.iter().all(|c| c.rect.w > 0.0));
}

#[test]
fn layout_with_dot_fails_gracefully_on_a_broken_program() {
    // A program that exists but is not dot: the pipeline reports, the
    // caller (sbk verify) falls back to the builtin engine.
    let (views, cells, flows, titles) = sample();
    let broken = dot::layout_with_dot("/bin/false".as_ref(), &views, &cells, &flows, &titles);
    assert!(broken.is_err());
    let missing = dot::layout_with_dot(
        "/nonexistent/dot-for-sbk-test".as_ref(),
        &views,
        &cells,
        &flows,
        &titles,
    );
    assert!(missing.is_err());
}

#[test]
fn build_dot_uses_roomier_spacing() {
    let (views, cells, flows, titles) = sample();
    let placed = layout(&views, &cells, &flows, &titles);
    let src = dot::build_dot(&views, &placed);
    assert!(src.contains("nodesep=0.9;\n"), "{src}");
    assert!(src.contains("ranksep=1.6;\n"), "{src}");
    assert!(
        src.contains("subgraph \"cluster_a\" {\nlabel=\"a\";\nmargin=20;\n"),
        "{src}"
    );
}

#[test]
fn variant_grid_size_is_two_columns_of_thumbnails() {
    // count 0: nothing; 1: one row; 2: one row; 3: two rows; titles under
    // each thumbnail and 8 px gaps (references/canvas.md).
    assert_eq!(dot::variant_grid_size(0), (0.0, 0.0));
    let (w1, h1) = dot::variant_grid_size(1);
    assert!((w1 - (2.0 * 112.0 + 8.0)).abs() < 1e-9);
    assert!((h1 - (75.0 + 16.0)).abs() < 1e-9);
    let (_, h2) = dot::variant_grid_size(3);
    assert!((h2 - (2.0 * (75.0 + 16.0) + 8.0)).abs() < 1e-9);
}

#[test]
fn expanded_size_grows_the_node_box() {
    // The dot box of an expanded node: thumbnail + title strip + gap +
    // the variant grid, at the usual node width.
    let (w, h) = dot::expanded_size(2);
    assert!((w - 240.0).abs() < 1e-9);
    let (_, grid_h) = dot::variant_grid_size(2);
    assert!((h - (185.76 + 8.0 + grid_h)).abs() < 1e-9);
}

#[test]
fn build_dot_with_sizes_overrides_only_the_named_nodes() {
    let (views, cells, flows, titles) = sample();
    let placed = layout(&views, &cells, &flows, &titles);
    let mut sizes = BTreeMap::new();
    sizes.insert("a/s1".to_string(), (240.0, 300.0));
    let src = dot::build_dot_with_sizes(&views, &placed, &sizes);
    // 240x300 px = 3.33x4.17 in (px / 72), emitted for the sized node only
    assert!(
        src.contains("\"a/s1\" [label=\"Start \\\"q\\\" \\\\ done\", width=3.33, height=4.17];"),
        "{src}"
    );
    assert!(src.contains("\"a/s2\" [label=\"a/s2\"];"), "{src}");
    // without overrides the source is exactly build_dot's
    assert_eq!(
        dot::build_dot_with_sizes(&views, &placed, &BTreeMap::new()),
        dot::build_dot(&views, &placed)
    );
}

#[test]
fn relayout_with_dot_moves_neighbours_away_from_an_expanded_node() {
    let Some(program) = dot::dot_program() else {
        eprintln!("skipping: dot not on PATH");
        return;
    };
    let (views, cells, flows, titles) = sample();
    let base = dot::layout_with_dot(&program, &views, &cells, &flows, &titles)
        .expect("dot layout succeeds");
    let mut sizes = BTreeMap::new();
    sizes.insert("a/s1".to_string(), dot::expanded_size(2));
    let grown = dot::relayout_with_dot(&program, &views, &base, &sizes).expect("relayout succeeds");
    assert_eq!(grown.engine, "dot");

    // the expanded node's stored rect covers thumbnail + strip + grid
    let (_, grid_h) = dot::variant_grid_size(2);
    let n = grown.nodes.iter().find(|n| n.state_id == "a/s1").unwrap();
    let r = n.rect.unwrap();
    assert!(
        (r.h - (186.0 + 8.0 + grid_h)).abs() < 0.5,
        "expanded rect must cover the whole box: {r:?}"
    );

    // collapsed nodes keep the plain thumbnail rect
    let plain = grown.nodes.iter().find(|n| n.state_id == "a/s2").unwrap();
    assert!((plain.rect.unwrap().h - 160.0).abs() < 0.5);

    // no two node boxes (rect + title strip under collapsed rects) overlap
    let boxes: Vec<(String, (f64, f64, f64, f64))> = grown
        .nodes
        .iter()
        .map(|n| {
            let r = n.rect.unwrap();
            let strip = if r.h > 186.0 { 0.0 } else { dot::TITLE_STRIP };
            (n.state_id.clone(), (r.x, r.y, r.w, r.h + strip))
        })
        .collect();
    for i in 0..boxes.len() {
        for j in (i + 1)..boxes.len() {
            let a = boxes[i].1;
            let b = boxes[j].1;
            let disjoint = a.0 + a.2 <= b.0 + 0.01
                || b.0 + b.2 <= a.0 + 0.01
                || a.1 + a.3 <= b.1 + 0.01
                || b.1 + b.3 <= a.1 + 0.01;
            assert!(
                disjoint,
                "{} overlaps {} after expansion: {a:?} vs {b:?}",
                boxes[i].0, boxes[j].0
            );
        }
    }

    // clusters never overlap either
    for i in 0..grown.clusters.len() {
        for j in (i + 1)..grown.clusters.len() {
            let (a, b) = (&grown.clusters[i].rect, &grown.clusters[j].rect);
            let disjoint = a.x + a.w <= b.x + 0.01
                || b.x + b.w <= a.x + 0.01
                || a.y + a.h <= b.y + 0.01
                || b.y + b.h <= a.y + 0.01;
            assert!(disjoint, "clusters overlap: {a:?} vs {b:?}");
        }
    }
}
