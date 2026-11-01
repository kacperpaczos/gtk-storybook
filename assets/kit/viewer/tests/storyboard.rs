//! Storyboard loading and viewer argument parsing (pure, no GTK).

use std::path::PathBuf;
use storybook_viewer::app::{StartPage, duplicate_groups, load_storyboard, parse_view_args};

fn write(path: PathBuf, contents: &str) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn sample_storybook(dir: &std::path::Path) {
    write(
        dir.join("inventory.json"),
        r#"{"views": [{"id": "v", "module": "ui::v", "vm_type": "Vm", "actions_type": "Actions",
                       "axes": [], "actions": ["on_save"], "unread_fields": []}],
            "findings": []}"#,
    );
    write(
        dir.join("layout.json"),
        r#"{"nodes": [{"state_id": "v/a", "lane": 0, "depth": 0, "order": 0},
                      {"state_id": "v/b", "lane": 0, "depth": 1, "order": 1}],
            "edges": [{"from": "v/a", "to": "v/b", "back": false, "label": "save"}]}"#,
    );
    write(
        dir.join("flows.toml"),
        r#"roots = ["v/a"]
[[edge]]
from = "v/a"
action = "on_save"
to = "v/b"
label = "Save"
"#,
    );
    write(dir.join("fixtures/v/a.json"), r#"{"name": "Alice"}"#);
    write(dir.join("fixtures/v/b.json"), r#"{"name": "Bob"}"#);
}

#[test]
fn parse_args_defaults_are_empty() {
    let args = parse_view_args(std::iter::empty());
    assert_eq!(args.story, None);
    assert!(!args.check_render);
}

#[test]
fn parse_args_reads_story_check_render_and_unknown_flags() {
    let args = parse_view_args([
        "--story".to_string(),
        "v/a".to_string(),
        "--check-render".to_string(),
    ]);
    assert_eq!(args.story.as_deref(), Some("v/a"));
    assert!(args.check_render);

    let args = parse_view_args(["--story=w/b-x".to_string(), "--gtk-debug=1".to_string()]);
    assert_eq!(args.story.as_deref(), Some("w/b-x"));
    assert!(!args.check_render);

    // A dangling --story must not swallow the flag behind it.
    let args = parse_view_args(["--story".to_string(), "--check-render".to_string()]);
    assert_eq!(args.story, None);
    assert!(args.check_render);
    let args = parse_view_args(["--story".to_string()]);
    assert_eq!(args.story, None);
}

#[test]
fn load_storyboard_reads_all_inputs() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("storybook");
    sample_storybook(&dir);
    let sb = load_storyboard(&dir).unwrap();
    assert_eq!(sb.inventory.views.len(), 1);
    assert_eq!(sb.inventory.views[0].id, "v");
    assert_eq!(sb.layout.nodes.len(), 2);
    assert_eq!(sb.layout.edges[0].label.as_deref(), Some("save"));
    assert_eq!(sb.flows.roots, vec!["v/a".to_string()]);
    assert_eq!(sb.flows.edges[0].to, "v/b");
    assert_eq!(sb.fixture_json("v/a").unwrap(), r#"{"name": "Alice"}"#);
    assert_eq!(sb.fixture_json("v/b").unwrap(), r#"{"name": "Bob"}"#);
    assert_eq!(sb.view_name(0), Some("v"));
    assert_eq!(sb.view_name(9), None);
}

#[test]
fn load_storyboard_lists_states_in_expand_order() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("storybook");
    sample_storybook(&dir);
    let sb = load_storyboard(&dir).unwrap();
    // Layout order drives the story list / canvas: (lane, depth, order) sorted.
    assert_eq!(sb.state_ids(), vec!["v/a".to_string(), "v/b".to_string()]);
}

#[test]
fn load_storyboard_missing_layout_points_at_verify() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("storybook");
    write(
        dir.join("inventory.json"),
        r#"{"views": [], "findings": []}"#,
    );
    let err = load_storyboard(&dir).unwrap_err();
    assert!(err.contains("layout.json"), "unexpected error: {err}");
    assert!(
        err.contains("sbk verify"),
        "error should tell how to fix it: {err}"
    );
}

