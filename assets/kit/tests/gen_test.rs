//! `r#gen` — deterministic fixture skeletons and structural drift.

mod common;

use common::minihost_copy;
use std::collections::BTreeMap;
use storybook_kit::expand::{expand, load_states};
use storybook_kit::r#gen::{Bucket, Skel, skeletons, structural_drift};
use storybook_kit::model::Cell;
use storybook_kit::scan::scan;

/// minihost + per-view states files with dispositions; returns cells.
fn full_setup(tag: &str, classes: &[(&str, &str)]) -> (Vec<Cell>, storybook_kit::scan::ScanReport) {
    let host = minihost_copy(tag);
    let storybook = host.join("storybook");
    let dir = storybook.join("states");
    std::fs::create_dir_all(&dir).unwrap();
    let report = scan(&host);
    let mut disp: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for finding in &report.inventory.findings {
        disp.entry(finding.view.as_str())
            .or_default()
            .push(finding.id.as_str());
    }
    for view in &report.inventory.views {
        let mut body = String::new();
        for (v, cls) in classes {
            if *v == view.id {
                body.push_str(cls);
            }
        }
        let ids = disp.get(view.id.as_str()).cloned().unwrap_or_default();
        if !ids.is_empty() {
            body.push_str("[disposition]\n");
            for id in ids {
                body.push_str(&format!("\"{id}\" = {{ excluded = \"test\" }}\n"));
            }
        }
        std::fs::write(
            dir.join(format!("{}.toml", view.id)),
            format!("view = \"{}\"\n{body}", view.id),
        )
        .unwrap();
    }
    let (states, problems) = load_states(&storybook, &report.inventory);
    assert!(problems.is_empty(), "{problems:?}");
    let expanded = expand(&report.inventory, &states);
    assert!(expanded.problems.is_empty(), "{:?}", expanded.problems);
    (expanded.cells, report)
}

#[test]
fn skeletons_are_byte_identical_across_runs() {
    let (cells, report) = full_setup("gen-determinism", &[]);
    let a = skeletons(&report, &cells);
    let b = skeletons(&report, &cells);
    assert_eq!(a, b);
    let create_states = a.get("create").unwrap();
    assert_eq!(create_states.len(), 24);
    let fix = create_states.get("Idle-empty-none-any").unwrap();
    let json: serde_json::Value = serde_json::from_str(&fix.json).unwrap();
    assert_eq!(json["phase"], "Idle");
    assert_eq!(json["tags"], serde_json::json!([]));
    assert_eq!(json["desc"], serde_json::Value::Null);

    // enum variants: bare variants serialize as strings, payload variants
    // as a one-key object
    let corners = a.get("corners").unwrap();
    let on_json: serde_json::Value = serde_json::from_str(
        &corners
            .get("any-any-empty-any-none-On-Pending-false-any-any")
            .unwrap()
            .json,
    )
    .unwrap();
    assert_eq!(on_json["mode"], serde_json::json!({ "On": false }));
    let custom_json: serde_json::Value = serde_json::from_str(
        &corners
            .get("any-any-empty-any-none-Custom-Pending-false-any-any")
            .unwrap()
            .json,
    )
    .unwrap();
    assert_eq!(
        custom_json["mode"],
        serde_json::json!({ "Custom": { "loud": false } })
    );
    assert_eq!(custom_json["unit"], serde_json::Value::Null);
    assert_eq!(custom_json["duo"], serde_json::json!([0, 0]));

    // a multi-field tuple variant is a one-key object over an array
    let pairy = a.get("pairy").unwrap();
    let yes_json: serde_json::Value =
        serde_json::from_str(&pairy.get("Yes-any-false-false").unwrap().json).unwrap();
    assert_eq!(yes_json["pick"], serde_json::json!({ "Yes": false }));
    let no_json: serde_json::Value =
        serde_json::from_str(&pairy.get("No-any-true-true").unwrap().json).unwrap();
    assert_eq!(no_json["pick"], serde_json::json!({ "No": [0, 0] }));
    assert_eq!(no_json["fixed"], serde_json::json!([0, 0, 0, 0]));
    assert!(!json["owner"]["name"].as_str().unwrap().is_empty());
    assert!(json["owner"]["email"].is_string());
}

