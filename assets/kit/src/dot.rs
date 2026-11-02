//! Graphviz `dot` as the flow-map layout engine (references/formats.md
//! "layout.json").
//!
//! `sbk verify` prefers `dot` when it is on `PATH` (override with the
//! `SBK_DOT` environment variable: a path, or `off` to disable). The
//! builtin geometry (lane/depth/order, merged edges, variants) is computed
//! first and kept — `dot` only *adds* px geometry: per node a `rect`, per
//! edge a cubic Bezier `points` chain (start, then whole triples), a
//! `label_pos` and an `arrow` polygon, and per view a cluster `rect`.
//!
//! Graphviz's y axis points up; everything stored here is flipped to the
//! viewer's y-down px using the graph bounding box. Coordinates are rounded
//! to 1/100 px so the same input yields a byte-identical `layout.json`.
// The kit's own workspace policy: it shells out to `dot` by design (the
// host repo's "no process spawning" rule targets host code, and tests
// already allow this for running `sbk` under test).
#![allow(clippy::disallowed_methods)]
use crate::layout;
use crate::model::{self, Cell, ClusterPos, ENGINE_DOT, Flows, GeoPoint, GeoRect, Layout};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Environment variable selecting the dot binary (`off` disables).
pub const ENV_DOT: &str = "SBK_DOT";
/// Node size in the DOT source: 3.33x2.58 in = 240x186 px at 72 dpi, i.e.
/// the viewer's 240x160 thumbnail plus the title strip under it, so dot
/// keeps titles clear of neighbouring nodes and cluster labels.
const DOT_NODE_W: &str = "3.33";
const DOT_NODE_H: &str = "2.58";
/// The plain node box height in px (thumbnail + title strip).
const NODE_BOX_H: f64 = 2.58 * DPI;
/// Title strip (px) at the bottom of each dot node box; the stored node
/// rect is the thumbnail above it.
pub const TITLE_STRIP: f64 = 26.0;
/// Points per inch: dot reports px at this scale, so 1 pt = 1 px.
const DPI: f64 = 72.0;
/// Wall-clock budget for the dot run.
const TIMEOUT: Duration = Duration::from_secs(30);

/// One node as dot placed it: state id -> frame (top-left, y down).
pub type NodeRects = BTreeMap<String, GeoRect>;

/// One cluster (view) as dot placed it.
#[derive(Debug)]
pub struct DotCluster {
    pub label: String,
    pub rect: GeoRect,
    /// State ids inside the cluster.
    pub nodes: Vec<String>,
}

/// One edge as dot placed it: tail/head state ids, the Bezier chain
/// (start, then whole triples), the label position and the arrowhead.
#[derive(Debug)]
pub struct DotEdge {
    pub tail: String,
    pub head: String,
    pub points: Vec<GeoPoint>,
    pub label_pos: Option<GeoPoint>,
    pub arrow: Option<Vec<GeoPoint>>,
}

/// Everything the parser extracted from a `dot -Tjson` run, y-down px.
#[derive(Debug)]
pub struct DotGeometry {
    pub nodes: NodeRects,
    pub clusters: Vec<DotCluster>,
    pub edges: Vec<DotEdge>,
}

/// Resolve the dot binary from an `SBK_DOT` value (`None` = unset):
/// `off` disables (None), an explicit path wins verbatim, otherwise the
/// first executable `dot` on `PATH`.
pub fn resolve_dot(env_value: Option<&str>) -> Option<PathBuf> {
    match env_value {
        Some("off") | Some("") => None,
        Some(path) => Some(PathBuf::from(path)),
        None => find_on_path("dot"),
    }
}

/// The dot binary to use, per the `SBK_DOT` environment variable.
pub fn dot_program() -> Option<PathBuf> {
    resolve_dot(std::env::var(ENV_DOT).ok().as_deref())
}

fn find_on_path(program: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(program))
        .find(|p| is_executable_file(p))
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Escape `text` for a double-quoted DOT string: backslashes and quotes
/// are doubled; newlines become `\\n` (a label cannot span lines).
pub fn escape_dot(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out
}

/// The DOT source for the flow map: one cluster per view (labeled with
/// the view id), one box per flow node (labeled with its title, else the
/// state id), one edge per merged route; edges marked `back` are dashed
/// and kept out of the ranking so the journey still reads left → right.
pub fn build_dot(views: &[String], placed: &Layout) -> String {
    build_dot_with_sizes(views, placed, &BTreeMap::new())
}

