//! `check::check_flows` — the `[variants]` rules and flow-node coverage
//! (formats.md): every cell is exactly one of a flow node or a variant,
//! variants never appear in the graph, same view, no dangling ids.

mod common;

use common::HostDir;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use storybook_kit::check::check_flows;
use storybook_kit::model::{Cell, Inventory, ViewInfo};

/// A storybook dir with just `flows.toml`; inventory names one view `v`
/// with the given actions, cells are the listed `v/<state>` ids.
fn book(
    flows_toml: &str,
    actions: &[&str],
    states: &[(&str, &str)],
) -> (HostDir, Inventory, Vec<Cell>) {
    let tmp = tempfile::Builder::new()
        .prefix("sbk-flows-")
        .tempdir()
        .expect("tempdir");
    let dir = Path::new(tmp.path()).join("storybook");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("flows.toml"), flows_toml).unwrap();
    let inventory = Inventory {
        views: vec![ViewInfo {
            id: "v".to_string(),
            module: "v".to_string(),
            vm_type: "Vm".to_string(),
            actions_type: "Actions".to_string(),
            axes: vec![],
            actions: actions.iter().map(|a| a.to_string()).collect(),
            unread_fields: vec![],
        }],
        findings: vec![],
    };
    let cells = states
        .iter()
        .map(|(view, state)| Cell {
            view: view.to_string(),
            state_id: state.to_string(),
            assignment: BTreeMap::new(),
        })
        .collect();
    // Hand ownership of the tempdir to the guard so it outlives the test.
    let host = HostDir::from_tmp(tmp);
    (host, inventory, cells)
}

const GOOD: &str = r#"
roots = ["v/many"]
[[edge]]
from = "v/many"
action = "on_open"
to = "v/detail"
label = "Open"
[[terminal]]
state = "v/many"
action = "on_refresh"
kind = "self"
[[terminal]]
state = "v/detail"
action = "on_open"
kind = "self"
[[terminal]]
state = "v/detail"
action = "on_refresh"
kind = "self"
[variants]
"v/many" = ["v/empty", "v/one"]
"#;

#[test]
fn variants_cover_cells_via_their_node() {
    // v/empty and v/one are variants: no edges, terminals or reachability
    // demanded of them, and the node carries the wiring for the view.
    let (host, inv, cells) = book(
        GOOD,
        &["on_open", "on_refresh"],
        &[
            ("v", "v/many"),
            ("v", "v/empty"),
            ("v", "v/one"),
            ("v", "v/detail"),
        ],
    );
    let out = check_flows(&host.join("storybook"), &inv, &cells);
    assert!(out.problems.is_empty(), "{:?}", out.problems);
}

#[test]
fn cell_neither_flow_node_nor_variant_is_named() {
    let (host, inv, cells) = book(
        GOOD,
        &["on_open", "on_refresh"],
        &[
            ("v", "v/many"),
            ("v", "v/empty"),
            ("v", "v/one"),
            ("v", "v/detail"),
            ("v", "v/other"),
        ],
    );
    let out = check_flows(&host.join("storybook"), &inv, &cells);
    assert!(
        out.problems
            .iter()
            .any(|p| p.contains("v/other") && p.contains("neither a flow node nor a variant")),
        "{:?}",
        out.problems
    );
}

#[test]
fn variant_in_an_edge_is_rejected_with_both_ids() {
    let toml = GOOD.replacen("to = \"v/detail\"", "to = \"v/empty\"", 1);
    let (host, inv, cells) = book(
        &toml,
        &["on_open", "on_refresh"],
        &[
            ("v", "v/many"),
            ("v", "v/empty"),
            ("v", "v/one"),
            ("v", "v/detail"),
        ],
    );
    let out = check_flows(&host.join("storybook"), &inv, &cells);
    assert!(
        out.problems
            .iter()
            .any(|p| p.contains("must not appear in edges")),
        "{:?}",
        out.problems
    );
}