#[test]
fn class_specific_values_follow_the_axis() {
    let (cells, report) = full_setup("gen-classes", &[]);
    let skel = skeletons(&report, &cells);
    let detail = skel.get("skill_detail").unwrap();
    let true_cell = detail.get("any-true-All").unwrap();
    let json: serde_json::Value = serde_json::from_str(&true_cell.json).unwrap();
    assert_eq!(json["dirty"], true);
    let many = skel
        .get("create")
        .unwrap()
        .get("Pending-many-some-any")
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&many.json).unwrap();
    assert_eq!(json["tags"].as_array().unwrap().len(), 120);
    // a payload variant of a phase-named enum keeps its tagged form
    let err_phase: serde_json::Value = serde_json::from_str(
        &skel
            .get("create")
            .unwrap()
            .get("Err-many-none-any")
            .unwrap()
            .json,
    )
    .unwrap();
    assert!(
        err_phase["phase"]["Err"]
            .as_str()
            .unwrap()
            .starts_with("text-")
    );
    // Result phase is serde-shaped
    let settings = skel.get("settings").unwrap();
    let err_cell = settings.get("any-any-any-any-none-err-any").unwrap();
    let json: serde_json::Value = serde_json::from_str(&err_cell.json).unwrap();
    assert!(json["load"].get("Err").is_some());
    let ok_cell = settings.get("any-any-any-any-Desc-ok-any").unwrap();
    let json: serde_json::Value = serde_json::from_str(&ok_cell.json).unwrap();
    assert!(json["load"].get("Ok").is_some());
    assert_eq!(json["sort"], "Desc");
}

/// Every generated shape must decode into the host's real serde types:
/// integers are integers, floats keep their point, chars are one character,
/// maps are objects, fixed arrays carry their length, unit is null.
#[test]
fn generated_values_round_trip_through_serde_shapes() {
    let (cells, report) = full_setup("gen-roundtrip", &[]);
    let skel = skeletons(&report, &cells);

    let settings = skel.get("settings").unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&settings.get("any-any-any-any-none-ok-any").unwrap().json).unwrap();
    let poll = &json["poll_seconds"];
    assert!(poll.is_i64(), "integer field must be an integer: {poll}");
    assert_eq!(poll.as_i64(), Some(0));
    assert_eq!(&json["ratio"], &serde_json::json!(0.0));
    assert!(
        !settings
            .get("any-any-any-any-none-ok-any")
            .unwrap()
            .json
            .contains("\"ratio\": 0,"),
        "floats keep their decimal point"
    );
    let marker = json["marker"].as_str().unwrap();
    assert_eq!(marker.chars().count(), 1, "char field is one character");

    // maps are JSON objects sized by the cell's bucket
    let alt = skel.get("alt").unwrap();
    let empty: serde_json::Value =
        serde_json::from_str(&alt.get("any-empty").unwrap().json).unwrap();
    assert_eq!(empty["table"], serde_json::json!({}));
    let one: serde_json::Value = serde_json::from_str(&alt.get("any-one").unwrap().json).unwrap();
    assert_eq!(
        one["table"].as_object().unwrap().values().next(),
        Some(&serde_json::json!(0)),
        "map values follow the value type"
    );
    let many: serde_json::Value = serde_json::from_str(&alt.get("any-many").unwrap().json).unwrap();
    assert_eq!(many["table"].as_object().unwrap().len(), 120);
}

#[test]
fn strings_are_seeded_by_view_and_path() {
    let (cells, report) = full_setup("gen-seeded", &[]);
    let skel = skeletons(&report, &cells);
    let detail = skel.get("skill_detail").unwrap();
    let a: serde_json::Value =
        serde_json::from_str(&detail.get("any-false-All").unwrap().json).unwrap();
    let b: serde_json::Value =
        serde_json::from_str(&detail.get("any-true-Modified").unwrap().json).unwrap();
    // same view+path -> same seed, regardless of cell
    assert_eq!(a["name"], b["name"]);
    // different path -> different seed
    assert_ne!(a["name"], a["internal_note"]);
    assert!(a["name"].as_str().unwrap().starts_with("text-"));
}

