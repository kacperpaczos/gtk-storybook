//! Deterministic fixture generation (`sbk gen`).
//!
//! One JSON view-model per cell, synthesized purely from the inventory's
//! type shapes: enums take their canonical variant (or the cell's class),
//! strings are seeded by `hash(view+path)` so re-runs are byte-identical,
//! and boundary classes get the conventional shapes (`empty` -> `""`,
//! `overflow` -> a very long string). Unresolvable types become wildcards
//! that curation may fill with anything.

use crate::model::Cell;
use crate::scan::{ScanReport, TypeShape};
use crate::sha_hex;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// How many items a `many` collection exercises (scrolling).
pub const MANY: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    Empty,
    One,
    Many,
}

/// Skeleton of a generated fixture: structure + bucket, values as generated.
/// Drift checking compares structure only (curation may change values).
#[derive(Debug, Clone, PartialEq)]
pub enum Skel {
    /// Unresolved type: anything the curator writes is fine.
    Wild,
    Null,
    Bool(bool),
    /// Integer value: JSON `0`, never `0.0` — `u32` rejects floats.
    Int(i64),
    /// Float value: JSON keeps the decimal point (`0.0`).
    Num(f64),
    /// A one-character string (`char`).
    Char,
    Str(String),
    Arr {
        bucket: Bucket,
        item: Box<Skel>,
    },
    /// A JSON object whose key count follows the bucket (maps).
    Map {
        bucket: Bucket,
        value: Box<Skel>,
    },
    /// A JSON array of an exact length (tuples, fixed arrays).
    Tup(Vec<Skel>),
    /// An externally tagged enum: `chosen` is the variant the generated
    /// fixture renders; drift accepts any declared variant, checking the
    /// payload against that variant's own skeleton (`None` = unit variant).
    /// `pinned` = the enum is an axis of this cell: only `chosen` is valid.
    Enum {
        chosen: usize,
        pinned: bool,
        variants: Vec<(String, Option<Skel>)>,
    },
    Obj(Vec<(String, Skel)>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fixture {
    pub skel: Skel,
    pub json: String,
}

/// view id -> state (without the `<view>/` prefix) -> fixture.
pub fn skeletons(
    report: &ScanReport,
    cells: &[Cell],
) -> BTreeMap<String, BTreeMap<String, Fixture>> {
    let mut out: BTreeMap<String, BTreeMap<String, Fixture>> = BTreeMap::new();
    for cell in cells {
        let shape = report
            .shapes
            .get(&cell.view)
            .cloned()
            .unwrap_or(TypeShape::Unresolved);
        let skel = skel_for(&shape, &cell.view, "", &cell.assignment);
        let json = format!(
            "{}\n",
            serde_json::to_string_pretty(&to_value(&skel)).unwrap_or_default()
        );
        out.entry(cell.view.clone())
            .or_default()
            .insert(state_of(cell), Fixture { skel, json });
    }
    out
}

/// `fixtures/<view>/<state>.json` for a cell.
pub fn fixture_path(storybook: &Path, cell: &Cell) -> PathBuf {
    storybook
        .join("fixtures")
        .join(&cell.view)
        .join(format!("{}.json", state_of(cell)))
}

fn state_of(cell: &Cell) -> String {
    cell.state_id
        .strip_prefix(&cell.view)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(&cell.state_id)
        .to_string()
}

fn seeded_text(view: &str, path: &str) -> String {
    format!("text-{}", &sha_hex(&[view, path])[..8])
}

fn overflow_text(view: &str, path: &str) -> String {
    let seed = seeded_text(view, path);
    seed.repeat(32)
}

fn skel_for(
    shape: &TypeShape,
    view: &str,
    path: &str,
    assignment: &BTreeMap<String, String>,
) -> Skel {
    let class = assignment.get(path).map(String::as_str);
    match shape {
        TypeShape::Unresolved => Skel::Wild,
        TypeShape::Bool => Skel::Bool(class == Some("true")),
        TypeShape::Number { int: true } => Skel::Int(0),
        TypeShape::Number { int: false } => Skel::Num(0.0),
        TypeShape::Char => Skel::Char,
        TypeShape::Text => Skel::Str(match class {
            Some("empty") => String::new(),
            Some("overflow") => overflow_text(view, path),
            _ => seeded_text(view, path),
        }),
        TypeShape::Unit => Skel::Null,
        // fields whose representation cannot be modeled are omitted; the
        // extractor-limit finding sends the curator to fill them by hand
        TypeShape::Struct { fields } => Skel::Obj(
            fields
                .iter()
                .filter(|f| f.limited.is_none())
                .map(|f| {
                    let child = if path.is_empty() {
                        f.name.clone()
                    } else {
                        format!("{path}.{}", f.name)
                    };
                    (f.name.clone(), skel_for(&f.shape, view, &child, assignment))
                })
                .collect(),
        ),
        TypeShape::Tuple(items) => Skel::Tup(
            items
                .iter()
                .map(|item| skel_for(item, view, path, assignment))
                .collect(),
        ),
        TypeShape::Array { item, len } => Skel::Tup(
            (0..*len)
                .map(|_| skel_for(item, view, path, assignment))
                .collect(),
        ),
        TypeShape::Option(inner) => match class {
            Some("none") => Skel::Null,
            // `some` must hold a present value: null would re-render `none`.
            Some("some") => class_skel(inner, None, view, path, assignment),
            Some(other) => class_skel(inner, Some(other), view, path, assignment),
            // no assigned class: both `null` and a present value are valid
            None => Skel::Wild,
        },
        TypeShape::Enum { variants } => enum_skel(variants, class, view, path, assignment),
        TypeShape::Phase { ok, err } => {
            if class == Some("err") {
                Skel::Obj(vec![("Err".into(), skel_for(err, view, path, assignment))])
            } else {
                Skel::Obj(vec![("Ok".into(), skel_for(ok, view, path, assignment))])
            }
        }
        TypeShape::Collection(item) => Skel::Arr {
            bucket: bucket_of(class),
            item: Box::new(skel_for(item, view, path, assignment)),
        },
        TypeShape::Map { value, .. } => Skel::Map {
            bucket: bucket_of(class),
            value: Box::new(skel_for(value, view, path, assignment)),
        },
    }
}

/// All variants of an externally tagged enum, with `class` (an axis
/// variant name) choosing which one the generated fixture renders.
fn enum_skel(
    variants: &[crate::scan::VariantInfo],
    class: Option<&str>,
    view: &str,
    path: &str,
    assignment: &BTreeMap<String, String>,
) -> Skel {
    let matched = variants.iter().position(|v| Some(v.name.as_str()) == class);
    Skel::Enum {
        chosen: matched.unwrap_or(0),
        pinned: matched.is_some(),
        variants: variants
            .iter()
            .map(|v| {
                (
                    v.name.clone(),
                    v.payload
                        .as_ref()
                        .map(|p| skel_for(p, view, path, assignment)),
                )
            })
            .collect(),
    }
}

fn bucket_of(class: Option<&str>) -> Bucket {
    match class {
        Some("one") => Bucket::One,
        Some("many") => Bucket::Many,
        _ => Bucket::Empty,
    }
}

/// A value for `shape` constrained to a finite inner class (`Asc`, `true`, …).
fn class_skel(
    shape: &TypeShape,
    class: Option<&str>,
    view: &str,
    path: &str,
    assignment: &BTreeMap<String, String>,
) -> Skel {
    match shape {
        TypeShape::Bool => Skel::Bool(class == Some("true")),
        TypeShape::Enum { variants } => enum_skel(variants, class, view, path, assignment),
        other => skel_for(other, view, path, assignment),
    }
}

fn to_value(skel: &Skel) -> Value {
    match skel {
        Skel::Wild | Skel::Null => Value::Null,
        Skel::Bool(b) => Value::Bool(*b),
        Skel::Int(n) => Value::from(*n),
        Skel::Num(n) => serde_json::json!(n),
        Skel::Char => Value::String("a".into()),
        Skel::Str(s) => Value::String(s.clone()),
        Skel::Arr { bucket, item } => Value::Array(vec![to_value(item); bucket_len(bucket)]),
        Skel::Map { bucket, value } => {
            let mut map = serde_json::Map::new();
            for i in 0..bucket_len(bucket) {
                map.insert(format!("k{i}"), to_value(value));
            }
            Value::Object(map)
        }
        Skel::Tup(items) => Value::Array(items.iter().map(to_value).collect()),
        Skel::Obj(fields) => Value::Object(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), to_value(v)))
                .collect(),
        ),
        Skel::Enum {
            chosen, variants, ..
        } => match variants.get(*chosen) {
            Some((name, Some(payload))) => {
                Value::Object([(name.clone(), to_value(payload))].into_iter().collect())
            }
            Some((name, None)) => Value::String(name.clone()),
            None => Value::Null,
        },
    }
}

