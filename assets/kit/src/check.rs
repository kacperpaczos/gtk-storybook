//! Gates behind `sbk check …` plus the template copy of `sbk scaffold`.
//!
//! * `check fixtures` — every file under `fixtures/` parses, names a cell,
//!   and its values agree with that cell's axis assignment (`references/
//!   discovery.md`: a story renders exactly its cell).
//! * `check coverage` — the storybook registry registers every view and
//!   every cell has a fixture file.
//! * `check flows` — `flows.toml` is closed: a known root, no dangling
//!   targets, every (state, action) pair covered by an edge or a
//!   `self`/`leaf` terminal, and every state reachable from the root.
//! * `scaffold` — copies the kit templates into the storybook; it never
//!   overwrites an authored file.
//!
//! All directory walks are sorted: the same inputs give the same problems
//! in the same order.

use crate::r#gen;
use crate::model::{Axis, AxisKind, Cell, Flows, Inventory};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

/// Result of a gate: how many things passed and the problems (empty = green).
pub struct Outcome {
    pub passed: usize,
    pub problems: Vec<String>,
}

// ---------------------------------------------------------------------------
// check fixtures

pub fn check_fixtures(storybook: &Path, inventory: &Inventory, cells: &[Cell]) -> Outcome {
    let mut problems = Vec::new();
    let mut passed = 0usize;
    let axes: BTreeMap<&str, &[Axis]> = inventory
        .views
        .iter()
        .map(|v| (v.id.as_str(), v.axes.as_slice()))
        .collect();
    let dir = storybook.join("fixtures");
    let Ok(view_entries) = std::fs::read_dir(&dir) else {
        // Nothing generated yet; `check coverage` reports the missing files.
        return Outcome { passed, problems };
    };
    let mut views: Vec<String> = view_entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .collect();
    views.sort();
    for view in &views {
        let Ok(file_entries) = std::fs::read_dir(dir.join(view)) else {
            continue;
        };
        let mut names: Vec<String> = file_entries
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(String::from))
            .filter(|n| n.ends_with(".json"))
            .collect();
        names.sort();
        for name in names {
            let state = name.trim_end_matches(".json");
            let rel = format!("fixtures/{view}/{name}");
            let Some(cell) = cells
                .iter()
                .find(|c| c.state_id == format!("{view}/{state}"))
            else {
                problems.push(format!("{rel} matches no cell in cells.json"));
                continue;
            };
            let text = match std::fs::read_to_string(dir.join(view).join(&name)) {
                Ok(text) => text,
                Err(err) => {
                    problems.push(format!("{rel} cannot be read: {err}"));
                    continue;
                }
            };
            let value: Value = match serde_json::from_str(&text) {
                Ok(value) => value,
                Err(err) => {
                    problems.push(format!("{rel} does not parse: {err}"));
                    continue;
                }
            };
            let fits = axes.get(view.as_str()).is_none_or(|axes| {
                axes.iter().all(|axis| {
                    let Some(class) = cell.assignment.get(&axis.path) else {
                        return true;
                    };
                    at_path(&value, &axis.path).is_some_and(|v| class_matches(&axis.kind, class, v))
                })
            });
            problems.extend(uncurated_placeholders(&value, &rel));
            if fits {
                passed += 1;
            } else {
                problems.push(format!(
                    "{rel} matches no cell (values disagree with {})",
                    cell.state_id
                ));
            }
        }
    }
    Outcome { passed, problems }
}

// ---------------------------------------------------------------------------
// uncurated placeholders + titles

/// Is `s` one of the placeholder strings `sbk gen` emits: a seeded
/// `text-<8 hex>` value, or its 32× overflow repeat? Near-misses (curated
/// strings that merely start like a seed) stay green.
pub fn is_placeholder(s: &str) -> bool {
    if let Some(hex) = s.strip_prefix("text-")
        && hex.len() == 8
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return true;
    }
    // overflow bucket: the seeded unit repeated 32 times
    const UNIT: usize = 13; // "text-" + 8 hex digits
    if s.len() == UNIT * 32 {
        return (0..32).all(|i| {
            let chunk = &s[i * UNIT..(i + 1) * UNIT];
            chunk.starts_with("text-") && is_placeholder(chunk)
        });
    }
    false
}