#[test]
fn variant_in_roots_or_terminals_is_rejected() {
    let rest = GOOD.trim_start().trim_start_matches("roots = [\"v/many\"]");
    let toml = format!("roots = [\"v/empty\"] {rest}");
    let (host, inv, cells) = book(
        &toml,
        &["on_open", "on_refresh"],
        &[
            ("v", "v/many"),
            ("v", "v/empty"),
            ("v", "v/one"),
            ("v", "v/detail"),
        ],
    );
    let out = check_flows(&host.join("storybook"), &inv, &cells);
    assert!(
        out.problems
            .iter()
            .any(|p| p.contains("root v/empty") && p.contains("variant of v/many")),
        "{:?}",
        out.problems
    );

    let toml = GOOD.replacen("state = \"v/many\"", "state = \"v/empty\"", 1);
    let (host, inv, cells) = book(
        &toml,
        &["on_open", "on_refresh"],
        &[
            ("v", "v/many"),
            ("v", "v/empty"),
            ("v", "v/one"),
            ("v", "v/detail"),
        ],
    );
    let out = check_flows(&host.join("storybook"), &inv, &cells);
    assert!(
        out.problems.iter().any(|p| p.contains("terminal")
            && p.contains("v/empty")
            && p.contains("variant of v/many")),
        "{:?}",
        out.problems
    );
}

#[test]
fn variant_of_another_view_is_rejected() {
    let (host, inv, cells) = book(
        GOOD,
        &["on_open", "on_refresh"],
        &[
            ("v", "v/many"),
            ("v", "v/detail"),
            // the variant cell lives in another view
            ("w", "v/empty"),
            ("v", "v/one"),
        ],
    );
    let out = check_flows(&host.join("storybook"), &inv, &cells);
    assert!(
        out.problems
            .iter()
            .any(|p| p.contains("v/empty") && p.contains("belongs to view")),
        "{:?}",
        out.problems
    );
}

#[test]
fn flow_node_cannot_be_a_variant() {
    let toml = GOOD.replacen(
        "\"v/many\" = [\"v/empty\", \"v/one\"]",
        "\"v/many\" = [\"v/empty\"]\n\"v/detail\" = [\"v/many\"]",
        1,
    );
    let (host, inv, cells) = book(
        &toml,
        &["on_open", "on_refresh"],
        &[
            ("v", "v/many"),
            ("v", "v/empty"),
            ("v", "v/one"),
            ("v", "v/detail"),
        ],
    );
    let out = check_flows(&host.join("storybook"), &inv, &cells);
    assert!(
        out.problems
            .iter()
            .any(|p| p.contains("v/many") && p.contains("a flow node cannot be a variant")),
        "{:?}",
        out.problems
    );
}

#[test]
fn dangling_variant_and_node_ids_are_named() {
    let toml = GOOD.replacen(
        "\"v/many\" = [\"v/empty\", \"v/one\"]",
        "\"v/many\" = [\"v/empty\", \"v/ghost\"]\n\"v/nowhere\" = [\"v/one\"]",
        1,
    );
    let (host, inv, cells) = book(
        &toml,
        &["on_open", "on_refresh"],
        &[
            ("v", "v/many"),
            ("v", "v/empty"),
            ("v", "v/one"),
            ("v", "v/detail"),
        ],
    );
    let out = check_flows(&host.join("storybook"), &inv, &cells);
    assert!(
        out.problems
            .iter()
            .any(|p| p.contains("v/ghost") && p.contains("not a state in cells.json")),
        "{:?}",
        out.problems
    );
    assert!(
        out.problems
            .iter()
            .any(|p| p.contains("v/nowhere") && p.contains("not a state in cells.json")),
        "{:?}",
        out.problems
    );
}

