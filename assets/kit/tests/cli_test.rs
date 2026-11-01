//! End-to-end CLI tests: gates, exit codes, `--json`, flows, verify, scaffold.

mod common;
use common::{edit, minihost_copy, sbk, sbk_env};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// A complete green storybook around a fresh minihost copy.
fn setup(tag: &str) -> (common::HostDir, PathBuf) {
    let host = minihost_copy(tag);
    let storybook = host.join("storybook");
    let states = storybook.join("states");
    fs::create_dir_all(&states).unwrap();
    let report = storybook_kit::scan::scan(&host);
    for view in &report.inventory.views {
        let mut body = format!("view = \"{}\"\n[disposition]\n", view.id);
        for finding in report
            .inventory
            .findings
            .iter()
            .filter(|f| f.view == view.id)
        {
            body.push_str(&format!("\"{}\" = {{ excluded = \"test\" }}\n", finding.id));
        }
        fs::write(states.join(format!("{}.toml", view.id)), body).unwrap();
    }
    // every cell gets a human title (`sbk check titles`; verify is strict)
    let states_map: BTreeMap<String, storybook_kit::model::StatesFile> = report
        .inventory
        .views
        .iter()
        .map(|v| {
            let text = fs::read_to_string(states.join(format!("{}.toml", v.id))).unwrap();
            (v.id.clone(), toml::from_str(&text).unwrap())
        })
        .collect();
    let expanded = storybook_kit::expand::expand(&report.inventory, &states_map);
    let mut titles: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for cell in &expanded.cells {
        titles
            .entry(cell.view.as_str())
            .or_default()
            .push(cell.state_id.as_str());
    }
    for (view, ids) in titles {
        let mut body = String::from("\n[titles]\n");
        for id in ids {
            body.push_str(&format!("\"{id}\" = \"Stan {id}\"\n"));
        }
        let file = states.join(format!("{view}.toml"));
        let text = fs::read_to_string(&file).unwrap();
        fs::write(file, format!("{text}{body}")).unwrap();
    }
    fs::create_dir_all(storybook.join("src")).unwrap();
    let mut registry = String::from("pub fn registry() -> Vec<(&str, StoryFn)> {\n    vec![\n");
    for view in &report.inventory.views {
        registry.push_str(&format!("        (\"{id}\", todo!()),\n", id = view.id));
    }
    registry.push_str("    ]\n}\n");
    fs::write(storybook.join("src/registry.rs"), registry).unwrap();
    write_flows(&storybook, &report);
    (host, storybook)
}

/// Chain every cell from the first; terminals cover the remaining pairs.
fn write_flows(storybook: &Path, report: &storybook_kit::scan::ScanReport) {
    use std::collections::BTreeMap;
    let states_dir = storybook.join("states");
    let states: BTreeMap<String, _> = report
        .inventory
        .views
        .iter()
        .map(|v| {
            let text = fs::read_to_string(states_dir.join(format!("{}.toml", v.id))).unwrap();
            (
                v.id.clone(),
                toml::from_str::<storybook_kit::model::StatesFile>(&text).unwrap(),
            )
        })
        .collect();
    let expanded = storybook_kit::expand::expand(&report.inventory, &states);
    let cells = &expanded.cells;
    let actions = |view: &str| -> Vec<String> {
        report
            .inventory
            .views
            .iter()
            .find(|v| v.id == view)
            .map(|v| v.actions.clone())
            .unwrap_or_default()
    };
    let mut toml = format!("roots = [\"{}\"]\n", cells[0].state_id);
    for i in 0..cells.len() - 1 {
        let from = &cells[i];
        let to = &cells[i + 1];
        toml.push_str(&format!(
            "\n[[edge]]\nfrom = \"{f}\"\naction = \"{a}\"\nto = \"{t}\"\n",
            f = from.state_id,
            a = actions(&from.view)[0],
            t = to.state_id
        ));
    }
    // one back edge from the last settings cell
    if let Some(back_from) = cells.iter().find(|c| c.view == "settings") {
        toml.push_str(&format!(
            "\n[[edge]]\nfrom = \"{f}\"\naction = \"{a}\"\nto = \"back\"\n",
            f = back_from.state_id,
            a = actions("settings")[1]
        ));
    }
    for cell in cells {
        for action in &actions(&cell.view) {
            let last = cell.state_id == cells.last().unwrap().state_id
                && Some(action.as_str()) == actions(&cell.view).last().map(String::as_str);
            let covered = toml.contains(&format!(
                "from = \"{}\"\naction = \"{action}\"",
                cell.state_id
            ));
            if covered {
                continue;
            }
            toml.push_str(&format!(
                "\n[[terminal]]\nstate = \"{s}\"\naction = \"{action}\"\nkind = \"{k}\"\n",
                s = cell.state_id,
                k = if last { "leaf" } else { "self" }
            ));
        }
    }
    fs::write(storybook.join("flows.toml"), toml).unwrap();
}