#[test]
fn boundary_classes_shape_strings() {
    let classes = (
        "skill_detail",
        "[classes]\n\"name\" = [\"empty\", \"short\", \"overflow\"]\n",
    );
    let (cells, report) = full_setup("gen-bounds", &[classes]);
    let skel = skeletons(&report, &cells);
    let detail = skel.get("skill_detail").unwrap();
    let empty: serde_json::Value =
        serde_json::from_str(&detail.get("empty-false-All").unwrap().json).unwrap();
    assert_eq!(empty["name"], "");
    let overflow: serde_json::Value =
        serde_json::from_str(&detail.get("overflow-false-All").unwrap().json).unwrap();
    assert!(overflow["name"].as_str().unwrap().len() >= 300);
    let short: serde_json::Value =
        serde_json::from_str(&detail.get("short-false-All").unwrap().json).unwrap();
    assert!(short["name"].as_str().unwrap().starts_with("text-"));
}

#[test]
fn curated_values_do_not_count_as_structural_drift() {
    let (cells, report) = full_setup("gen-drift", &[]);
    let skel = skeletons(&report, &cells);
    let fix = skel
        .get("skill_detail")
        .unwrap()
        .get("any-false-All")
        .unwrap();
    let curated = fix.json.replace("\"dirty\": false", "\"dirty\": true");
    assert_eq!(structural_drift(Some(&fix.skel), &curated), None);
    assert_eq!(structural_drift(Some(&fix.skel), &fix.json), None);
}

#[test]
fn drift_checker_flags_missing_extra_retyped_keys_and_buckets() {
    let skel = Skel::Obj(vec![
        ("a".into(), Skel::Str("x".into())),
        ("b".into(), Skel::Bool(true)),
        (
            "c".into(),
            Skel::Arr {
                bucket: Bucket::Empty,
                item: Box::new(Skel::Num(0.0)),
            },
        ),
        ("d".into(), Skel::Wild),
    ]);
    let ok = r#"{"a": "curated", "b": false, "c": [], "d": {"anything": 1}}"#;
    assert_eq!(structural_drift(Some(&skel), ok), None);
    let missing = r#"{"a": "x", "b": true, "d": null}"#;
    assert!(
        structural_drift(Some(&skel), missing)
            .unwrap()
            .contains("c")
    );
    let extra = r#"{"a": "x", "b": true, "c": [], "d": null, "e": 1}"#;
    assert!(structural_drift(Some(&skel), extra).unwrap().contains("e"));
    let retyped = r#"{"a": 5, "b": true, "c": [], "d": null}"#;
    assert!(
        structural_drift(Some(&skel), retyped)
            .unwrap()
            .contains("a")
    );
    let bad_bucket = r#"{"a": "x", "b": true, "c": [1,2,3], "d": null}"#;
    assert!(
        structural_drift(Some(&skel), bad_bucket)
            .unwrap()
            .contains("c")
    );
    let broken = "not json";
    assert!(
        structural_drift(Some(&skel), broken)
            .unwrap()
            .contains("parse")
    );
    assert!(
        structural_drift(None, "{}")
            .unwrap()
            .contains("missing skeleton")
    );
}

