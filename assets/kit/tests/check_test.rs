//! The fixture-to-cell matching rule (`sbk check fixtures`), rule by rule.

use serde_json::json;
use std::collections::BTreeMap;
use storybook_kit::check::class_matches;
use storybook_kit::model::AxisKind;

fn fits(kind: &AxisKind, class: &str, value: serde_json::Value) -> bool {
    class_matches(kind, class, &value)
}

#[test]
fn bool_class_matches_only_its_value() {
    assert!(fits(&AxisKind::Bool, "false", json!(false)));
    assert!(fits(&AxisKind::Bool, "true", json!(true)));
    assert!(!fits(&AxisKind::Bool, "false", json!(true)));
    assert!(!fits(&AxisKind::Bool, "true", json!(false)));
    assert!(!fits(&AxisKind::Bool, "true", json!("true")));
}

#[test]
fn enum_class_matches_its_variant_only() {
    let kind = AxisKind::Enum {
        variants: vec!["All".into(), "Installed".into()],
    };
    assert!(fits(&kind, "All", json!("All")));
    assert!(!fits(&kind, "All", json!("Installed")));
    assert!(!fits(&kind, "All", json!(3)));
    // a variant with payload renders as a one-key object named after it
    assert!(fits(&kind, "All", json!({ "All": { "q": 1 } })));
    assert!(!fits(&kind, "All", json!({ "Installed": { "q": 1 } })));
    // two keys cannot be a single variant
    assert!(!fits(&kind, "All", json!({ "All": 1, "Installed": 2 })));
}

#[test]
fn phase_class_matches_its_result_arm_only() {
    assert!(fits(&AxisKind::Phase, "ok", json!({ "Ok": 1 })));
    assert!(fits(&AxisKind::Phase, "err", json!({ "Err": "boom" })));
    assert!(!fits(&AxisKind::Phase, "ok", json!({ "Err": 1 })));
    assert!(!fits(&AxisKind::Phase, "err", json!({ "Ok": 1 })));
    assert!(!fits(&AxisKind::Phase, "ok", json!("ok")));
    assert!(!fits(
        &AxisKind::Phase,
        "ok",
        json!({ "Ok": 1, "extra": 2 })
    ));
}

#[test]
fn option_class_matches_presence_only() {
    assert!(fits(&AxisKind::Option, "none", json!(null)));
    assert!(!fits(&AxisKind::Option, "none", json!("x")));
    assert!(fits(&AxisKind::Option, "some", json!("")));
    assert!(fits(&AxisKind::Option, "some", json!(0)));
    assert!(!fits(&AxisKind::Option, "some", json!(null)));
}

#[test]
fn collection_class_matches_its_size_bucket() {
    assert!(fits(&AxisKind::Collection, "empty", json!([])));
    assert!(fits(&AxisKind::Collection, "one", json!([1])));
    assert!(fits(&AxisKind::Collection, "many", json!([1, 2])));
    assert!(!fits(&AxisKind::Collection, "many", json!([1])));
    assert!(!fits(&AxisKind::Collection, "one", json!([])));
    assert!(!fits(&AxisKind::Collection, "empty", json!([1])));
    assert!(!fits(&AxisKind::Collection, "one", json!({})));
}

#[test]
fn text_number_struct_classes_accept_anything() {
    for kind in [&AxisKind::Text, &AxisKind::Number, &AxisKind::Struct] {
        assert!(fits(kind, "any", json!(null)));
        assert!(fits(kind, "overflow", json!("long text")));
        assert!(fits(kind, "empty", json!(0)));
    }
}

#[test]
fn option_class_matches_the_variant_it_names() {
    // an Option<Enum> axis carries the inner variants as classes
    assert!(fits(&AxisKind::Option, "On", json!({ "On": false })));
    assert!(fits(&AxisKind::Option, "On", json!("On")));
    assert!(!fits(&AxisKind::Option, "On", json!({ "Custom": { } })));
    assert!(!fits(&AxisKind::Option, "On", json!("Custom")));
    assert!(!fits(&AxisKind::Option, "On", json!(null)));
    // `some` still means any present value
    assert!(fits(&AxisKind::Option, "some", json!({ "On": false })));
}

