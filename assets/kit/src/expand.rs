//! Cell expansion: the product of axis classes, narrowed by `states/<view>.toml`
//! classes, merged by equivalence rules, pruned by guards, checked against the
//! budget and the finding dispositions (`references/discovery.md`).

use crate::model::{AxisKind, Cell, CellsFile, DEFAULT_BUDGET, Disposition, Inventory, StatesFile};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Expanded {
    pub cells: Vec<Cell>,
    pub problems: Vec<String>,
}

/// Kinds whose classes are fixed by the type; authors may only narrow them.
fn fixed_kind(kind: &AxisKind) -> bool {
    !matches!(kind, AxisKind::Text | AxisKind::Number | AxisKind::Struct)
}

/// Read `states/<view>.toml` for every view; extra files are problems.
pub fn load_states(
    storybook: &Path,
    inventory: &Inventory,
) -> (BTreeMap<String, StatesFile>, Vec<String>) {
    let mut out = BTreeMap::new();
    let mut problems = Vec::new();
    let dir = storybook.join("states");
    for view in &inventory.views {
        let path = dir.join(format!("{}.toml", view.id));
        match std::fs::read_to_string(&path) {
            Err(_) => problems.push(format!(
                "states/{0}.toml not found (author it for view {0})",
                view.id
            )),
            Ok(text) => match toml::from_str::<StatesFile>(&text) {
                Ok(mut states) => {
                    if states.view != view.id {
                        problems.push(format!(
                            "states/{}.toml declares view {} but must declare {}",
                            view.id, states.view, view.id
                        ));
                        states.view = view.id.clone();
                    }
                    out.insert(view.id.clone(), states);
                }
                Err(err) => problems.push(format!("states/{}.toml does not parse: {err}", view.id)),
            },
        }
    }
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let mut names: Vec<String> = entries
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(String::from))
            .filter(|n| n.ends_with(".toml"))
            .collect();
        names.sort();
        for name in names {
            let view = name.trim_end_matches(".toml");
            if !inventory.views.iter().any(|v| v.id == view) {
                problems.push(format!(
                    "states/{name} has no view named {view} in the inventory"
                ));
            }
        }
    }
    (out, problems)
}

