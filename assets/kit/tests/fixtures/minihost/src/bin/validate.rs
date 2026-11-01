//! `storybook/validate` for the Rust adapter (references/adapters.md):
//! decodes every generated fixture under `<storybook>/fixtures` into the
//! host's real view-model types. A fixture that does not round-trip proves
//! the checker invented a value the host cannot consume.
//!
//! Usage: `cargo run --bin validate -- <storybook-dir>`; exit 0 green,
//! exit 1 with one `fixtures/<view>/<state>.json: <error>` line per failure.

use std::path::Path;
use std::process::ExitCode;

use minihost::corners::{CornersVm, alt::AltVm, pairy::PairVm, pf::PfVm, pm::PmVm, pv::PvVm};
use minihost::create::CreateVm;
use minihost::settings::SettingsVm;
use minihost::skill_detail::SkillDetailVm;
use nestedhost::inner::deep::DeepVm;

fn main() -> ExitCode {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("usage: validate <storybook-dir>");
        return ExitCode::FAILURE;
    };
    match run(&dir) {
        Ok(count) => {
            println!("validated {count} fixtures");
            ExitCode::SUCCESS
        }
        Err(problems) => {
            for problem in problems {
                eprintln!("{problem}");
            }
            ExitCode::FAILURE
        }
    }
}

fn run(dir: &str) -> Result<usize, Vec<String>> {
    let fixtures = Path::new(dir).join("fixtures");
    let mut problems = Vec::new();
    let mut count = 0usize;
    let Some(entries) = sorted_dirs(&fixtures) else {
        return Err(vec![format!(
            "fixtures directory {} not found",
            fixtures.display()
        )]);
    };
    for view in entries {
        let Some(states) = sorted_json(&fixtures.join(&view)) else {
            problems.push(format!("fixtures/{view}: cannot read directory"));
            continue;
        };
        if states.is_empty() {
            problems.push(format!("fixtures/{view}: no fixtures"));
            continue;
        }
        for state in states {
            let rel = format!("fixtures/{view}/{state}");
            let Ok(text) = std::fs::read_to_string(fixtures.join(&view).join(&state)) else {
                problems.push(format!("{rel}: cannot read"));
                continue;
            };
            let outcome = match view.as_str() {
                "create" => decode::<CreateVm>(&text),
                "settings" => decode::<SettingsVm>(&text),
                "skill_detail" => decode::<SkillDetailVm>(&text),
                "corners" => decode::<CornersVm>(&text),
                "pairy" => decode::<PairVm>(&text),
                "pm" => decode::<PmVm>(&text),
                "pf" => decode::<PfVm>(&text),
                "pv" => decode::<PvVm>(&text),
                "alt" => decode::<AltVm>(&text),
                "deep" => decode::<DeepVm>(&text),
                other => Err(format!("no view-model type mapped for view {other}")),
            };
            match outcome {
                Ok(()) => count += 1,
                Err(err) => problems.push(format!("{rel}: {err}")),
            }
        }
    }
    if problems.is_empty() {
        Ok(count)
    } else {
        Err(problems)
    }
}

fn decode<T: serde::de::DeserializeOwned>(document: &str) -> Result<(), String> {
    serde_json::from_str::<T>(document)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn sorted_dirs(dir: &Path) -> Option<Vec<String>> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .collect();
    out.sort();
    Some(out)
}

fn sorted_json(dir: &Path) -> Option<Vec<String>> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .filter(|n| n.ends_with(".json"))
        .collect();
    out.sort();
    Some(out)
}
