//! `sbk` — the storybook kit CLI.
//!
//! Every command is a gate: exit 0 green, exit 1 red with a problem list
//! (human text on stdout, or JSON everywhere with `--json`). The workflow
//! the commands serve is documented in the skill's SKILL.md.

use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use storybook_kit::check;
use storybook_kit::dot;
use storybook_kit::expand;
use storybook_kit::r#gen;
use storybook_kit::layout;
use storybook_kit::model::{Cell, CellsFile, Flows, Inventory};
use storybook_kit::scan;
use storybook_kit::{probe, write_atomic};

#[derive(Parser)]
#[command(
    name = "sbk",
    about = "storybook kit: prove every state of every view is covered"
)]
struct Cli {
    /// Host workspace root to scan.
    #[arg(long, global = true, default_value = ".")]
    workspace: PathBuf,
    /// Storybook directory (default: <workspace>/storybook).
    #[arg(long, global = true)]
    storybook: Option<PathBuf>,
    /// Machine-readable JSON output.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Report the host's contract tier and view census.
    Probe,
    /// Gate: the host must meet the contract (overall tier A).
    /// With `--check` the command gates (exit 1 on problems); without it,
    /// it reports the tier and problems and stays green.
    Contract {
        /// Check the host against the contract.
        #[arg(long)]
        check: bool,
    },
    /// Write or check `inventory.json` (exactly one of --write/--check).
    Scan {
        #[arg(long)]
        write: bool,
        #[arg(long)]
        check: bool,
    },
    /// Write or check `cells.json` from states/<view>.toml.
    Expand {
        #[arg(long)]
        write: bool,
        #[arg(long)]
        check: bool,
    },
    /// Write or check the fixture skeletons under fixtures/.
    /// `--write` only fills missing fixtures; `--force` rewrites existing
    /// ones (curated values are never clobbered by accident).
    Gen {
        #[arg(long)]
        write: bool,
        #[arg(long)]
        check: bool,
        #[arg(long)]
        force: bool,
    },
    /// Run one gate: fixtures, coverage or flows.
    Check {
        gate: Gate,
        /// Gate on warnings too (the titles gate warns by default).
        #[arg(long)]
        strict: bool,
    },
    /// Run every gate; on green also write `layout.json`.
    Verify,
    /// Copy the kit templates into the storybook (never overwrites).
    Scaffold {
        /// Templates directory (default: <storybook>/kit/templates).
        #[arg(long)]
        templates: Option<PathBuf>,
    },
}

#[derive(clap::ValueEnum, Clone, Copy)]
enum Gate {
    Fixtures,
    Coverage,
    Flows,
    Titles,
}
fn main() -> ExitCode {
    let cli = Cli::parse();
    let storybook = cli
        .storybook
        .clone()
        .unwrap_or_else(|| cli.workspace.join("storybook"));
    if run(&cli.command, &cli.workspace, &storybook, cli.json) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn run(command: &Command, workspace: &Path, storybook: &Path, json: bool) -> bool {
    match command {
        Command::Probe => cmd_probe(workspace, json),
        Command::Contract { check } => cmd_contract(workspace, *check, json),
        Command::Scan { write, check } => cmd_scan(workspace, storybook, *write, *check, json),
        Command::Expand { write, check } => cmd_expand(storybook, *write, *check, json),
        Command::Gen {
            write,
            check,
            force,
        } => cmd_gen(workspace, storybook, *write, *check, *force, json),
        Command::Check { gate, strict } => cmd_check(workspace, storybook, *gate, *strict, json),
        Command::Verify => cmd_verify(workspace, storybook, json),
        Command::Scaffold { templates } => cmd_scaffold(
            storybook,
            templates
                .clone()
                .unwrap_or_else(|| storybook.join("kit").join("templates")),
            json,
        ),
    }
}

// ---------------------------------------------------------------------------
// output

/// Green output: one human line, or the JSON payload with `ok: true`.
fn emit(json: bool, text: String, mut payload: Value) {
    if json {
        if let Value::Object(map) = &mut payload {
            map.insert("ok".to_string(), Value::Bool(true));
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
    } else {
        println!("{text}");
    }
}

/// Red output: the problem list. The caller exits 1.
fn fail(json: bool, problems: &[String]) -> bool {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "ok": false, "problems": problems }))
                .unwrap_or_default()
        );
    } else {
        for problem in problems {
            println!("- {problem}");
        }
    }
    false
}

