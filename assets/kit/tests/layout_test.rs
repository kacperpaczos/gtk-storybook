//! `layout::layout` — deterministic journey-map geometry: flow nodes only,
//! longest-path columns, duplicate-merged edges, self terminals and data
//! variants stacked under their node.

mod common;

use common::minihost_copy;
use std::collections::BTreeMap;
use storybook_kit::layout::layout;
use storybook_kit::model::{
    Cell, Edge, Flows, Layout, NodePos, Terminal, TerminalKind, VariantPos,
};

fn cell(view: &str, state: &str) -> Cell {
    Cell {
        view: view.to_string(),
        state_id: state.to_string(),
        assignment: BTreeMap::new(),
    }
}

fn edge(from: &str, action: &str, to: &str) -> Edge {
    edge_labeled(from, action, to, None)
}

fn edge_labeled(from: &str, action: &str, to: &str, label: Option<&str>) -> Edge {
    Edge {
        from: from.to_string(),
        action: action.to_string(),
        to: to.to_string(),
        label: label.map(str::to_string),
    }
}

fn sample() -> (Vec<String>, Vec<Cell>, Flows) {
    let views = vec![
        "create".to_string(),
        "settings".to_string(),
        "skill_detail".to_string(),
    ];
    let cells = vec![
        cell("create", "create/a"),
        cell("create", "create/a2"),
        cell("create", "create/b"),
        cell("settings", "settings/s"),
        cell("skill_detail", "skill_detail/d"),
    ];
    let flows = Flows {
        roots: vec!["create/a".to_string()],
        edges: vec![
            edge("create/a", "on_confirm", "create/b"),
            edge("create/b", "on_confirm", "settings/s"),
            edge("settings/s", "on_close", "back"),
            edge("settings/s", "on_reset", "create/a"),
        ],
        terminals: vec![Terminal {
            state: "skill_detail/d".to_string(),
            action: "on_copy_path".to_string(),
            kind: TerminalKind::Leaf,
        }],
        variants: BTreeMap::from([("create/a".to_string(), vec!["create/a2".to_string()])]),
    };
    (views, cells, flows)
}

fn node<'a>(l: &'a Layout, id: &str) -> &'a NodePos {
    l.nodes.iter().find(|n| n.state_id == id).unwrap()
}

fn variant<'a>(l: &'a Layout, id: &str) -> &'a VariantPos {
    l.variants.iter().find(|v| v.state_id == id).unwrap()
}

#[test]
fn lanes_are_view_indexes() {
    let (views, cells, flows) = sample();
    let l = layout(&views, &cells, &flows, &BTreeMap::new());
    assert_eq!(node(&l, "create/a").lane, 0);
    assert_eq!(node(&l, "create/b").lane, 0);
    assert_eq!(node(&l, "settings/s").lane, 1);
    assert_eq!(node(&l, "skill_detail/d").lane, 2);
}

#[test]
fn only_flow_nodes_get_journey_columns() {
    let (views, cells, flows) = sample();
    let l = layout(&views, &cells, &flows, &BTreeMap::new());
    // create/a2 is a data variant, not a node
    assert_eq!(l.nodes.len(), 4);
    assert!(l.nodes.iter().all(|n| n.state_id != "create/a2"));
    // the variant is listed separately, under its node, same lane/depth
    assert_eq!(variant(&l, "create/a2").variant_of, "create/a");
    assert_eq!(variant(&l, "create/a2").lane, node(&l, "create/a").lane);
    assert_eq!(variant(&l, "create/a2").depth, node(&l, "create/a").depth);
    assert_eq!(variant(&l, "create/a2").order, 0);
    // the node carries its variants and the count
    assert_eq!(node(&l, "create/a").variants, vec!["create/a2".to_string()]);
    assert_eq!(node(&l, "create/a").variant_count, 1);
    assert_eq!(node(&l, "create/b").variant_count, 0);
}

#[test]
fn depth_is_longest_path_from_the_roots() {
    // Diamond: a -> b, a -> c, b -> d, c -> d. BFS would put d at 1 from
    // the shorter side; the journey extends right: d lands at layer 2.
    let views = vec!["v".to_string()];
    let cells = [
        cell("v", "v/a"),
        cell("v", "v/b"),
        cell("v", "v/c"),
        cell("v", "v/d"),
    ];
    let flows = Flows {
        roots: vec!["v/a".to_string()],
        edges: vec![
            edge("v/a", "on_b", "v/b"),
            edge("v/a", "on_c", "v/c"),
            edge("v/b", "on_d", "v/d"),
            edge("v/c", "on_d", "v/d"),
        ],
        terminals: vec![],
        variants: BTreeMap::new(),
    };
    let l = layout(&views, &cells, &flows, &BTreeMap::new());
    assert_eq!(node(&l, "v/a").depth, 0);
    assert_eq!(node(&l, "v/b").depth, 1);
    assert_eq!(node(&l, "v/c").depth, 1);
    assert_eq!(node(&l, "v/d").depth, 2);
}