/// [`build_dot`] with per-node size overrides (px): an expanded flow node
/// passes [`expanded_size`] so its box holds the variant grid and dot moves
/// everything else out of the way. Nodes without an entry keep the default
/// box.
pub fn build_dot_with_sizes(
    views: &[String],
    placed: &Layout,
    sizes: &BTreeMap<String, (f64, f64)>,
) -> String {
    let mut out = String::from("digraph G {\nrankdir=LR;\nsplines=true;\n");
    out.push_str("nodesep=0.9;\nranksep=1.6;\n");
    out.push_str(&format!(
        "node[shape=box,width={DOT_NODE_W},height={DOT_NODE_H},fixedsize=true];\n"
    ));
    for (lane, view) in views.iter().enumerate() {
        let nodes: Vec<&model::NodePos> = placed
            .nodes
            .iter()
            .filter(|n| n.lane as usize == lane)
            .collect();
        if nodes.is_empty() {
            continue;
        }
        // cluster label: the view's display name, else the humanized id
        let label = placed
            .view_titles
            .get(view)
            .cloned()
            .unwrap_or_else(|| model::humanize(view));
        out.push_str(&format!("subgraph \"cluster_{}\" {{\n", escape_dot(view)));
        out.push_str(&format!("label=\"{}\";\nmargin=20;\n", escape_dot(&label)));
        for node in nodes {
            let label = node.title.as_deref().unwrap_or(node.state_id.as_str());
            let attrs = match sizes.get(&node.state_id) {
                Some((w, h)) => {
                    format!(", width={}, height={}", round2(w / DPI), round2(h / DPI))
                }
                None => String::new(),
            };
            out.push_str(&format!(
                "\"{}\" [label=\"{}\"{attrs}];\n",
                escape_dot(&node.state_id),
                escape_dot(label)
            ));
        }
        out.push_str("}\n");
    }
    for edge in &placed.edges {
        let attrs = match (edge.back, &edge.label) {
            (true, Some(label)) => {
                format!(
                    "[label=\"{}\", constraint=false, style=dashed]",
                    escape_dot(label)
                )
            }
            (true, None) => "[constraint=false, style=dashed]".to_string(),
            (false, Some(label)) => format!("[label=\"{}\"]", escape_dot(label)),
            (false, None) => String::new(),
        };
        out.push_str(&format!(
            "\"{}\" -> \"{}\" {};\n",
            escape_dot(&edge.from),
            escape_dot(&edge.to),
            attrs
        ));
    }
    out.push_str("}\n");
    out
}

/// Variant grid constants (references/canvas.md): 2 columns of 112x75 px
/// thumbnails with a 16 px state title under each, 8 px gaps.
pub const GRID_COLS: u32 = 2;
pub const GRID_CELL_W: f64 = 112.0;
pub const GRID_THUMB_H: f64 = 75.0;
pub const GRID_TITLE_H: f64 = 16.0;
pub const GRID_GAP: f64 = 8.0;

/// The px size of the variant grid under a node expanded to `count`
/// variants: rows of [`GRID_COLS`] cells (0 variants — nothing).
pub fn variant_grid_size(count: u32) -> (f64, f64) {
    if count == 0 {
        return (0.0, 0.0);
    }
    let rows = count.div_ceil(GRID_COLS);
    let w = GRID_COLS as f64 * GRID_CELL_W + (GRID_COLS - 1) as f64 * GRID_GAP;
    let cell_h = GRID_THUMB_H + GRID_TITLE_H;
    (w, rows as f64 * cell_h + (rows - 1) as f64 * GRID_GAP)
}

/// The px size of the dot box of a node whose `count` variants are
/// expanded: the plain 240x186 box plus an 8 px gap and the variant grid.
/// The stored rect of such a node covers the whole box (thumbnail, title
/// strip, gap and grid).
pub fn expanded_size(count: u32) -> (f64, f64) {
    let (_, grid_h) = variant_grid_size(count);
    (240.0, NODE_BOX_H + GRID_GAP + grid_h)
}

/// A unique scratch file for dot's input (removed by [`run_dot`]).
fn scratch_file() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("sbk-dot-{}-{seq}.dot", std::process::id()))
}

