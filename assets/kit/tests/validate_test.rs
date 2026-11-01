//! The Rust adapter's `storybook/validate` (references/adapters.md): every
//! generated minihost fixture must decode into the host's real view-model
//! types. This is the round-trip half of `sbk gen`: a fixture that parses
//! as JSON but not as the VM type proves the checker invented a shape the
//! host cannot consume.
//!
//! The validator binary is built once into a shared on-disk target dir so
//! repeated runs (and the mutation gate) do not recompile serde.

mod common;

use common::{HostDir, kit_dir, minihost_copy, minihost_src};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use storybook_kit::expand::{expand, load_states};
use storybook_kit::r#gen;
use storybook_kit::scan::scan;

/// minihost + dispositions + expanded cells + generated fixture files.
fn storybook_with_fixtures(tag: &str) -> (HostDir, PathBuf) {
    let host = minihost_copy(tag);
    let storybook = host.join("storybook");
    let states = storybook.join("states");
    fs::create_dir_all(&states).unwrap();
    let report = scan(&host);
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
    let (loaded, problems) = load_states(&storybook, &report.inventory);
    assert!(problems.is_empty(), "{problems:?}");
    let expanded = expand(&report.inventory, &loaded);
    assert!(expanded.problems.is_empty(), "{:?}", expanded.problems);
    for (view, fixtures) in &r#gen::skeletons(&report, &expanded.cells) {
        for (state, fixture) in fixtures {
            let path = storybook
                .join("fixtures")
                .join(view)
                .join(format!("{state}.json"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, &fixture.json).unwrap();
        }
    }
    (host, storybook)
}

fn shared_target() -> PathBuf {
    std::env::temp_dir().join("sbk-minihost-validate-target")
}
// The validator is an external host tool: the kit's test harness builds and
// runs it, which the host repo's disallowed-methods lint forbids elsewhere.
#[allow(clippy::disallowed_methods)]
#[test]
fn generated_fixtures_decode_into_the_real_view_model_types() {
    let (_host, storybook) = storybook_with_fixtures("validate-green");
    let manifest = minihost_src().join("Cargo.toml");
    let status = Command::new("cargo")
        .env("CARGO_TARGET_DIR", shared_target())
        .arg("build")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--offline")
        .arg("--bin")
        .arg("validate")
        .status()
        .expect("cargo build runs");
    assert!(status.success());
    let out = Command::new(shared_target().join("debug/validate"))
        .arg(&storybook)
        .output()
        .expect("validator runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "validator red:\n{stdout}\n{stderr}");
    assert!(stdout.contains("validated"), "{stdout}");
    assert!(stdout.contains("132 fixtures"), "{stdout}");

    // a corrupted fixture must be caught by the validator, not accepted
    let fixture = storybook.join("fixtures/settings/any-any-any-any-none-ok-any.json");
    let text = fs::read_to_string(&fixture).unwrap();
    fs::write(
        &fixture,
        text.replace("\"ratio\": 0.0", "\"ratio\": \"flat\""),
    )
    .unwrap();
    let out = Command::new(shared_target().join("debug/validate"))
        .arg(&storybook)
        .output()
        .expect("validator runs");
    assert!(
        !out.status.success(),
        "validator must reject a retyped field"
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("fixtures/settings/any-any-any-any-none-ok-any.json"),
        "{stderr}"
    );
}

/// Keep the fixture-tree helpers linked (and sanity-check the tree).
#[test]
fn minihost_fixture_tree_exists() {
    let src = minihost_src();
    assert!(src.join("src/lib.rs").is_file());
    assert!(src.join("src/bin/validate.rs").is_file());
    assert!(kit_dir().join("Cargo.toml").is_file());
}
