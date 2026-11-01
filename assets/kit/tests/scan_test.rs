//! `scan::scan` — view discovery, projection, axis classes, findings.

mod common;

use common::{edit, minihost_copy};
use std::collections::BTreeSet;
use storybook_kit::model::{AxisKind, Inventory};
use storybook_kit::scan::scan;

fn kinds_axes(inv: &Inventory, view: &str) -> Vec<(String, AxisKind, Vec<String>)> {
    inv.views
        .iter()
        .find(|v| v.id == view)
        .unwrap_or_else(|| panic!("view {view} missing"))
        .axes
        .iter()
        .map(|a| (a.path.clone(), a.kind.clone(), a.classes.clone()))
        .collect()
}

fn findings<'a>(
    inv: &'a Inventory,
    view: &str,
    kind: &str,
) -> Vec<&'a storybook_kit::model::Finding> {
    inv.findings
        .iter()
        .filter(|f| f.view == view && f.kind == kind)
        .collect()
}

fn owned(rows: Vec<(&str, AxisKind, Vec<&str>)>) -> Vec<(String, AxisKind, Vec<String>)> {
    rows.into_iter()
        .map(|(p, k, cs)| (p.to_string(), k, cs.into_iter().map(String::from).collect()))
        .collect()
}
#[test]
fn discovers_every_view_in_sorted_file_order() {
    let inv = scan(&minihost_copy("scan-views")).inventory;
    let ids: Vec<&str> = inv.views.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "deep",
            "corners",
            "pairy",
            "pm",
            "pf",
            "pv",
            "alt",
            "create",
            "settings",
            "skill_detail"
        ]
    );
    let create = inv.views.iter().find(|v| v.id == "create").unwrap();
    assert_eq!(create.module, "minihost::create");
    assert_eq!(create.vm_type, "CreateVm");
    assert_eq!(create.actions_type, "CreateActions");
    assert_eq!(create.actions, vec!["on_confirm", "on_pick_dir"]);
    let alt = inv.views.iter().find(|v| v.id == "alt").unwrap();
    assert_eq!(alt.module, "minihost::corners::alt");
    assert_eq!(alt.vm_type, "AltVm");
    let deep = inv.views.iter().find(|v| v.id == "deep").unwrap();
    assert_eq!(deep.module, "nestedhost::inner::deep");
    assert_eq!(deep.vm_type, "DeepVm");
}

#[test]
fn axis_classes_per_type_follow_discovery_rules() {
    let inv = scan(&minihost_copy("scan-axes")).inventory;
    // create: an enum axis is its variant classes even when the variants
    // spell a phase; only Result models a phase axis (discovery.md)
    assert_eq!(
        kinds_axes(&inv, "create"),
        owned(vec![
            (
                "phase",
                AxisKind::Enum {
                    variants: vec!["Idle".into(), "Pending".into(), "Ok".into(), "Err".into()]
                },
                vec!["Idle", "Pending", "Ok", "Err"]
            ),
            ("tags", AxisKind::Collection, vec!["empty", "one", "many"]),
            ("desc", AxisKind::Option, vec!["none", "some"]),
            ("owner.name", AxisKind::Text, vec!["any"]),
        ])
    );
    // settings: int and float numbers, char, Option with structurally-read
    // enum inner, Result phase, struct read as a whole
    assert_eq!(
        kinds_axes(&inv, "settings"),
        owned(vec![
            ("poll_seconds", AxisKind::Number, vec!["any"]),
            ("ratio", AxisKind::Number, vec!["any"]),
            ("marker", AxisKind::Text, vec!["any"]),
            ("prompt", AxisKind::Text, vec!["any"]),
            ("sort", AxisKind::Option, vec!["none", "Asc", "Desc"]),
            ("load", AxisKind::Phase, vec!["ok", "err"]),
            ("meta", AxisKind::Struct, vec!["any"]),
        ])
    );
    // skill_detail: text, bool, plain enum
    assert_eq!(
        kinds_axes(&inv, "skill_detail"),
        owned(vec![
            ("name", AxisKind::Text, vec!["any"]),
            ("dirty", AxisKind::Bool, vec!["false", "true"]),
            (
                "filter",
                AxisKind::Enum {
                    variants: vec!["All".into(), "Installed".into(), "Modified".into()]
                },
                vec!["All", "Installed", "Modified"],
            ),
        ])
    );
}