#[test]
fn collection_class_matches_maps_by_key_count() {
    assert!(fits(&AxisKind::Collection, "empty", json!({})));
    assert!(fits(&AxisKind::Collection, "one", json!({ "k": 1 })));
    assert!(fits(
        &AxisKind::Collection,
        "many",
        json!({ "a": 1, "b": 2 })
    ));
    assert!(!fits(
        &AxisKind::Collection,
        "one",
        json!({ "a": 1, "b": 2 })
    ));
    assert!(!fits(&AxisKind::Collection, "many", json!({})));
    assert!(!fits(&AxisKind::Collection, "one", json!("x")));
}

#[test]
fn phase_class_matches_unit_arms_as_strings() {
    // Result<(), E> serializes its unit arms as bare strings
    assert!(fits(&AxisKind::Phase, "ok", json!("Ok")));
    assert!(fits(&AxisKind::Phase, "err", json!("Err")));
    assert!(!fits(&AxisKind::Phase, "ok", json!("Err")));
    assert!(!fits(&AxisKind::Phase, "err", json!("Ok")));
}

// ---------------------------------------------------------------------------
// uncurated-placeholder + titles gates

use storybook_kit::check::{check_titles, is_placeholder, uncurated_placeholders};
use storybook_kit::model::Cell;

fn cell(view: &str, state_id: &str) -> Cell {
    Cell {
        view: view.to_string(),
        state_id: state_id.to_string(),
        assignment: BTreeMap::new(),
    }
}

#[test]
fn placeholder_shapes_are_exactly_the_generated_ones() {
    assert!(is_placeholder("text-0a1b2c3d"));
    assert!(is_placeholder("text-ffffffff"));
    // the overflow placeholder: the same seed repeated 32 times
    assert!(is_placeholder(&"text-0a1b2c3d".repeat(32)));
    // overflow-sized, but its unit is not a valid seed
    assert!(!is_placeholder(&"text-zzzzzzzz".repeat(32)));
    // near-misses stay green: curation must not be blocked
    assert!(!is_placeholder("text-0a1b2c3dz"));
    assert!(!is_placeholder("text-0a1b2c3"));
    assert!(!is_placeholder("text-0a1b2c3d4"));
    assert!(!is_placeholder("nope-0a1b2c3d"));
    assert!(!is_placeholder("curated text-0a1b2c3d"));
    assert!(!is_placeholder(&"text-0a1b2c3d".repeat(31)));
    assert!(!is_placeholder(""));
}

#[test]
fn uncurated_placeholders_report_json_paths() {
    let value = json!({
        "name": "text-0a1b2c3d",
        "items": [{"q": "ok"}, {"q": "text-ffffffff"}],
        "nested": {"deep": "text-11111111"},
        "n": 3,
        "flag": true
    });
    assert_eq!(
        uncurated_placeholders(&value, "fixtures/v/s.json"),
        vec![
            "uncurated placeholder at fixtures/v/s.json:name; replace with a realistic value"
                .to_string(),
            "uncurated placeholder at fixtures/v/s.json:items[1].q; replace with a realistic value"
                .to_string(),
            "uncurated placeholder at fixtures/v/s.json:nested.deep; replace with a realistic value"
                .to_string(),
        ]
    );
    assert_eq!(
        uncurated_placeholders(&json!("text-0a1b2c3d"), "f.json"),
        vec!["uncurated placeholder at f.json; replace with a realistic value".to_string()]
    );
}

#[test]
fn check_titles_counts_titled_cells_and_names_the_gaps() {
    let cells = vec![cell("v", "v/a"), cell("v", "v/b"), cell("w", "w/c")];
    let titles = BTreeMap::from([("v/a".to_string(), "Start".to_string())]);
    let outcome = check_titles(&cells, &titles);
    assert_eq!(outcome.passed, 1);
    assert_eq!(
        outcome.problems,
        vec![
            "no title for state v/b (add [titles] to states/v.toml)".to_string(),
            "no title for state w/c (add [titles] to states/w.toml)".to_string()
        ]
    );
}
