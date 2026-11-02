//! Inline variant expansion: animation math, the relayout generation
//! counter, builtin expansion geometry, variant grid cells, and the
//! no-overlap invariant over the SkillCraft storybook layout (pure, no GUI;
//! the dot path is skipped when `dot` is absent).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use storybook_kit::model::Layout;
use storybook_kit::{dot, model::NodePos};
use storybook_viewer::anim::{self, Relay};
use storybook_viewer::canvas::{
    self, CanvasPlan, NODE_H, NodeGeometry, Rect, TITLE_H, plan, plan_expanded, variant_cells,
};

fn node(state_id: &str, lane: u32, depth: u32, order: u32, variants: u32) -> NodePos {
    NodePos {
        state_id: state_id.to_string(),
        lane,
        depth,
        order,
        title: None,
        self_actions: vec![],
        variants: (1..=variants).map(|i| format!("{state_id}/v{i}")).collect(),
        variant_count: variants,
        rect: None,
    }
}

fn geo(id: &str, x: f64, y: f64, w: f64, h: f64) -> NodeGeometry {
    NodeGeometry {
        state_id: id.to_string(),
        lane: 0,
        depth: 0,
        order: 0,
        title: None,
        variant_count: 0,
        rect: Rect { x, y, w, h },
    }
}

#[test]
fn ease_out_is_0_at_0_1_at_1_and_monotone() {
    assert_eq!(anim::ease_out(0.0), 0.0);
    assert!((anim::ease_out(1.0) - 1.0).abs() < 1e-12);
    let mut last = 0.0;
    for i in 1..=20 {
        let t = i as f64 / 20.0;
        let e = anim::ease_out(t);
        assert!(e >= last - 1e-12, "not monotone at {t}: {e} < {last}");
        assert!(e <= 1.0);
        last = e;
    }
    // ease-out starts fast: a quarter of the time is more than halfway.
    assert!(anim::ease_out(0.25) > 0.5);
}

#[test]
fn blend_returns_old_at_0_new_at_1_and_midpoints_between() {
    let old = vec![geo("a", 0.0, 0.0, 240.0, 160.0)];
    let new = vec![geo("a", 100.0, 40.0, 240.0, 300.0)];
    let b0 = anim::blend_nodes(&old, &new, 0.0);
    assert_eq!(b0[0].rect, old[0].rect);
    let b1 = anim::blend_nodes(&old, &new, 1.0);
    assert_eq!(b1[0].rect, new[0].rect);
    let bm = anim::blend_nodes(&old, &new, 0.5);
    assert_eq!(
        bm[0].rect,
        Rect {
            x: 50.0,
            y: 20.0,
            w: 240.0,
            h: 230.0
        }
    );

    // monotone in t
    let mut last_y = 0.0;
    for i in 0..=10 {
        let b = anim::blend_nodes(&old, &new, i as f64 / 10.0);
        assert!(b[0].rect.y >= last_y);
        last_y = b[0].rect.y;
    }

    // nodes are matched by state id; one missing in old starts at the new
    // position (nothing to animate from).
    let old2 = vec![geo("a", 0.0, 0.0, 240.0, 160.0)];
    let new2 = vec![
        geo("a", 0.0, 0.0, 240.0, 160.0),
        geo("b", 30.0, 30.0, 240.0, 160.0),
    ];
    let b = anim::blend_nodes(&old2, &new2, 0.0);
    assert_eq!(b.len(), 2);
    assert_eq!(b[1].rect, new2[1].rect);
}

#[test]
fn blend_clusters_interpolates_frames_by_view() {
    let old = vec![canvas::ClusterFrame {
        view: "a".into(),
        rect: Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        },
    }];
    let new = vec![canvas::ClusterFrame {
        view: "a".into(),
        rect: Rect {
            x: 50.0,
            y: 0.0,
            w: 200.0,
            h: 100.0,
        },
    }];
    let b = anim::blend_clusters(&old, &new, 0.5);
    assert_eq!(
        b[0].rect,
        Rect {
            x: 25.0,
            y: 0.0,
            w: 150.0,
            h: 100.0
        }
    );
}

#[test]
fn plain_edges_drop_dot_splines_for_the_animation() {
    let mut e = storybook_kit::model::EdgeRoute {
        from: "a".into(),
        to: "b".into(),
        back: false,
        label: Some("go".into()),
        points: Some(vec![
            storybook_kit::model::GeoPoint { x: 0.0, y: 0.0 },
            storybook_kit::model::GeoPoint { x: 1.0, y: 1.0 },
            storybook_kit::model::GeoPoint { x: 2.0, y: 2.0 },
            storybook_kit::model::GeoPoint { x: 3.0, y: 3.0 },
        ]),
        label_pos: Some(storybook_kit::model::GeoPoint { x: 9.0, y: 9.0 }),
        arrow: Some(vec![]),
    };
    let plain = anim::plain_edges(std::slice::from_ref(&e));
    assert!(plain[0].points.is_none());
    assert!(plain[0].label_pos.is_none());
    assert!(plain[0].arrow.is_none());
    assert_eq!(plain[0].label.as_deref(), Some("go"));
    // the input is untouched
    assert!(e.points.is_some());
    e.arrow = None;
}