fn bucket_len(bucket: &Bucket) -> usize {
    match bucket {
        Bucket::Empty => 0,
        Bucket::One => 1,
        Bucket::Many => MANY,
    }
}

/// Structural drift between a skeleton and a (possibly curated) fixture.
/// `None` = no drift. Scalar values are ignored; key sets, JSON types,
/// collection size classes and null-ness must match.
pub fn structural_drift(expected: Option<&Skel>, actual: &str) -> Option<String> {
    let Some(expected) = expected else {
        return Some("missing skeleton (view not in inventory)".to_string());
    };
    let value: Value = match serde_json::from_str(actual) {
        Ok(value) => value,
        Err(err) => return Some(format!("fixture does not parse: {err}")),
    };
    drift(expected, &value, "$")
}

fn drift(skel: &Skel, value: &Value, path: &str) -> Option<String> {
    match skel {
        Skel::Wild => None,
        Skel::Null => value
            .is_null()
            .then_some(())
            .map_or(Some(format!("{path}: expected null")), |_| None),
        Skel::Bool(_) => {
            if value.is_boolean() {
                None
            } else {
                Some(format!("{path}: expected bool"))
            }
        }
        // an integer field must stay an integer: `u32` rejects `0.0`
        Skel::Int(_) => {
            if value.is_i64() || value.is_u64() {
                None
            } else {
                Some(format!("{path}: expected integer"))
            }
        }
        Skel::Num(_) => {
            if value.is_number() {
                None
            } else {
                Some(format!("{path}: expected number"))
            }
        }
        Skel::Char => {
            let one = value.as_str().is_some_and(|s| s.chars().count() == 1);
            if one {
                None
            } else {
                Some(format!("{path}: expected a one-character string"))
            }
        }
        Skel::Str(_) => {
            if value.is_string() {
                None
            } else {
                Some(format!("{path}: expected string"))
            }
        }
        Skel::Arr { bucket, item } => {
            let Some(items) = value.as_array() else {
                return Some(format!("{path}: expected array"));
            };
            if bucket_of_len(items.len()) != *bucket {
                return Some(format!("{path}: collection changed size class"));
            }
            items
                .iter()
                .enumerate()
                .find_map(|(i, v)| drift(item, v, &format!("{path}[{i}]")))
        }
        Skel::Map {
            bucket,
            value: item,
        } => {
            let Some(map) = value.as_object() else {
                return Some(format!("{path}: expected object (map)"));
            };
            if bucket_of_len(map.len()) != *bucket {
                return Some(format!("{path}: map changed size class"));
            }
            map.values()
                .enumerate()
                .find_map(|(i, v)| drift(item, v, &format!("{path}[{i}]")))
        }
        // a tuple or fixed array keeps its exact length
        Skel::Tup(items) => {
            let Some(actual) = value.as_array() else {
                return Some(format!("{path}: expected array"));
            };
            if actual.len() != items.len() {
                return Some(format!("{path}: fixed-length value changed length"));
            }
            items
                .iter()
                .zip(actual)
                .enumerate()
                .find_map(|(i, (skel, v))| drift(skel, v, &format!("{path}[{i}]")))
        }
        // externally tagged: any declared variant, payload checked
        // against that variant's own skeleton
        Skel::Enum {
            chosen,
            pinned,
            variants,
        } => {
            // an axis enum pins the cell's variant; elsewhere any variant
            let variants: &[(String, Option<Skel>)] = if *pinned {
                variants.get(*chosen..=*chosen).unwrap_or(&[])
            } else {
                variants
            };
            let allowed = || {
                variants
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            match value {
                Value::String(s) => match variants.iter().find(|(n, p)| p.is_none() && n == s) {
                    Some(_) => None,
                    None => Some(format!(
                        "{path}: unknown variant {s}; allowed: {}",
                        allowed()
                    )),
                },
                Value::Object(map) if map.len() == 1 => {
                    let (name, payload) = map.iter().next().unwrap();
                    match variants.iter().find(|(n, _)| n == name) {
                        Some((_, Some(skel))) => drift(skel, payload, &format!("{path}.{name}")),
                        Some((_, None)) => {
                            Some(format!("{path}: unit variant {name} must be a string"))
                        }
                        None => Some(format!(
                            "{path}: unknown variant {name}; allowed: {}",
                            allowed()
                        )),
                    }
                }
                _ => Some(format!(
                    "{path}: expected a variant object or string; allowed: {}",
                    allowed()
                )),
            }
        }
        Skel::Obj(fields) => {
            let Some(map) = value.as_object() else {
                return Some(format!("{path}: expected object"));
            };
            for (key, child) in fields {
                match map.get(key) {
                    None => return Some(format!("{path}.{key}: missing key")),
                    Some(v) => {
                        if let Some(problem) = drift(child, v, &format!("{path}.{key}")) {
                            return Some(problem);
                        }
                    }
                }
            }
            for key in map.keys() {
                if !fields.iter().any(|(k, _)| k == key) {
                    return Some(format!("{path}.{key}: unexpected key"));
                }
            }
            None
        }
    }
}

fn bucket_of_len(len: usize) -> Bucket {
    match len {
        0 => Bucket::Empty,
        1 => Bucket::One,
        _ => Bucket::Many,
    }
}