/// Exactly one of --write/--check is required; `Ok(true)` = write.
fn mode(write: bool, check: bool) -> Result<bool, Vec<String>> {
    match (write, check) {
        (false, false) | (true, true) => {
            Err(vec!["pass exactly one of --write or --check".to_string()])
        }
        (true, false) => Ok(true),
        (false, true) => Ok(false),
    }
}

// ---------------------------------------------------------------------------
// commands

fn cmd_probe(workspace: &Path, json: bool) -> bool {
    let report = probe::probe(&scan::scan(workspace));
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::to_value(&report).unwrap_or_default())
                .unwrap_or_default()
        );
    } else {
        println!("tier: {}", tier_str(report.tier));
        for view in &report.views {
            println!(
                "  {} ({}): vm {} actions {}",
                view.id, view.module, view.vm_type, view.actions_type
            );
        }
    }
    true
}

fn tier_str(tier: probe::Tier) -> &'static str {
    match tier {
        probe::Tier::A => "a",
        probe::Tier::B => "b",
        probe::Tier::C => "c",
    }
}

fn cmd_contract(workspace: &Path, check: bool, json: bool) -> bool {
    let report = probe::probe(&scan::scan(workspace));
    let problems = probe::contract_problems(&report);
    if !check {
        // report mode: the tier and every problem, but no gate
        if json {
            let payload = json!({
                "tier": tier_str(report.tier),
                "problems": problems,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&payload).unwrap_or_default()
            );
        } else {
            println!("tier: {}", tier_str(report.tier));
            for problem in &problems {
                println!("- {problem}");
            }
        }
        return true;
    }
    if problems.is_empty() {
        emit(
            json,
            format!("tier: {}", tier_str(report.tier)),
            json!({ "tier": tier_str(report.tier) }),
        );
        true
    } else {
        fail(json, &problems)
    }
}

fn cmd_scan(workspace: &Path, storybook: &Path, write: bool, check: bool, json: bool) -> bool {
    let write = match mode(write, check) {
        Ok(write) => write,
        Err(problems) => return fail(json, &problems),
    };
    let report = scan::scan(workspace);
    let path = storybook.join("inventory.json");
    if write {
        if let Err(err) = write_atomic(&path, &inventory_bytes(&report.inventory)) {
            return fail(json, &[format!("cannot write inventory.json: {err}")]);
        }
        emit(
            json,
            format!(
                "wrote inventory.json ({} views, {} findings)",
                report.inventory.views.len(),
                report.inventory.findings.len()
            ),
            json!({
                "written": "inventory.json",
                "views": report.inventory.views.len(),
                "findings": report.inventory.findings.len(),
            }),
        );
        true
    } else {
        match fresh_problem(storybook, &report.inventory) {
            None => {
                emit(
                    json,
                    "inventory.json is fresh".to_string(),
                    json!({ "inventory": "fresh" }),
                );
                true
            }
            Some(problem) => fail(json, &[problem]),
        }
    }
}