#[test]
fn drift_checker_flags_int_float_char_map_and_tuple_shapes() {
    let skel = Skel::Obj(vec![
        ("i".into(), Skel::Int(0)),
        ("f".into(), Skel::Num(0.0)),
        ("c".into(), Skel::Char),
        (
            "m".into(),
            Skel::Map {
                bucket: Bucket::One,
                value: Box::new(Skel::Int(0)),
            },
        ),
        ("t".into(), Skel::Tup(vec![Skel::Int(0), Skel::Int(0)])),
    ]);
    let ok = r#"{"i": 7, "f": 1.5, "c": "z", "m": {"any": 9}, "t": [3, 4]}"#;
    assert_eq!(structural_drift(Some(&skel), ok), None);
    // an integer field cannot become a float: u32 rejects 0.0
    let float_for_int = r#"{"i": 0.0, "f": 0, "c": "z", "m": {"k": 0}, "t": [0, 0]}"#;
    assert!(
        structural_drift(Some(&skel), float_for_int)
            .unwrap()
            .contains("i")
    );
    // a char is exactly one character
    let two_chars = r#"{"i": 0, "f": 0.0, "c": "ab", "m": {"k": 0}, "t": [0, 0]}"#;
    assert!(
        structural_drift(Some(&skel), two_chars)
            .unwrap()
            .contains("c")
    );
    // maps keep their bucket
    let map_bucket = r#"{"i": 0, "f": 0.0, "c": "a", "m": {"k": 0, "j": 1}, "t": [0, 0]}"#;
    assert!(
        structural_drift(Some(&skel), map_bucket)
            .unwrap()
            .contains("m")
    );
    let map_array = r#"{"i": 0, "f": 0.0, "c": "a", "m": [0], "t": [0, 0]}"#;
    assert!(
        structural_drift(Some(&skel), map_array)
            .unwrap()
            .contains("m")
    );
    // a tuple keeps its exact length
    let short_tuple = r#"{"i": 0, "f": 0.0, "c": "a", "m": {"k": 0}, "t": [0]}"#;
    assert!(
        structural_drift(Some(&skel), short_tuple)
            .unwrap()
            .contains("t")
    );
}

/// Fields whose serde representation the extractor cannot model are omitted
/// from the skeleton: the curator writes them by hand against the finding.
#[test]
fn limited_fields_are_omitted_from_skeletons() {
    let dir = minihost_copy("gen-limited");
    std::fs::write(
        dir.join("src/lib.rs"),
        r#"pub mod gtk { pub struct Widget; }
pub mod panel;
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("src/panel.rs"),
        r#"use crate::gtk::Widget;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Meta {
    pub revision: u32,
}

#[derive(Serialize, Deserialize)]
pub struct PanelVm {
    pub meta: Meta,
    #[serde(default)]
    pub hint: String,
    pub title: String,
}

pub struct PanelActions;

pub fn build(vm: &PanelVm, _actions: &PanelActions) -> Widget {
    let _ = (&vm.meta, &vm.hint, &vm.title);
    Widget
}
"#,
    )
    .unwrap();
    let report = scan(&dir);
    let cell = Cell {
        view: "panel".into(),
        state_id: "panel/any".into(),
        assignment: BTreeMap::from([("title".to_string(), "any".to_string())]),
    };
    let skel = skeletons(&report, std::slice::from_ref(&cell));
    let fixture = skel.get("panel").unwrap().get("any").unwrap();
    let json: serde_json::Value = serde_json::from_str(&fixture.json).unwrap();
    let object = json.as_object().unwrap();
    assert!(!object.contains_key("meta"), "{json}");
    assert!(!object.contains_key("hint"), "{json}");
    assert!(object.contains_key("title"), "{json}");
}

