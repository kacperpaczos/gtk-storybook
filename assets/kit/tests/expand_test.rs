//! `expand::expand` — cells as the class product minus guards, merged by
//! equivalence rules, with finding dispositions and budget enforcement.

mod common;

use common::{HostDir, minihost_copy};
use std::collections::BTreeMap;
use std::path::Path;
use storybook_kit::expand::{expand, load_states};
use storybook_kit::model::{AxisKind, Inventory, ViewInfo};
use storybook_kit::scan::scan;

/// Scan minihost, load states files from `dir` (writes each file first).
fn setup(
    tag: &str,
    states: &[(String, String)],
) -> (HostDir, std::path::PathBuf, storybook_kit::scan::ScanReport) {
    let host = minihost_copy(tag);
    let storybook = host.join("storybook");
    let dir = storybook.join("states");
    std::fs::create_dir_all(&dir).unwrap();
    for (view, body) in states {
        std::fs::write(dir.join(format!("{view}.toml")), body).unwrap();
    }
    let report = scan(&host);
    (host, storybook, report)
}

fn dispositions_for(report: &storybook_kit::scan::ScanReport, view: &str) -> String {
    let mut out = format!("view = \"{view}\"\n[disposition]\n");
    for finding in report.inventory.findings.iter().filter(|f| f.view == view) {
        if finding.view == "settings" {
            out.push_str(&format!("\"{}\" = {{ excluded = \"ok\" }}\n", finding.id));
        } else {
            out.push_str(&format!(
                "\"{}\" = {{ covered-by = \"{}\" }}\n",
                finding.id,
                covered_by(report, &finding.view)
            ));
        }
    }
    out
}

fn covered_by(report: &storybook_kit::scan::ScanReport, view: &str) -> String {
    let v = report
        .inventory
        .views
        .iter()
        .find(|v| v.id == view)
        .unwrap();
    let classes: Vec<&str> = v
        .axes
        .iter()
        .map(|a| match a.kind {
            AxisKind::Bool => "true",
            AxisKind::Option => a.classes.last().unwrap(),
            _ => a.classes.first().unwrap(),
        })
        .collect();
    format!("{view}/{}", classes.join("-"))
}

#[test]
fn cells_are_the_full_class_product_in_axis_order() {
    let (_host, storybook, report) = setup(
        "expand-product",
        &[
            ("create".into(), "".into()),
            ("settings".into(), "".into()),
            ("skill_detail".into(), "".into()),
        ],
    );
    let dir = storybook.join("states");
    for v in &report.inventory.views {
        let (view, body) = (v.id.clone(), dispositions_for(&report, &v.id));
        std::fs::write(dir.join(format!("{view}.toml")), body).unwrap();
    }
    let (states, problems) = load_states(&storybook, &report.inventory);
    assert!(problems.is_empty(), "{problems:?}");
    let expanded = expand(&report.inventory, &states);
    assert!(expanded.problems.is_empty(), "{:?}", expanded.problems);
    let create: Vec<&storybook_kit::model::Cell> = expanded
        .cells
        .iter()
        .filter(|c| c.view == "create")
        .collect();
    assert_eq!(create.len(), 4 * 3 * 2); // phase x tags x desc (owner.name has 1 class)
    let first = create[0];
    assert_eq!(first.state_id, "create/Idle-empty-none-any");
    assert_eq!(
        first.assignment.get("phase").map(String::as_str),
        Some("Idle")
    );
    assert_eq!(
        first.assignment.get("tags").map(String::as_str),
        Some("empty")
    );
    assert_eq!(
        first.assignment.get("desc").map(String::as_str),
        Some("none")
    );
    let total: usize = report
        .inventory
        .views
        .iter()
        .map(|v| v.axes.iter().map(|a| a.classes.len()).product::<usize>())
        .sum();
    assert_eq!(expanded.cells.len(), total);
}