#[test]
fn projection_drops_unread_fields() {
    let inv = scan(&minihost_copy("scan-proj")).inventory;
    let by_id = |id: &str| inv.views.iter().find(|v| v.id == id).unwrap();
    let create = by_id("create");
    assert_eq!(create.unread_fields, vec!["owner.email"]);
    let detail = by_id("skill_detail");
    assert_eq!(detail.unread_fields, vec!["internal_note"]);
    let settings = by_id("settings");
    assert!(settings.unread_fields.is_empty());
    // no axis mentions the unread fields, and `owner` itself is not an axis
    assert!(
        create
            .axes
            .iter()
            .all(|a| a.path != "owner" && a.path != "owner.email")
    );
}

#[test]
fn corners_fixture_exercises_the_shape_and_projection_rules() {
    let inv = scan(&minihost_copy("scan-corners")).inventory;
    // resolver corners: references, parens, slices, arrays, unit tuple,
    // Cow, a structurally-read Option<bool> through all binding forms, and
    // an enum-in-Option with tuple and struct payloads
    assert_eq!(
        kinds_axes(&inv, "corners"),
        owned(vec![
            ("suffix", AxisKind::Text, vec!["any"]),
            ("parens", AxisKind::Text, vec!["any"]),
            ("raw", AxisKind::Collection, vec!["empty", "one", "many"]),
            ("unit", AxisKind::Struct, vec!["any"]),
            ("autosave", AxisKind::Option, vec!["none", "false", "true"]),
            ("mode", AxisKind::Option, vec!["none", "On", "Custom"]),
            (
                "otp",
                AxisKind::Enum {
                    variants: vec!["Pending".into()]
                },
                vec!["Pending"]
            ),
            ("debug_trace", AxisKind::Bool, vec!["false", "true"]),
            ("cow", AxisKind::Text, vec!["any"]),
            // a pair tuple is a fixed shape, not a size-classed collection
            ("duo", AxisKind::Struct, vec!["any"]),
        ])
    );
    // alt: by-value view params, HashMap collection axis
    assert_eq!(
        kinds_axes(&inv, "alt"),
        owned(vec![
            ("greeting", AxisKind::Text, vec!["any"]),
            ("table", AxisKind::Collection, vec!["empty", "one", "many"]),
        ])
    );
    // deep: a view in a nested crate's subdirectory
    assert_eq!(
        kinds_axes(&inv, "deep"),
        owned(vec![("note", AxisKind::Text, vec!["any"])])
    );

    // the helper views isolate one binding form or type shape each
    assert_eq!(
        kinds_axes(&inv, "pairy"),
        owned(vec![
            (
                "pick",
                AxisKind::Enum {
                    variants: vec!["Yes".into(), "No".into()]
                },
                vec!["Yes", "No"]
            ),
            ("fixed", AxisKind::Struct, vec!["any"]),
            ("boxed", AxisKind::Bool, vec!["false", "true"]),
            ("experimental", AxisKind::Bool, vec!["false", "true"]),
        ])
    );
    assert_eq!(
        kinds_axes(&inv, "pm"),
        owned(vec![
            ("opt", AxisKind::Option, vec!["none", "false", "true"]),
            ("hint", AxisKind::Text, vec!["any"]),
        ])
    );
    assert_eq!(
        kinds_axes(&inv, "pf"),
        owned(vec![
            ("opt", AxisKind::Option, vec!["none", "false", "true"]),
            ("job", AxisKind::Option, vec!["none", "ok", "err"]),
        ])
    );
    assert_eq!(
        kinds_axes(&inv, "pv"),
        owned(vec![
            ("pick", AxisKind::Option, vec!["none", "One", "Named"]),
            ("pick2", AxisKind::Option, vec!["none", "Pair"]),
            ("pick3", AxisKind::Option, vec!["none", "Y", "Z"]),
        ])
    );
    // decoy locals must not leak into the alt view
    assert!(findings(&inv, "alt", "widget-local-state").is_empty());

    // the pair tuple resolves as a fixed shape; nothing in corners is
    // unresolvable any more
    assert!(findings(&inv, "corners", "unresolved-type").is_empty());
    // dyn carriers live in the panel test below; corners has none
    assert!(findings(&inv, "corners", "dynamic-registration").is_empty());
    // one expression macro
    assert_eq!(findings(&inv, "corners", "macro").len(), 1);
    assert!(
        findings(&inv, "corners", "macro")[0]
            .detail
            .contains("trace_val")
    );
    // edge candidates: three literal targets plus two computed ones
    let edges: Vec<&str> = findings(&inv, "corners", "edge-candidate")
        .iter()
        .map(|f| f.detail.rsplit("→ ").next().unwrap_or_default())
        .collect();
    assert_eq!(
        edges,
        vec![
            "corners/base",
            "settings/any-any-none-ok-any",
            "skill_detail/any-true-All",
            "page",
            "nav"
        ]
    );
    // widget-local state: `let mut` and a Cell::new in build, `let mut` in
    // update; Cell::replace is not an initializer
    let locals: Vec<&str> = findings(&inv, "corners", "widget-local-state")
        .iter()
        .map(|f| {
            f.detail
                .strip_prefix("local '")
                .and_then(|d| d.split('\'').next())
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(locals, vec!["edited", "counter", "clicks", "passes"]);
}

#[test]
fn findings_for_widget_local_state_macro_and_unresolved_type() {
    let inv = scan(&minihost_copy("scan-findings")).inventory;
    // widget-local: both the `let mut` counter and the `Cell` captured
    let local = findings(&inv, "skill_detail", "widget-local-state");
    assert_eq!(local.len(), 2);
    assert!(
        local
            .iter()
            .all(|f| f.location.starts_with("src/skill_detail.rs:"))
    );
    assert!(local.iter().any(|f| f.detail.contains("clicks")));

    let mac = findings(&inv, "create", "macro");
    assert_eq!(mac.len(), 1);
    assert_eq!(
        mac[0].detail,
        "macro invocation cannot be inspected: trace_view"
    );
    assert!(mac[0].location.starts_with("src/create.rs:"));

    // settings has no unresolvable field any more; unresolved types are
    // covered by the borrowed host below (its Duration field)
    assert!(findings(&inv, "settings", "unresolved-type").is_empty());

    // feature-gated field never becomes an axis
    assert_eq!(findings(&inv, "settings", "feature-gated").len(), 1);
    assert!(
        !inv.views[1]
            .axes
            .iter()
            .any(|a| a.path.contains("experimental"))
    );

    // unread fields are findings too
    assert_eq!(findings(&inv, "create", "unread-field").len(), 1);
    assert_eq!(findings(&inv, "skill_detail", "unread-field").len(), 1);
}

#[test]
fn edge_candidate_for_dialog_present() {
    let inv = scan(&minihost_copy("scan-edge")).inventory;
    let edges = findings(&inv, "create", "edge-candidate");
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].detail, "present → dir_dialog");
}