#[test]
fn load_storyboard_missing_inventory_names_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    let err = load_storyboard(tmp.path()).unwrap_err();
    assert!(err.contains("inventory.json"), "unexpected error: {err}");
}

#[test]
fn load_storyboard_bad_json_reports_file_and_error() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("storybook");
    write(dir.join("inventory.json"), "{ not json");
    let err = load_storyboard(&dir).unwrap_err();
    assert!(err.contains("inventory.json"), "unexpected error: {err}");
}

#[test]
fn load_storyboard_bad_flows_reports_toml_error() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("storybook");
    write(
        dir.join("inventory.json"),
        r#"{"views": [], "findings": []}"#,
    );
    write(dir.join("layout.json"), r#"{"nodes": [], "edges": []}"#);
    write(dir.join("flows.toml"), "roots = \nnot toml = = 1\n");
    let err = load_storyboard(&dir).unwrap_err();
    assert!(err.contains("flows.toml"), "unexpected error: {err}");
}

#[test]
fn load_storyboard_missing_fixture_is_an_error_on_demand() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("storybook");
    sample_storybook(&dir);
    let sb = load_storyboard(&dir).unwrap();
    let err = sb.fixture_json("v/missing").unwrap_err();
    assert!(err.contains("v/missing"), "unexpected error: {err}");
    // Without any fixtures directory the load still succeeds (canvas shows
    // placeholders); only building a story needs the fixture.
    std::fs::remove_dir_all(dir.join("fixtures")).unwrap();
    let sb = load_storyboard(&dir).unwrap();
    assert!(sb.fixture_json("v/a").is_err());
}

#[test]
fn duplicate_groups_reports_only_colliding_signatures_sorted() {
    use std::collections::BTreeMap;
    let sigs: BTreeMap<String, String> = [
        ("v/a", "s1"),
        ("v/b", "s2"),
        ("w/c", "s1"),
        ("w/d", "s3"),
        ("w/e", "s2"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let groups = duplicate_groups(&sigs);
    assert_eq!(
        groups,
        vec![
            vec!["v/a".to_string(), "w/c".to_string()],
            vec!["v/b".to_string(), "w/e".to_string()]
        ]
    );
}

#[test]
fn duplicate_groups_is_empty_without_collisions() {
    use std::collections::BTreeMap;
    let sigs: BTreeMap<String, String> = [("v/a", "s1"), ("v/b", "s2")]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    assert!(duplicate_groups(&sigs).is_empty());
}

#[test]
fn parse_args_start_defaults_to_map() {
    let args = parse_view_args(std::iter::empty());
    assert_eq!(args.start, StartPage::Map);
    let args = parse_view_args(["--start".to_string(), "list".to_string()]);
    assert_eq!(args.start, StartPage::List);
    let args = parse_view_args(["--start=map".to_string()]);
    assert_eq!(args.start, StartPage::Map);
    // an unknown value is left to GTK, the default stays
    let args = parse_view_args(["--start".to_string(), "atlas".to_string()]);
    assert_eq!(args.start, StartPage::Map);
}

#[test]
fn load_storyboard_reads_node_titles() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("storybook");
    sample_storybook(&dir);
    let layout = dir.join("layout.json");
    let text = std::fs::read_to_string(&layout).unwrap();
    std::fs::write(
        &layout,
        text.replace(
            "\"state_id\": \"v/a\", \"lane\": 0",
            "\"state_id\": \"v/a\", \"title\": \"Start\", \"lane\": 0",
        ),
    )
    .unwrap();
    let sb = load_storyboard(&dir).unwrap();
    assert_eq!(sb.title_of("v/a"), Some("Start"));
    assert_eq!(sb.title_of("v/b"), None);
}