#[test]
fn guards_prune_impossible_cells() {
    let guard_body = r#"
[[guard]]
when = { "tags" = "many", "phase" = "Pending" }
reason = "rows load only after the phase completes"
"#;
    let (_host, storybook, report) = setup(
        "expand-guard",
        &[
            ("create".into(), guard_body.into()),
            ("settings".into(), "".into()),
            ("skill_detail".into(), "".into()),
        ],
    );
    let dir = storybook.join("states");
    for v in &report.inventory.views {
        let (view, body) = (v.id.clone(), dispositions_for(&report, &v.id));
        let extra = if view == "create" { guard_body } else { "" };
        std::fs::write(dir.join(format!("{view}.toml")), format!("{body}{extra}")).unwrap();
    }
    let (states, problems) = load_states(&storybook, &report.inventory);
    assert!(problems.is_empty(), "{problems:?}");
    let expanded = expand(&report.inventory, &states);
    assert!(expanded.problems.is_empty(), "{:?}", expanded.problems);
    let create: Vec<&storybook_kit::model::Cell> = expanded
        .cells
        .iter()
        .filter(|c| c.view == "create")
        .collect();
    assert_eq!(create.len(), 24 - 2);
    assert!(
        create
            .iter()
            .all(|c| !(c.assignment.get("tags") == Some(&"many".to_string())
                && c.assignment.get("phase") == Some(&"Pending".to_string())))
    );
}

#[test]
fn equivalence_merges_classes() {
    let eq_body = r#"
[[equivalence]]
axis = "tags"
merge = ["one", "many"]
reason = "same layout after the first row"
"#;
    let (_host, storybook, report) = setup(
        "expand-eq",
        &[
            ("create".into(), eq_body.into()),
            ("settings".into(), "".into()),
            ("skill_detail".into(), "".into()),
        ],
    );
    let dir = storybook.join("states");
    for v in &report.inventory.views {
        let (view, body) = (v.id.clone(), dispositions_for(&report, &v.id));
        let extra = if view == "create" { eq_body } else { "" };
        std::fs::write(dir.join(format!("{view}.toml")), format!("{body}{extra}")).unwrap();
    }
    let (states, _) = load_states(&storybook, &report.inventory);
    let expanded = expand(&report.inventory, &states);
    assert!(expanded.problems.is_empty(), "{:?}", expanded.problems);
    let create: Vec<&storybook_kit::model::Cell> = expanded
        .cells
        .iter()
        .filter(|c| c.view == "create")
        .collect();
    assert_eq!(create.len(), 4 * 2 * 2);
    assert!(
        create
            .iter()
            .all(|c| c.assignment.get("tags") != Some(&"many".to_string()))
    );
}

#[test]
fn equivalence_may_merge_three_or_more_classes() {
    let eq3 = "[[equivalence]]\naxis = \"tags\"\nmerge = [\"empty\", \"one\", \"many\"]\nreason = \"list length never matters here\"\n";
    let (_host, storybook, report) = setup(
        "expand-eq3",
        &[
            ("create".into(), eq3.into()),
            ("settings".into(), "".into()),
            ("skill_detail".into(), "".into()),
        ],
    );
    let dir = storybook.join("states");
    for v in &report.inventory.views {
        let (view, body) = (v.id.clone(), dispositions_for(&report, &v.id));
        let extra = if view == "create" { eq3 } else { "" };
        std::fs::write(dir.join(format!("{view}.toml")), format!("{body}{extra}")).unwrap();
    }
    let (states, _) = load_states(&storybook, &report.inventory);
    let expanded = expand(&report.inventory, &states);
    assert!(expanded.problems.is_empty(), "{:?}", expanded.problems);
    let create: Vec<&storybook_kit::model::Cell> = expanded
        .cells
        .iter()
        .filter(|c| c.view == "create")
        .collect();
    // all three tags classes collapse onto the first named class
    assert_eq!(create.len(), 4 * 2); // the three tags classes collapse to one
    assert!(
        create
            .iter()
            .all(|c| c.assignment.get("tags") == Some(&"empty".to_string()))
    );
}
#[test]
fn over_budget_is_red_until_guards_prune() {
    let budget = "view = \"skill_detail\"\nbudget = 4\n";
    // skill_detail has 1*2*3 = 6 cells > 4: red
    let (_host, storybook, report) = setup(
        "expand-budget",
        &[
            ("create".into(), "".into()),
            ("settings".into(), "".into()),
            ("skill_detail".into(), budget.to_string()),
        ],
    );
    let (states, _) = load_states(&storybook, &report.inventory);
    let expanded = expand(&report.inventory, &states);
    assert!(
        expanded
            .problems
            .iter()
            .any(|p| p.contains("skill_detail") && p.contains("budget"))
    );

    // with a guard pruning 2 cells the budget holds: green
    let with_guard = format!(
        "{budget}[[guard]]\nwhen = {{ \"filter\" = \"Modified\" }}\nreason = \"modified filter implies dirty and its own cell\"\n"
    );
    let dir = storybook.join("states");
    std::fs::write(dir.join("skill_detail.toml"), with_guard).unwrap();
    let (states, _) = load_states(&storybook, &report.inventory);
    let expanded = expand(&report.inventory, &states);
    assert!(
        expanded.problems.iter().all(|p| !p.contains("budget")),
        "{:?}",
        expanded.problems
    );
    let detail: Vec<&storybook_kit::model::Cell> = expanded
        .cells
        .iter()
        .filter(|c| c.view == "skill_detail")
        .collect();
    assert_eq!(detail.len(), 4);
}