#[test]
fn sample_depths_follow_the_longest_path() {
    let (views, cells, flows) = sample();
    let l = layout(&views, &cells, &flows, &BTreeMap::new());
    assert_eq!(node(&l, "create/a").depth, 0);
    assert_eq!(node(&l, "create/b").depth, 1);
    assert_eq!(node(&l, "settings/s").depth, 2);
    // unreachable from every root: layer 0, never a panic
    assert_eq!(node(&l, "skill_detail/d").depth, 0);
}

#[test]
fn back_edges_resolve_to_the_dfs_parent_and_are_flagged() {
    let (views, cells, flows) = sample();
    let l = layout(&views, &cells, &flows, &BTreeMap::new());
    // `back` from settings/s pops to its DFS parent create/b, not the
    // authored reset target create/a (also a back arc, drawn hidden)
    assert!(
        l.edges
            .iter()
            .any(|e| e.from == "settings/s" && e.back && e.to == "create/b")
    );
    // an edge to a DFS ancestor is a back arc even without to = "back"
    let fwd_again = l
        .edges
        .iter()
        .find(|e| e.from == "settings/s" && e.to == "create/a")
        .unwrap();
    assert!(fwd_again.back);
    let forward = l
        .edges
        .iter()
        .find(|e| e.to == "create/b" && !e.back)
        .unwrap();
    assert_eq!(forward.from, "create/a");
}

#[test]
fn self_terminals_become_node_actions_not_edges() {
    let views = vec!["v".to_string()];
    let cells = [cell("v", "v/a"), cell("v", "v/b")];
    let flows = Flows {
        roots: vec!["v/a".to_string()],
        edges: vec![edge("v/a", "on_go", "v/b")],
        terminals: vec![Terminal {
            state: "v/a".to_string(),
            action: "on_refresh".to_string(),
            kind: TerminalKind::Self_,
        }],
        variants: BTreeMap::new(),
    };
    let l = layout(&views, &cells, &flows, &BTreeMap::new());
    assert_eq!(node(&l, "v/a").self_actions, vec!["on_refresh".to_string()]);
    assert_eq!(l.edges.len(), 1);
}

#[test]
fn duplicate_edges_merge_with_joined_labels() {
    let views = vec!["v".to_string()];
    let cells = [cell("v", "v/a"), cell("v", "v/b")];
    let flows = Flows {
        roots: vec!["v/a".to_string()],
        edges: vec![
            edge_labeled("v/a", "on_one", "v/b", Some("One")),
            edge_labeled("v/a", "on_two", "v/b", Some("Two")),
            edge_labeled("v/a", "on_three", "v/b", Some("One")), // duplicate label
            edge_labeled("v/a", "on_four", "v/b", None),
        ],
        terminals: vec![],
        variants: BTreeMap::new(),
    };
    let l = layout(&views, &cells, &flows, &BTreeMap::new());
    assert_eq!(l.edges.len(), 1);
    let merged = &l.edges[0];
    assert_eq!(merged.from, "v/a");
    assert_eq!(merged.to, "v/b");
    assert!(!merged.back);
    assert_eq!(merged.label.as_deref(), Some("One / Two"));
}

#[test]
fn layout_is_deterministic() {
    let (views, cells, flows) = sample();
    assert_eq!(
        layout(&views, &cells, &flows, &BTreeMap::new()),
        layout(&views, &cells, &flows, &BTreeMap::new())
    );
}

#[test]
fn missing_root_is_not_a_panic() {
    let (views, cells, mut flows) = sample();
    flows.roots = vec!["nowhere".to_string()];
    let l = layout(&views, &cells, &flows, &BTreeMap::new());
    // every flow node stays visible at layer 0; nothing was seeded
    assert_eq!(l.nodes.len(), 4);
    assert!(l.nodes.iter().all(|n| n.depth == 0), "{:?}", l.nodes);
}

#[test]
fn a_second_root_sits_at_its_longest_layer() {
    let (views, cells, mut flows) = sample();
    flows.roots = vec!["create/a".to_string(), "settings/s".to_string()];
    let l = layout(&views, &cells, &flows, &BTreeMap::new());
    assert_eq!(node(&l, "create/a").depth, 0);
    assert_eq!(node(&l, "create/b").depth, 1);
    // settings/s is also a root, but the journey reaches it deeper via
    // create/a -> create/b; longest path wins so the map stays monotone
    assert_eq!(node(&l, "settings/s").depth, 2);
}

// Keep the helper linked (minihost copy also sanity-checks the fixture tree).
#[test]
fn minihost_fixture_tree_exists() {
    let dir = minihost_copy("layout");
    assert!(dir.join("Cargo.toml").is_file());
    assert!(dir.join("src/skill_detail.rs").is_file());
}

