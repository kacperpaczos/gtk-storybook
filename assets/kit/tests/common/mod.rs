//! Shared helpers for the kit's integration tests.
#![allow(dead_code)] // helpers are shared; each test binary uses a subset

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn kit_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn minihost_src() -> PathBuf {
    kit_dir().join("tests/fixtures/minihost")
}

/// Recursively copy `src` into a fresh directory under a tempdir.
pub fn copy_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    let mut entries: Vec<_> = fs::read_dir(src).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        // the fixture's build output is never scanned; copying it would
        // drop ~50 MB into /tmp per test copy
        if entry.file_name().to_string_lossy() == "target" {
            continue;
        }
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &to);
        } else {
            fs::copy(entry.path(), &to).unwrap();
        }
    }
}

/// A fresh temp copy of the minihost fixture, removed when the guard drops.
/// Derefs to the copy's path, so `&host` works anywhere `&Path` does.
pub struct HostDir {
    _tmp: tempfile::TempDir,
}

impl HostDir {
    /// Adopt an existing tempdir (tests that build their own layout).
    pub fn from_tmp(tmp: tempfile::TempDir) -> Self {
        HostDir { _tmp: tmp }
    }
}

impl std::ops::Deref for HostDir {
    type Target = std::path::Path;
    fn deref(&self) -> &Self::Target {
        self._tmp.path()
    }
}

pub fn minihost_copy(tag: &str) -> HostDir {
    let tmp = tempfile::Builder::new()
        .prefix(&format!("sbk-minihost-{tag}-"))
        .tempdir()
        .expect("fresh tempdir for the host copy");
    copy_tree(&minihost_src(), tmp.path());
    HostDir { _tmp: tmp }
}

/// Replace exact text in a file (panics if absent — tests are explicit).
pub fn edit(path: &Path, from: &str, to: &str) {
    let text = fs::read_to_string(path).unwrap();
    assert!(
        text.contains(from),
        "fixture edit: {from:?} not found in {path:?}"
    );
    fs::write(path, text.replacen(from, to, 1)).unwrap();
}

// The kit ships its own workspace; a host repo's disallowed-methods list
// (no process spawning) does not apply to running the checker under test.
#[allow(clippy::disallowed_methods)]
pub fn sbk(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_sbk"))
        .args(args)
        .output()
        .expect("sbk binary runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Run `sbk` with extra environment variables (e.g. `SBK_DOT=off`).
#[allow(clippy::disallowed_methods)]
pub fn sbk_env(args: &[&str], envs: &[(&str, &str)]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_sbk"))
        .args(args)
        .envs(envs.iter().copied())
        .output()
        .expect("sbk binary runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Is a `dot` binary reachable on PATH (kit dot-engine tests skip without)?
#[allow(clippy::disallowed_methods)]
pub fn has_dot_on_path() -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                #[cfg(unix)]
                use std::os::unix::fs::PermissionsExt;
                let candidate = dir.join("dot");
                candidate.is_file()
                    && std::fs::metadata(&candidate)
                        .map(|m| m.permissions().mode() & 0o111 != 0)
                        .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// Extract a JSON object from stdout.
pub fn json_out(stdout: &str) -> serde_json::Value {
    serde_json::from_str(stdout).unwrap()
}