/// Every fixture string still equal to a generated placeholder, as
/// `uncurated placeholder at <json path>; replace with a realistic value`
/// problems. `json_path` is the fixture file; nested keys are dotted on,
/// array items `[i]`.
pub fn uncurated_placeholders(value: &Value, json_path: &str) -> Vec<String> {
    fn walk(value: &Value, path: &mut String, root: bool, out: &mut Vec<String>) {
        match value {
            Value::String(s) => {
                if is_placeholder(s) {
                    out.push(format!(
                        "uncurated placeholder at {path}; replace with a realistic value"
                    ));
                }
            }
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    let len = path.len();
                    path.push_str(&format!("[{i}]"));
                    walk(item, path, false, out);
                    path.truncate(len);
                }
            }
            Value::Object(map) => {
                for (key, value) in map {
                    let len = path.len();
                    // the first key separates from the file with ':',
                    // deeper keys are dotted on
                    path.push(if root { ':' } else { '.' });
                    path.push_str(key);
                    walk(value, path, false, out);
                    path.truncate(len);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(value, &mut json_path.to_string(), true, &mut out);
    out
}

/// `sbk check titles`: every cell carries a human title from
/// `states/<view>.toml` `[titles]`.
pub fn check_titles(cells: &[Cell], titles: &BTreeMap<String, String>) -> Outcome {
    let mut passed = 0usize;
    let mut problems = Vec::new();
    for cell in cells {
        if titles.contains_key(&cell.state_id) {
            passed += 1;
        } else {
            problems.push(format!(
                "no title for state {} (add [titles] to states/{}.toml)",
                cell.state_id, cell.view
            ));
        }
    }
    Outcome { passed, problems }
}

/// `a.b` in a JSON object tree.
fn at_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(value, |v, seg| v.get(seg))
}

/// Does `value` sit in `class` of the axis (`references/discovery.md`)?
/// Public so hosts and tests can verify the matching rule itself.
pub fn class_matches(kind: &AxisKind, class: &str, value: &Value) -> bool {
    match kind {
        AxisKind::Bool => value.as_bool().is_some_and(|b| b == (class == "true")),
        // An enum class is the variant name: a bare variant renders as a
        // string, a variant with payload as a one-key object.
        AxisKind::Enum { .. } => enum_form_matches(class, value),
        // A `Result` phase renders as serde's externally tagged form:
        // `{"Ok": …}` / `{"Err": …}`, or a bare `"Ok"` / `"Err"` string
        // when both arms are unit.
        AxisKind::Phase => match value {
            Value::Object(fields) if fields.len() == 1 => {
                let key = fields.keys().next().map_or("", |k| k.as_str());
                (class == "ok" && key == "Ok") || (class == "err" && key == "Err")
            }
            Value::String(s) => (class == "ok" && s == "Ok") || (class == "err" && s == "Err"),
            _ => false,
        },
        AxisKind::Option => {
            if class == "none" {
                value.is_null()
            } else if class == "some" {
                !value.is_null()
            } else if class == "false" || class == "true" {
                // a structurally-read bool inner contributes its classes
                value.as_bool().is_some_and(|b| (class == "true") == b)
            } else if class == "ok" || class == "err" {
                // a Result inner (Option<Result<T, E>>)
                class_matches(&AxisKind::Phase, class, value)
            } else {
                // a variant class of the inner enum: the value must carry
                // that variant, not merely be present
                !value.is_null() && enum_form_matches(class, value)
            }
        }
        // Lists and maps share the size classes: maps count their keys.
        AxisKind::Collection => item_count(value).is_some_and(|len| match class {
            "empty" => len == 0,
            "one" => len == 1,
            _ => len > 1,
        }),
        // Text, Number and Struct classes constrain the value only through
        // the skeleton (`sbk gen --check`); any value fits the cell.
        AxisKind::Text | AxisKind::Number | AxisKind::Struct => true,
    }
}

/// A bare or payload-carrying enum variant of `class`.
fn enum_form_matches(class: &str, value: &Value) -> bool {
    match value {
        Value::String(s) => s == class,
        Value::Object(fields) if fields.len() == 1 => {
            fields.keys().next().is_some_and(|k| k == class)
        }
        _ => false,
    }
}

/// Entries of a JSON array, or keys of a JSON object (a map fixture).
fn item_count(value: &Value) -> Option<usize> {
    match value {
        Value::Array(items) => Some(items.len()),
        Value::Object(map) => Some(map.len()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// check coverage

pub fn check_coverage(storybook: &Path, inventory: &Inventory, cells: &[Cell]) -> Outcome {
    let mut problems = Vec::new();
    // the registry is either a single src/registry.rs or a src/registry/
    // module directory (one module per view id)
    let registry_file = storybook.join("src/registry.rs");
    let registry_dir = storybook.join("src/registry");
    let text = if registry_file.is_file() {
        std::fs::read_to_string(&registry_file).unwrap_or_default()
    } else if registry_dir.is_dir() {
        let mut files: Vec<_> = std::fs::read_dir(&registry_dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "rs"))
            .collect();
        files.sort();
        files
            .iter()
            .map(|p| std::fs::read_to_string(p).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        problems.push(
            "src/registry.rs (or src/registry/ with one module per view) not found (run sbk scaffold, then author the registry)"
                .to_string(),
        );
        String::new()
    };
    for view in &inventory.views {
        if !text.contains(&format!("\"{}\"", view.id)) {
            problems.push(format!(
                "src/registry.rs does not register view {}",
                view.id
            ));
        }
    }
    let mut passed = 0usize;
    for cell in cells {
        if r#gen::fixture_path(storybook, cell).is_file() {
            passed += 1;
        } else {
            problems.push(format!(
                "fixtures/{}/{state}.json missing for cell {} (run sbk gen --write)",
                cell.view,
                cell.state_id,
                state = cell
                    .state_id
                    .strip_prefix(&format!("{}/", cell.view))
                    .unwrap_or(&cell.state_id)
            ));
        }
    }
    Outcome { passed, problems }
}

// ---------------------------------------------------------------------------
// check flows

/// `check flows` outcome: the parsed graph (consumed by `sbk verify`'s
/// layout) plus the problems.
pub struct FlowsOutcome {
    pub flows: Flows,
    pub problems: Vec<String>,
}

pub fn check_flows(storybook: &Path, inventory: &Inventory, cells: &[Cell]) -> FlowsOutcome {
    let empty = Flows {
        roots: Vec::new(),
        edges: Vec::new(),
        terminals: Vec::new(),
        variants: BTreeMap::new(),
    };
    let flows = match std::fs::read_to_string(storybook.join("flows.toml")) {
        Err(_) => {
            return FlowsOutcome {
                flows: empty,
                problems: vec![
                    "flows.toml not found (author roots, [[edge]] and [[terminal]])".to_string(),
                ],
            };
        }
        Ok(text) => match toml::from_str::<Flows>(&text) {
            Ok(flows) => flows,
            Err(err) => {
                return FlowsOutcome {
                    flows: empty,
                    problems: vec![format!("flows.toml does not parse: {err}")],
                };
            }
        },
    };

    let mut problems = Vec::new();
    let states: BTreeSet<&str> = cells.iter().map(|c| c.state_id.as_str()).collect();
    let view_of = |state: &str| -> Option<&str> {
        cells
            .iter()
            .find(|c| c.state_id == state)
            .map(|c| c.view.as_str())
    };
    let actions_of = |view: &str| -> &[String] {
        inventory
            .views
            .iter()
            .find(|v| v.id == view)
            .map_or(&[][..], |v| v.actions.as_slice())
    };
    if flows.roots.is_empty() {
        problems.push("flows.toml declares no roots (set roots = [\"<state>\"])".to_string());
    }

    // Flow nodes: roots, edge endpoints and terminal states (formats.md).
    let mut flow: BTreeSet<&str> = BTreeSet::new();
    for root in &flows.roots {
        if states.contains(root.as_str()) {
            flow.insert(root);
        }
    }
    for edge in &flows.edges {
        if states.contains(edge.from.as_str()) {
            flow.insert(&edge.from);
        }
        if edge.to != "back" && states.contains(edge.to.as_str()) {
            flow.insert(&edge.to);
        }
    }
    for terminal in &flows.terminals {
        if states.contains(terminal.state.as_str()) {
            flow.insert(&terminal.state);
        }
    }

    // [variants]: node -> its data variants; each variant belongs to
    // exactly one flow node of the same view and never appears in the graph.
    let mut variant_of: BTreeMap<&str, &str> = BTreeMap::new();
    for (node, list) in &flows.variants {
        if !states.contains(node.as_str()) {
            problems.push(format!("variants node {node} is not a state in cells.json"));
            continue;
        }
        if !flow.contains(node.as_str()) {
            problems.push(format!(
                "variants node {node} is not a flow node (it appears in no root, edge or terminal)"
            ));
        }
        for variant in list {
            if !states.contains(variant.as_str()) {
                problems.push(format!(
                    "variant {variant} of {node} is not a state in cells.json"
                ));
                continue;
            }
            if flow.contains(variant.as_str()) {
                problems.push(format!(
                    "variant {variant} of {node} is itself a flow node; a flow node cannot be a variant"
                ));
            }
            if let Some(&other) = variant_of.get(variant.as_str()) {
                problems.push(format!(
                    "variant {variant} is listed under both {other} and {node}"
                ));
            } else {
                variant_of.insert(variant, node);
            }
            match (view_of(variant), view_of(node)) {
                (Some(v), Some(n)) if v != n => problems.push(format!(
                    "variant {variant} of {node} belongs to view {v}, but the node belongs to {n}"
                )),
                _ => {}
            }
        }
    }

    let known_roots: Vec<&str> = flows
        .roots
        .iter()
        .filter(|root| states.contains(root.as_str()))
        .map(String::as_str)
        .collect();
    for root in &flows.roots {
        if !states.contains(root.as_str()) {
            problems.push(format!("root {root} is not a state in cells.json"));
        } else if let Some(&node) = variant_of.get(root.as_str()) {
            problems.push(format!(
                "root {root} is a variant of {node}; a variant must not appear in roots"
            ));
        }
    }

    for edge in &flows.edges {
        if let Some(&node) = variant_of.get(edge.from.as_str()) {
            problems.push(format!(
                "edge from {edge_from} (variant of {node}); a variant must not appear in edges",
                edge_from = edge.from
            ));
        }
        if edge.to != "back"
            && let Some(&node) = variant_of.get(edge.to.as_str())
        {
            problems.push(format!(
                "edge {from} --{action}--> {to}: {to} is a variant of {node}; a variant must not appear in edges",
                from = edge.from,
                action = edge.action,
                to = edge.to
            ));
        }
        if !states.contains(edge.from.as_str()) {
            problems.push(format!(
                "edge from unknown state {} (action {})",
                edge.from, edge.action
            ));
        } else if !variant_of.contains_key(edge.from.as_str()) {
            let view = edge.from.split('/').next().unwrap_or(&edge.from);
            if !actions_of(view).iter().any(|a| a == &edge.action) {
                problems.push(format!(
                    "edge from {} names action {} the view does not have",
                    edge.from, edge.action
                ));
            }
        }
        if edge.to != "back" && !states.contains(edge.to.as_str()) {
            problems.push(format!(
                "dangling edge {} --{}--> {} (no such state)",
                edge.from, edge.action, edge.to
            ));
        }
    }
    for terminal in &flows.terminals {
        if let Some(&node) = variant_of.get(terminal.state.as_str()) {
            problems.push(format!(
                "terminal for {} (variant of {node}); a variant must not appear in terminals",
                terminal.state
            ));
        }
        if !states.contains(terminal.state.as_str()) {
            problems.push(format!(
                "terminal for unknown state {} (action {})",
                terminal.state, terminal.action
            ));
        } else if !variant_of.contains_key(terminal.state.as_str()) {
            let view = terminal.state.split('/').next().unwrap_or(&terminal.state);
            if !actions_of(view).iter().any(|a| a == &terminal.action) {
                problems.push(format!(
                    "terminal {}/{} names an action the view does not have",
                    terminal.state, terminal.action
                ));
            }
        }
    }

    // Every cell is either a flow node or a variant of exactly one.
    for state in &states {
        if !flow.contains(state) && !variant_of.contains_key(state) {
            problems.push(format!(
                "state {state} is neither a flow node nor a variant (add an [[edge]]/[[terminal]] or list it under [variants])"
            ));
        }
    }

    // every (flow node, action) pair needs an edge or a self/leaf terminal;
    // variants share their node's wiring
    for cell in cells {
        if !flow.contains(cell.state_id.as_str()) {
            continue;
        }
        for action in actions_of(&cell.view) {
            let edged = flows
                .edges
                .iter()
                .any(|e| e.from == cell.state_id && e.action == *action);
            let terminated = flows
                .terminals
                .iter()
                .any(|t| t.state == cell.state_id && t.action == *action);
            if !edged && !terminated {
                problems.push(format!(
                    "missing edge: {} --{}--> ? (add an [[edge]] or a [[terminal]] with kind self/leaf)",
                    cell.state_id, action
                ));
            }
        }
    }

    // reachability of every flow node from the roots over forward edges;
    // variants are covered via their node
    if !known_roots.is_empty() {
        let mut adjacency: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for edge in &flows.edges {
            if edge.to == "back" {
                continue;
            }
            if states.contains(edge.to.as_str()) {
                adjacency
                    .entry(edge.from.as_str())
                    .or_default()
                    .push(edge.to.as_str());
            }
        }
        let mut reachable: BTreeSet<&str> = BTreeSet::new();
        let mut queue: VecDeque<&str> = VecDeque::new();
        for root in known_roots {
            if reachable.insert(root) {
                queue.push_back(root);
            }
        }
        while let Some(current) = queue.pop_front() {
            if let Some(nexts) = adjacency.get(current) {
                for next in nexts {
                    if reachable.insert(next) {
                        queue.push_back(next);
                    }
                }
            }
        }
        for state in &flow {
            if !reachable.contains(state) {
                problems.push(format!(
                    "state {state} is unreachable from the roots {}",
                    flows.roots.join(", ")
                ));
            }
        }
    }

    FlowsOutcome { flows, problems }
}

// ---------------------------------------------------------------------------
// scaffold

pub struct Scaffold {
    /// Template-relative paths copied into the storybook, in walk order.
    pub copied: Vec<String>,
    pub problems: Vec<String>,
}

/// Copy `templates` into `storybook`; existing files are never overwritten.
pub fn scaffold(storybook: &Path, templates: &Path) -> Scaffold {
    let mut outcome = Scaffold {
        copied: Vec::new(),
        problems: Vec::new(),
    };
    if !templates.is_dir() {
        outcome.problems.push(format!(
            "templates directory {} not found (the kit ships templates under assets/kit/templates)",
            templates.display()
        ));
        return outcome;
    }
    let mut plan: Vec<(String, PathBuf)> = Vec::new();
    collect_templates(templates, templates, &mut plan);
    for (rel, _src) in &plan {
        if storybook.join(rel).exists() {
            outcome.problems.push(format!(
                "{rel} exists (scaffold never overwrites; move it aside first)"
            ));
        }
    }
    if !outcome.problems.is_empty() {
        return outcome;
    }
    for (rel, src) in &plan {
        let to = storybook.join(rel);
        if let Some(parent) = to.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::copy(src, &to) {
            Ok(_) => outcome.copied.push(rel.clone()),
            Err(err) => outcome
                .problems
                .push(format!("{rel} cannot be copied: {err}")),
        }
    }
    outcome
}

/// Sorted recursive file list; dot entries (`.gitkeep`, editor droppings)
/// are not storybook files.
fn collect_templates(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut sorted: Vec<_> = entries.flatten().collect();
    sorted.sort_by_key(|e| e.file_name());
    for entry in sorted {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_templates(root, &path, out);
        } else {
            let rel = path
                .strip_prefix(root)
                .map_or_else(|_| name.clone(), |r| r.to_string_lossy().into_owned());
            out.push((rel, path));
        }
    }
}