fn cmd_expand(storybook: &Path, write: bool, check: bool, json: bool) -> bool {
    let write = match mode(write, check) {
        Ok(write) => write,
        Err(problems) => return fail(json, &problems),
    };
    let inventory = match load_inventory(storybook) {
        Ok(inventory) => inventory,
        Err(problem) => return fail(json, &[problem]),
    };
    let (states, mut problems) = expand::load_states(storybook, &inventory);
    let expanded = expand::expand(&inventory, &states);
    problems.extend(expanded.problems);
    if write {
        if !problems.is_empty() {
            return fail(json, &problems);
        }
        if let Err(err) = expand::write_cells(storybook, &expanded.cells) {
            return fail(json, &[format!("cannot write cells.json: {err}")]);
        }
        emit(
            json,
            format!("wrote cells.json ({} cells)", expanded.cells.len()),
            json!({ "written": "cells.json", "cells": expanded.cells.len() }),
        );
        true
    } else {
        problems.extend(expand::check_cells(storybook, &expanded.cells));
        if problems.is_empty() {
            emit(
                json,
                format!("cells.json is fresh ({} cells)", expanded.cells.len()),
                json!({ "cells": expanded.cells.len() }),
            );
            true
        } else {
            fail(json, &problems)
        }
    }
}

fn cmd_gen(
    workspace: &Path,
    storybook: &Path,
    write: bool,
    check: bool,
    force: bool,
    json: bool,
) -> bool {
    let write = match mode(write, check) {
        Ok(write) => write,
        Err(problems) => return fail(json, &problems),
    };
    let cells = match load_cells(storybook) {
        Ok(cells) => cells,
        Err(problem) => return fail(json, &[problem]),
    };
    let report = scan::scan(workspace);
    let skeletons = r#gen::skeletons(&report, &cells);
    if write {
        // only missing fixtures are written: curated values survive a
        // re-run unless --force explicitly rewrites them
        let mut written = 0usize;
        let mut skipped = 0usize;
        for (view, states) in &skeletons {
            for (state, fixture) in states {
                let path = fixture_file(storybook, view, state);
                if !force && path.exists() {
                    skipped += 1;
                    continue;
                }
                if let Err(err) = write_atomic(&path, fixture.json.as_bytes()) {
                    return fail(json, &[format!("cannot write {}: {err}", path.display())]);
                }
                written += 1;
            }
        }
        let text = if skipped == 0 {
            format!("wrote {written} fixtures")
        } else {
            format!("wrote {written} fixtures ({skipped} existing kept; --force rewrites)")
        };
        emit(
            json,
            text,
            json!({ "written": written, "skipped": skipped }),
        );
        true
    } else {
        let mut problems = Vec::new();
        let mut passed = 0usize;
        for (view, states) in &skeletons {
            for (state, fixture) in states {
                let rel = format!("fixtures/{view}/{state}.json");
                match std::fs::read_to_string(fixture_file(storybook, view, state)) {
                    Err(_) => problems.push(format!("{rel} not found (run sbk gen --write)")),
                    Ok(text) => match r#gen::structural_drift(Some(&fixture.skel), &text) {
                        None => passed += 1,
                        Some(drift) => problems.push(format!("{rel} drifted: {drift}")),
                    },
                }
            }
        }
        if problems.is_empty() {
            emit(
                json,
                format!("{passed} fixtures match their skeletons"),
                json!({ "fixtures": passed }),
            );
            true
        } else {
            fail(json, &problems)
        }
    }
}