fn green(args: &[&str]) -> Value {
    let (code, out, err) = sbk(args);
    assert_eq!(code, 0, "expected green, stderr:\n{err}\nstdout:\n{out}");
    serde_json::from_str(&out).unwrap_or(Value::Null)
}

fn red(args: &[&str]) -> (Value, String) {
    let (code, out, err) = sbk(args);
    assert_eq!(code, 1, "expected red, stderr:\n{err}\nstdout:\n{out}");
    (serde_json::from_str(&out).unwrap_or(Value::Null), out)
}

/// Replace every generated placeholder string in every fixture with a
/// realistic-looking value — the author-side half of `sbk check fixtures`.
fn curate(storybook: &Path) {
    fn scrub(value: &mut Value) {
        match value {
            Value::String(s) => {
                if storybook_kit::check::is_placeholder(s) {
                    *s = "realistic value".to_string();
                }
            }
            Value::Array(items) => items.iter_mut().for_each(scrub),
            Value::Object(map) => map.iter_mut().for_each(|(_, v)| scrub(v)),
            _ => {}
        }
    }
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let mut entries: Vec<_> = fs::read_dir(dir).unwrap().flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            if entry.file_type().unwrap().is_dir() {
                walk(&entry.path(), out);
            } else if entry.path().extension().is_some_and(|e| e == "json") {
                out.push(entry.path());
            }
        }
    }
    let mut files = Vec::new();
    walk(&storybook.join("fixtures"), &mut files);
    for file in files {
        let mut value: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        scrub(&mut value);
        let text = serde_json::to_string_pretty(&value).unwrap();
        fs::write(&file, format!("{text}\n")).unwrap();
    }
}

#[test]
fn probe_reports_tier_json() {
    let (host, _) = setup("cli-probe");
    let json = green(&["probe", "--workspace", host.to_str().unwrap(), "--json"]);
    assert_eq!(json["tier"], "a");
    assert_eq!(json["views"].as_array().unwrap().len(), 10);
}