#[test]
fn finding_ids_stable_and_fingerprint_tracks_source() {
    let dir = minihost_copy("scan-stable");
    let first = scan(&dir).inventory;
    let second = scan(&dir).inventory;
    assert_eq!(first, second);

    let before = findings(&first, "skill_detail", "widget-local-state")[0].clone();
    edit(&dir.join("src/skill_detail.rs"), "0u32", "1u32");
    let after = scan(&dir).inventory;
    let changed = findings(&after, "skill_detail", "widget-local-state")
        .into_iter()
        .find(|f| f.detail == before.detail)
        .unwrap();
    assert_eq!(changed.id, before.id, "id must not move when source edits");
    assert_ne!(changed.fingerprint, before.fingerprint);

    // editing the VM struct itself changes field findings' fingerprints
    // (they hash the VM tokens) while their ids stay
    let unread_before = findings(&first, "skill_detail", "unread-field")[0].clone();
    edit(
        &dir.join("src/skill_detail.rs"),
        "pub internal_note: String",
        "pub internal_note: String,\n    pub spare: u8",
    );
    let after_vm = scan(&dir).inventory;
    let unread_after = findings(&after_vm, "skill_detail", "unread-field")
        .into_iter()
        .find(|f| f.detail.contains("internal_note"))
        .unwrap();
    assert_eq!(unread_after.id, unread_before.id);
    assert_ne!(unread_after.fingerprint, unread_before.fingerprint);

    // ids are the documented shape
    assert!(before.id.starts_with("f-") && before.id.len() == 8);
    let mut ids = BTreeSet::new();
    for f in &first.findings {
        assert!(f.fingerprint.len() == 16, "fingerprint is 16 hex chars");
        ids.insert(f.id.clone());
    }
    assert_eq!(ids.len(), first.findings.len(), "finding ids unique");
}