fn cmd_check(workspace: &Path, storybook: &Path, gate: Gate, strict: bool, json: bool) -> bool {
    // like `verify`: the gates read inventory.json + cells.json, so a stale
    // prerequisite is caught here instead of silently re-scanned
    let report = scan::scan(workspace);
    let inventory = match load_inventory(storybook) {
        Ok(inventory) => inventory,
        Err(problem) => return fail(json, &[problem]),
    };
    if let Some(problem) = fresh_problem(storybook, &report.inventory) {
        return fail(json, &[problem]);
    }
    let cells = match load_cells(storybook) {
        Ok(cells) => cells,
        Err(problem) => return fail(json, &[problem]),
    };
    let (states, mut problems) = expand::load_states(storybook, &inventory);
    problems.extend(expand::expand(&inventory, &states).problems);
    if let Gate::Titles = gate {
        let titles: BTreeMap<String, String> = states
            .values()
            .flat_map(|file| file.titles.iter().map(|(k, v)| (k.clone(), v.clone())))
            .collect();
        let outcome = check::check_titles(&cells, &titles);
        problems.extend(outcome.problems);
        if strict || problems.is_empty() {
            return done(json, outcome.passed, "titles", problems);
        }
        // warning-level: report, stay green
        if !json {
            for problem in &problems {
                println!("warning: {problem}");
            }
        }
        emit(
            json,
            format!(
                "titles: {} ok, {} warnings (use --strict to gate)",
                outcome.passed,
                problems.len()
            ),
            json!({ "passed": outcome.passed, "warnings": problems }),
        );
        return true;
    }
    let outcome = match gate {
        Gate::Fixtures => check::check_fixtures(storybook, &inventory, &cells),
        Gate::Coverage => check::check_coverage(storybook, &inventory, &cells),
        Gate::Flows => {
            let flows = check::check_flows(storybook, &inventory, &cells);
            let edges = flows.flows.edges.len() + flows.flows.terminals.len();
            problems.extend(flows.problems);
            return done(json, edges, "edges", problems);
        }
        // handled above; unreachable, but the match must stay exhaustive
        Gate::Titles => return true,
    };
    let what = match gate {
        Gate::Fixtures => "fixtures",
        Gate::Coverage => "stories",
        Gate::Flows => "edges",
        Gate::Titles => "titles",
    };
    problems.extend(outcome.problems);
    done(json, outcome.passed, what, problems)
}

fn cmd_verify(workspace: &Path, storybook: &Path, json: bool) -> bool {
    let report = scan::scan(workspace);
    let probed = probe::probe(&report);
    let mut problems = probe::contract_problems(&probed);
    if let Some(problem) = fresh_problem(storybook, &report.inventory) {
        problems.push(problem);
    }
    let chain = if problems.is_empty() {
        verify_chain(storybook, &report)
    } else {
        Err(problems)
    };
    match chain {
        Ok(chain) => {
            let builtin = layout::layout(&chain.views, &chain.cells, &chain.flows, &chain.titles);
            let (placed, warning) = match dot::dot_program() {
                Some(program) => match dot::layout_with_dot(
                    &program,
                    &chain.views,
                    &chain.cells,
                    &chain.flows,
                    &chain.titles,
                ) {
                    Ok(placed) => (placed, None),
                    Err(err) => (
                        builtin,
                        Some(format!("dot layout failed ({err}); builtin engine")),
                    ),
                },
                None => (
                    builtin,
                    Some(
                        "dot not available (SBK_DOT=off or not on PATH); builtin engine"
                            .to_string(),
                    ),
                ),
            };
            if let Some(warning) = warning {
                eprintln!("warning: {warning}");
            }
            let bytes = serde_json::to_string_pretty(&placed).unwrap_or_default();
            if let Err(err) = write_atomic(
                &storybook.join("layout.json"),
                format!("{bytes}\n").as_bytes(),
            ) {
                return fail(json, &[format!("cannot write layout.json: {err}")]);
            }
            emit(
                json,
                format!(
                    "all gates green; wrote layout.json ({} nodes, {} edges, engine {})",
                    placed.nodes.len(),
                    placed.edges.len(),
                    placed.engine
                ),
                json!({
                    "layout": {
                        "nodes": placed.nodes.len(),
                        "edges": placed.edges.len(),
                        "engine": placed.engine
                    }
                }),
            );
            true
        }
        Err(problems) => fail(json, &problems),
    }
}