/// A synthetic `Option<bool>` axis: the `some` cell must serialize a present
/// value (false until a class says otherwise), never `null`.
#[test]
fn option_bool_some_skeleton_is_a_present_false() {
    use std::collections::BTreeSet;
    use storybook_kit::model::{Axis, AxisKind, Inventory, ViewInfo};
    use storybook_kit::scan::{FieldInfo, ScanReport, TypeShape};

    let report = ScanReport {
        inventory: Inventory {
            views: vec![ViewInfo {
                id: "syn".into(),
                module: "syn".into(),
                vm_type: "SynVm".into(),
                actions_type: "SynActions".into(),
                axes: vec![Axis {
                    path: "flag".into(),
                    kind: AxisKind::Option,
                    classes: vec!["none".into(), "false".into(), "true".into()],
                }],
                actions: vec![],
                unread_fields: vec![],
            }],
            findings: vec![],
        },
        shapes: BTreeMap::from([(
            "syn".to_string(),
            TypeShape::Struct {
                fields: vec![FieldInfo {
                    name: "flag".into(),
                    shape: TypeShape::Option(Box::new(TypeShape::Bool)),
                    location: String::new(),
                    feature_gated: false,
                    has_dyn: false,
                    limited: None,
                }],
            },
        )]),
        types: BTreeMap::new(),
        io_modules: BTreeSet::new(),
        root_modules_pub: BTreeMap::new(),
    };
    let cells = vec![
        Cell {
            view: "syn".into(),
            state_id: "syn/none".into(),
            assignment: BTreeMap::from([("flag".to_string(), "none".to_string())]),
        },
        Cell {
            view: "syn".into(),
            state_id: "syn/false".into(),
            assignment: BTreeMap::from([("flag".to_string(), "false".to_string())]),
        },
        Cell {
            view: "syn".into(),
            state_id: "syn/true".into(),
            assignment: BTreeMap::from([("flag".to_string(), "true".to_string())]),
        },
    ];
    let skel = skeletons(&report, &cells);
    let syn = skel.get("syn").unwrap();
    for state in ["false", "true"] {
        let json: serde_json::Value = serde_json::from_str(&syn.get(state).unwrap().json).unwrap();
        assert!(json["flag"].is_boolean(), "{state}: {json}");
    }
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&syn.get("none").unwrap().json).unwrap()["flag"],
        serde_json::Value::Null
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&syn.get("false").unwrap().json).unwrap()["flag"],
        false
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&syn.get("true").unwrap().json).unwrap()["flag"],
        true
    );
}

/// An integer skeleton accepts any integer spelling and rejects floats —
/// `u32` rejects `0.0`, so `7.0` is real drift, `7` and `7.0`'s integer
/// friends are not.
#[test]
fn integer_skeleton_drift_rejects_floats_not_integers() {
    let skel = Skel::Int(0);
    assert_eq!(structural_drift(Some(&skel), "7"), None);
    assert_eq!(structural_drift(Some(&skel), "-3"), None);
    assert_eq!(structural_drift(Some(&skel), "18446744073709551615"), None);
    let msg = structural_drift(Some(&skel), "7.0").unwrap();
    assert!(msg.contains("expected integer"), "{msg}");
}

/// A `Vec<Item>` where `Item` is an externally tagged enum: the skeleton
/// records every variant, so curation may show any of them.
#[test]
fn drift_accepts_any_enum_variant_in_a_collection() {
    let skel = Skel::Obj(vec![(
        "items".into(),
        Skel::Arr {
            bucket: Bucket::Many,
            item: Box::new(Skel::Enum {
                chosen: 0,
                variants: vec![
                    (
                        "User".into(),
                        Some(Skel::Obj(vec![("text".into(), Skel::Str("a".into()))])),
                    ),
                    (
                        "Agent".into(),
                        Some(Skel::Obj(vec![("text".into(), Skel::Str("b".into()))])),
                    ),
                    ("Login".into(), None),
                ],
            }),
        },
    )]);
    let curated = r#"{"items": [{"User": {"text": "a"}}, {"Agent": {"text": "b"}}, "Login"]}"#;
    assert_eq!(structural_drift(Some(&skel), curated), None);
}