#[test]
fn contract_gate_green_then_red() {
    let (host, _) = setup("cli-contract");
    let ws = host.to_str().unwrap();
    green(&["contract", "--workspace", ws, "--check", "--json"]);
    edit(
        &host.join("src/create.rs"),
        "#[derive(Clone, PartialEq, Serialize, Deserialize)]\npub struct CreateVm",
        "#[derive(Clone)]\npub struct CreateVm",
    );
    let (json, _) = red(&["contract", "--workspace", ws, "--check", "--json"]);
    assert_eq!(json["ok"], false);
    assert!(
        json["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().contains("Serialize")),
        "{json}"
    );
}

#[test]
fn scan_write_then_check_detects_drift() {
    let (host, storybook) = setup("cli-scan");
    let ws = host.to_str().unwrap();
    let (json, _) = red(&["scan", "--workspace", ws, "--check", "--json"]);
    assert!(
        json["problems"].as_array().unwrap()[0]
            .as_str()
            .unwrap()
            .contains("inventory.json")
    );
    green(&["scan", "--workspace", ws, "--write", "--json"]);
    assert!(storybook.join("inventory.json").is_file());
    green(&["scan", "--workspace", ws, "--check", "--json"]);
    edit(&host.join("src/skill_detail.rs"), "0u32", "1u32");
    red(&["scan", "--workspace", ws, "--check", "--json"]);
}

#[test]
fn mode_flags_are_required_exactly_once() {
    let (host, _) = setup("cli-mode");
    let ws = host.to_str().unwrap();
    let (json, _) = red(&["scan", "--workspace", ws, "--json"]);
    assert!(
        json["problems"].as_array().unwrap()[0]
            .as_str()
            .unwrap()
            .contains("--write")
    );
    let (_, out) = red(&["scan", "--workspace", ws, "--write", "--check", "--json"]);
    assert!(out.contains("--write"));
}

#[test]
fn expand_gen_and_check_fixtures_gates() {
    let (host, storybook) = setup("cli-expand");
    let ws = host.to_str().unwrap();
    green(&["scan", "--workspace", ws, "--write", "--json"]);
    green(&["expand", "--workspace", ws, "--write", "--json"]);
    assert!(storybook.join("cells.json").is_file());
    // cells.json drift is red until regenerated
    let cells_file = storybook.join("cells.json");
    let cells_text = fs::read_to_string(&cells_file).unwrap();
    fs::write(
        &cells_file,
        cells_text.replace("skill_detail/any-false-All", "skill_detail/tampered"),
    )
    .unwrap();
    let (json, _) = red(&["expand", "--workspace", ws, "--check", "--json"]);
    assert!(
        json["problems"].as_array().unwrap()[0]
            .as_str()
            .unwrap()
            .contains("out of date")
    );
    fs::write(&cells_file, &cells_text).unwrap();
    let json = green(&["gen", "--workspace", ws, "--write", "--json"]);
    assert_eq!(json["written"].as_u64().unwrap(), 132);
    let json = green(&["gen", "--workspace", ws, "--check", "--json"]);
    assert_eq!(json["fixtures"].as_u64().unwrap(), 132);

    // uncurated placeholders are red until every generated string is replaced
    let (json, _) = red(&["check", "--workspace", ws, "fixtures", "--json"]);
    assert!(
        json["problems"].as_array().unwrap().iter().any(|p| p
            .as_str()
            .unwrap()
            .starts_with("uncurated placeholder at fixtures/skill_detail/any-false-All.json:")),
        "{json}"
    );
    curate(&storybook);

    // curation: values may change, keys may not
    let fixture = storybook.join("fixtures/skill_detail/any-false-All.json");
    let text = fs::read_to_string(&fixture).unwrap();
    fs::write(
        &fixture,
        text.replace(
            "\"name\": \"realistic value\"",
            "\"name\": \"curated-name\"",
        ),
    )
    .unwrap();
    green(&["gen", "--workspace", ws, "--check", "--json"]);
    let text = fs::read_to_string(&fixture).unwrap();
    let curated = text.replace("\"dirty\": false", "\"dirty\": true");
    fs::write(&fixture, &curated).unwrap();
    green(&["gen", "--workspace", ws, "--check", "--json"]);
    let curated = curated.replace("\"filter\": \"All\",\n", "");
    fs::write(&fixture, &curated).unwrap();
    red(&["gen", "--workspace", ws, "--check", "--json"]);
    fs::write(&fixture, &text).unwrap();

    let json = green(&["check", "--workspace", ws, "fixtures", "--json"]);
    assert_eq!(json["passed"].as_u64().unwrap(), 132);
    // corrupt fixture: invalid JSON naming the file
    fs::write(&fixture, "{ nope").unwrap();
    let (json, _) = red(&["check", "--workspace", ws, "fixtures", "--json"]);
    assert!(
        json["problems"].as_array().unwrap()[0]
            .as_str()
            .unwrap()
            .contains("any-false-All")
    );
    // fixture for an unknown state
    fs::write(&fixture, "{}\n").unwrap();
    let (json, _) = red(&["check", "--workspace", ws, "fixtures", "--json"]);
    assert!(
        json["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().contains("matches no cell")),
        "{json}"
    );
    // all keys present but values from a different cell: still no match
    let swapped = text.replace("\"dirty\": false", "\"dirty\": true");
    assert_ne!(&swapped, &text);
    fs::write(&fixture, &swapped).unwrap();
    let (json, _) = red(&["check", "--workspace", ws, "fixtures", "--json"]);
    assert!(
        json["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().contains("matches no cell")),
        "{json}"
    );
    fs::write(&fixture, &text).unwrap();
    green(&["check", "--workspace", ws, "fixtures", "--json"]);
}

#[test]
fn coverage_gate_needs_registry_and_fixtures() {
    let (host, storybook) = setup("cli-cov");
    let ws = host.to_str().unwrap();
    green(&["scan", "--workspace", ws, "--write", "--json"]);
    green(&["expand", "--workspace", ws, "--write", "--json"]);
    green(&["gen", "--workspace", ws, "--write", "--json"]);
    fs::remove_file(storybook.join("src/registry.rs")).unwrap();
    let (json, _) = red(&["check", "--workspace", ws, "coverage", "--json"]);
    assert!(
        json["problems"].as_array().unwrap()[0]
            .as_str()
            .unwrap()
            .contains("registry.rs")
    );
    let registry = "pub fn registry() -> Vec<(&str, usize)> { vec![(\"skill_detail\", 0), (\"settings\", 0)] }";
    fs::write(storybook.join("src/registry.rs"), registry).unwrap();
    let (json, _) = red(&["check", "--workspace", ws, "coverage", "--json"]);
    assert!(
        json["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().contains("create")),
        "{json}"
    );
    fs::write(
        storybook.join("src/registry.rs"),
        "pub fn registry() -> Vec<(&str, usize)> { vec![(\"deep\", 0), (\"corners\", 0), (\"pairy\", 0), (\"pm\", 0), (\"pf\", 0), (\"pv\", 0), (\"alt\", 0), (\"create\", 0), (\"settings\", 0), (\"skill_detail\", 0)] }",
    )
    .unwrap();
    let json = green(&["check", "--workspace", ws, "coverage", "--json"]);
    assert_eq!(json["passed"].as_u64().unwrap(), 132);
    fs::remove_file(storybook.join("fixtures/create/Idle-empty-none-any.json")).unwrap();
    let (json, _) = red(&["check", "--workspace", ws, "coverage", "--json"]);
    assert!(
        json["problems"].as_array().unwrap()[0]
            .as_str()
            .unwrap()
            .contains("Idle-empty-none-any")
    );
}

#[test]
fn flows_gate_covers_missing_dangling_and_unreachable() {
    let (host, storybook) = setup("cli-flows");
    let ws = host.to_str().unwrap();
    green(&["scan", "--workspace", ws, "--write", "--json"]);
    green(&["expand", "--workspace", ws, "--write", "--json"]);
    let json = green(&["check", "--workspace", ws, "flows", "--json"]);
    assert_eq!(json["passed"].as_u64().unwrap(), 168); // 132 edges + 36 terminals

    let flows = storybook.join("flows.toml");
    let good = fs::read_to_string(&flows).unwrap();

    // missing edge: drop one terminal
    let missing = good.replace(&good[good.find("[[terminal]]").unwrap()..].to_string(), "");
    fs::write(&flows, &missing).unwrap();
    let (json, _) = red(&["check", "--workspace", ws, "flows", "--json"]);
    assert!(
        json["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().contains("missing edge")),
        "{json}"
    );

    // dangling target
    let dangling = good.replace("to = \"back\"", "to = \"nowhere/state\"");
    fs::write(&flows, &dangling).unwrap();
    let (json, _) = red(&["check", "--workspace", ws, "flows", "--json"]);
    assert!(
        json["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().contains("dangling")),
        "{json}"
    );

    // unreachable: delete the second edge block, cutting the chain
    let second = good.find("[[edge]]").unwrap();
    let third = good[second..]
        .find("\n[[edge]]")
        .map(|i| second + i)
        .unwrap();
    let fourth = good[third..].find("\n\n").map(|i| third + i).unwrap();
    let broken = format!("{}{}", &good[..third], &good[fourth..]);
    fs::write(&flows, broken).unwrap();
    let (json, _) = red(&["check", "--workspace", ws, "flows", "--json"]);
    assert!(
        json["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().contains("unreachable")),
        "{json}"
    );

    // drop only the first edge: its (state, action) pair loses coverage even
    // though the state keeps a terminal of another action
    let first_edge = good.find("[[edge]]").unwrap();
    let second_edge = good[first_edge..]
        .find("\n\n[[edge]]")
        .map(|i| first_edge + i)
        .unwrap();
    let no_first_edge = format!("{}{}", &good[..first_edge], &good[second_edge..]);
    fs::write(&flows, &no_first_edge).unwrap();
    let (json, _) = red(&["check", "--workspace", ws, "flows", "--json"]);
    assert!(
        json["problems"].as_array().unwrap().iter().any(|p| p
            .as_str()
            .unwrap()
            .contains("missing edge: deep/any --on_leave")),
        "{json}"
    );
    // drop only the first terminal: its pair must lose coverage even though
    // the state still has an edge of another action
    let first_term = good.find("[[terminal]]").unwrap();
    let after_first_term = good[first_term..]
        .find("\n[[terminal]]")
        .map(|i| first_term + i)
        .unwrap();
    let no_first_term = format!("{}{}", &good[..first_term], &good[after_first_term..]);
    fs::write(&flows, &no_first_term).unwrap();
    let (json, _) = red(&["check", "--workspace", ws, "flows", "--json"]);
    assert!(
        json["problems"].as_array().unwrap().iter().any(|p| p
            .as_str()
            .unwrap()
            .contains("missing edge: create/Idle-empty-none-any --on_pick_dir")),
        "{json}"
    );

    let root_state = good
        .lines()
        .find(|l| l.starts_with("roots"))
        .and_then(|l| l.split('"').nth(1))
        .unwrap_or("create/Idle-empty-none-any");
    let bogus_action = format!(
        "{good}\n[[terminal]]\nstate = \"{root_state}\"\naction = \"no-such-action\"\nkind = \"self\"\n"
    );
    fs::write(&flows, &bogus_action).unwrap();
    let (json, _) = red(&["check", "--workspace", ws, "flows", "--json"]);
    assert!(
        json["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().contains("no-such-action")),
        "{json}"
    );

    let bogus_state = format!(
        "{good}\n[[terminal]]\nstate = \"no-view/base\"\naction = \"on_save\"\nkind = \"leaf\"\n"
    );
    fs::write(&flows, &bogus_state).unwrap();
    let (json, _) = red(&["check", "--workspace", ws, "flows", "--json"]);
    assert!(
        json["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().contains("no-view/base")),
        "{json}"
    );

    fs::write(&flows, &good).unwrap();
    green(&["check", "--workspace", ws, "flows", "--json"]);
}

#[test]
fn flows_gate_accepts_multiple_roots_and_validates_edge_actions() {
    let (host, storybook) = setup("cli-flows-roots");
    let ws = host.to_str().unwrap();
    green(&["scan", "--workspace", ws, "--write", "--json"]);
    green(&["expand", "--workspace", ws, "--write", "--json"]);

    let flows = storybook.join("flows.toml");
    let good = fs::read_to_string(&flows).unwrap();
    let first_root = good.split('"').nth(1).unwrap().to_string();
    let last_state = good
        .lines()
        .rfind(|l| l.starts_with("from = "))
        .and_then(|l| l.split('"').nth(1))
        .unwrap()
        .to_string();

    // two roots: everything stays reachable, the gate stays green
    let two_roots = good.replace(
        good.lines().next().unwrap(),
        &format!("roots = [\"{first_root}\", \"{last_state}\"]"),
    );
    fs::write(&flows, &two_roots).unwrap();
    green(&["check", "--workspace", ws, "flows", "--json"]);

    // no roots at all is red
    let no_roots = two_roots.replace(two_roots.lines().next().unwrap(), "roots = []");
    fs::write(&flows, &no_roots).unwrap();
    let (json, _) = red(&["check", "--workspace", ws, "flows", "--json"]);
    assert!(
        json["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().contains("no roots")),
        "{json}"
    );

    // an edge whose action the view does not have is red
    let bad_action = format!(
        "{two_roots}\n[[edge]]\nfrom = \"{last_state}\"\naction = \"not-an-action\"\nto = \"{last_state}\"\n"
    );
    fs::write(&flows, &bad_action).unwrap();
    let (json, _) = red(&["check", "--workspace", ws, "flows", "--json"]);
    assert!(
        json["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().contains("not-an-action")),
        "{json}"
    );
    fs::write(&flows, &good).unwrap();
}

#[test]
fn verify_runs_every_gate_and_writes_layout() {
    let (host, storybook) = setup("cli-verify");
    let ws = host.to_str().unwrap();
    green(&["scan", "--workspace", ws, "--write", "--json"]);
    green(&["expand", "--workspace", ws, "--write", "--json"]);
    green(&["gen", "--workspace", ws, "--write", "--json"]);
    curate(&storybook);
    let json = green(&["verify", "--workspace", ws, "--json"]);
    assert_eq!(json["layout"]["nodes"].as_u64().unwrap(), 132);
    let layout_text = fs::read_to_string(storybook.join("layout.json")).unwrap();
    let layout: storybook_kit::model::Layout = serde_json::from_str(&layout_text).unwrap();
    assert_eq!(layout.nodes.len(), 132);
    // titles from states/<view>.toml land in layout nodes
    assert!(
        layout.nodes.iter().all(|n| n.title.is_some()),
        "{:?}",
        layout
            .nodes
            .iter()
            .map(|n| n.state_id.clone())
            .take(3)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        node_title(&layout, "skill_detail/any-false-All"),
        Some("Stan skill_detail/any-false-All".to_string())
    );
    assert!(!layout.edges.is_empty());

    // a red gate keeps verify red and layout untouched
    let before = fs::read_to_string(storybook.join("layout.json")).unwrap();
    fs::remove_file(storybook.join("fixtures/skill_detail/any-false-All.json")).unwrap();
    let (json, _) = red(&["verify", "--workspace", ws, "--json"]);
    assert_eq!(json["ok"], false);
    assert_eq!(
        fs::read_to_string(storybook.join("layout.json")).unwrap(),
        before
    );
}

fn node_title(l: &storybook_kit::model::Layout, id: &str) -> Option<String> {
    l.nodes
        .iter()
        .find(|n| n.state_id == id)
        .and_then(|n| n.title.clone())
}

#[test]
fn scaffold_copies_templates_without_overwriting() {
    let (host, storybook) = setup("cli-scaffold");
    let ws = host.to_str().unwrap();
    let templates = storybook.join("kit-templates-for-test");
    fs::create_dir_all(templates.join("src")).unwrap();
    fs::write(templates.join("Cargo.toml"), "# template\n").unwrap();
    fs::write(templates.join("src/main.rs"), "// template\n").unwrap();
    fs::write(templates.join(".gitkeep"), "").unwrap();

    let json = green(&[
        "scaffold",
        "--workspace",
        ws,
        "--templates",
        templates.to_str().unwrap(),
        "--json",
    ]);
    assert!(storybook.join("Cargo.toml").is_file());
    assert!(storybook.join("src/main.rs").is_file());
    assert!(!storybook.join(".gitkeep").exists());
    assert_eq!(json["copied"].as_array().unwrap().len(), 2);

    fs::write(templates.join("src/main.rs"), "// changed template\n").unwrap();
    let (json, _) = red(&[
        "scaffold",
        "--workspace",
        ws,
        "--templates",
        templates.to_str().unwrap(),
        "--json",
    ]);
    assert!(
        json["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().contains("exists")),
        "{json}"
    );
    assert_eq!(
        fs::read_to_string(storybook.join("src/main.rs")).unwrap(),
        "// template\n"
    );

    let missing = storybook.join("no-such-templates");
    let (json, _) = red(&[
        "scaffold",
        "--workspace",
        ws,
        "--templates",
        missing.to_str().unwrap(),
        "--json",
    ]);
    assert!(
        json["problems"].as_array().unwrap()[0]
            .as_str()
            .unwrap()
            .contains("templates")
    );
}
#[test]
fn text_mode_lists_problems_on_stdout() {
    let (host, storybook) = setup("cli-text");
    let ws = host.to_str().unwrap();
    let (code, out, _) = sbk(&["contract", "--workspace", ws, "--check"]);
    assert_eq!(code, 0);
    assert!(out.contains("tier: a"), "{out}");
    sbk(&["scan", "--workspace", ws, "--write"]);
    sbk(&["expand", "--workspace", ws, "--write"]);
    let (code, _, _) = sbk(&["check", "--workspace", ws, "flows"]);
    assert_eq!(code, 0);
    edit(
        &host.join("src/create.rs"),
        "pub struct CreateVm",
        "struct CreateVm",
    );
    let (code, out, _) = sbk(&["contract", "--workspace", ws, "--check"]);
    assert_eq!(code, 1);
    assert!(out.contains("- view create: vm not pub"), "{out}");

    // without --check, contract reports the tier without gating
    let (code, out, _) = sbk(&["contract", "--workspace", ws]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("tier:"), "{out}");
    assert!(out.contains("vm not pub"), "{out}");
    let _ = storybook;
}

/// `check` gates read inventory.json + cells.json like `verify` does: a
/// missing or stale prerequisite is a red gate naming the file, and states
/// problems are never swallowed.
#[test]
fn check_gates_read_the_written_files_and_report_states_problems() {
    let (host, storybook) = setup("cli-check-files");
    let ws = host.to_str().unwrap();
    green(&["scan", "--workspace", ws, "--write", "--json"]);
    green(&["expand", "--workspace", ws, "--write", "--json"]);
    green(&["gen", "--workspace", ws, "--write", "--json"]);
    curate(&storybook);

    // a stale inventory is red: the host changed after scan --write
    edit(&host.join("src/skill_detail.rs"), "0u32", "1u32");
    let (json, _) = red(&["check", "--workspace", ws, "fixtures", "--json"]);
    assert!(
        json["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().contains("out of date")),
        "{json}"
    );
    green(&["scan", "--workspace", ws, "--write", "--json"]);

    // cells.json is a prerequisite of every check gate
    let cells = fs::read_to_string(storybook.join("cells.json")).unwrap();
    fs::remove_file(storybook.join("cells.json")).unwrap();
    let (json, _) = red(&["check", "--workspace", ws, "coverage", "--json"]);
    assert!(
        json["problems"].as_array().unwrap()[0]
            .as_str()
            .unwrap()
            .contains("cells.json"),
        "{json}"
    );
    fs::write(storybook.join("cells.json"), &cells).unwrap();

    // broken states files surface in check, not only in expand/verify
    let states_file = storybook.join("states/skill_detail.toml");
    let states = fs::read_to_string(&states_file).unwrap();
    fs::write(&states_file, "not toml = = =\n").unwrap();
    let (json, _) = red(&["check", "--workspace", ws, "fixtures", "--json"]);
    assert!(
        json["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().contains("states/skill_detail.toml")),
        "{json}"
    );
    fs::write(&states_file, &states).unwrap();
    green(&["check", "--workspace", ws, "fixtures", "--json"]);
}

/// `gen --write` never clobbers curated fixtures; `--force` rewrites them.
#[test]
fn gen_write_keeps_existing_fixtures_until_forced() {
    let (host, storybook) = setup("cli-gen-force");
    let ws = host.to_str().unwrap();
    green(&["scan", "--workspace", ws, "--write", "--json"]);
    green(&["expand", "--workspace", ws, "--write", "--json"]);
    let (code, out, err) = sbk(&["gen", "--workspace", ws, "--write"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out.trim(), "wrote 132 fixtures", "{out}");
    let (code, out, err) = sbk(&["gen", "--workspace", ws, "--write", "--json"]);
    assert_eq!(code, 0, "{err}");
    let json: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["written"].as_u64().unwrap(), 0);
    assert_eq!(json["skipped"].as_u64().unwrap(), 132);

    let fixture = storybook.join("fixtures/skill_detail/any-false-All.json");
    let generated = fs::read_to_string(&fixture).unwrap();
    fs::write(&fixture, generated.replace("text-", "curated-")).unwrap();

    // a second --write reports everything as kept, nothing overwritten
    let (code, out, err) = sbk(&["gen", "--workspace", ws, "--write"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out.trim(),
        "wrote 0 fixtures (132 existing kept; --force rewrites)",
        "{out}"
    );
    let (code, out, err) = sbk(&["gen", "--workspace", ws, "--write", "--json"]);
    assert_eq!(code, 0, "{err}");
    let json: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["written"].as_u64().unwrap(), 0);
    assert_eq!(json["skipped"].as_u64().unwrap(), 132);
    assert!(fs::read_to_string(&fixture).unwrap().contains("curated-"));

    // --force restores the skeleton
    let json = green(&["gen", "--workspace", ws, "--write", "--force", "--json"]);
    assert_eq!(json["written"].as_u64().unwrap(), 132);
    assert!(fs::read_to_string(&fixture).unwrap().contains("text-"));
}

#[test]
fn coverage_accepts_a_registry_module_directory() {
    let (host, storybook) = setup("cli-cov-dir");
    let ws = host.to_str().unwrap();
    green(&["scan", "--workspace", ws, "--write", "--json"]);
    green(&["expand", "--workspace", ws, "--write", "--json"]);
    green(&["gen", "--workspace", ws, "--write", "--json"]);
    let reg = fs::read_to_string(storybook.join("src/registry.rs")).unwrap();
    fs::remove_file(storybook.join("src/registry.rs")).unwrap();
    let dir = storybook.join("src/registry");
    fs::create_dir_all(&dir).unwrap();
    // one module per view id (a registry.rs split into src/registry/)
    let view_line = |id: &str| format!("        (\"{id}\", todo!()),\n");
    let ids: Vec<String> = reg
        .lines()
        .filter_map(|l| {
            l.trim()
                .strip_prefix("(\"")
                .and_then(|r| r.split("\",").next().map(|id| id.trim().to_string()))
        })
        .collect();
    for id in &ids {
        fs::write(
            dir.join(format!("{id}.rs")),
            format!("pub const ID: &str = \"{id}\";\n"),
        )
        .unwrap();
    }
    let mut mod_rs = String::from("pub fn registry() -> Vec<(&str, usize)> {\n    vec![\n");
    for id in &ids {
        mod_rs.push_str(&view_line(id));
    }
    mod_rs.push_str("    ]\n}\n");
    fs::write(dir.join("mod.rs"), &mod_rs).unwrap();
    let json = green(&["check", "--workspace", ws, "coverage", "--json"]);
    assert_eq!(json["passed"].as_u64().unwrap(), 132);
    // a view missing from the directory is still caught
    let settings = ids.iter().find(|i| i.as_str() == "settings").unwrap();
    let pruned = mod_rs.replace(&view_line(settings), "");
    fs::remove_file(dir.join(format!("{settings}.rs"))).unwrap();
    fs::write(dir.join("mod.rs"), &pruned).unwrap();
    let (json, _) = red(&["check", "--workspace", ws, "coverage", "--json"]);
    assert!(
        json["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().contains("settings")),
        "{json}"
    );
}

/// `check titles` is warning-level unless `--strict`; `verify` is strict.
#[test]
fn titles_gate_warns_by_default_and_gates_on_strict() {
    let (host, storybook) = setup("cli-titles");
    let ws = host.to_str().unwrap();
    green(&["scan", "--workspace", ws, "--write", "--json"]);
    green(&["expand", "--workspace", ws, "--write", "--json"]);

    // every cell titled (setup wrote [titles]): green either way
    let json = green(&["check", "--workspace", ws, "titles", "--json"]);
    assert_eq!(json["passed"].as_u64().unwrap(), 132);
    green(&["check", "--workspace", ws, "titles", "--strict", "--json"]);

    // drop one title: a warning by default (exit 0), red with --strict
    let states_file = storybook.join("states/skill_detail.toml");
    let states = fs::read_to_string(&states_file).unwrap();
    let line = states
        .lines()
        .find(|l| l.contains("\"skill_detail/any-false-All\""))
        .unwrap()
        .to_string();
    fs::write(&states_file, states.replace(&line, "")).unwrap();
    let (code, out, err) = sbk(&["check", "--workspace", ws, "titles", "--json"]);
    assert_eq!(
        code, 0,
        "warning-level gate stays green\nstderr:\n{err}\nstdout:\n{out}"
    );
    let json: Value = serde_json::from_str(&out).unwrap();
    assert!(
        json["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("skill_detail/any-false-All")),
        "{json}"
    );
    assert_eq!(json["passed"].as_u64().unwrap(), 131);
    let (json, _) = red(&["check", "--workspace", ws, "titles", "--strict", "--json"]);
    assert!(
        json["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().contains("skill_detail/any-false-All")),
        "{json}"
    );

    // verify is strict: the missing title keeps it red
    green(&["gen", "--workspace", ws, "--write", "--json"]);
    curate(&storybook);
    let (json, _) = red(&["verify", "--workspace", ws, "--json"]);
    assert!(
        json["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().contains("skill_detail/any-false-All")),
        "{json}"
    );
    fs::write(&states_file, states).unwrap();
    green(&["verify", "--workspace", ws, "--json"]);
}

#[test]
fn verify_falls_back_to_builtin_layout_without_dot() {
    let (host, storybook) = setup("cli-dot-fallback");
    let ws = host.to_str().unwrap();
    green(&["scan", "--workspace", ws, "--write", "--json"]);
    green(&["expand", "--workspace", ws, "--write", "--json"]);
    green(&["gen", "--workspace", ws, "--write", "--json"]);
    curate(&storybook);

    // SBK_DOT=off: builtin engine, a warning line, no stored geometry
    let (code, out, err) = sbk_env(&["verify", "--workspace", ws], &[("SBK_DOT", "off")]);
    assert_eq!(code, 0, "{out} {err}");
    assert!(err.contains("warning"), "{err}");
    let layout: storybook_kit::model::Layout =
        serde_json::from_str(&fs::read_to_string(storybook.join("layout.json")).unwrap()).unwrap();
    assert_eq!(layout.engine, "builtin");
    assert!(layout.nodes.iter().all(|n| n.rect.is_none()));
    assert!(layout.clusters.is_empty());

    // a broken SBK_DOT path: dot fails, verify still goes green on builtin
    let (code, out, err) = sbk_env(
        &["verify", "--workspace", ws],
        &[("SBK_DOT", "/nonexistent/dot-for-sbk-test")],
    );
    assert_eq!(code, 0, "{out} {err}");
    assert!(err.contains("warning"), "{err}");
    let layout: storybook_kit::model::Layout =
        serde_json::from_str(&fs::read_to_string(storybook.join("layout.json")).unwrap()).unwrap();
    assert_eq!(layout.engine, "builtin");
}