/// The gates after contract + inventory freshness; on success everything the
/// layout needs. Fails fast: a missing prerequisite skips the gates that
/// cannot run (their own commands report the details).
fn verify_chain(storybook: &Path, report: &scan::ScanReport) -> Result<Chain, Vec<String>> {
    let inventory = load_inventory(storybook).map_err(|p| vec![p])?;
    let (states, mut problems) = expand::load_states(storybook, &inventory);
    let expanded = expand::expand(&inventory, &states);
    problems.extend(expanded.problems);
    problems.extend(expand::check_cells(storybook, &expanded.cells));
    if !problems.is_empty() {
        return Err(problems);
    }
    let cells = load_cells(storybook).map_err(|p| vec![p])?;

    // gen --check
    let skeletons = r#gen::skeletons(report, &cells);
    for (view, states) in &skeletons {
        for (state, fixture) in states {
            match std::fs::read_to_string(fixture_file(storybook, view, state)) {
                Err(_) => problems.push(format!(
                    "fixtures/{view}/{state}.json not found (run sbk gen --write)"
                )),
                Ok(text) => {
                    if let Some(drift) = r#gen::structural_drift(Some(&fixture.skel), &text) {
                        problems.push(format!("fixtures/{view}/{state}.json drifted: {drift}"));
                    }
                }
            }
        }
    }

    problems.extend(check::check_fixtures(storybook, &inventory, &cells).problems);
    problems.extend(check::check_coverage(storybook, &inventory, &cells).problems);
    let flows = check::check_flows(storybook, &inventory, &cells);
    problems.extend(flows.problems);
    // verify gates titles strictly (`check titles` warns without --strict)
    let titles: BTreeMap<String, String> = states
        .values()
        .flat_map(|file| file.titles.iter().map(|(k, v)| (k.clone(), v.clone())))
        .collect();
    problems.extend(check::check_titles(&cells, &titles).problems);
    if !problems.is_empty() {
        return Err(problems);
    }
    Ok(Chain {
        views: inventory.views.iter().map(|v| v.id.clone()).collect(),
        cells,
        flows: flows.flows,
        titles,
    })
}

struct Chain {
    views: Vec<String>,
    cells: Vec<Cell>,
    flows: Flows,
    titles: BTreeMap<String, String>,
}

fn cmd_scaffold(storybook: &Path, templates: PathBuf, json: bool) -> bool {
    let outcome = check::scaffold(storybook, &templates);
    if outcome.problems.is_empty() {
        emit(
            json,
            format!("scaffolded {} files", outcome.copied.len()),
            json!({ "copied": outcome.copied }),
        );
        true
    } else {
        fail(json, &outcome.problems)
    }
}

// ---------------------------------------------------------------------------
// shared helpers

fn done(json: bool, passed: usize, what: &str, problems: Vec<String>) -> bool {
    if problems.is_empty() {
        emit(
            json,
            format!("{what}: {passed} ok"),
            json!({ "passed": passed }),
        );
        true
    } else {
        fail(json, &problems)
    }
}

fn inventory_bytes(inventory: &Inventory) -> Vec<u8> {
    let text = serde_json::to_string_pretty(inventory).unwrap_or_default();
    format!("{text}\n").into_bytes()
}

/// `None` when `inventory.json` on disk equals the fresh scan.
fn fresh_problem(storybook: &Path, inventory: &Inventory) -> Option<String> {
    match std::fs::read_to_string(storybook.join("inventory.json")) {
        Ok(text) if text.as_bytes() == inventory_bytes(inventory) => None,
        Ok(_) => Some("inventory.json is out of date (run sbk scan --write)".to_string()),
        Err(_) => Some("inventory.json not found (run sbk scan --write)".to_string()),
    }
}

fn load_inventory(storybook: &Path) -> Result<Inventory, String> {
    let text = std::fs::read_to_string(storybook.join("inventory.json"))
        .map_err(|_| "inventory.json not found (run sbk scan --write)".to_string())?;
    serde_json::from_str(&text).map_err(|err| format!("inventory.json does not parse: {err}"))
}

fn load_cells(storybook: &Path) -> Result<Vec<Cell>, String> {
    let text = std::fs::read_to_string(storybook.join("cells.json"))
        .map_err(|_| "cells.json not found (run sbk expand --write)".to_string())?;
    let cells: CellsFile =
        serde_json::from_str(&text).map_err(|err| format!("cells.json does not parse: {err}"))?;
    Ok(cells.cells)
}

fn fixture_file(storybook: &Path, view: &str, state: &str) -> PathBuf {
    storybook
        .join("fixtures")
        .join(view)
        .join(format!("{state}.json"))
}