#[test]
fn drift_rejects_an_unknown_enum_variant() {
    let skel = Skel::Obj(vec![(
        "items".into(),
        Skel::Arr {
            bucket: Bucket::One,
            item: Box::new(Skel::Enum {
                chosen: 0,
                variants: vec![
                    (
                        "User".into(),
                        Some(Skel::Obj(vec![("text".into(), Skel::Str("a".into()))])),
                    ),
                    ("Login".into(), None),
                ],
            }),
        },
    )]);
    let msg = structural_drift(Some(&skel), r#"{"items": [{"Robot": {}}]}"#).unwrap();
    assert!(msg.contains("$.items[0]"), "{msg}");
    assert!(msg.contains("Robot"), "{msg}");
    assert!(msg.contains("User") && msg.contains("Login"), "{msg}");
}

#[test]
fn drift_checks_the_chosen_variants_payload() {
    let skel = Skel::Obj(vec![(
        "items".into(),
        Skel::Arr {
            bucket: Bucket::One,
            item: Box::new(Skel::Enum {
                chosen: 0,
                variants: vec![
                    (
                        "User".into(),
                        Some(Skel::Obj(vec![("text".into(), Skel::Str("a".into()))])),
                    ),
                    (
                        "Agent".into(),
                        Some(Skel::Obj(vec![("text".into(), Skel::Str("b".into()))])),
                    ),
                ],
            }),
        },
    )]);
    let msg = structural_drift(Some(&skel), r#"{"items": [{"Agent": {"txt": "b"}}]}"#).unwrap();
    assert!(msg.contains("text"), "{msg}");
    // and the payload must be an object at all
    let msg = structural_drift(Some(&skel), r#"{"items": [{"Agent": "b"}]}"#).unwrap();
    assert!(msg.contains("Agent"), "{msg}");
}

#[test]
fn option_without_a_class_accepts_null_and_a_value() {
    // a non-axis Option (nested in a struct axis) has no assigned class,
    // so both `null` and a present value are structurally valid
    let tmp = tempfile::TempDir::new().unwrap();
    let ws = tmp.path().join("gen-opt");
    std::fs::create_dir_all(ws.join("host/src")).unwrap();
    std::fs::write(ws.join("Cargo.toml"), "[workspace]\nmembers = [\"host\"]\n").unwrap();
    std::fs::write(
        ws.join("host/Cargo.toml"),
        "[package]\nname = \"host\"\nedition = \"2024\"\n",
    )
    .unwrap();
    std::fs::write(
        ws.join("host/src/lib.rs"),
        "pub struct Widget;\npub mod page;\n",
    )
    .unwrap();
    std::fs::write(
        ws.join("host/src/page.rs"),
        concat!(
            "use crate::Widget;\n",
            "pub struct Meta {\n    pub tag: String,\n    pub note: Option<String>,\n}\n",
            "pub struct PageVm {\n    pub meta: Meta,\n}\n",
            "pub struct PageActions;\n",
            "pub fn build(vm: &PageVm, _actions: &PageActions) -> Widget {\n",
            "    let _ = vm.meta.tag.len();\n    Widget\n}\n",
        ),
    )
    .unwrap();
    let storybook = ws.join("storybook");
    let states = storybook.join("states");
    std::fs::create_dir_all(&states).unwrap();
    let report = scan(&ws);
    let mut body = String::from("view = \"page\"\n[disposition]\n");
    for finding in &report.inventory.findings {
        if finding.view == "page" {
            body.push_str(&format!("\"{}\" = {{ excluded = \"test\" }}\n", finding.id));
        }
    }
    std::fs::write(states.join("page.toml"), body).unwrap();
    let (state_files, problems) = load_states(&storybook, &report.inventory);
    assert!(problems.is_empty(), "{problems:?}");
    let expanded = expand(&report.inventory, &state_files);
    assert!(expanded.problems.is_empty(), "{:?}", expanded.problems);
    let skels = skeletons(&report, &expanded.cells);
    let page = skels.get("page").expect("page view");
    let fix = page.values().next().expect("one skeleton");
    let json = fix.json.clone();
    assert_eq!(structural_drift(Some(&fix.skel), &json), None);
    let set = |v: serde_json::Value| -> String { serde_json::to_string(&v).unwrap() };
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let with_text = {
        let mut v = value.clone();
        v["meta"]["note"] = serde_json::json!("curated");
        set(v)
    };
    let with_null = {
        let mut v = value;
        v["meta"]["note"] = serde_json::Value::Null;
        set(v)
    };
    let skel = Some(&fix.skel);
    assert_eq!(structural_drift(skel, &with_text), None, "some must pass");
    assert_eq!(structural_drift(skel, &with_null), None, "none must pass");
}