pub fn expand(inventory: &Inventory, states: &BTreeMap<String, StatesFile>) -> Expanded {
    let mut problems = Vec::new();
    let mut cells: Vec<Cell> = Vec::new();

    for view in &inventory.views {
        let empty = StatesFile {
            view: view.id.clone(),
            title: None,
            budget: None,
            classes: BTreeMap::new(),
            guards: Vec::new(),
            equivalences: Vec::new(),
            dispositions: BTreeMap::new(),
            titles: BTreeMap::new(),
            labels: BTreeMap::new(),
        };
        let file = states.get(&view.id).unwrap_or(&empty);
        for unknown in file
            .classes
            .keys()
            .filter(|k| !view.axes.iter().any(|a| a.path == **k))
        {
            problems.push(format!(
                "view {}: class override for unknown axis {unknown}",
                view.id
            ));
        }

        // final class lists per axis, in axis order
        let mut columns: Vec<Vec<String>> = Vec::new();
        let mut axis_paths: Vec<String> = Vec::new();
        for axis in &view.axes {
            let mut classes = axis.classes.clone();
            if let Some(declared) = file.classes.get(&axis.path) {
                if declared.is_empty() {
                    problems.push(format!(
                        "view {}: class override for axis {} is empty",
                        view.id, axis.path
                    ));
                    classes = Vec::new();
                } else if fixed_kind(&axis.kind) {
                    let unknown: Vec<&String> =
                        declared.iter().filter(|c| !classes.contains(c)).collect();
                    if !unknown.is_empty() {
                        problems.push(format!(
                            "view {}: unknown class {} for axis {}",
                            view.id,
                            unknown
                                .iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join(", "),
                            axis.path
                        ));
                        classes = declared
                            .clone()
                            .into_iter()
                            .filter(|c| axis.classes.contains(c))
                            .collect();
                    } else {
                        classes = declared.clone();
                    }
                } else {
                    classes = declared.clone();
                }
            }
            columns.push(classes);
            axis_paths.push(axis.path.clone());
        }

        // [labels] keys name an axis ("<axis>") or one of its final classes
        // ("<axis>.<class>"); a typo would silently fall back to the id
        for key in file.labels.keys() {
            let known = axis_paths.iter().zip(&columns).any(|(path, classes)| {
                key == path
                    || key
                        .strip_prefix(path.as_str())
                        .and_then(|rest| rest.strip_prefix('.'))
                        .is_some_and(|class| classes.iter().any(|c| c == class))
            });
            if !known {
                problems.push(format!(
                    "view {}: label for unknown axis or class {key}",
                    view.id
                ));
            }
        }

        // equivalence merges: merged classes collapse onto the first listed
        for eq in &file.equivalences {
            let Some(idx) = axis_paths.iter().position(|p| *p == eq.axis) else {
                problems.push(format!(
                    "view {}: equivalence names unknown axis {}",
                    view.id, eq.axis
                ));
                continue;
            };
            if eq.merge.len() < 2 {
                problems.push(format!(
                    "view {}: equivalence on {} needs at least two classes to merge",
                    view.id, eq.axis
                ));
                continue;
            }
            let missing: Vec<&String> = eq
                .merge
                .iter()
                .filter(|c| !columns[idx].contains(c))
                .collect();
            if !missing.is_empty() {
                problems.push(format!(
                    "view {}: cannot merge unknown class {} on axis {}",
                    view.id,
                    missing
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    eq.axis
                ));
                continue;
            }
            let canonical = eq.merge[0].clone();
            let merged: BTreeSet<&String> = eq.merge.iter().collect();
            columns[idx] = columns[idx]
                .iter()
                .map(|c| {
                    if merged.contains(c) {
                        canonical.clone()
                    } else {
                        c.clone()
                    }
                })
                .collect();
            let mut deduped: Vec<String> = Vec::new();
            for class in columns[idx].drain(..) {
                if !deduped.contains(&class) {
                    deduped.push(class);
                }
            }
            columns[idx] = deduped;
        }

        // guards: validate, keep the applicable ones
        let mut guards: Vec<&BTreeMap<String, String>> = Vec::new();
        for guard in &file.guards {
            if guard.when.is_empty() {
                problems.push(format!(
                    "view {}: guard with empty when ({})",
                    view.id, guard.reason
                ));
                continue;
            }
            let mut valid = true;
            for (axis, class) in &guard.when {
                match axis_paths.iter().position(|p| p == axis) {
                    None => {
                        problems.push(format!(
                            "view {}: guard names unknown axis {}",
                            view.id, axis
                        ));
                        valid = false;
                    }
                    Some(i) => {
                        if !columns[i].contains(class) {
                            problems.push(format!(
                                "view {}: guard uses unknown class {class} for axis {axis}",
                                view.id
                            ));
                            valid = false;
                        }
                    }
                }
            }
            if valid {
                guards.push(&guard.when);
            }
        }

        // product in axis order
        let view_cells = product(&view.id, &axis_paths, &columns);
        let view_cells: Vec<Cell> = view_cells
            .into_iter()
            .filter(|cell| {
                !guards.iter().any(|when| {
                    when.iter()
                        .all(|(axis, class)| cell.assignment.get(axis) == Some(class))
                })
            })
            .collect();

        let budget = file.budget.unwrap_or(DEFAULT_BUDGET);
        if view_cells.len() as u32 > budget {
            problems.push(format!(
                "view {}: {} cells exceed budget {budget} (add guards or equivalence rules)",
                view.id,
                view_cells.len()
            ));
        }

        // dispositions
        for finding in inventory.findings.iter().filter(|f| f.view == view.id) {
            if !file.dispositions.contains_key(&finding.id) {
                problems.push(format!(
                    "view {}: finding {} ({}) lacks disposition",
                    view.id, finding.id, finding.kind
                ));
            }
        }
        for id in file.dispositions.keys() {
            let known = inventory
                .findings
                .iter()
                .any(|f| f.view == view.id && f.id == *id);
            // file-level findings (unparseable sources) belong to no view:
            // any states file may dispose them
            let file_level = inventory
                .findings
                .iter()
                .any(|f| f.view.is_empty() && f.id == *id);
            if !known && !file_level {
                problems.push(format!(
                    "view {}: disposition for unknown finding {id}",
                    view.id
                ));
            }
        }
        for (id, disposition) in &file.dispositions {
            if let Disposition::CoveredBy { covered_by } = disposition
                && !view_cells.iter().any(|c| c.state_id == *covered_by)
            {
                problems.push(format!(
                    "view {}: disposition of {id} references no cell {covered_by} (covered-by must name a cell of the same view)",
                    view.id
                ));
            }
        }

        cells.extend(view_cells);
    }

    // findings that belong to no view (files that do not parse) still need
    // a disposition; any states file may carry it
    let disposed_anywhere: BTreeSet<&String> = states
        .values()
        .flat_map(|file| file.dispositions.keys())
        .collect();
    for finding in inventory.findings.iter().filter(|f| f.view.is_empty()) {
        if !disposed_anywhere.contains(&finding.id) {
            problems.push(format!(
                "finding {} ({}) lacks disposition (dispose it in any states/<view>.toml)",
                finding.id, finding.kind
            ));
        }
    }

    Expanded { cells, problems }
}

/// Cartesian product in axis order; a view with no axes gets `<view>/base`.
fn product(view: &str, axis_paths: &[String], columns: &[Vec<String>]) -> Vec<Cell> {
    let mut combos: Vec<Vec<String>> = vec![Vec::new()];
    for classes in columns {
        let mut next = Vec::new();
        for combo in &combos {
            for class in classes {
                let mut extended = combo.clone();
                extended.push(class.clone());
                next.push(extended);
            }
        }
        combos = next;
    }
    combos
        .into_iter()
        .map(|classes| Cell {
            view: view.to_string(),
            state_id: if classes.is_empty() {
                format!("{view}/base")
            } else {
                format!("{view}/{}", classes.join("-"))
            },
            assignment: axis_paths.iter().cloned().zip(classes).collect(),
        })
        .collect()
}

/// Write `cells.json` (atomic).
pub fn write_cells(storybook: &Path, cells: &[Cell]) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(&CellsFile {
        cells: cells.to_vec(),
    })
    .unwrap_or_default();
    crate::write_atomic(
        &storybook.join("cells.json"),
        format!("{json}\n").as_bytes(),
    )
}

/// `expand --check`: recomputed cells must equal the file byte-for-byte.
pub fn check_cells(storybook: &Path, cells: &[Cell]) -> Vec<String> {
    let path = storybook.join("cells.json");
    let expected = serde_json::to_string_pretty(&CellsFile {
        cells: cells.to_vec(),
    })
    .unwrap_or_default();
    match std::fs::read_to_string(&path) {
        Err(_) => vec!["cells.json not found (run sbk expand --write)".to_string()],
        Ok(text) if text == format!("{expected}\n") => Vec::new(),
        Ok(_) => vec!["cells.json is out of date (run sbk expand --write)".to_string()],
    }
}