#[test]
fn unparseable_file_becomes_a_finding_not_a_panic() {
    let dir = minihost_copy("scan-bad");
    std::fs::write(dir.join("src/garbage.rs"), "this is not rust {{{\n").unwrap();
    let inv = scan(&dir).inventory;
    let f = inv
        .findings
        .iter()
        .find(|f| f.location.starts_with("src/garbage.rs"))
        .unwrap();
    assert_eq!(f.kind, "unresolved-type");
    assert_eq!(f.view, "");
}

#[test]
fn dynamic_registration_finding_for_dyn_vm_field() {
    let dir = minihost_copy("scan-dyn");
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
use std::rc::Rc;

pub struct PanelVm {
    pub handler: Rc<dyn Fn()>,
    pub paren_cb: &'static (dyn Fn()),
    pub slice_cb: &'static [dyn Fn()],
    pub arr_cb: [&'static dyn Fn(); 2],
    pub title: String,
}

pub struct PanelActions;

pub fn build(vm: &PanelVm, _actions: &PanelActions) -> Widget {
    let _ = &vm.title;
    let _ = &vm.handler;
    let _ = (vm.paren_cb, vm.slice_cb, vm.arr_cb);
    Widget
}
"#,
    )
    .unwrap();
    let inv = scan(&dir).inventory;
    let dyns = findings(&inv, "panel", "dynamic-registration");
    assert_eq!(dyns.len(), 4);
    assert!(dyns[0].detail.contains("handler"));
    // the dyn field is neither an axis nor unread
    let panel = inv.views.iter().find(|v| v.id == "panel").unwrap();
    assert!(panel.unread_fields.is_empty());
    assert_eq!(panel.axes.len(), 1);
    assert_eq!(panel.axes[0].path, "title");
}

#[test]
fn inventory_round_trips_through_json() {
    let inv = scan(&minihost_copy("scan-json")).inventory;
    let text = serde_json::to_string_pretty(&inv).unwrap();
    let back: Inventory = serde_json::from_str(&text).unwrap();
    assert_eq!(inv, back);
}

/// A tiny synthetic host: one crate, one view, VM fields of the borrowed
/// shapes (`&str`, `&[u8]`) the minihost no longer carries because they do
/// not round-trip through JSON.
fn borrowed_host(dir: &std::path::Path) {
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
pub struct PanelVm {
    pub label: &'static str,
    pub bytes: &'static [u8],
    pub when: std::time::Duration,
}

pub struct PanelActions;

pub fn build(vm: &PanelVm, _actions: &PanelActions) -> Widget {
    let _ = (vm.label, vm.bytes, vm.when);
    Widget
}
"#,
    )
    .unwrap();
}

#[test]
fn borrowed_str_and_slice_fields_resolve_to_text_and_collection() {
    let dir = minihost_copy("scan-borrowed");
    borrowed_host(&dir);
    let inv = scan(&dir).inventory;
    assert_eq!(
        kinds_axes(&inv, "panel"),
        owned(vec![
            ("label", AxisKind::Text, vec!["any"]),
            ("bytes", AxisKind::Collection, vec!["empty", "one", "many"]),
        ])
    );
    // the std type resolves to no axis: a finding, never a guess
    let unresolved = findings(&inv, "panel", "unresolved-type");
    assert_eq!(unresolved.len(), 1);
    assert!(unresolved[0].detail.contains("when"));
}

