//! Static host analysis: view discovery, projection, axes, findings.
//!
//! `sbk` reads the host with `syn` — syntax, not semantics. Anything it
//! cannot resolve becomes a [`Finding`] instead of a guess
//! (`references/discovery.md`). File walks are sorted and every map is
//! ordered, so scanning is fully deterministic.
//!
//! Known limitations (deliberate, syntax-level only): helper functions that
//! take a *sub-struct* of the VM instead of the whole VM do not project;
//! reads inside macros (e.g. `glib::clone!` callbacks) are invisible and
//! surface as `macro` findings; serde attributes that change the wire
//! representation (rename_all, rename, tag, untagged, skip) are reported
//! as `extractor-limit` findings and their fields carry no generated
//! fixture value. `#[serde(default)]` is not a limit: the field keeps
//! its skeleton and is recorded as optional (the key may be absent).

use crate::model::{Axis, AxisKind, Finding, Inventory, ViewInfo};
use crate::{sha_hex, snake_case};
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Expr, ExprField, ExprPath, Fields, File, Item, ItemFn, Member, Pat, UseTree};

pub const K_UNRESOLVED: &str = "unresolved-type";
pub const K_MACRO: &str = "macro";
pub const K_WIDGET_LOCAL: &str = "widget-local-state";
pub const K_DYNAMIC: &str = "dynamic-registration";
pub const K_UNREAD: &str = "unread-field";
pub const K_FEATURE: &str = "feature-gated";
pub const K_EDGE: &str = "edge-candidate";
pub const K_EXTRACTOR_LIMIT: &str = "extractor-limit";

/// Macros the checker effectively sees through (pure value-level syntax).
const MACRO_ALLOWLIST: &[&str] = &[
    "vec",
    "format",
    "matches",
    "println",
    "eprintln",
    "write",
    "writeln",
    "stringify",
    "dbg",
    "assert",
    "assert_eq",
    "assert_ne",
    "panic",
    "unreachable",
    "todo",
    "unimplemented",
];
/// Navigation effects that produce edge candidates.
const NAV_METHODS: &[&str] = &["push", "push_by_tag", "present", "pop"];
const NAV_METHOD_PREFIX: &str = "set_visible_child";
/// `std::` modules that mean I/O inside a view (impure, tier C).
const IO_STD_MODULES: &[&str] = &["fs", "process", "env", "net"];
/// Directories never scanned for host views.
const SKIP_DIRS: &[&str] = &[
    "target",
    "storybook",
    "kit",
    ".git",
    ".cargo",
    "tests",
    "benches",
    "examples",
    "fixtures",
    "node_modules",
];
const VIEW_FNS: &[&str] = &["build", "build_content", "build_with_handle"];
const INT_NUMBERS: &[&str] = &[
    "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize",
];
const FLOAT_NUMBERS: &[&str] = &["f32", "f64"];
/// Serde attributes that change the wire representation and that this
/// syntax-level extractor refuses to model (`references/discovery.md`:
/// whatever it cannot resolve is a finding, never a guess). `default`
/// is not a limit: it only lets a key be absent, which the skeleton
/// records as an optional field.
const SERDE_LIMITS: &[&str] = &["rename_all", "rename", "tag", "untagged", "skip"];

// ---------------------------------------------------------------------------
// type shapes

#[derive(Debug, Clone)]
pub struct FieldInfo {
    pub name: String,
    pub shape: TypeShape,
    /// `file:line` of the declaration (nested types may live in another file).
    pub location: String,
    pub feature_gated: bool,
    pub has_dyn: bool,
    /// Why the wire representation cannot be modeled (serde attributes on
    /// the field or on a type it mentions); `None` = modelable.
    pub limited: Option<String>,
    /// `#[serde(default)]` on the field or on its struct: generated
    /// fixtures carry the key, curated ones may omit it.
    pub optional: bool,
}

#[derive(Debug, Clone)]
pub struct VariantInfo {
    pub name: String,
    /// Payload shape: the single field of a one-field tuple variant, or
    /// the struct/tuple shape of struct-like and multi-field variants.
    pub payload: Option<TypeShape>,
}

#[derive(Debug, Clone)]
pub enum TypeShape {
    Bool,
    /// `char`: a text axis whose fixture carries exactly one character.
    Char,
    Text,
    /// Integers and floats accept different JSON: the flag keeps `0` and
    /// `0.0` apart so fixtures round-trip into the host types.
    Number {
        int: bool,
    },
    /// `()` serializes as `null`.
    Unit,
    Option(Box<TypeShape>),
    /// Sequences with a size class: Vec, HashSet, slices.
    Collection(Box<TypeShape>),
    /// Maps: JSON objects, keys rendered by the key shape.
    Map {
        key: Box<TypeShape>,
        value: Box<TypeShape>,
    },
    /// `[T; N]`: always exactly N items, no size class.
    Array {
        item: Box<TypeShape>,
        len: usize,
    },
    /// Tuples: a fixed-shape JSON array.
    Tuple(Vec<TypeShape>),
    /// `Result<T, E>`: the only phase axis, classes `ok` / `err`.
    Phase {
        ok: Box<TypeShape>,
        err: Box<TypeShape>,
    },
    Enum {
        variants: Vec<VariantInfo>,
    },
    Struct {
        fields: Vec<FieldInfo>,
    },
    Unresolved,
}

impl TypeShape {
    pub fn as_struct(&self) -> Option<&[FieldInfo]> {
        match self {
            TypeShape::Struct { fields } => Some(fields),
            _ => None,
        }
    }
}

/// Host type metadata `probe` needs beyond the shape.
#[derive(Debug, Clone)]
pub struct TypeInfo {
    pub public: bool,
    pub serde: bool,
    pub de: bool,
    /// Derives both `Clone` and `PartialEq` (tier B capability).
    pub clone_eq: bool,
    pub location: String,
}

#[derive(Debug, Clone)]
struct RawType {
    info: TypeInfo,
    tokens: String,
    kind: RawKind,
    /// Item-level serde attributes this extractor does not model.
    limits: Vec<String>,
    /// `#[serde(default)]` at item level: every field of the struct may
    /// be absent from a fixture.
    default_fields: bool,
}

#[derive(Debug, Clone)]
enum RawKind {
    Struct(Vec<RawField>),
    Enum(Vec<RawVariant>),
}

#[derive(Debug, Clone)]
struct RawField {
    name: String,
    type_src: String,
    location: String,
    feature_gated: bool,
    has_dyn: bool,
    /// Field-level serde attributes this extractor does not model.
    limits: Vec<String>,
    /// `#[serde(default)]` / `#[serde(default = "…")]` on this field.
    serde_default: bool,
}

/// A variant payload as written: one unnamed field, several unnamed fields
/// (a tuple), or named fields (struct-like).
#[derive(Debug, Clone)]
enum RawPayload {
    One(String),
    Tuple(Vec<String>),
    Struct(Vec<RawField>),
}

#[derive(Debug, Clone)]
struct RawVariant {
    name: String,
    payload: Option<RawPayload>,
}

// ---------------------------------------------------------------------------
// report

#[derive(Debug, Clone)]
pub struct ScanReport {
    pub inventory: Inventory,
    /// view id -> VM shape (consumed by `gen` and `check fixtures`).
    pub shapes: BTreeMap<String, TypeShape>,
    pub types: BTreeMap<String, TypeInfo>,
    /// Crate-qualified modules whose source mentions I/O effects.
    pub io_modules: BTreeSet<String>,
    /// Crate name -> its crate-root modules -> whether the module is `pub`.
    pub root_modules_pub: BTreeMap<String, BTreeMap<String, bool>>,
}

// ---------------------------------------------------------------------------
// entry point

struct AstFile {
    rel: String,
    /// Crate name + module path within the crate (`minihost::corners`).
    module_base: Vec<String>,
    /// True when this file sits at its crate root (`src/lib.rs`).
    crate_root: bool,
    ast: File,
}

struct ViewCandidate {
    module: String,
    id: String,
    vm: String,
    actions: String,
    fn_tokens: String,
    fn_location: String,
}