#[test]
fn findings_must_be_disposed() {
    let (_host, storybook, report) = setup(
        "expand-dispose",
        &[
            ("create".into(), "".into()),
            ("settings".into(), "".into()),
            ("skill_detail".into(), "".into()),
        ],
    );
    let (states, _) = load_states(&storybook, &report.inventory);
    let expanded = expand(&report.inventory, &states);
    let lacking: Vec<_> = expanded
        .problems
        .iter()
        .filter(|p| p.contains("lacks disposition"))
        .collect();
    assert_eq!(lacking.len(), report.inventory.findings.len());

    let dir = storybook.join("states");
    for v in &report.inventory.views {
        let (view, body) = (v.id.clone(), dispositions_for(&report, &v.id));
        std::fs::write(dir.join(format!("{view}.toml")), body).unwrap();
    }
    let (states, _) = load_states(&storybook, &report.inventory);
    let expanded = expand(&report.inventory, &states);
    assert!(expanded.problems.is_empty(), "{:?}", expanded.problems);
}

#[test]
fn bad_states_files_are_reported() {
    let (_host, storybook, report) = setup(
        "expand-bad",
        &[
            ("create".into(), "view = \"create\"\n[classes]\n\"nope\" = [\"x\"]\n[[equivalence]]\naxis = \"tags\"\nmerge = [\"one\", \"missing\"]\nreason = \"x\"\n".into()),
            ("settings".into(), "view = \"settings\"\n[classes]\n\"sort\" = [\"none\", \"bogus\"]\n[[guard]]\nwhen = {}\nreason = \"x\"\n".into()),
            ("skill_detail".into(), "view = \"skill_detail\"\n[disposition]\n\"f-ffff\" = { covered-by = \"nowhere\" }\n".into()),
        ],
    );
    let (states, _) = load_states(&storybook, &report.inventory);
    let expanded = expand(&report.inventory, &states);
    let problems = expanded.problems.join("\n");
    assert!(problems.contains("unknown axis nope"), "{problems}");
    assert!(
        problems.contains("cannot merge unknown class missing"),
        "{problems}"
    );
    assert!(problems.contains("unknown class bogus"), "{problems}");
    assert!(problems.contains("guard with empty when"), "{problems}");
    assert!(problems.contains("unknown finding f-ffff"), "{problems}");
    // covered-by pointing nowhere is a problem too (real dispositions absent here)
    assert!(problems.contains("lacks disposition"), "{problems}");
}

#[test]
fn view_without_axes_gets_a_single_base_cell() {
    let inventory = Inventory {
        views: vec![ViewInfo {
            id: "blank".into(),
            module: "blank".into(),
            vm_type: "BlankVm".into(),
            actions_type: "BlankActions".into(),
            axes: vec![],
            actions: vec!["on_x".into()],
            unread_fields: vec![],
        }],
        findings: vec![],
    };
    let states: BTreeMap<String, _> = BTreeMap::new();
    let expanded = expand(&inventory, &states);
    assert_eq!(expanded.cells.len(), 1);
    assert_eq!(expanded.cells[0].state_id, "blank/base");
    assert!(expanded.cells[0].assignment.is_empty());
    // the missing states file is reported by load_states
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("storybook/states")).unwrap();
    let (_, problems) = load_states(&tmp.path().join("storybook"), &inventory);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("states/blank.toml") && p.contains("not found"))
    );
}