#[test]
fn views_sharing_a_module_path_across_crates_are_both_kept() {
    let dir = minihost_copy("scan-twin");
    // a second create.rs inside the nested crate: same module path tail,
    // different crate
    std::fs::write(
        dir.join("nested/src/create.rs"),
        r#"use crate::gtk::Widget;

pub struct CreateVm {
    pub title: String,
}

pub struct CreateActions;

pub fn build(vm: &CreateVm, _actions: &CreateActions) -> Widget {
    let _ = vm.title.len();
    Widget
}
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("nested/src/lib.rs"),
        "pub mod gtk { pub struct Widget; }\npub mod inner;\npub mod create;\n",
    )
    .unwrap();
    let inv = scan(&dir).inventory;
    let creates: Vec<&storybook_kit::model::ViewInfo> = inv
        .views
        .iter()
        .filter(|v| v.id.contains("create"))
        .collect();
    assert_eq!(creates.len(), 2, "{:?}", inv.views);
    // module ids are crate-qualified, so both views survive; the later
    // scan-order collision gets a crate-prefixed id
    let modules: Vec<&str> = creates.iter().map(|v| v.module.as_str()).collect();
    assert!(modules.contains(&"minihost::create"), "{modules:?}");
    assert!(modules.contains(&"nestedhost::create"), "{modules:?}");
    let ids: Vec<&str> = creates.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, vec!["create", "minihost-create"], "{ids:?}");
}

#[test]
fn serde_attributes_become_extractor_limit_findings() {
    let dir = minihost_copy("scan-serde-attrs");
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
pub enum Wire {
    Plain,
    Fancy { loud: bool },
}

#[derive(Serialize, Deserialize)]
pub struct PanelVm {
    pub meta: Meta,
    pub wire: Wire,
    #[serde(default)]
    pub hint: String,
    pub title: String,
}

pub struct PanelActions;

pub fn build(vm: &PanelVm, _actions: &PanelActions) -> Widget {
    let _ = (&vm.meta, &vm.wire, &vm.hint, &vm.title);
    Widget
}
"#,
    )
    .unwrap();
    let inv = scan(&dir).inventory;
    let limits = findings(&inv, "panel", "extractor-limit");
    // one finding for the renamed type, one per affected field (the type
    // finding plus the field whose representation depends on it)
    let details: Vec<&str> = limits.iter().map(|f| f.detail.as_str()).collect();
    assert_eq!(limits.len(), 3, "{details:?}");
    assert!(
        details
            .iter()
            .any(|d| d.contains("Meta") && d.contains("rename_all"))
    );
    assert!(details.iter().any(|d| d.contains("field 'meta'")));
    assert!(
        details
            .iter()
            .any(|d| d.contains("hint") && d.contains("default"))
    );
    // every finding points at the offending declaration
    assert!(
        limits
            .iter()
            .all(|f| f.location.starts_with("src/panel.rs:"))
    );
    // the affected fields are not axes: the checker refuses to invent a
    // representation it cannot model; clean fields stay axes
    let panel = inv.views.iter().find(|v| v.id == "panel").unwrap();
    let paths: Vec<&str> = panel.axes.iter().map(|a| a.path.as_str()).collect();
    assert_eq!(paths, vec!["wire", "title"]);
}

/// A view fn at the crate root would collide with the crate name, so its
/// id is derived from the fn name instead.
#[test]
fn crate_root_view_id_comes_from_the_fn_name() {
    let dir = minihost_copy("scan-root-view");
    std::fs::write(
        dir.join("src/lib.rs"),
        r#"pub mod gtk { pub struct Widget; }

use crate::gtk::Widget;

pub struct RootVm {
    pub name: String,
}

pub struct RootActions;

pub fn build_content(vm: &RootVm, _actions: &RootActions) -> Widget {
    let _ = vm.name.len();
    Widget
}
"#,
    )
    .unwrap();
    let inv = scan(&dir).inventory;
    let root = inv
        .views
        .iter()
        .find(|v| v.module == "minihost")
        .expect("crate-root view is discovered");
    // the crate name is its module, not its id
    assert_eq!(root.id, "content", "{:?}", inv.views);
}

/// A tiny throwaway host workspace (no fixture): `(crate name, lib.rs)`.
fn write_host(tag: &str, members: &[(&str, &str)]) -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::TempDir::new().unwrap();
    let ws = tmp.path().join(tag);
    let names: Vec<String> = members.iter().map(|(n, _)| format!("\"{n}\"")).collect();
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(
        ws.join("Cargo.toml"),
        format!("[workspace]\nmembers = [{}]\n", names.join(", ")),
    )
    .unwrap();
    for (name, body) in members {
        std::fs::create_dir_all(ws.join(name).join("src")).unwrap();
        std::fs::write(
            ws.join(name).join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\nedition = \"2024\"\n"),
        )
        .unwrap();
        std::fs::write(ws.join(name).join("src").join("lib.rs"), body).unwrap();
    }
    (tmp, ws)
}