pub fn scan(workspace: &Path) -> ScanReport {
    let mut files: Vec<AstFile> = Vec::new();
    let mut findings: Vec<Finding> = Vec::new();

    for (rel, krate, module_base, crate_root) in host_source_files(workspace) {
        let text = match std::fs::read_to_string(workspace.join(&rel)) {
            Ok(text) => text,
            Err(err) => {
                findings.push(mk_finding(
                    "",
                    K_UNRESOLVED,
                    &rel,
                    &rel,
                    &rel,
                    format!("unreadable source file: {err}"),
                ));
                continue;
            }
        };
        match syn::parse_file(&text) {
            Ok(ast) => {
                let mut base = vec![krate];
                base.extend(module_base);
                files.push(AstFile {
                    rel,
                    module_base: base,
                    crate_root,
                    ast,
                })
            }
            Err(err) => findings.push(Finding {
                id: format!("f-{}", &sha_hex(&["", &rel, K_UNRESOLVED])[..6]),
                view: String::new(),
                kind: K_UNRESOLVED.into(),
                location: rel.clone(),
                fingerprint: sha_hex(&[&text])[..16].to_string(),
                detail: format!("file does not parse: {err}"),
            }),
        }
    }

    // Type table keyed by `{crate}::{Name}` (a bare name may exist in
    // several crates), the I/O census and crate-root module visibility.
    let mut raw: BTreeMap<String, RawType> = BTreeMap::new();
    let mut index: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut io_modules: BTreeSet<String> = BTreeSet::new();
    let mut root_modules_pub: BTreeMap<String, BTreeMap<String, bool>> = BTreeMap::new();
    for file in &files {
        let krate = file.module_base[0].clone();
        collect_types(&file.ast.items, &file.rel, &krate, &mut raw, &mut index);
        if file_has_io(&file.ast) {
            io_modules.insert(file.module_base.join("::"));
        }
        if file.crate_root {
            let mods = root_modules_pub.entry(krate).or_default();
            collect_root_mods(&file.ast.items, mods);
        }
    }
    for keys in index.values_mut() {
        keys.sort();
    }
    let limited_closure = limited_reachability(&raw, &index);
    let mut resolver = Resolver {
        raw: &raw,
        index: &index,
        scope: String::new(),
        memo_named: BTreeMap::new(),
        memo_src: BTreeMap::new(),
        limited_closure: &limited_closure,
    };

    // Views and their findings.
    let mut views: Vec<ViewInfo> = Vec::new();
    let mut shapes: BTreeMap<String, TypeShape> = BTreeMap::new();
    let mut seen_modules: BTreeSet<String> = BTreeSet::new();
    let mut seen_view_ids: BTreeSet<String> = BTreeSet::new();
    for file in &files {
        for cand in discover_views(file, &mut seen_modules, &mut seen_view_ids) {
            // a bare type name resolves inside the view's own crate first
            // (a same-named type in another crate never shadows it)
            resolver.scope = file.module_base[0].clone();
            let vm_shape = resolver.named(&cand.vm, &mut Vec::new());
            let actions = resolver
                .named(&cand.actions, &mut Vec::new())
                .as_struct()
                .map(|fields| fields.iter().map(|f| f.name.clone()).collect())
                .unwrap_or_default();

            // Types reachable from the VM whose serde attributes this
            // extractor cannot model: one finding each, never a guess.
            let vm_key = resolve_key(&index, &cand.vm, &file.module_base[0]);
            for key in reachable_types(&raw, &index, vm_key) {
                let Some(entry) = raw.get(&key) else { continue };
                let limits = effective_limits(entry);
                if !limits.is_empty() {
                    let name = bare_name(&key);
                    findings.push(mk_finding(
                        &cand.id,
                        K_EXTRACTOR_LIMIT,
                        &name,
                        &entry.info.location,
                        &entry.tokens,
                        format!(
                            "type {name} uses serde attribute(s) {} the extractor does not model; its fixtures are hand-curated",
                            limits.join(", ")
                        ),
                    ));
                }
            }

            let (reads, inner) = project_file(file, &cand.vm);
            let mut axes: Vec<Axis> = Vec::new();
            let mut unread: Vec<String> = Vec::new();
            match vm_shape.as_struct() {
                Some(fields) => walk_fields(
                    fields,
                    "",
                    &reads,
                    &inner,
                    &cand.id,
                    vm_tokens(&raw, &index, &cand.vm, &file.module_base[0]),
                    &mut axes,
                    &mut unread,
                    &mut findings,
                ),
                None => findings.push(mk_finding(
                    &cand.id,
                    K_UNRESOLVED,
                    &cand.vm,
                    &cand.fn_location,
                    &cand.fn_tokens,
                    format!("VM type {} not found or not a struct", cand.vm),
                )),
            }
            scan_view_fns(file, &cand, &mut findings);
            shapes.insert(cand.id.clone(), vm_shape);
            views.push(ViewInfo {
                id: cand.id,
                module: cand.module,
                vm_type: cand.vm,
                actions_type: cand.actions,
                axes,
                actions,
                unread_fields: unread,
            });
        }
    }

    // census keyed by the bare type name (view types are resolved by it)
    let types = raw
        .iter()
        .map(|(key, r)| (bare_name(key), r.info.clone()))
        .collect();

    // ids are (view, path, kind); two sites producing the same finding (for
    // example two `pop` calls proposing the same edge) are one finding.
    let mut seen_ids = BTreeSet::new();
    findings.retain(|f| seen_ids.insert(f.id.clone()));
    ScanReport {
        inventory: Inventory { views, findings },
        shapes,
        types,
        io_modules,
        root_modules_pub,
    }
}

fn vm_tokens<'a>(
    raw: &'a BTreeMap<String, RawType>,
    index: &BTreeMap<String, Vec<String>>,
    vm: &str,
    scope: &str,
) -> &'a str {
    resolve_key(index, vm, scope).map_or("", |key| raw[key].tokens.as_str())
}

/// The bare type name of a qualified key (`a::B` -> `B`).
fn bare_name(key: &str) -> String {
    key.split_once("::")
        .map_or(key, |(_, name)| name)
        .to_string()
}

/// Resolve a bare type name inside `scope`: the type declared in the same
/// crate wins; a name unique across the workspace resolves to it; a name
/// declared in several other crates stays unresolved — never a guess.
fn resolve_key<'a>(
    index: &'a BTreeMap<String, Vec<String>>,
    name: &str,
    scope: &str,
) -> Option<&'a str> {
    let keys = index.get(name)?;
    let scoped = format!("{scope}::{name}");
    if let Some(k) = keys.iter().find(|k| **k == scoped) {
        return Some(k.as_str());
    }
    match keys.len() {
        1 => keys.first().map(String::as_str),
        _ => None,
    }
}

/// The type's serde limits that actually change the structural skeleton.
/// `rename`/`rename_all` on an enum of unit variants only renames wire
/// VALUES, and structural drift ignores scalar values — so such an enum
/// does not force hand-curated fixtures.
fn effective_limits(entry: &RawType) -> &[String] {
    let cosmetic = |l: &String| l == "rename" || l == "rename_all";
    if let RawKind::Enum(variants) = &entry.kind
        && variants.iter().all(|v| v.payload.is_none())
        && entry.limits.iter().all(cosmetic)
    {
        return &[];
    }
    &entry.limits
}

fn mk_finding(
    view: &str,
    kind: &str,
    symbol: &str,
    location: &str,
    tokens: &str,
    detail: String,
) -> Finding {
    Finding {
        id: format!("f-{}", &sha_hex(&[view, symbol, kind])[..6]),
        view: view.to_string(),
        kind: kind.to_string(),
        location: location.to_string(),
        fingerprint: sha_hex(&[tokens])[..16].to_string(),
        detail,
    }
}

/// The crate a qualified type key belongs to (`a::B` -> `a`).
fn owner_of(key: &str) -> &str {
    key.split_once("::").map_or(key, |(owner, _)| owner)
}

// ---------------------------------------------------------------------------
// file discovery