#[test]
fn class_overrides_narrow_or_add_boundaries() {
    let classes_body = r#"
[classes]
"name" = ["short", "overflow"]
"filter" = ["All", "Modified"]
"#;
    let (_host, storybook, report) = setup(
        "expand-classes",
        &[
            ("create".into(), "".into()),
            ("settings".into(), "".into()),
            ("skill_detail".into(), classes_body.into()),
        ],
    );
    let dir = storybook.join("states");
    for v in &report.inventory.views {
        let mut body = dispositions_for(&report, &v.id);
        if v.id == "skill_detail" {
            // the override renames the first name class: fix the covered-by targets
            body = body.replace("skill_detail/any-true-All", "skill_detail/short-true-All");
            body.push_str(classes_body);
        }
        let view = v.id.clone();
        std::fs::write(dir.join(format!("{view}.toml")), body).unwrap();
    }
    let (states, _) = load_states(&storybook, &report.inventory);
    let expanded = expand(&report.inventory, &states);
    assert!(expanded.problems.is_empty(), "{:?}", expanded.problems);
    let detail: Vec<&storybook_kit::model::Cell> = expanded
        .cells
        .iter()
        .filter(|c| c.view == "skill_detail")
        .collect();
    assert_eq!(detail.len(), 2 * 2 * 2); // name short/overflow x dirty x filter narrowed to 2
    assert!(
        detail
            .iter()
            .all(|c| c.assignment.get("name") == Some(&"short".to_string())
                || c.assignment.get("name") == Some(&"overflow".to_string()))
    );
    // narrow fixed-kind classes must stay within the defaults
    let bad = r#"[classes]
"filter" = ["All", "Nope"]
"#;
    let dir = storybook.join("states");
    std::fs::write(
        dir.join("skill_detail.toml"),
        format!("{}{bad}", dispositions_for(&report, "skill_detail")),
    )
    .unwrap();
    let (states, _) = load_states(&storybook, &report.inventory);
    let expanded = expand(&report.inventory, &states);
    assert!(
        expanded
            .problems
            .iter()
            .any(|p| p.contains("unknown class Nope")),
        "{:?}",
        expanded.problems
    );
}

#[test]
fn covered_by_must_reference_a_real_cell_of_the_view() {
    let (_host, storybook, report) = setup(
        "expand-cov",
        &[
            ("create".into(), "".into()),
            ("settings".into(), "".into()),
            ("skill_detail".into(), "".into()),
        ],
    );
    let dir = storybook.join("states");
    for view in ["create", "settings", "skill_detail"] {
        std::fs::write(
            dir.join(format!("{view}.toml")),
            format!(
                "{}\"f-000000\" = {{ covered-by = \"create/not-a-cell\" }}\n",
                dispositions_for(&report, view)
            ),
        )
        .unwrap();
    }
    let (states, _) = load_states(&storybook, &report.inventory);
    let expanded = expand(&report.inventory, &states);
    assert!(
        expanded
            .problems
            .iter()
            .any(|p| p.contains("unknown finding f-000000")),
        "{:?}",
        expanded.problems
    );
    // real finding disposed toward a non-existent cell
    let some_finding = &report.inventory.findings[0];
    std::fs::write(
        dir.join(format!("{}.toml", some_finding.view)),
        format!(
            "{}\"{}\" = {{ covered-by = \"{}/nope\" }}\n",
            dispositions_for(&report, &some_finding.view),
            some_finding.id,
            some_finding.view
        ),
    )
    .unwrap();
    let (states, _) = load_states(&storybook, &report.inventory);
    let expanded = expand(&report.inventory, &states);
    assert!(
        expanded.problems.iter().any(|p| p.contains("no cell")),
        "{:?}",
        expanded.problems
    );
}

#[test]
fn load_states_reports_missing_and_extra_files() {
    let host = minihost_copy("expand-load");
    let storybook = host.join("storybook");
    std::fs::create_dir_all(storybook.join("states")).unwrap();
    std::fs::write(storybook.join("states/ghost.toml"), "view = \"ghost\"\n").unwrap();
    let report = scan(&host);
    let (_, problems) = load_states(&storybook, &report.inventory);
    let text = problems.join("\n");
    assert!(text.contains("states/create.toml not found"), "{text}");
    assert!(text.contains("ghost"), "{text}");
    assert!(!Path::new(&storybook).join("states/ghost.toml").is_dir());
}