#[test]
fn relay_drops_stale_results_and_keeps_the_latest() {
    let relay = Relay::default();
    let first = relay.issue();
    assert!(relay.is_current(first));
    // a second click supersedes the first while its relayout still runs
    let second = relay.issue();
    assert!(!relay.is_current(first));
    assert!(relay.is_current(second));
    let third = relay.issue();
    assert!(!relay.is_current(second));
    assert!(relay.is_current(third));
}

#[test]
fn variant_cells_lay_out_a_two_column_grid_inside_the_node() {
    // node with a 2-variant grid: rect = thumb 160 + strip + gap + grid
    let (_, grid_h) = dot::variant_grid_size(2);
    let node = Rect {
        x: 100.0,
        y: 50.0,
        w: 240.0,
        h: NODE_H + TITLE_H + 8.0 + grid_h,
    };
    let cells = variant_cells(node, 2);
    assert_eq!(cells.len(), 2);
    assert!((cells[0].x - (100.0 + 4.0)).abs() < 1e-9, "{cells:?}");
    assert!((cells[0].w - 112.0).abs() < 1e-9);
    assert!((cells[0].h - (75.0 + 16.0)).abs() < 1e-9);
    // second cell sits 8 px right of the first
    assert!((cells[1].x - cells[0].max_x() - 8.0).abs() < 1e-9);
    assert!((cells[1].y - cells[0].y).abs() < 1e-9);

    // three variants: the third starts a second row
    let (_, grid_h3) = dot::variant_grid_size(3);
    let node3 = Rect {
        x: 0.0,
        y: 0.0,
        w: 240.0,
        h: NODE_H + TITLE_H + 8.0 + grid_h3,
    };
    let cells3 = variant_cells(node3, 3);
    assert_eq!(cells3.len(), 3);
    assert!(cells3[2].y > cells3[0].y);

    // a collapsed node has no room for any cell
    assert!(
        variant_cells(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 240.0,
                h: NODE_H
            },
            2
        )
        .is_empty()
    );
}

#[test]
fn plan_expanded_shifts_everything_below_and_keeps_rows_apart() {
    // lane 0: two stacked nodes; the first has 2 variants
    let nodes = vec![
        node("v/a", 0, 0, 0, 2),
        node("v/b", 0, 0, 1, 0),
        node("v/c", 1, 5, 0, 0),
    ];
    let variants = vec![
        storybook_kit::model::VariantPos {
            state_id: "v/a/v1".into(),
            variant_of: "v/a".into(),
            title: None,
            lane: 0,
            depth: 0,
            order: 0,
        },
        storybook_kit::model::VariantPos {
            state_id: "v/a/v2".into(),
            variant_of: "v/a".into(),
            title: None,
            lane: 0,
            depth: 0,
            order: 1,
        },
    ];
    let edges = vec![];
    let collapsed = plan(&nodes, &variants);
    let (_, grid_h) = dot::variant_grid_size(2);
    let mut expanded = BTreeMap::new();
    expanded.insert("v/a".to_string(), TITLE_H + 8.0 + grid_h);
    let grown = plan_expanded(&nodes, &variants, &edges, &expanded);

    let a = grown.nodes.iter().find(|n| n.state_id == "v/a").unwrap();
    assert!(
        (a.rect.h - (NODE_H + TITLE_H + 8.0 + grid_h)).abs() < 1e-9,
        "expanded node grows: {:?}",
        a.rect
    );
    let b_old = collapsed
        .nodes
        .iter()
        .find(|n| n.state_id == "v/b")
        .unwrap();
    let b = grown.nodes.iter().find(|n| n.state_id == "v/b").unwrap();
    assert!(b.rect.y > b_old.rect.y, "the node below moves down");
    assert!(
        b.rect.y >= a.rect.max_y() + 1.0,
        "no overlap: b {:?} vs a bottom {}",
        b.rect,
        a.rect.max_y()
    );
    // nothing overlaps after expansion
    assert!(no_node_overlaps(&grown));
}

fn no_node_overlaps(p: &CanvasPlan) -> bool {
    let boxes: Vec<Rect> = p
        .nodes
        .iter()
        .map(|n| {
            // collapsed nodes carry their title strip below the rect
            let strip = if n.rect.h > NODE_H + 0.5 {
                0.0
            } else {
                TITLE_H
            };
            Rect {
                h: n.rect.h + strip,
                ..n.rect
            }
        })
        .collect();
    disjoint(&boxes)
}

