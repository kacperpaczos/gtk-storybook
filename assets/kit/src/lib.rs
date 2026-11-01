//! storybook-kit — the engine behind the `sbk` CLI.
//!
//! The kit is generic: it knows the *contract* a GTK/libadwaita host must
//! follow (view-model + actions + pure `build`), never any concrete host.
//! All deterministic checking lives here; the viewer is a separate consumer
//! of [`model::Layout`] and the generated JSON files.

pub mod check;
pub mod dot;
pub mod expand;
pub mod r#gen;
pub mod layout;
pub mod model;
pub mod probe;
pub mod scan;

use sha2::{Digest, Sha256};
use std::io;
use std::path::Path;

/// Stable hash input separator (a control char that cannot appear in ids).
const SEP: u8 = 0x1f;

/// sha256 over the parts joined with `SEP`, as lowercase hex.
pub(crate) fn sha_hex(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            hasher.update([SEP]);
        }
        hasher.update(part.as_bytes());
    }
    to_hex(&hasher.finalize())
}

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    out
}

/// `DirDialog` -> `dir_dialog`; a run of capitals stays glued
/// (`HTTPServer` -> `httpserver`).
pub(crate) fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let mut prev_lower = false;
    for ch in name.chars() {
        if ch.is_uppercase() {
            if prev_lower {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
            prev_lower = false;
        } else {
            prev_lower = ch.is_alphanumeric() && !ch.is_uppercase();
            out.push(ch);
        }
    }
    out
}

/// Atomic file write: `*.tmp` in the same directory, then rename. Public:
/// the `sbk` binary writes its generated files the same way as the library.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut tmp = parent.to_path_buf();
    let mut name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    name.push_str(".tmp");
    tmp.push(name);
    std::fs::write(&tmp, bytes)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = std::fs::remove_file(&tmp);
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha_hex_joins_parts_with_only_the_separator() {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"a");
        h.update([SEP]);
        h.update(b"b");
        assert_eq!(sha_hex(&["a", "b"]), to_hex(&h.finalize()));
    }
    #[test]
    fn sha_hex_separates_parts_so_ab_differs_from_a_b() {
        assert_ne!(sha_hex(&["a", "b"]), sha_hex(&["ab"]));
    }

    #[test]
    fn to_hex_is_lowercase_hex_of_the_bytes() {
        assert_eq!(to_hex(&[0x00, 0x0f, 0xa3, 0xff]), "000fa3ff");
    }

    #[test]
    fn snake_case_rules() {
        assert_eq!(snake_case("DirDialog"), "dir_dialog");
        assert_eq!(snake_case("HTTPServer"), "httpserver");
        // an underscore is already a word boundary: no doubling
        assert_eq!(snake_case("A_B"), "a_b");
    }
}