/// Findings for files that do not parse belong to no view; they must still
/// be dispositioned (any states file may carry them) instead of being
/// silently dropped by the gates.
#[test]
fn file_level_findings_need_a_disposition_too() {
    let host = minihost_copy("expand-file-finding");
    let storybook = host.join("storybook");
    let dir = storybook.join("states");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(host.join("src/garbage.rs"), "not rust {{{\n").unwrap();
    let report = scan(&host);
    let file_finding = report
        .inventory
        .findings
        .iter()
        .find(|f| f.view.is_empty())
        .unwrap();
    for view in &report.inventory.views {
        std::fs::write(
            dir.join(format!("{}.toml", view.id)),
            dispositions_for(&report, &view.id),
        )
        .unwrap();
    }
    let (states, _) = load_states(&storybook, &report.inventory);
    let expanded = expand(&report.inventory, &states);
    assert!(
        expanded
            .problems
            .iter()
            .any(|p| p.contains(&file_finding.id) && p.contains("disposition")),
        "{:?}",
        expanded.problems
    );

    // disposing it in any view's states file clears the problem
    std::fs::write(
        dir.join("skill_detail.toml"),
        format!(
            "{}\"{}\" = {{ excluded = \"host tool droppings\" }}\n",
            dispositions_for(&report, "skill_detail"),
            file_finding.id
        ),
    )
    .unwrap();
    let (states, _) = load_states(&storybook, &report.inventory);
    let expanded = expand(&report.inventory, &states);
    assert!(
        expanded
            .problems
            .iter()
            .all(|p| !p.contains(&file_finding.id)),
        "{:?}",
        expanded.problems
    );
}

/// A disposition may not silently re-target a finding that belongs to a
/// different view: only this view's findings and file-level findings are
/// disposable here.
#[test]
fn disposition_for_another_views_finding_is_unknown() {
    let (_host, storybook, report) = setup(
        "expand-foreign-disposition",
        &[
            ("create".into(), "".into()),
            ("settings".into(), "".into()),
            ("skill_detail".into(), "".into()),
        ],
    );
    let dir = storybook.join("states");
    let mut inventory = report.inventory.clone();
    let foreign = storybook_kit::model::Finding {
        id: "f-0bad42".to_string(),
        view: "settings".to_string(),
        kind: "widget-local-state".to_string(),
        location: "src/settings.rs:9".to_string(),
        fingerprint: "fp".to_string(),
        detail: "foreign".to_string(),
    };
    inventory.findings.push(foreign.clone());
    let body_for = |inv: &storybook_kit::model::Inventory, view: &str| -> String {
        let mut out = format!("view = \"{view}\"\n[disposition]\n");
        for finding in inv.findings.iter().filter(|f| f.view == view) {
            if finding.view == "settings" {
                out.push_str(&format!("\"{}\" = {{ excluded = \"ok\" }}\n", finding.id));
            } else {
                out.push_str(&format!(
                    "\"{}\" = {{ covered-by = \"{}\" }}\n",
                    finding.id,
                    covered_by(&report, &finding.view)
                ));
            }
        }
        out
    };
    for view in &inventory.views {
        std::fs::write(
            dir.join(format!("{}.toml", view.id)),
            body_for(&inventory, &view.id),
        )
        .unwrap();
    }
    // create's states try to dispose settings' finding
    let create_toml = std::fs::read_to_string(dir.join("create.toml")).unwrap();
    std::fs::write(
        dir.join("create.toml"),
        format!(
            "{create_toml}\"{}\" = {{ excluded = \"not mine\" }}\n",
            foreign.id
        ),
    )
    .unwrap();
    let (states, _) = load_states(&storybook, &inventory);
    let expanded = expand(&inventory, &states);
    assert!(
        expanded
            .problems
            .iter()
            .any(|p| p.contains("view create") && p.contains(&foreign.id)),
        "{:?}",
        expanded.problems
    );
}

#[test]
fn labels_must_name_a_known_axis_or_class() {
    let (_host, storybook, report) = setup(
        "expand-labels",
        &[(
            "settings".into(),
            "view = \"settings\"\n[labels]\n\"sort\" = \"Kolejność\"\n\"sort.none\" = \"brak\"\n\"sort.bogus\" = \"x\"\n\"nope\" = \"y\"\n".into(),
        )],
    );
    let (states, _) = load_states(&storybook, &report.inventory);
    let expanded = expand(&report.inventory, &states);
    let problems: Vec<&String> = expanded
        .problems
        .iter()
        .filter(|p| p.contains("label for unknown"))
        .collect();
    let text = format!("{problems:?}");
    assert_eq!(problems.len(), 2, "{text}");
    assert!(text.contains("unknown axis or class sort.bogus"), "{text}");
    assert!(text.contains("unknown axis or class nope"), "{text}");
}