/// Run `program -Tjson` on `source`; `Err` explains, the caller falls
/// back to the builtin engine.
pub fn run_dot(program: &Path, source: &str) -> Result<String, String> {
    run_dot_with_timeout(program, source, TIMEOUT)
}

/// [`run_dot`] with an explicit wall-clock budget (tests use a short one).
pub fn run_dot_with_timeout(
    program: &Path,
    source: &str,
    timeout: Duration,
) -> Result<String, String> {
    let input = scratch_file();
    let output = input.with_extension("json");
    let result = (|| {
        std::fs::write(&input, source).map_err(|err| format!("cannot write dot input: {err}"))?;
        // Output goes to a file: a large graph would overflow the pipe
        // buffer and deadlock a stdout-draining reader.
        let mut child = Command::new(program)
            .arg("-Tjson")
            .arg("-o")
            .arg(&output)
            .arg(&input)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("{}: {err}", program.display()))?;
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("{}: timed out", program.display()));
                }
                Err(err) => return Err(format!("{}: {err}", program.display())),
            }
        }
        let out = child
            .wait_with_output()
            .map_err(|err| format!("dot: {err}"))?;
        if !out.status.success() {
            return Err(format!(
                "dot failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        std::fs::read_to_string(&output).map_err(|err| format!("dot output: {err}"))
    })();
    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&output);
    result
}