#[test]
fn field_types_resolve_in_the_crate_that_declares_the_field() {
    // two `Row`s: the VM's field must resolve to the Row of the same
    // crate as the field's declaration, not to the first `Row` in
    // sorted scan order (alpha sorts before beta)
    let (_tmp, ws) = write_host(
        "scan-scope",
        &[
            (
                "alpha",
                "pub struct Widget;\npub struct Row {\n    pub alpha_only: bool,\n}\n",
            ),
            ("beta", concat!("pub struct Widget;\n", "pub mod page;\n",)),
        ],
    );
    std::fs::write(
        ws.join("beta/src/page.rs"),
        concat!(
            "use crate::Widget;\n",
            "pub struct Row {\n    pub flag: bool,\n}\n",
            "pub struct PageVm {\n    pub row: Row,\n}\n",
            "pub struct PageActions;\n",
            "pub fn build(vm: &PageVm, _actions: &PageActions) -> Widget {\n",
            "    let _ = vm.row.flag;\n    Widget\n}\n",
        ),
    )
    .unwrap();
    let inv = scan(&ws).inventory;
    assert!(
        findings(&inv, "page", "unresolved-type").is_empty(),
        "{:?}",
        inv.findings
    );
    assert_eq!(
        kinds_axes(&inv, "page"),
        owned(vec![("row.flag", AxisKind::Bool, vec!["false", "true"])])
    );
}

#[test]
fn unit_variant_enum_renames_do_not_limit_fixtures() {
    // `rename_all`/`rename` on an enum of unit variants only renames the
    // wire VALUES; structural drift ignores scalar values, so the field
    // keeps its skeleton instead of becoming a hand-curated limit
    let (_tmp, ws) = write_host(
        "scan-vec-enum",
        &[("gamma", concat!("pub struct Widget;\n", "pub mod page;\n",))],
    );
    std::fs::write(
        ws.join("gamma/src/page.rs"),
        concat!(
            "use crate::Widget;\n",
            "#[derive(serde::Serialize, serde::Deserialize)]\n",
            "#[serde(rename_all = \"kebab-case\")]\n",
            "pub enum Mode {\n    Symlink,\n    Copy,\n}\n",
            "#[derive(serde::Serialize, serde::Deserialize)]\n",
            "pub struct ModeVm {\n    pub mode: Mode,\n    pub label: String,\n}\n",
            "#[derive(serde::Serialize, serde::Deserialize)]\n",
            "pub struct PageVm {\n    pub modes: Vec<ModeVm>,\n}\n",
            "pub struct PageActions;\n",
            "pub fn build(vm: &PageVm, _actions: &PageActions) -> Widget {\n",
            "    let _ = vm.modes.len();\n    Widget\n}\n",
        ),
    )
    .unwrap();
    let inv = scan(&ws).inventory;
    assert!(
        findings(&inv, "page", "extractor-limit").is_empty(),
        "{:?}",
        inv.findings
    );
    assert_eq!(
        kinds_axes(&inv, "page"),
        owned(vec![(
            "modes",
            AxisKind::Collection,
            vec!["empty", "one", "many"]
        )])
    );
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if src.is_dir() {
            copy_tree(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

/// Copies of the host inside a hidden directory or a nested git worktree
/// (e.g. `.superpowers/…/wt/t1/`) belong to another checkout: scanning
/// them would duplicate views and make locations depend on the scan site.
#[test]
fn hidden_dirs_and_nested_worktrees_are_not_scanned() {
    let dir = minihost_copy("scan-nested");
    let before = scan(&dir).inventory;
    let snapshot = tempfile::TempDir::new().unwrap();
    copy_tree(&dir, snapshot.path());
    copy_tree(snapshot.path(), &dir.join(".tool-state/copy"));
    let wt = dir.join("wt/t1");
    copy_tree(snapshot.path(), &wt);
    std::fs::write(wt.join(".git"), "gitdir: /elsewhere/.git/worktrees/t1\n").unwrap();
    let after = scan(&dir).inventory;
    assert_eq!(after.views, before.views);
    let json = serde_json::to_string(&after).unwrap();
    assert!(
        !json.contains(".tool-state") && !json.contains("wt/t1"),
        "{json}"
    );
}