fn disjoint(boxes: &[Rect]) -> bool {
    for i in 0..boxes.len() {
        for j in (i + 1)..boxes.len() {
            let (a, b) = (&boxes[i], &boxes[j]);
            if !(a.max_x() <= b.x + 0.01
                || b.max_x() <= a.x + 0.01
                || a.max_y() <= b.y + 0.01
                || b.max_y() <= a.y + 0.01)
            {
                return false;
            }
        }
    }
    true
}

fn clusters_disjoint(layout: &Layout) -> bool {
    let rects: Vec<Rect> = layout
        .clusters
        .iter()
        .map(|c| Rect {
            x: c.rect.x,
            y: c.rect.y,
            w: c.rect.w,
            h: c.rect.h,
        })
        .collect();
    disjoint(&rects)
}

fn layout_boxes(layout: &Layout) -> Vec<Rect> {
    layout
        .nodes
        .iter()
        .map(|n| {
            let r = n.rect.unwrap();
            let strip = if r.h > 186.0 { 0.0 } else { dot::TITLE_STRIP };
            Rect {
                x: r.x,
                y: r.y,
                w: r.w,
                h: r.h + strip,
            }
        })
        .collect()
}

/// The SkillCraft storybook layout: for the stored layout and for every
/// combination of up to 2 expanded nodes (and all expanded at once), no two
/// node boxes intersect and no two cluster frames overlap — builtin plan
/// and dot (skipped when dot is absent).
#[test]
fn storybook_layout_never_overlaps_for_expansion_combinations() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../storybook/layout.json"
    );
    let Ok(text) = fs::read_to_string(path) else {
        // the kit is vendored into hosts without a storybook fixture next
        // to it; the storybook copy runs the full invariant
        eprintln!("skipping: no storybook/layout.json next to the kit");
        return;
    };
    let layout: Layout = serde_json::from_str(&text).expect("layout.json parses");
    let with_variants: Vec<&NodePos> = layout
        .nodes
        .iter()
        .filter(|n| n.variant_count > 0)
        .collect();
    assert!(!with_variants.is_empty(), "fixture must have variants");

    let mut combos: Vec<BTreeSet<String>> = vec![BTreeSet::new()];
    for a in 0..with_variants.len() {
        let mut one = BTreeSet::new();
        one.insert(with_variants[a].state_id.clone());
        combos.push(one);
        for b in (a + 1)..with_variants.len() {
            let mut two = BTreeSet::new();
            two.insert(with_variants[a].state_id.clone());
            two.insert(with_variants[b].state_id.clone());
            combos.push(two);
        }
    }
    combos.push(with_variants.iter().map(|n| n.state_id.clone()).collect());

    // builtin path
    for combo in &combos {
        let expanded: BTreeMap<String, f64> = combo
            .iter()
            .filter_map(|id| {
                layout.nodes.iter().find(|n| &n.state_id == id).map(|n| {
                    (
                        id.clone(),
                        TITLE_H + 8.0 + dot::variant_grid_size(n.variant_count).1,
                    )
                })
            })
            .collect();
        let plan = plan_expanded(&layout.nodes, &layout.variants, &layout.edges, &expanded);
        assert!(no_node_overlaps(&plan), "builtin overlap with {combo:?}");
    }

    // dot path
    let Some(program) = dot::dot_program() else {
        eprintln!("skipping dot path: dot not on PATH");
        return;
    };
    let views: Vec<String> = {
        let mut ids: Vec<(u32, String)> = layout
            .nodes
            .iter()
            .map(|n| {
                (
                    n.lane,
                    n.state_id.split('/').next().unwrap_or("").to_string(),
                )
            })
            .collect();
        ids.sort();
        ids.dedup();
        ids.into_iter().map(|(_, v)| v).collect()
    };
    for combo in &combos {
        let sizes: BTreeMap<String, (f64, f64)> = combo
            .iter()
            .filter_map(|id| {
                layout
                    .nodes
                    .iter()
                    .find(|n| &n.state_id == id)
                    .map(|n| (id.clone(), dot::expanded_size(n.variant_count)))
            })
            .collect();
        let relaid = dot::relayout_with_dot(&program, &views, &layout, &sizes)
            .unwrap_or_else(|err| panic!("dot relayout failed for {combo:?}: {err}"));
        assert!(
            disjoint(&layout_boxes(&relaid)),
            "dot overlap with {combo:?}"
        );
        assert!(clusters_disjoint(&relaid), "cluster overlap with {combo:?}");
    }
}