/// All `src/**/*.rs` files of every crate under `workspace`, sorted. Each
/// entry is (relative path, crate name, module path within the crate,
/// is-crate-root-file): module ids are qualified by crate so views in
/// different crates that share a module path are both kept.
fn host_source_files(workspace: &Path) -> Vec<(String, String, Vec<String>, bool)> {
    let mut crates: Vec<(PathBuf, String)> = Vec::new(); // (src dir, crate name)
    let mut visit = |dir: &Path| {
        let manifest = dir.join("Cargo.toml");
        if manifest.is_file()
            && let Some(name) = crate_name(&manifest)
        {
            let src = dir.join("src");
            if src.is_dir() {
                crates.push((src, name));
            }
        }
    };
    visit(workspace);
    walk_dirs(workspace, &mut visit);
    crates.sort();

    let mut out = Vec::new();
    for (src, krate) in &crates {
        // files stay workspace-relative (scan reads them at that path);
        // the module path is the part below the crate's `src/` directory
        let Ok(src_rel) = src.strip_prefix(workspace) else {
            continue;
        };
        let src_prefix = format!("{}/", src_rel.to_string_lossy().replace('\\', "/"));
        let mut rels = Vec::new();
        collect_rs(src, workspace, &mut rels);
        for rel in rels {
            let (within, crate_root) = match rel.strip_prefix(&src_prefix) {
                Some(in_crate) => (
                    module_within(in_crate),
                    matches!(in_crate, "lib.rs" | "main.rs" | "mod.rs"),
                ),
                None => (Vec::new(), false),
            };
            out.push((rel, krate.clone(), within, crate_root));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// `[package] name` of a manifest, if the file declares a package.
fn crate_name(manifest: &Path) -> Option<String> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let value: toml::Value = toml::from_str(&text).ok()?;
    let name = value.get("package")?.get("name")?.as_str()?;
    Some(name.to_string())
}

/// Directories the scanner never enters: build/test trees ([`SKIP_DIRS`]),
/// hidden directories (tool state such as `.superpowers/`, `.flatpak-builder/`)
/// and nested repositories or git worktrees (a `.git` file or directory),
/// whose code belongs to another checkout and would make the inventory
/// depend on where it was scanned.
fn skip_dir(path: &Path, name: &str) -> bool {
    SKIP_DIRS.contains(&name) || name.starts_with('.') || path.join(".git").exists()
}

/// Depth-first, entries sorted by name; `f` sees every directory.
fn walk_dirs(dir: &Path, f: &mut dyn FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if skip_dir(&path, name) {
            continue;
        }
        f(&path);
        walk_dirs(&path, f);
    }
}

fn collect_rs(dir: &Path, crate_root: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir()
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| !skip_dir(&path, name))
        {
            collect_rs(&path, crate_root, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs")
            && let Ok(rel) = path.strip_prefix(crate_root)
        {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// `views/create.rs` (relative to a crate's `src/`) -> `["views", "create"]`;
/// `lib.rs` -> `[]`. The crate name is prepended separately.
fn module_within(rel: &str) -> Vec<String> {
    let mut parts: Vec<String> = rel
        .trim_end_matches(".rs")
        .split('/')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    match parts.last().map(String::as_str) {
        Some("lib") | Some("main") | Some("mod") => {
            parts.pop();
            parts
        }
        _ => parts,
    }
}

// ---------------------------------------------------------------------------
// type collection

fn collect_types(
    items: &[Item],
    rel: &str,
    krate: &str,
    raw: &mut BTreeMap<String, RawType>,
    index: &mut BTreeMap<String, Vec<String>>,
) {
    for item in items {
        match item {
            Item::Struct(s) => {
                let name = s.ident.to_string();
                let key = format!("{krate}::{name}");
                if !raw.contains_key(&key) {
                    let fields = s.fields.iter().map(|f| raw_field(f, rel)).collect();
                    raw.insert(
                        key.clone(),
                        RawType {
                            info: type_info(&s.attrs, is_public(&s.vis), s.ident.span(), rel),
                            tokens: s.to_token_stream().to_string(),
                            kind: RawKind::Struct(fields),
                            limits: serde_limits(&s.attrs),
                            default_fields: has_serde_default(&s.attrs),
                        },
                    );
                    index.entry(name).or_default().push(key);
                }
            }
            Item::Enum(e) => {
                let name = e.ident.to_string();
                let key = format!("{krate}::{name}");
                if !raw.contains_key(&key) {
                    let variants = e
                        .variants
                        .iter()
                        .map(|v| RawVariant {
                            name: enum_variant_wire_name(&e.attrs, &v.attrs, &v.ident),
                            payload: match &v.fields {
                                Fields::Unnamed(fields) => match fields.unnamed.len() {
                                    1 => Some(RawPayload::One(
                                        fields.unnamed[0].ty.to_token_stream().to_string(),
                                    )),
                                    0 => None,
                                    _ => Some(RawPayload::Tuple(
                                        fields
                                            .unnamed
                                            .iter()
                                            .map(|f| f.ty.to_token_stream().to_string())
                                            .collect(),
                                    )),
                                },
                                Fields::Named(fields) => Some(RawPayload::Struct(
                                    fields.named.iter().map(|f| raw_field(f, rel)).collect(),
                                )),
                                Fields::Unit => None,
                            },
                        })
                        .collect();
                    raw.insert(
                        key.clone(),
                        RawType {
                            info: type_info(&e.attrs, is_public(&e.vis), e.ident.span(), rel),
                            tokens: e.to_token_stream().to_string(),
                            kind: RawKind::Enum(variants),
                            limits: serde_limits(&e.attrs),
                            default_fields: false,
                        },
                    );
                    index.entry(name).or_default().push(key);
                }
            }
            Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    collect_types(inner, rel, krate, raw, index);
                }
            }
            _ => {}
        }
    }
}

fn raw_field(f: &syn::Field, rel: &str) -> RawField {
    RawField {
        name: f.ident.as_ref().map(|i| i.to_string()).unwrap_or_default(),
        type_src: f.ty.to_token_stream().to_string(),
        location: format!("{rel}:{}", f.span().start().line),
        feature_gated: is_feature_gated(f),
        has_dyn: has_dyn(&f.ty),
        limits: serde_limits(&f.attrs),
        serde_default: has_serde_default(&f.attrs),
    }
}

fn type_info(
    attrs: &[syn::Attribute],
    public: bool,
    span: proc_macro2::Span,
    rel: &str,
) -> TypeInfo {
    let derive = |needle: &str| {
        attrs.iter().any(|a| {
            a.path().is_ident("derive") && a.to_token_stream().to_string().contains(needle)
        })
    };
    TypeInfo {
        public,
        serde: derive("Serialize"),
        de: derive("Deserialize"),
        clone_eq: derive("Clone") && derive("PartialEq"),
        location: format!("{rel}:{}", span.start().line),
    }
}

/// Which of [`SERDE_LIMITS`] an attribute set mentions.
fn serde_limits(attrs: &[syn::Attribute]) -> Vec<String> {
    let text = attrs
        .iter()
        .filter(|a| a.path().is_ident("serde"))
        .map(|a| a.to_token_stream().to_string())
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        return Vec::new();
    }
    SERDE_LIMITS
        .iter()
        .filter(|needle| text.contains(*needle))
        .map(|needle| (*needle).to_string())
        .collect()
}

/// The enum-level `#[serde(rename_all = "…")]` casing, if present.
fn serde_rename_all(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs.iter().filter(|a| a.path().is_ident("serde")) {
        let tokens = attr.to_token_stream().to_string();
        let Some((_, style)) = tokens.split_once("rename_all") else {
            continue;
        };
        let Some(style) = style.split('"').nth(1) else {
            continue;
        };
        if !style.is_empty() {
            return Some(style.to_string());
        }
    }
    None
}

/// A variant's wire name: the enum-level `rename_all` casing applied to
/// its identifier (serde renames every variant this way). A variant with
/// its own `rename` keeps the identifier: the rename limit already flags
/// the type, and a half-renamed skeleton would lie.
fn enum_variant_wire_name(
    enum_attrs: &[syn::Attribute],
    variant_attrs: &[syn::Attribute],
    ident: &syn::Ident,
) -> String {
    let Some(style) = serde_rename_all(enum_attrs) else {
        return ident.to_string();
    };
    if serde_rename(variant_attrs).is_some() {
        return ident.to_string();
    }
    // serde_derive's `RenameRule::apply_to_variant`, character for
    // character: variants are assumed PascalCase as written
    let ident = ident.to_string();
    let snake = || {
        let mut out = String::new();
        for (i, ch) in ident.char_indices() {
            if i > 0 && ch.is_uppercase() {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        }
        out
    };
    match style.as_str() {
        "lowercase" => ident.to_ascii_lowercase(),
        "UPPERCASE" => ident.to_ascii_uppercase(),
        "PascalCase" => ident,
        "camelCase" => ident[..1].to_ascii_lowercase() + &ident[1..],
        "snake_case" => snake(),
        "SCREAMING_SNAKE_CASE" => snake().to_ascii_uppercase(),
        "kebab-case" => snake().replace('_', "-"),
        "SCREAMING-KEBAB-CASE" => snake().to_ascii_uppercase().replace('_', "-"),
        // unknown style: serde rejects it at compile time; keep the
        // identifier and let the rename limit flag the type
        _ => ident,
    }
}

/// A variant-level `#[serde(rename = "…")]`, if present.
fn serde_rename(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs.iter().filter(|a| a.path().is_ident("serde")) {
        let tokens = attr.to_token_stream().to_string();
        if let Some((_, rest)) = tokens.split_once("rename")
            && !rest.trim_start().starts_with('_')
            && let Some(name) = rest.trim_start_matches(['=', ' ']).split('"').nth(1)
        {
            return Some(name.to_string());
        }
    }
    None
}

/// `#[serde(default)]` or `#[serde(default = "…")]`: the wire shape is
/// unchanged, the key may simply be absent — recorded as an optional
/// field, never a limit.
fn has_serde_default(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .filter(|a| a.path().is_ident("serde"))
        .any(|a| {
            let mut found = false;
            // `parse_nested_meta` walks `default`, `default = "f"`,
            // `rename = "default"` etc. by key, never by substring
            let _ = a.parse_nested_meta(|meta| {
                if meta.path.is_ident("default") {
                    found = true;
                }
                if meta.input.peek(syn::Token![=]) {
                    let _: syn::Expr = meta.value()?.parse()?;
                } else if meta.input.peek(syn::token::Paren) {
                    let _content;
                    syn::parenthesized!(_content in meta.input);
                }
                Ok(())
            });
            found
        })
}

fn is_public(vis: &syn::Visibility) -> bool {
    matches!(vis, syn::Visibility::Public(_))
}

fn is_feature_gated(field: &syn::Field) -> bool {
    field
        .attrs
        .iter()
        .any(|a| a.path().is_ident("cfg") && a.to_token_stream().to_string().contains("feature"))
}

fn has_dyn(ty: &syn::Type) -> bool {
    use syn::Type;
    match ty {
        Type::TraitObject(_) => true,
        Type::Reference(r) => has_dyn(&r.elem),
        Type::Paren(p) => has_dyn(&p.elem),
        // Group types only arise from macro expansion; parsing source never yields one
        Type::Slice(s) => has_dyn(&s.elem),
        Type::Array(a) => has_dyn(&a.elem),
        Type::Path(p) => p.path.segments.iter().any(|seg| match &seg.arguments {
            syn::PathArguments::AngleBracketed(args) => args.args.iter().any(|arg| match arg {
                syn::GenericArgument::Type(t) => has_dyn(t),
                _ => false,
            }),
            _ => false,
        }),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// shape resolution

struct Resolver<'a> {
    raw: &'a BTreeMap<String, RawType>,
    index: &'a BTreeMap<String, Vec<String>>,
    /// Crate the type currently being resolved is declared in; bare field
    /// type names resolve there first.
    scope: String,
    memo_named: BTreeMap<String, TypeShape>,
    memo_src: BTreeMap<String, TypeShape>,
    /// Qualified type key -> its resolution touches a type with serde limits.
    limited_closure: &'a BTreeMap<String, bool>,
}

impl Resolver<'_> {
    fn named(&mut self, name: &str, stack: &mut Vec<String>) -> TypeShape {
        let memo_key = format!("{}\u{0}{name}", self.scope);
        if let Some(shape) = self.memo_named.get(&memo_key) {
            return shape.clone();
        }
        if stack.iter().any(|s| s == name) {
            return TypeShape::Unresolved;
        }
        let Some(key) = resolve_key(self.index, name, &self.scope) else {
            return TypeShape::Unresolved;
        };
        let raw = &self.raw[key];
        let outer = std::mem::replace(&mut self.scope, owner_of(key).to_string());
        stack.push(name.to_string());
        let shape = self.raw_shape(raw, stack);
        stack.pop();
        self.scope = outer;
        self.memo_named.insert(memo_key, shape.clone());
        shape
    }

    fn src(&mut self, src: &str, stack: &mut Vec<String>) -> TypeShape {
        let memo_key = format!("{}\u{0}{src}", self.scope);
        if let Some(shape) = self.memo_src.get(&memo_key) {
            return shape.clone();
        }
        let shape = match syn::parse_str::<syn::Type>(src) {
            Ok(ty) => self.type_shape(&ty, stack),
            Err(_) => TypeShape::Unresolved,
        };
        self.memo_src.insert(memo_key, shape.clone());
        shape
    }

    fn raw_shape(&mut self, raw: &RawType, stack: &mut Vec<String>) -> TypeShape {
        match &raw.kind {
            RawKind::Struct(fields) => TypeShape::Struct {
                fields: fields
                    .iter()
                    .map(|f| self.field_info(f, raw.default_fields, stack))
                    .collect(),
            },
            RawKind::Enum(variants) => TypeShape::Enum {
                variants: variants
                    .iter()
                    .map(|v| VariantInfo {
                        name: v.name.clone(),
                        payload: match &v.payload {
                            Some(RawPayload::One(src)) => Some(self.src(src, stack)),
                            Some(RawPayload::Tuple(srcs)) => Some(TypeShape::Tuple(
                                srcs.iter().map(|s| self.src(s, stack)).collect(),
                            )),
                            Some(RawPayload::Struct(fields)) => Some(TypeShape::Struct {
                                fields: fields
                                    .iter()
                                    .map(|f| self.field_info(f, false, stack))
                                    .collect(),
                            }),
                            None => None,
                        },
                    })
                    .collect(),
            },
        }
    }

    fn field_info(
        &mut self,
        f: &RawField,
        struct_default: bool,
        stack: &mut Vec<String>,
    ) -> FieldInfo {
        let shape = self.src(&f.type_src, stack);
        FieldInfo {
            name: f.name.clone(),
            shape,
            location: f.location.clone(),
            feature_gated: f.feature_gated,
            has_dyn: f.has_dyn,
            limited: self.field_limit(f),
            optional: f.serde_default || struct_default,
        }
    }

    /// Why a field's wire representation cannot be modeled, if it cannot.
    fn field_limit(&self, f: &RawField) -> Option<String> {
        field_limit(self.index, &self.scope, self.limited_closure, f)
    }

    fn type_shape(&mut self, ty: &syn::Type, stack: &mut Vec<String>) -> TypeShape {
        use syn::Type;
        match ty {
            Type::Reference(r) => self.type_shape(&r.elem, stack),
            Type::Paren(p) => self.type_shape(&p.elem, stack),
            // Group types only arise from macro expansion; parsing source never yields one
            Type::Slice(s) => TypeShape::Collection(Box::new(self.type_shape(&s.elem, stack))),
            Type::Array(a) => match array_len(a) {
                Some(len) => TypeShape::Array {
                    item: Box::new(self.type_shape(&a.elem, stack)),
                    len,
                },
                None => TypeShape::Unresolved,
            },
            Type::Tuple(t) if t.elems.is_empty() => TypeShape::Unit,
            Type::Tuple(t) => {
                TypeShape::Tuple(t.elems.iter().map(|e| self.type_shape(e, stack)).collect())
            }
            Type::Path(p) => self.path_shape(p, stack),
            _ => TypeShape::Unresolved,
        }
    }

    fn path_shape(&mut self, p: &syn::TypePath, stack: &mut Vec<String>) -> TypeShape {
        let Some(last) = p.path.segments.last() else {
            return TypeShape::Unresolved;
        };
        let ident = last.ident.to_string();
        let args: Vec<String> = match &last.arguments {
            syn::PathArguments::AngleBracketed(a) => a
                .args
                .iter()
                .filter_map(|arg| match arg {
                    syn::GenericArgument::Type(t) => Some(t.to_token_stream().to_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let mut one = |i: usize, me: &mut Self| -> TypeShape {
            match args.get(i) {
                Some(src) => me.src(src, stack),
                None => TypeShape::Unresolved,
            }
        };
        // Generic containers always carry their type arguments in valid
        // Rust; a missing one resolves to Unresolved through `one`.
        match ident.as_str() {
            "bool" => TypeShape::Bool,
            "char" => TypeShape::Char,
            "String" | "str" => TypeShape::Text,
            n if INT_NUMBERS.contains(&n) => TypeShape::Number { int: true },
            n if FLOAT_NUMBERS.contains(&n) => TypeShape::Number { int: false },
            "Option" => TypeShape::Option(Box::new(one(0, self))),
            "Vec" | "HashSet" | "BTreeSet" => TypeShape::Collection(Box::new(one(0, self))),
            "HashMap" | "BTreeMap" => TypeShape::Map {
                key: Box::new(one(0, self)),
                value: Box::new(one(1, self)),
            },
            "Result" => TypeShape::Phase {
                ok: Box::new(one(0, self)),
                err: Box::new(one(1, self)),
            },
            "Rc" | "Arc" | "Box" => one(0, self),
            "Cow" => match args.last() {
                Some(src) => self.src(src, stack),
                None => TypeShape::Unresolved,
            },
            _ => self.named(&ident, stack),
        }
    }
}

/// `[T; N]` with a literal length.
fn array_len(a: &syn::TypeArray) -> Option<usize> {
    match &a.len {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Int(len),
            ..
        }) => len.base10_parse().ok(),
        _ => None,
    }
}

/// Why a field's wire representation cannot be modeled, if it cannot.
fn field_limit(
    index: &BTreeMap<String, Vec<String>>,
    scope: &str,
    limited_closure: &BTreeMap<String, bool>,
    f: &RawField,
) -> Option<String> {
    if !f.limits.is_empty() {
        return Some(format!("field attribute(s) {}", f.limits.join(", ")));
    }
    idents_of(&f.type_src).into_iter().find_map(|ident| {
        let key = resolve_key(index, &ident, scope)?;
        limited_closure.get(key).is_some_and(|b| *b).then(|| {
            format!("its type {ident} has serde attribute(s) the extractor does not model")
        })
    })
}

/// Identifiers mentioned by a type source (`HashMap < String , u32 >` ->
/// both idents), in order of appearance.
fn idents_of(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in src.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            current.push(ch);
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Qualified type key -> does resolving it (transitively) touch a type that
/// carries serde limits? Fields of such types get no generated fixture value.
fn limited_reachability(
    raw: &BTreeMap<String, RawType>,
    index: &BTreeMap<String, Vec<String>>,
) -> BTreeMap<String, bool> {
    let mut out: BTreeMap<String, bool> = BTreeMap::new();
    for key in raw.keys() {
        reach_limited(key, raw, index, &mut out);
    }
    out
}

fn reach_limited(
    key: &str,
    raw: &BTreeMap<String, RawType>,
    index: &BTreeMap<String, Vec<String>>,
    memo: &mut BTreeMap<String, bool>,
) -> bool {
    if let Some(cached) = memo.get(key) {
        return *cached;
    }
    memo.insert(key.to_string(), false); // cycle guard
    let Some(entry) = raw.get(key) else {
        return false;
    };
    let scope = owner_of(key);
    let limited = !effective_limits(entry).is_empty()
        || type_srcs(&entry.kind)
            .iter()
            .flat_map(|src| idents_of(src))
            .filter_map(|ident| resolve_key(index, &ident, scope))
            .any(|next| reach_limited(next, raw, index, memo));
    memo.insert(key.to_string(), limited);
    limited
}

/// Every type source string mentioned by a raw type's fields or payloads.
fn type_srcs(kind: &RawKind) -> Vec<String> {
    match kind {
        RawKind::Struct(fields) => fields.iter().map(|f| f.type_src.clone()).collect(),
        RawKind::Enum(variants) => variants
            .iter()
            .filter_map(|v| v.payload.as_ref())
            .flat_map(|payload| match payload {
                RawPayload::One(src) => vec![src.clone()],
                RawPayload::Tuple(srcs) => srcs.clone(),
                RawPayload::Struct(fields) => fields.iter().map(|f| f.type_src.clone()).collect(),
            })
            .collect(),
    }
}

/// Every named type reachable from `start` (including itself), following
/// the identifiers each declaration mentions; bare names resolve in the
/// declaring crate of the type that mentions them.
fn reachable_types(
    raw: &BTreeMap<String, RawType>,
    index: &BTreeMap<String, Vec<String>>,
    start: Option<&str>,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(start) = start else {
        return out;
    };
    let mut stack = vec![start.to_string()];
    while let Some(key) = stack.pop() {
        if !out.insert(key.clone()) {
            continue;
        }
        if let Some(entry) = raw.get(&key) {
            let scope = owner_of(&key);
            for ident in type_srcs(&entry.kind).iter().flat_map(|src| idents_of(src)) {
                if let Some(next) = resolve_key(index, &ident, scope)
                    && !out.contains(next)
                {
                    stack.push(next.to_string());
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// I/O census + root module visibility

fn file_has_io(ast: &File) -> bool {
    struct IoVisit {
        io: bool,
    }
    impl Visit<'_> for IoVisit {
        fn visit_path(&mut self, p: &syn::Path) {
            if p.segments.len() >= 2
                && p.segments[0].ident == "std"
                && IO_STD_MODULES.contains(&p.segments[1].ident.to_string().as_str())
            {
                self.io = true;
            }
            visit::visit_path(self, p);
        }
        fn visit_item_use(&mut self, u: &syn::ItemUse) {
            if use_io(&u.tree, false) {
                self.io = true;
            }
            visit::visit_item_use(self, u);
        }
    }
    let mut v = IoVisit { io: false };
    v.visit_file(ast);
    v.io
}

fn use_io(tree: &UseTree, under_std: bool) -> bool {
    match tree {
        UseTree::Path(p) => {
            let name = p.ident.to_string();
            if under_std && IO_STD_MODULES.contains(&name.as_str()) {
                return true;
            }
            use_io(&p.tree, under_std || name == "std")
        }
        UseTree::Name(n) => under_std && IO_STD_MODULES.contains(&n.ident.to_string().as_str()),
        UseTree::Group(g) => g.items.iter().any(|t| use_io(t, under_std)),
        UseTree::Rename(r) => under_std && IO_STD_MODULES.contains(&r.ident.to_string().as_str()),
        UseTree::Glob(_) => false,
    }
}

fn collect_root_mods(items: &[Item], out: &mut BTreeMap<String, bool>) {
    for item in items {
        if let Item::Mod(m) = item {
            out.entry(m.ident.to_string()).or_insert(is_public(&m.vis));
        }
    }
}

// ---------------------------------------------------------------------------
// view discovery

fn type_name_of(ty: &syn::Type) -> Option<String> {
    let path = match ty {
        syn::Type::Reference(r) => match &*r.elem {
            syn::Type::Path(p) => p,
            _ => return None,
        },
        syn::Type::Path(p) => p,
        _ => return None,
    };
    path.path.segments.last().map(|s| s.ident.to_string())
}

fn fn_param(f: &ItemFn, index: usize) -> Option<(String, String)> {
    let pat_type = f.sig.inputs.iter().nth(index)?;
    let syn::FnArg::Typed(pt) = pat_type else {
        return None;
    };
    let syn::Pat::Ident(pi) = &*pt.pat else {
        return None;
    };
    let ty = type_name_of(&pt.ty)?;
    Some((pi.ident.to_string(), ty))
}

fn returns_widget(output: &syn::ReturnType) -> bool {
    let syn::ReturnType::Type(_, ty) = output else {
        return false;
    };
    let is_widget = |t: &syn::Type| matches!(t, syn::Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "Widget"));
    match &**ty {
        t if is_widget(t) => true,
        syn::Type::Tuple(t) => t.elems.first().is_some_and(is_widget),
        _ => false,
    }
}

fn discover_views(
    file: &AstFile,
    seen: &mut BTreeSet<String>,
    seen_ids: &mut BTreeSet<String>,
) -> Vec<ViewCandidate> {
    let mut out = Vec::new();
    walk_fns(&file.ast.items, &mut Vec::new(), &mut |f, mod_path| {
        if !matches!(f.vis, syn::Visibility::Public(_))
            || !VIEW_FNS.contains(&f.sig.ident.to_string().as_str())
        {
            return;
        }
        let krate = file.module_base[0].clone();
        let module = file
            .module_base
            .iter()
            .chain(mod_path.iter())
            .cloned()
            .collect::<Vec<_>>()
            .join("::");
        if seen.contains(&module) {
            return;
        }
        let Some((_, vm_type)) = fn_param(f, 0) else {
            return;
        };
        let Some((_, actions_type)) = fn_param(f, 1) else {
            return;
        };
        if !returns_widget(&f.sig.output) {
            return;
        }
        let id = module.rsplit("::").next().unwrap_or_default().to_string();
        let id = if id.is_empty() || id == krate {
            f.sig
                .ident
                .to_string()
                .trim_start_matches("build")
                .trim_matches('_')
                .to_string()
        } else {
            id
        };
        if id.is_empty() {
            return;
        }
        // Two crates may end at the same module tail; the later one in scan
        // order gets a crate-prefixed id so states/ and fixtures/ stay
        // one-directory-per-view.
        let id = if seen_ids.contains(&id) {
            format!("{krate}-{id}")
        } else {
            id
        };
        seen.insert(module.clone());
        seen_ids.insert(id.clone());
        out.push(ViewCandidate {
            module,
            id,
            vm: vm_type,
            actions: actions_type,
            fn_tokens: f.to_token_stream().to_string(),
            fn_location: format!("{}:{}", file.rel, f.sig.ident.span().start().line),
        });
    });
    out
}

/// Walk items (and inline modules) calling `f` with every function item.
fn walk_fns<'a>(
    items: &'a [Item],
    mod_path: &mut Vec<String>,
    f: &mut impl FnMut(&'a ItemFn, &[String]),
) {
    for item in items {
        match item {
            Item::Fn(fun) => f(fun, mod_path),
            Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    mod_path.push(m.ident.to_string());
                    walk_fns(inner, mod_path, f);
                    mod_path.pop();
                }
            }
            _ => {}
        }
    }
}
// ---------------------------------------------------------------------------
// projection: which VM fields are read

fn project_file(file: &AstFile, vm: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut reads: BTreeSet<String> = BTreeSet::new();
    let mut inner: BTreeSet<String> = BTreeSet::new();
    walk_fns(&file.ast.items, &mut Vec::new(), &mut |f, _| {
        let roots: Vec<String> = (0..f.sig.inputs.len())
            .filter_map(|i| fn_param(f, i))
            .filter(|(_, ty)| ty == vm)
            .map(|(ident, _)| ident)
            .collect();
        if roots.is_empty() {
            return;
        }
        let mut proj = Proj {
            roots: &roots,
            binds: BTreeMap::new(),
            reads: BTreeSet::new(),
            inner: BTreeSet::new(),
        };
        proj.visit_item_fn(f);
        reads.extend(proj.reads);
        inner.extend(proj.inner);
    });
    (reads, inner)
}

struct Proj<'a> {
    roots: &'a [String],
    binds: BTreeMap<String, String>,
    reads: BTreeSet<String>,
    inner: BTreeSet<String>,
}

impl<'a> Visit<'a> for Proj<'a> {
    fn visit_expr_field(&mut self, field: &'a ExprField) {
        if let Some(prefix) = self.dotted(&field.base)
            && let Member::Named(ident) = &field.member
        {
            let path = if prefix.is_empty() {
                ident.to_string()
            } else {
                format!("{prefix}.{ident}")
            };
            self.reads.insert(path);
        }
        visit::visit_expr_field(self, field);
    }

    fn visit_expr_path(&mut self, p: &'a ExprPath) {
        if let Some(id) = p.path.get_ident().map(|i| i.to_string())
            && let Some(path) = self.binds.get(&id)
        {
            self.inner.insert(path.clone());
        }
        visit::visit_expr_path(self, p);
    }

    fn visit_expr_let(&mut self, e: &'a syn::ExprLet) {
        self.bind_pattern(&e.pat, Some(&e.expr));
        visit::visit_expr_let(self, e);
    }

    fn visit_expr_match(&mut self, m: &'a syn::ExprMatch) {
        if let Some(path) = self.dotted(&m.expr)
            && !path.is_empty()
        {
            for arm in &m.arms {
                bind_pat_idents(&arm.pat, &path, &mut self.binds);
            }
        }
        visit::visit_expr_match(self, m);
    }

    fn visit_expr_for_loop(&mut self, f: &'a syn::ExprForLoop) {
        self.bind_pattern(&f.pat, Some(&f.expr));
        visit::visit_expr_for_loop(self, f);
    }

    fn visit_local(&mut self, l: &'a syn::Local) {
        if let Some(init) = &l.init {
            self.bind_pattern(&l.pat, Some(&init.expr));
        }
        visit::visit_local(self, l);
    }
}

impl Proj<'_> {
    fn bind_pattern(&mut self, pat: &Pat, expr: Option<&Expr>) {
        if let Some(expr) = expr
            && let Some(path) = self.dotted(expr)
            && !path.is_empty()
        {
            bind_pat_idents(pat, &path, &mut self.binds);
        }
    }

    /// Dotted VM path of an expression rooted at a VM-typed binding.
    fn dotted(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Field(f) => {
                let prefix = self.dotted(&f.base)?;
                match &f.member {
                    Member::Named(ident) => Some(if prefix.is_empty() {
                        ident.to_string()
                    } else {
                        format!("{prefix}.{ident}")
                    }),
                    Member::Unnamed(_) => None,
                }
            }
            Expr::Path(p) => {
                let id = p.path.get_ident()?.to_string();
                if self.roots.contains(&id) {
                    Some(String::new())
                } else {
                    self.binds.get(&id).cloned()
                }
            }
            Expr::Paren(p) => self.dotted(&p.expr),
            Expr::Reference(r) => self.dotted(&r.expr),
            _ => None,
        }
    }
}

fn bind_pat_idents(pat: &Pat, path: &str, binds: &mut BTreeMap<String, String>) {
    match pat {
        Pat::Ident(pi) => {
            if !matches!(pi.ident, ref i if i == "_") {
                binds.insert(pi.ident.to_string(), path.to_string());
            }
        }
        Pat::TupleStruct(ts) => ts
            .elems
            .iter()
            .for_each(|e| bind_pat_idents(e, path, binds)),
        Pat::Tuple(t) => t.elems.iter().for_each(|e| bind_pat_idents(e, path, binds)),
        Pat::Struct(s) => s
            .fields
            .iter()
            .for_each(|f| bind_pat_idents(&f.pat, path, binds)),
        Pat::Reference(r) => bind_pat_idents(&r.pat, path, binds),
        // slice patterns bind collection elements; size classes never depend on inner reads
        Pat::Or(o) => o.cases.iter().for_each(|e| bind_pat_idents(e, path, binds)),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// axes + field findings

fn finite_classes(shape: &TypeShape) -> Option<Vec<String>> {
    match shape {
        TypeShape::Bool => Some(vec!["false".into(), "true".into()]),
        TypeShape::Enum { variants } => Some(variants.iter().map(|v| v.name.clone()).collect()),
        TypeShape::Phase { .. } => Some(vec!["ok".into(), "err".into()]),
        _ => None,
    }
}

/// Axis for a read field; `None` when the type cannot be resolved.
/// A phase axis comes only from `Result` — an enum stays an enum with its
/// variants as classes, whatever they spell (`references/discovery.md`).
fn axis_of(shape: &TypeShape, structurally_read: bool) -> Option<(AxisKind, Vec<String>)> {
    let (kind, classes) = match shape {
        TypeShape::Bool => (AxisKind::Bool, vec!["false".into(), "true".into()]),
        TypeShape::Char | TypeShape::Text => (AxisKind::Text, vec!["any".into()]),
        TypeShape::Number { .. } => (AxisKind::Number, vec!["any".into()]),
        TypeShape::Unit => (AxisKind::Struct, vec!["any".into()]),
        TypeShape::Struct { .. } => (AxisKind::Struct, vec!["any".into()]),
        TypeShape::Tuple(_) | TypeShape::Array { .. } => (AxisKind::Struct, vec!["any".into()]),
        TypeShape::Collection { .. } | TypeShape::Map { .. } => (
            AxisKind::Collection,
            vec!["empty".into(), "one".into(), "many".into()],
        ),
        TypeShape::Phase { .. } => (AxisKind::Phase, vec!["ok".into(), "err".into()]),
        TypeShape::Enum { variants } => {
            let names: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();
            (
                AxisKind::Enum {
                    variants: names.clone(),
                },
                names,
            )
        }
        TypeShape::Option(inner) => {
            let mut classes = vec!["none".into()];
            match if structurally_read {
                finite_classes(inner)
            } else {
                None
            } {
                Some(extra) => classes.extend(extra),
                None => classes.push("some".into()),
            }
            (AxisKind::Option, classes)
        }
        TypeShape::Unresolved => return None,
    };
    Some((kind, classes))
}

#[allow(clippy::too_many_arguments)]
fn walk_fields(
    fields: &[FieldInfo],
    prefix: &str,
    reads: &BTreeSet<String>,
    inner: &BTreeSet<String>,
    view: &str,
    tokens: &str,
    axes: &mut Vec<Axis>,
    unread: &mut Vec<String>,
    findings: &mut Vec<Finding>,
) {
    for field in fields {
        let path = if prefix.is_empty() {
            field.name.clone()
        } else {
            format!("{prefix}.{}", field.name)
        };
        if field.feature_gated {
            findings.push(mk_finding(
                view,
                K_FEATURE,
                &path,
                &field.location,
                tokens,
                "field is behind #[cfg(feature)]".into(),
            ));
            continue;
        }
        if let Some(limit) = &field.limited {
            findings.push(mk_finding(
                view,
                K_EXTRACTOR_LIMIT,
                &path,
                &field.location,
                tokens,
                format!("field '{path}' has {limit}; no fixture value is generated for it"),
            ));
            continue;
        }
        if field.has_dyn {
            findings.push(mk_finding(
                view,
                K_DYNAMIC,
                &path,
                &field.location,
                tokens,
                format!("field '{path}' holds a trait object (dyn) in its type"),
            ));
            continue;
        }
        let read_below_prefix = format!("{path}.");
        let read_here = reads.contains(&path);
        let read_below = reads.iter().any(|r| r.starts_with(&read_below_prefix));
        if !read_here && !read_below {
            unread.push(path.clone());
            findings.push(mk_finding(
                view,
                K_UNREAD,
                &path,
                &field.location,
                tokens,
                format!("field '{path}' is never read in build/update"),
            ));
            continue;
        }
        if let TypeShape::Struct { fields: sub } = &field.shape
            && read_below
        {
            walk_fields(
                sub, &path, reads, inner, view, tokens, axes, unread, findings,
            );
            continue;
        }
        match axis_of(&field.shape, inner.contains(&path)) {
            Some((kind, classes)) => axes.push(Axis {
                path,
                kind,
                classes,
            }),
            None => findings.push(mk_finding(
                view,
                K_UNRESOLVED,
                &path,
                &field.location,
                tokens,
                format!("type of field '{path}' cannot be resolved to an axis"),
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// view-function findings: widget-local state, macros, edge candidates

struct LocalCand {
    ident: String,
    line: u32,
}

fn scan_view_fns(file: &AstFile, cand: &ViewCandidate, findings: &mut Vec<Finding>) {
    walk_fns(&file.ast.items, &mut Vec::new(), &mut |f, _| {
        let name = f.sig.ident.to_string();
        let is_view_fn = VIEW_FNS.contains(&name.as_str()) || name == "update";
        if !is_view_fn {
            return;
        }
        let has_vm = (0..f.sig.inputs.len())
            .filter_map(|i| fn_param(f, i))
            .any(|(_, ty)| ty == cand.vm);
        if !has_vm {
            return;
        }
        let has_actions = (0..f.sig.inputs.len())
            .filter_map(|i| fn_param(f, i))
            .any(|(_, ty)| ty == cand.actions);

        // widget-local candidates: `let mut` or a Cell/RefCell initializer
        let mut cands: Vec<LocalCand> = Vec::new();
        for stmt in &f.block.stmts {
            if let syn::Stmt::Local(local) = stmt
                && let syn::Pat::Ident(pi) = &local.pat
            {
                let interesting = pi.mutability.is_some()
                    || local.init.as_ref().is_some_and(|i| is_cell_new(&i.expr));
                if interesting {
                    cands.push(LocalCand {
                        ident: pi.ident.to_string(),
                        line: local.span().start().line as u32,
                    });
                }
            }
        }
        for cand_local in &cands {
            findings.push(mk_finding(
                &cand.id,
                K_WIDGET_LOCAL,
                &cand_local.ident,
                &format!("{}:{}", file.rel, cand_local.line),
                &cand.fn_tokens,
                format!(
                    "local '{}' is state held outside the view model",
                    cand_local.ident
                ),
            ));
        }

        let mut scan = FnScan {
            view: &cand.id,
            file: &file.rel,
            tokens: &cand.fn_tokens,
            has_actions,
            findings,
        };
        scan.visit_item_fn(f);
    });
}

/// `Cell::new(..)` / `RefCell::new(..)` (any path form).
fn is_cell_new(expr: &Expr) -> bool {
    let Expr::Call(call) = expr else { return false };
    let Expr::Path(p) = &*call.func else {
        return false;
    };
    let segs = &p.path.segments;
    segs.len() >= 2
        && segs.last().is_some_and(|s| s.ident == "new")
        && matches!(
            segs[segs.len() - 2].ident.to_string().as_str(),
            "Cell" | "RefCell"
        )
}

struct FnScan<'a> {
    view: &'a str,
    file: &'a str,
    tokens: &'a str,
    has_actions: bool,
    findings: &'a mut Vec<Finding>,
}

impl<'a> Visit<'a> for FnScan<'a> {
    fn visit_expr_method_call(&mut self, m: &'a syn::ExprMethodCall) {
        let name = m.method.to_string();
        if self.has_actions
            && (NAV_METHODS.contains(&name.as_str()) || name.starts_with(NAV_METHOD_PREFIX))
        {
            let guess = match m.args.first() {
                Some(Expr::Lit(lit)) => match &lit.lit {
                    syn::Lit::Str(s) => s.value(),
                    _ => root_ident_of(&m.receiver)
                        .map(|r| snake_case(&r))
                        .unwrap_or_else(|| "unknown".into()),
                },
                _ => root_ident_of(&m.receiver)
                    .map(|r| snake_case(&r))
                    .unwrap_or_else(|| "unknown".into()),
            };
            self.findings.push(mk_finding(
                self.view,
                K_EDGE,
                &format!("{name}:{guess}"),
                &format!("{}:{}", self.file, m.span().start().line),
                self.tokens,
                format!("{name} → {guess}"),
            ));
        }
        visit::visit_expr_method_call(self, m);
    }

    fn visit_expr_macro(&mut self, m: &'a syn::ExprMacro) {
        self.macro_finding(&m.mac);
        visit::visit_expr_macro(self, m);
    }

    fn visit_stmt_macro(&mut self, m: &'a syn::StmtMacro) {
        self.macro_finding(&m.mac);
        visit::visit_stmt_macro(self, m);
    }
}

impl FnScan<'_> {
    fn macro_finding(&mut self, mac: &syn::Macro) {
        let Some(name) = mac.path.segments.last().map(|s| s.ident.to_string()) else {
            return;
        };
        if MACRO_ALLOWLIST.contains(&name.as_str()) {
            return;
        }
        self.findings.push(mk_finding(
            self.view,
            K_MACRO,
            &name,
            &format!("{}:{}", self.file, mac.span().start().line),
            self.tokens,
            format!("macro invocation cannot be inspected: {name}"),
        ));
    }
}

fn root_ident_of(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Path(p) => p.path.segments.first().map(|s| s.ident.to_string()),
        Expr::Call(c) => root_ident_of(&c.func),
        Expr::MethodCall(m) => root_ident_of(&m.receiver),
        Expr::Field(f) => root_ident_of(&f.base),
        Expr::Reference(r) => root_ident_of(&r.expr),
        Expr::Paren(p) => root_ident_of(&p.expr),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn index_of(raw: &BTreeMap<String, RawType>) -> BTreeMap<String, Vec<String>> {
        raw.keys()
            .map(|k| (bare_name(k), vec![k.clone()]))
            .collect()
    }

    fn plain(kind: RawKind) -> RawType {
        RawType {
            info: TypeInfo {
                public: true,
                serde: false,
                de: false,
                clone_eq: false,
                location: String::new(),
            },
            tokens: String::new(),
            kind,
            limits: Vec::new(),
            default_fields: false,
        }
    }

    fn struct_of(fields: &[(&str, &str)]) -> RawKind {
        RawKind::Struct(
            fields
                .iter()
                .map(|(name, type_src)| RawField {
                    name: (*name).to_string(),
                    type_src: (*type_src).to_string(),
                    location: String::new(),
                    feature_gated: false,
                    has_dyn: false,
                    limits: Vec::new(),
                    serde_default: false,
                })
                .collect(),
        )
    }

    #[test]
    fn idents_of_splits_on_non_ident_chars_and_keeps_underscores() {
        assert_eq!(
            idents_of("HashMap<String, u32>"),
            vec!["HashMap", "String", "u32"]
        );
        assert_eq!(idents_of("a_b c"), vec!["a_b", "c"]);
        assert_eq!(idents_of("Vec < ( u8 , u8 ) >"), vec!["Vec", "u8", "u8"]);
        assert!(idents_of("   ").is_empty());
    }

    #[test]
    fn reach_limited_follows_only_limited_named_types() {
        let mut raw = BTreeMap::new();
        raw.insert(
            "Outer".to_string(),
            plain(struct_of(&[("inner", "Plain"), ("prim", "u32")])),
        );
        raw.insert("Plain".to_string(), plain(struct_of(&[("x", "bool")])));
        let mut tagged = plain(struct_of(&[]));
        tagged.limits.push("rename_all".to_string());
        raw.insert("Tagged".to_string(), tagged);
        raw.insert(
            "UsesTagged".to_string(),
            plain(struct_of(&[("t", "Tagged")])),
        );
        let index = index_of(&raw);
        let mut memo = BTreeMap::new();
        assert!(!reach_limited("Outer", &raw, &index, &mut memo));
        assert!(!reach_limited("Plain", &raw, &index, &mut memo));
        assert!(reach_limited("Tagged", &raw, &index, &mut memo));
        assert!(reach_limited("UsesTagged", &raw, &index, &mut memo));
    }

    #[test]
    fn reachable_types_sticks_to_declared_named_types() {
        let mut raw = BTreeMap::new();
        raw.insert(
            "A".to_string(),
            plain(struct_of(&[("b", "B"), ("n", "u32")])),
        );
        raw.insert("B".to_string(), plain(struct_of(&[("s", "String")])));
        let index = index_of(&raw);
        let out = reachable_types(&raw, &index, Some("A"));
        assert_eq!(out, BTreeSet::from(["A".to_string(), "B".to_string()]));
    }

    #[test]
    fn unit_enum_variants_carry_no_payload() {
        let ast = syn::parse_file(
            "pub enum E { Unit, Paren(u8), Two(u8, u8), Named { x: bool }, Void() }",
        )
        .unwrap();
        let mut raw = BTreeMap::new();
        let mut index = BTreeMap::new();
        collect_types(&ast.items, "src/lib.rs", "crate", &mut raw, &mut index);
        let RawKind::Enum(variants) = &raw["crate::E"].kind else {
            panic!("E is not an enum: {:?}", raw["E"].kind);
        };
        let payloads: Vec<(&str, bool)> = variants
            .iter()
            .map(|v| (v.name.as_str(), v.payload.is_some()))
            .collect();
        // `Void()` has unnamed fields but zero of them: no payload either
        assert_eq!(
            payloads,
            vec![
                ("Unit", false),
                ("Paren", true),
                ("Two", true),
                ("Named", true),
                ("Void", false)
            ]
        );
    }

    #[test]
    fn serde_default_is_matched_by_key_not_substring() {
        let attrs = |src: &str| syn::parse_str::<syn::DeriveInput>(src).unwrap().attrs;
        assert!(has_serde_default(&attrs("#[serde(default)] struct A;")));
        assert!(has_serde_default(&attrs(
            "#[serde(rename = \"x\", default = \"make\")] struct A;"
        )));
        assert!(!has_serde_default(&attrs(
            "#[serde(skip_serializing_if = \"is_default\")] struct A;"
        )));
        assert!(!has_serde_default(&attrs(
            "#[serde(rename = \"default\")] struct A;"
        )));
        assert!(!has_serde_default(&attrs("#[derive(Default)] struct A;")));
    }

    fn variant_attrs_of(src: &str) -> Vec<syn::Attribute> {
        let ast = syn::parse_str::<syn::DeriveInput>(src).unwrap();
        let syn::Data::Enum(data) = ast.data else {
            panic!("not an enum: {src}");
        };
        data.variants.into_iter().next().unwrap().attrs
    }

    fn wire_name(enum_src: &str, variant_src: &str, ident: &str) -> String {
        let attrs = syn::parse_str::<syn::DeriveInput>(enum_src).unwrap().attrs;
        let ident = syn::parse_str::<syn::Ident>(ident).unwrap();
        enum_variant_wire_name(&attrs, &variant_attrs_of(variant_src), &ident)
    }

    #[test]
    fn wire_names_follow_every_rename_all_style() {
        let cases = [
            ("lowercase", "copyall", "link", "copy_all"),
            ("UPPERCASE", "COPYALL", "LINK", "COPY_ALL"),
            ("PascalCase", "CopyAll", "Link", "copy_all"),
            ("camelCase", "copyAll", "link", "copy_all"),
            ("snake_case", "copy_all", "link", "copy_all"),
            ("SCREAMING_SNAKE_CASE", "COPY_ALL", "LINK", "COPY_ALL"),
            ("kebab-case", "copy-all", "link", "copy-all"),
            ("SCREAMING-KEBAB-CASE", "COPY-ALL", "LINK", "COPY-ALL"),
        ];
        for (style, multi, single, snake) in cases {
            let src = format!("#[serde(rename_all = \"{style}\")] enum E {{ CopyAll, Link }}");
            let variants = "enum E { CopyAll }".to_string();
            assert_eq!(wire_name(&src, &variants, "CopyAll"), multi, "{style}");
            assert_eq!(wire_name(&src, &variants, "Link"), single, "{style}");
            // a snake_case ident: serde applies the rule per character,
            // underscores are kept (no word model)
            assert_eq!(wire_name(&src, &variants, "copy_all"), snake, "{style}");
        }
    }

    #[test]
    fn a_variant_with_its_own_rename_keeps_the_identifier() {
        // the variant's `rename` wins over the enum's `rename_all` styling
        // and the wire name falls back to the raw identifier
        let src = "#[serde(rename_all = \"lowercase\")] enum E { #[serde(rename = \"on-the-wire\")] CopyAll }";
        let variants = "enum E { #[serde(rename = \"on-the-wire\")] CopyAll }";
        assert_eq!(wire_name(src, variants, "CopyAll"), "CopyAll");
    }

    #[test]
    fn rename_all_on_a_variant_attr_is_not_a_rename() {
        // no enum-level style: identifier untouched either way
        let plain = "enum E { #[serde(rename_all = \"kebab-case\")] CopyAll }";
        assert_eq!(wire_name(plain, plain, "CopyAll"), "CopyAll");
        // with an enum-level style the enum style applies; the variant's
        // rename_all is not a rename and must not short-circuit it
        let styled = "#[serde(rename_all = \"snake_case\")] enum E { #[serde(rename_all = \"kebab-case\")] CopyAll }";
        assert_eq!(wire_name(styled, styled, "CopyAll"), "copy_all");
    }
}