/// The Graphviz version string from `program -V` (it prints to stderr).
pub fn dot_version(program: &Path) -> Option<String> {
    let out = Command::new(program).arg("-V").output().ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let after = text.split("version ").nth(1)?;
    let version = after.split_whitespace().next()?;
    version.contains('.').then(|| version.to_string())
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Parse `"x,y"` / `"x,y,z,w"` dot coordinate strings.
fn parse_coords(text: &str) -> Vec<f64> {
    text.split(',')
        .filter_map(|t| t.parse::<f64>().ok())
        .collect()
}

/// Flip a dot y (up) to the viewer's y-down using the graph top.
fn flip(y: f64, top: f64) -> f64 {
    round2(top - y)
}

/// Parse `"x,y"` into a point, flipped to y-down.
fn parse_point(text: &str, top: f64) -> Option<GeoPoint> {
    let c = parse_coords(text);
    if c.len() != 2 {
        return None;
    }
    Some(GeoPoint {
        x: round2(c[0]),
        y: flip(c[1], top),
    })
}

/// Parse `dot -Tjson` output into y-down px geometry. `None` on any
/// structural surprise — the caller falls back to the builtin engine.
pub fn parse_dot_json(text: &str) -> Option<DotGeometry> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    let bb = parse_coords(root.get("bb")?.as_str()?);
    if bb.len() != 4 {
        return None;
    }
    let top = bb[3];

    let objects = root
        .get("objects")
        .and_then(|o| o.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let name_at = |objects: &[serde_json::Value], idx: &serde_json::Value| -> Option<String> {
        idx.as_u64()
            .and_then(|i| objects.get(i as usize))
            .and_then(|obj| obj.get("name"))
            .and_then(|n| n.as_str())
            .map(str::to_string)
    };

    let mut nodes: NodeRects = BTreeMap::new();
    let mut clusters: Vec<DotCluster> = Vec::new();
    for obj in objects {
        let name = obj.get("name").and_then(|n| n.as_str());
        if let (Some(pos), Some(name)) = (obj.get("pos").and_then(|p| p.as_str()), name) {
            let (Ok(w_in), Ok(h_in)) = (
                obj.get("width")
                    .and_then(|w| w.as_str())
                    .unwrap_or("")
                    .parse::<f64>(),
                obj.get("height")
                    .and_then(|h| h.as_str())
                    .unwrap_or("")
                    .parse::<f64>(),
            ) else {
                continue;
            };
            let Some(p) = parse_point(pos, top) else {
                continue;
            };
            let (w, h) = (w_in * DPI, h_in * DPI);
            // A plain node's rect is the thumbnail: the box minus the
            // title strip (the strip is drawn below the rect). An expanded
            // node's box is taller than the plain box; its rect covers the
            // whole box — thumbnail, strip, gap and variant grid.
            let rect_h = if h > NODE_BOX_H + 0.5 {
                h
            } else {
                (h - TITLE_STRIP).max(0.0)
            };
            nodes.insert(
                name.to_string(),
                GeoRect {
                    x: round2(p.x - w / 2.0),
                    y: round2(p.y - h / 2.0),
                    w: round2(w),
                    h: round2(rect_h),
                },
            );
        } else if let Some(cbb) = obj.get("bb").and_then(|b| b.as_str()) {
            // no pos but a bb: a cluster
            let c = parse_coords(cbb);
            if c.len() != 4 {
                continue;
            }
            let label = obj
                .get("label")
                .and_then(|l| l.as_str())
                .unwrap_or_default()
                .to_string();
            let member_names: Vec<String> = obj
                .get("nodes")
                .and_then(|n| n.as_array())
                .map(|idx| idx.iter().filter_map(|i| name_at(objects, i)).collect())
                .unwrap_or_default();
            clusters.push(DotCluster {
                label,
                rect: GeoRect {
                    x: round2(c[0]),
                    y: flip(c[3], top),
                    w: round2(c[2] - c[0]),
                    h: round2(c[3] - c[1]),
                },
                nodes: member_names,
            });
        }
    }
    if nodes.is_empty() {
        return None;
    }

    let mut edges: Vec<DotEdge> = Vec::new();
    for edge in root
        .get("edges")
        .and_then(|e| e.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        let (Some(tail), Some(head)) = (
            edge.get("tail").and_then(|t| name_at(objects, t)),
            edge.get("head").and_then(|h| name_at(objects, h)),
        ) else {
            continue;
        };
        let Some(pos) = edge.get("pos").and_then(|p| p.as_str()) else {
            continue;
        };
        // Spline tokens: optional "e,x,y"/"s,x,y" markers around the plain
        // start → (c1, c2, end)* point list; the markers repeat what the
        // chain already ends with, so the plain tokens are the chain.
        let mut points: Vec<GeoPoint> = Vec::new();
        for token in pos.split_whitespace() {
            if token.starts_with("e,") || token.starts_with("s,") {
                continue;
            }
            if let Some((x, y)) = token.split_once(',')
                && let (Ok(x), Ok(y)) = (x.parse::<f64>(), y.parse::<f64>())
            {
                points.push(GeoPoint {
                    x: round2(x),
                    y: flip(y, top),
                });
            }
        }
        if points.is_empty() || !(points.len() - 1).is_multiple_of(3) {
            continue;
        }
        let label_pos = edge
            .get("lp")
            .and_then(|lp| lp.as_str())
            .and_then(|lp| parse_point(lp, top));
        let arrow = edge
            .get("_hdraw_")
            .and_then(|ops| ops.as_array())
            .and_then(|ops| {
                ops.iter().find_map(|op| {
                    let pts: Vec<GeoPoint> = op
                        .get("points")
                        .and_then(|p| p.as_array())
                        .map(|pts| {
                            pts.iter()
                                .filter_map(|pt| pt.as_array())
                                .filter_map(|xy| {
                                    let x = xy.first()?.as_f64()?;
                                    let y = xy.get(1)?.as_f64()?;
                                    Some(GeoPoint {
                                        x: round2(x),
                                        y: flip(y, top),
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    (!pts.is_empty()).then_some(pts)
                })
            });
        edges.push(DotEdge {
            tail,
            head,
            points,
            label_pos,
            arrow,
        });
    }

    Some(DotGeometry {
        nodes,
        clusters,
        edges,
    })
}

/// Full layout via `dot`: builtin geometry plus px geometry from the dot
/// run. `Err` explains why dot could not place the map — the caller falls
/// back to the builtin layout.
pub fn layout_with_dot(
    program: &Path,
    views_in_order: &[String],
    cells: &[Cell],
    flows: &Flows,
    titles: &BTreeMap<String, String>,
    view_titles: &BTreeMap<String, String>,
) -> Result<Layout, String> {
    let placed = layout::layout(views_in_order, cells, flows, titles, view_titles);
    let source = build_dot(views_in_order, &placed);
    let json = run_dot(program, &source)?;
    let geo = parse_dot_json(&json).ok_or_else(|| "dot output not understood".to_string())?;
    merge_dot_geometry(placed, &geo, views_in_order, program)
}

/// Re-run `dot` over an already placed layout with per-node size overrides
/// (px): expanding a node's variant grid grows its box (see
/// [`expanded_size`]) and everything else moves out of the way. The logical
/// graph (lanes, depths, orders, edges) is unchanged. `Err` explains why
/// dot could not place the map — the caller falls back to the builtin plan.
pub fn relayout_with_dot(
    program: &Path,
    views_in_order: &[String],
    placed: &Layout,
    sizes: &BTreeMap<String, (f64, f64)>,
) -> Result<Layout, String> {
    let source = build_dot_with_sizes(views_in_order, placed, sizes);
    let json = run_dot(program, &source)?;
    let geo = parse_dot_json(&json).ok_or_else(|| "dot output not understood".to_string())?;
    merge_dot_geometry(placed.clone(), &geo, views_in_order, program)
}

/// Overwrite a placed layout's px geometry with what dot produced.
fn merge_dot_geometry(
    mut placed: Layout,
    geo: &DotGeometry,
    views_in_order: &[String],
    program: &Path,
) -> Result<Layout, String> {
    for node in &mut placed.nodes {
        node.rect = geo.nodes.get(&node.state_id).copied();
    }
    if placed.nodes.iter().any(|n| n.rect.is_none()) {
        return Err("dot placed only some nodes".to_string());
    }
    for route in &mut placed.edges {
        if let Some(edge) = geo
            .edges
            .iter()
            .find(|e| e.tail == route.from && e.head == route.to)
        {
            route.points = Some(edge.points.clone());
            route.label_pos = edge.label_pos;
            route.arrow = edge.arrow.clone();
        }
    }
    // A cluster's view: the part before '/' of its member state ids.
    let view_of = |state: &str| -> String { state.split('/').next().unwrap_or(state).to_string() };
    placed.clusters = geo
        .clusters
        .iter()
        .filter(|c| !c.nodes.is_empty())
        .map(|c| {
            let view = views_in_order
                .iter()
                .find(|v| c.nodes.iter().any(|n| view_of(n) == v.as_str()))
                .cloned()
                .unwrap_or_else(|| view_of(&c.nodes[0]));
            // frame label: the view's display name, else the humanized id
            let label = placed
                .view_titles
                .get(&view)
                .cloned()
                .unwrap_or_else(|| crate::model::humanize(&view));
            ClusterPos {
                label,
                view,
                rect: c.rect,
            }
        })
        .collect();
    placed.clusters.sort_by(|a, b| a.view.cmp(&b.view));
    placed.engine = ENGINE_DOT.to_string();
    placed.dot_version = dot_version(program);
    Ok(placed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EdgeRoute, Layout, NodePos};
    use std::sync::Mutex;

    /// Env-touching tests mutate process-global state; keep them exclusive.
    static ENV: Mutex<()> = Mutex::new(());

    fn write_file(path: &Path, body: &str, mode: u32) {
        std::fs::write(path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        #[cfg(not(unix))]
        {
            let _ = mode;
        }
    }

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn node(state: &str, lane: u32, order: u32) -> NodePos {
        NodePos {
            state_id: state.to_string(),
            lane,
            depth: 0,
            order,
            title: None,
            self_actions: Vec::new(),
            variants: Vec::new(),
            variant_count: 0,
            rect: None,
        }
    }

    fn route(from: &str, to: &str) -> EdgeRoute {
        EdgeRoute {
            from: from.to_string(),
            to: to.to_string(),
            back: false,
            label: None,
            points: None,
            label_pos: None,
            arrow: None,
        }
    }

    /// The text from `start` (a `subgraph "..."` line) to the closing `}`.
    fn cluster_block<'a>(src: &'a str, cluster: &str) -> &'a str {
        let needle = format!("subgraph \"{cluster}\"");
        let start = src
            .find(&needle)
            .unwrap_or_else(|| panic!("{needle} in {src}"));
        let end = start + src[start..].find("}\n").expect("cluster closes");
        &src[start..end]
    }

    #[test]
    fn is_executable_file_checks_metadata_and_mode() {
        let dir = tmp();
        let exec = dir.path().join("exec");
        let plain = dir.path().join("plain");
        write_file(&exec, "#!/bin/sh\n", 0o755);
        write_file(&plain, "data\n", 0o644);
        assert!(is_executable_file(&exec), "executable regular file");
        assert!(!is_executable_file(&plain), "no exec bit");
        assert!(!is_executable_file(dir.path()), "a directory");
        assert!(!is_executable_file(&dir.path().join("missing")), "absent");
    }

    #[test]
    fn find_on_path_skips_non_executable_and_directory_matches() {
        let _guard = ENV.lock().unwrap();
        let old = std::env::var_os("PATH");

        let dir = tmp();
        write_file(&dir.path().join("dot"), "#!/bin/sh\n", 0o644);
        unsafe { std::env::set_var("PATH", dir.path().as_os_str()) };
        let non_exec = find_on_path("dot");

        std::fs::remove_file(dir.path().join("dot")).unwrap();
        std::fs::create_dir(dir.path().join("dot")).unwrap();
        unsafe { std::env::set_var("PATH", dir.path().as_os_str()) };
        let as_dir = find_on_path("dot");
        std::fs::remove_dir_all(dir.path().join("dot")).unwrap();
        write_file(&dir.path().join("dot"), "#!/bin/sh\n", 0o755);
        unsafe { std::env::set_var("PATH", dir.path().as_os_str()) };
        let exec = find_on_path("dot");

        match old {
            Some(p) => unsafe { std::env::set_var("PATH", p) },
            None => unsafe { std::env::remove_var("PATH") },
        }
        assert_eq!(non_exec, None, "a 0o644 match is not executable");
        assert_eq!(as_dir, None, "a directory named dot is not a file");
        assert_eq!(exec, Some(dir.path().join("dot")));
    }

    #[test]
    fn dot_program_reads_the_env_override() {
        let _guard = ENV.lock().unwrap();
        let dir = tmp();
        let fake = dir.path().join("dot");
        write_file(&fake, "#!/bin/sh\n", 0o755);
        unsafe { std::env::set_var(ENV_DOT, fake.as_os_str()) };
        let got = dot_program();
        unsafe { std::env::remove_var(ENV_DOT) };
        assert_eq!(got, Some(fake));
    }

    #[test]
    fn escape_dot_turns_newlines_into_dot_line_breaks() {
        assert_eq!(
            escape_dot("two\nlines \"q\" \\done"),
            "two\\nlines \\\"q\\\" \\\\done"
        );
    }

    #[test]
    fn build_dot_puts_each_node_in_its_own_views_cluster() {
        let views = vec!["one".to_string(), "two".to_string()];
        let placed = Layout {
            nodes: vec![node("one/a", 0, 0), node("two/b", 1, 0)],
            variants: Vec::new(),
            edges: Vec::new(),
            engine: String::new(),
            dot_version: None,
            clusters: Vec::new(),
            view_titles: BTreeMap::new(),
        };
        let src = build_dot(&views, &placed);
        let one = cluster_block(&src, "cluster_one");
        assert!(one.contains("\"one/a\" [label=\"one/a\"];"), "{src}");
        assert!(!one.contains("two/b"), "{src}");
        let two = cluster_block(&src, "cluster_two");
        assert!(two.contains("\"two/b\" [label=\"two/b\"];"), "{src}");
        assert!(!two.contains("one/a"), "{src}");
    }

    #[test]
    fn parse_dot_json_at_the_expanded_boundary_keeps_the_title_strip() {
        // 2.5869444444444443 in * 72 = 186.26 px = NODE_BOX_H + 0.5
        // exactly: still a plain box, so the rect is the thumbnail above
        // the title strip. A clearly taller box is expanded and keeps the
        // whole height.
        let text = "{\"bb\":\"0,0,600,400\",\"objects\":[\
            {\"name\":\"plain\",\"pos\":\"100,100\",\"width\":\"3.33\",\
             \"height\":\"2.5869444444444443\"},\
            {\"name\":\"expanded\",\"pos\":\"400,100\",\"width\":\"3.33\",\
             \"height\":\"4.0\"}]}";
        let geo = parse_dot_json(text).expect("parses");
        let plain = geo.nodes["plain"];
        assert!(
            (plain.h - (186.26 - TITLE_STRIP)).abs() < 0.005,
            "{plain:?}"
        );
        let expanded = geo.nodes["expanded"];
        assert!((expanded.h - 288.0).abs() < 0.005, "{expanded:?}");
    }

    #[test]
    fn parse_dot_json_drops_edges_without_a_whole_bezier_chain() {
        // marker-only spline (no chain) and two bare points (not a whole
        // triple) are both dropped.
        let text = "{\"bb\":\"0,0,600,400\",\"objects\":[\
            {\"name\":\"a\",\"pos\":\"100,100\",\"width\":\"3.33\",\"height\":\"2.58\"},\
            {\"name\":\"b\",\"pos\":\"400,100\",\"width\":\"3.33\",\"height\":\"2.58\"}],\
            \"edges\":[{\"tail\":0,\"head\":1,\"pos\":\"e,5,5 s,6,6\"},\
                      {\"tail\":0,\"head\":1,\"pos\":\"1,1 2,2\"}]}";
        let geo = parse_dot_json(text).expect("parses");
        assert!(geo.edges.is_empty(), "{:?}", geo.edges);
    }

    #[test]
    fn run_dot_with_timeout_kills_a_hung_program() {
        let dir = tmp();
        let slow = dir.path().join("slow-dot");
        // pure shell builtins: the fake must survive a scrubbed PATH
        // (cargo-mutants runs tests with a minimal environment).
        write_file(
            &slow,
            "#!/bin/sh\ni=0\nwhile [ $i -lt 5000000 ]; do i=$((i+1)); done\n",
            0o755,
        );
        let err = run_dot_with_timeout(&slow, "digraph G {}", Duration::from_millis(150))
            .expect_err("a hung program must time out");
        assert!(err.contains("timed out"), "{err}");
    }

    fn geo_for_merge() -> DotGeometry {
        let rect = |x: f64| GeoRect {
            x,
            y: 0.0,
            w: 240.0,
            h: 160.0,
        };
        DotGeometry {
            nodes: BTreeMap::from([
                ("v1/a".to_string(), rect(0.0)),
                ("v1/b".to_string(), rect(300.0)),
                ("v1/c".to_string(), rect(600.0)),
                ("v2/n".to_string(), rect(900.0)),
            ]),
            clusters: vec![DotCluster {
                label: "v2".to_string(),
                rect: rect(900.0),
                nodes: vec!["v2/n".to_string()],
            }],
            edges: vec![
                DotEdge {
                    tail: "v1/a".to_string(),
                    head: "v1/b".to_string(),
                    points: vec![GeoPoint { x: 1.0, y: 1.0 }],
                    label_pos: None,
                    arrow: None,
                },
                DotEdge {
                    tail: "v1/a".to_string(),
                    head: "v1/c".to_string(),
                    points: vec![GeoPoint { x: 9.0, y: 9.0 }],
                    label_pos: None,
                    arrow: None,
                },
            ],
        }
    }

    #[test]
    fn merge_matches_edges_on_both_endpoints_and_views_by_membership() {
        let placed = Layout {
            nodes: vec![
                node("v1/a", 0, 0),
                node("v1/b", 0, 1),
                node("v1/c", 0, 2),
                node("v2/n", 1, 0),
            ],
            variants: Vec::new(),
            edges: vec![route("v1/a", "v1/b"), route("v1/a", "v1/c")],
            engine: String::new(),
            dot_version: None,
            clusters: Vec::new(),
            view_titles: BTreeMap::new(),
        };
        let geo = geo_for_merge();
        let merged = merge_dot_geometry(
            placed,
            &geo,
            &["v1".to_string(), "v2".to_string()],
            Path::new("dt"),
        )
        .expect("merge succeeds");

        // each route gets its own edge's chain, not the first tail match
        let to_c = merged
            .edges
            .iter()
            .find(|r| r.to == "v1/c")
            .expect("route a->c");
        assert_eq!(to_c.points, Some(vec![GeoPoint { x: 9.0, y: 9.0 }]));
        let to_b = merged
            .edges
            .iter()
            .find(|r| r.to == "v1/b")
            .expect("route a->b");
        assert_eq!(to_b.points, Some(vec![GeoPoint { x: 1.0, y: 1.0 }]));

        // the cluster of v2/n belongs to view v2; no authored title, so it
        // is labeled with the humanized id (same fallback as the menu)
        assert_eq!(merged.clusters.len(), 1);
        assert_eq!(merged.clusters[0].view, "v2");
        assert_eq!(merged.clusters[0].label, "V2");
        assert_eq!(merged.engine, crate::model::ENGINE_DOT);
    }
}