#[test]
fn variant_listed_under_two_nodes_is_rejected() {
    let toml = GOOD.replacen(
        "\"v/many\" = [\"v/empty\", \"v/one\"]",
        "\"v/many\" = [\"v/empty\"]\n\"v/detail\" = [\"v/empty\"]",
        1,
    );
    let (host, inv, cells) = book(
        &toml,
        &["on_open", "on_refresh"],
        &[
            ("v", "v/many"),
            ("v", "v/empty"),
            ("v", "v/one"),
            ("v", "v/detail"),
        ],
    );
    let out = check_flows(&host.join("storybook"), &inv, &cells);
    assert!(
        out.problems
            .iter()
            .any(|p| p.contains("v/empty") && p.contains("listed under both")),
        "{:?}",
        out.problems
    );
}

#[test]
fn missing_table_means_no_variants_and_still_parses() {
    let no_variants = GOOD.split("[variants]").next().unwrap();
    // v/empty and v/one are plain cells now: they need their own wiring or
    // a variants entry — both problems must be named.
    let (host, inv, cells) = book(
        no_variants,
        &["on_open", "on_refresh"],
        &[
            ("v", "v/many"),
            ("v", "v/empty"),
            ("v", "v/one"),
            ("v", "v/detail"),
        ],
    );
    let out = check_flows(&host.join("storybook"), &inv, &cells);
    assert!(
        out.problems
            .iter()
            .any(|p| p.contains("v/empty") && p.contains("neither a flow node nor a variant")),
        "{:?}",
        out.problems
    );
}

#[test]
fn dangling_edge_target_is_not_declared_unreachable() {
    // v/ghost is no state at all: a dangling-edge problem, never an
    // unreachable-flow-node problem (only cells can be flow nodes)
    let toml = GOOD.replacen("to = \"v/detail\"", "to = \"v/ghost\"", 1);
    let (host, inv, cells) = book(
        &toml,
        &["on_open", "on_refresh"],
        &[
            ("v", "v/many"),
            ("v", "v/empty"),
            ("v", "v/one"),
            ("v", "v/detail"),
        ],
    );
    let out = check_flows(&host.join("storybook"), &inv, &cells);
    assert!(
        out.problems
            .iter()
            .any(|p| p.contains("dangling edge v/many --on_open--> v/ghost")),
        "{:?}",
        out.problems
    );
    assert!(
        !out.problems
            .iter()
            .any(|p| p.contains("v/ghost") && p.contains("unreachable")),
        "{:?}",
        out.problems
    );
}

#[test]
fn edge_only_target_is_a_flow_node() {
    // w/b appears in no root, terminal or [variants] entry: the plain edge
    // from v/a makes it a flow node (its view has no actions to wire)
    let tmp = tempfile::Builder::new()
        .prefix("sbk-flows-")
        .tempdir()
        .expect("tempdir");
    let dir = Path::new(tmp.path()).join("storybook");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("flows.toml"),
        r#"
roots = ["v/a"]
[[edge]]
from = "v/a"
action = "go"
to = "w/b"
"#,
    )
    .unwrap();
    let inv = Inventory {
        views: vec![
            ViewInfo {
                id: "v".to_string(),
                module: "v".to_string(),
                vm_type: "Vm".to_string(),
                actions_type: "Actions".to_string(),
                axes: vec![],
                actions: vec!["go".to_string()],
                unread_fields: vec![],
            },
            ViewInfo {
                id: "w".to_string(),
                module: "w".to_string(),
                vm_type: "Vm".to_string(),
                actions_type: "Actions".to_string(),
                axes: vec![],
                actions: vec![],
                unread_fields: vec![],
            },
        ],
        findings: vec![],
    };
    let cells = vec![
        Cell {
            view: "v".to_string(),
            state_id: "v/a".to_string(),
            assignment: BTreeMap::new(),
        },
        Cell {
            view: "w".to_string(),
            state_id: "w/b".to_string(),
            assignment: BTreeMap::new(),
        },
    ];
    let host = HostDir::from_tmp(tmp);
    let out = check_flows(&host.join("storybook"), &inv, &cells);
    assert!(out.problems.is_empty(), "{:?}", out.problems);
}