#[test]
fn titles_land_in_nodes_and_fall_back_to_none() {
    let (views, cells, flows) = sample();
    let titles = BTreeMap::from([
        ("create/a".to_string(), "Pierwsze uruchomienie".to_string()),
        ("settings/s".to_string(), "Ustawienia".to_string()),
    ]);
    let l = layout(&views, &cells, &flows, &titles);
    assert_eq!(
        node(&l, "create/a").title.as_deref(),
        Some("Pierwsze uruchomienie")
    );
    assert_eq!(node(&l, "settings/s").title.as_deref(), Some("Ustawienia"));
    assert_eq!(node(&l, "create/b").title, None);
}

#[test]
fn rows_within_a_lane_count_up() {
    let (views, cells, flows) = sample();
    let l = layout(&views, &cells, &flows, &BTreeMap::new());
    // create/a and create/b share lane 0: each row in the lane is distinct
    assert_eq!(node(&l, "create/a").order, 0);
    assert_eq!(node(&l, "create/b").order, 1);
}

#[test]
fn self_actions_only_from_this_nodes_self_terminals() {
    let views = vec!["v".to_string()];
    let cells = [cell("v", "v/a"), cell("v", "v/b")];
    let flows = Flows {
        roots: vec!["v/a".to_string()],
        edges: vec![edge("v/a", "on_go", "v/b")],
        terminals: vec![
            // a leaf terminal on v/a and a self terminal on v/b: neither
            // may leak a self action into the other node
            Terminal {
                state: "v/a".to_string(),
                action: "on_done".to_string(),
                kind: TerminalKind::Leaf,
            },
            Terminal {
                state: "v/b".to_string(),
                action: "on_done".to_string(),
                kind: TerminalKind::Self_,
            },
        ],
        variants: BTreeMap::new(),
    };
    let l = layout(&views, &cells, &flows, &BTreeMap::new());
    assert_eq!(node(&l, "v/a").self_actions, Vec::<String>::new());
    assert_eq!(node(&l, "v/b").self_actions, vec!["on_done".to_string()]);
}

#[test]
fn unknown_variant_ids_are_dropped_not_drawn() {
    let views = vec!["v".to_string()];
    let cells = [cell("v", "v/a"), cell("v", "v/x")];
    let flows = Flows {
        roots: vec!["v/a".to_string()],
        edges: vec![edge("v/a", "on_go", "back")],
        terminals: vec![],
        variants: BTreeMap::from([(
            "v/a".to_string(),
            vec!["v/x".to_string(), "v/ghost".to_string()],
        )]),
    };
    let l = layout(&views, &cells, &flows, &BTreeMap::new());
    // v/ghost is no cell: neither counted on the node nor positioned
    assert_eq!(node(&l, "v/a").variant_count, 1);
    assert_eq!(node(&l, "v/a").variants, vec!["v/x".to_string()]);
    assert_eq!(l.variants.len(), 1);
    assert_eq!(variant(&l, "v/x").state_id, "v/x");
    assert!(l.variants.iter().all(|v| v.state_id != "v/ghost"));
}

#[test]
fn back_target_state_is_never_a_flow_node() {
    let views = vec!["v".to_string()];
    // a cell literally named "back" exists: `to = "back"` still means the
    // pop arc, not an edge onto that cell
    let cells = [cell("v", "v/a"), cell("v", "back")];
    let flows = Flows {
        roots: vec!["v/a".to_string()],
        edges: vec![edge("v/a", "on_go", "back")],
        terminals: vec![],
        variants: BTreeMap::new(),
    };
    let l = layout(&views, &cells, &flows, &BTreeMap::new());
    assert_eq!(l.nodes.len(), 1);
    assert!(l.nodes.iter().all(|n| n.state_id != "back"));
    assert!(l.variants.is_empty());
}

#[test]
fn edges_and_nodes_stay_within_flow_nodes() {
    let views = vec!["v".to_string()];
    let cells = [cell("v", "v/a"), cell("v", "v/b")];
    let flows = Flows {
        roots: vec!["v/a".to_string()],
        edges: vec![
            // v/ghost is no cell: no node, no route onto or from it
            edge("v/a", "on_go", "v/ghost"),
            edge("v/ghost", "on_go", "v/b"),
        ],
        terminals: vec![],
        variants: BTreeMap::new(),
    };
    let l = layout(&views, &cells, &flows, &BTreeMap::new());
    assert_eq!(l.nodes.len(), 2);
    assert!(l.nodes.iter().all(|n| n.state_id != "v/ghost"));
    assert!(
        l.edges
            .iter()
            .all(|e| e.from != "v/ghost" && e.to != "v/ghost"),
        "{:?}",
        l.edges
    );
}
