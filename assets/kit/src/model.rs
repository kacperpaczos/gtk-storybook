//! Data model shared by every `sbk` command and by the storybook viewer.
//!
//! `Inventory`, `StatesFile`, `Cell`, `Flows` and `Layout` double as the
//! on-disk JSON/TOML formats described in `references/formats.md`.

use std::collections::BTreeMap;

/// Default cell budget per view (`references/discovery.md`).
pub const DEFAULT_BUDGET: u32 = 64;

/// Everything `sbk scan` extracts from the host (`inventory.json`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Inventory {
    pub views: Vec<ViewInfo>,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ViewInfo {
    pub id: String,
    pub module: String,
    pub vm_type: String,
    pub actions_type: String,
    pub axes: Vec<Axis>,
    pub actions: Vec<String>,
    pub unread_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Axis {
    pub path: String,
    pub kind: AxisKind,
    pub classes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AxisKind {
    Bool,
    Option,
    Enum { variants: Vec<String> },
    Collection,
    Phase,
    Text,
    Number,
    Struct,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Finding {
    /// Stable id: `f-<6 hex>`; hash of view + path + kind (never of location).
    pub id: String,
    pub view: String,
    /// One of the finding kinds from `references/discovery.md`.
    pub kind: String,
    /// `file:line`, relative to the host workspace root.
    pub location: String,
    /// Hash of the surrounding item's tokens; changes when the source edits.
    pub fingerprint: String,
    pub detail: String,
}

/// `states/<view>.toml` — authored per view (`references/formats.md`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StatesFile {
    pub view: String,
    pub budget: Option<u32>,
    #[serde(default)]
    pub classes: BTreeMap<String, Vec<String>>,
    #[serde(default, rename = "guard")]
    pub guards: Vec<Guard>,
    #[serde(default, rename = "equivalence")]
    pub equivalences: Vec<Equivalence>,
    #[serde(default, rename = "disposition")]
    pub dispositions: BTreeMap<String, Disposition>,
    #[serde(default)]
    pub titles: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Guard {
    /// Axis path -> class; a cell matching every entry is impossible.
    pub when: BTreeMap<String, String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Equivalence {
    pub axis: String,
    /// Classes that render identically; the first is the canonical name.
    pub merge: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum Disposition {
    CoveredBy {
        #[serde(rename = "covered-by")]
        covered_by: String,
    },
    Excluded {
        excluded: String,
    },
}

/// A point in `layout.json` geometry (px, y grows down).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GeoPoint {
    pub x: f64,
    pub y: f64,
}

/// A rectangle in `layout.json` geometry (top-left + size, px).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GeoRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// One view cluster of a `dot` layout: the frame drawn around a view's
/// nodes, labeled with the view id.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ClusterPos {
    pub view: String,
    pub label: String,
    pub rect: GeoRect,
}

/// The layout engine that produced `layout.json` (references/formats.md):
/// `"dot"` (Graphviz, when available) or `"builtin"`.
pub const ENGINE_DOT: &str = "dot";
/// The deterministic built-in engine (always available).
pub const ENGINE_BUILTIN: &str = "builtin";

fn engine_builtin() -> String {
    ENGINE_BUILTIN.to_string()
}

/// `cells.json` written by `sbk expand --write`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CellsFile {
    pub cells: Vec<Cell>,
}

/// One state of one view: `<view>/<class>-<class>-...` in axis order.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Cell {
    pub view: String,
    pub state_id: String,
    pub assignment: BTreeMap<String, String>,
}

/// `flows.toml` — authored navigation graph.
///
/// A **flow node** is a state that appears in `roots`, in any edge
/// (from, or to ≠ `back`) or as a terminal state. Other cells are **data
/// variants** of exactly one flow node (`[variants]`): same view, same
/// wiring — the journey map draws one node per user-reachable step and
/// stacks the variants under it (formats.md).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Flows {
    /// Entry states: several when startup is conditional (formats.md).
    #[serde(default)]
    pub roots: Vec<String>,
    #[serde(default, rename = "edge")]
    pub edges: Vec<Edge>,
    #[serde(default, rename = "terminal")]
    pub terminals: Vec<Terminal>,
    /// Flow node state id -> its data variants (never in roots/edges/
    /// terminals; same view as the node).
    #[serde(default)]
    pub variants: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Edge {
    pub from: String,
    pub action: String,
    /// A state id, or `back` (return to the previous node on the path).
    pub to: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Terminal {
    pub state: String,
    pub action: String,
    pub kind: TerminalKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TerminalKind {
    #[serde(rename = "self")]
    Self_,
    Leaf,
}

/// `layout.json` written by `sbk verify`: deterministic journey geometry.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Layout {
    pub nodes: Vec<NodePos>,
    /// Data variants, stacked under their flow node when expanded.
    #[serde(default)]
    pub variants: Vec<VariantPos>,
    pub edges: Vec<EdgeRoute>,
    /// Which engine placed the geometry: `"dot"` or `"builtin"`.
    #[serde(default = "engine_builtin")]
    pub engine: String,
    /// Graphviz version string (`dot -V`); dot layouts only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dot_version: Option<String>,
    /// View clusters (frames around one view's nodes); dot layouts only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clusters: Vec<ClusterPos>,
}

/// One journey-map node (= one flow node of `flows.toml`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NodePos {
    pub state_id: String,
    /// Lane = index of the view in the manifest (inventory) order.
    pub lane: u32,
    /// Journey column: longest-path layer from the roots over the DAG
    /// obtained by dropping back edges.
    pub depth: u32,
    /// Row within the lane (nodes sorted by depth, then state id).
    pub order: u32,
    /// Human state title from `states/<view>.toml` `[titles]`; `None` falls
    /// back to the state id.
    #[serde(default)]
    pub title: Option<String>,
    /// Actions wired as `kind = "self"` terminals: they stay in place, so
    /// they are node data, not edges.
    #[serde(default)]
    pub self_actions: Vec<String>,
    /// Data-variant state ids of this node (flows.toml `[variants]`).
    #[serde(default)]
    pub variants: Vec<String>,
    /// `variants.len()`, carried so consumers can badge without counting.
    #[serde(default)]
    pub variant_count: u32,
    /// Node frame from the `dot` engine (top-left px, y down); `None` in
    /// builtin layouts, where the viewer computes geometry from
    /// lane/depth/order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<GeoRect>,
}

/// One data variant: positioned in the stack under its flow node.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VariantPos {
    pub state_id: String,
    /// The flow node this variant belongs to.
    pub variant_of: String,
    pub lane: u32,
    pub depth: u32,
    /// Stack index under the node (0 = directly below it).
    pub order: u32,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EdgeRoute {
    pub from: String,
    pub to: String,
    /// True for `back` targets and edges that point backwards in depth.
    pub back: bool,
    #[serde(default)]
    pub label: Option<String>,
    /// `dot` spline as a cubic Bezier chain: start point, then whole
    /// triples (c1, c2, end) — `GskPath::cubic_to` per triple.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<GeoPoint>>,
    /// Centre of the edge-label pill; dot layouts only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_pos: Option<GeoPoint>,
    /// Filled arrowhead polygon at the chain's end; dot layouts only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrow: Option<Vec<GeoPoint>>,
}
