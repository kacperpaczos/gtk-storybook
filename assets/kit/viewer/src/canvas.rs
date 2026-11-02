//! Flow-map canvas (references/canvas.md).
//!
//! Split in two halves: a pure geometry module — logical coordinates, the
//! pan/zoom transform, deterministic layout planning from `layout.json`
//! nodes, cubic edge control points with side arcs for back edges, arrowheads
//! and inverse-transform hit testing — and the GTK widget that draws it with
//! live `WidgetPaintable` thumbnails and wires pan/zoom/pinch/fit input.
//! Zoom always scales the drawing, never the layout, so host breakpoints stay
//! true.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use gtk::gdk;
use gtk::glib;
use gtk::glib::clone;
use gtk::graphene;
use gtk::gsk;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use storybook_kit::model::{EdgeRoute, NodePos, VariantPos};

// ---------------------------------------------------------------------------
// Pure geometry
// ---------------------------------------------------------------------------

/// Top-left margin of the logical coordinate space.
pub const MARGIN: f64 = 24.0;
/// Height of the view-name header drawn above each lane segment.
pub const LANE_HEADER_H: f64 = 26.0;
pub const NODE_W: f64 = 240.0;
pub const NODE_H: f64 = 160.0;
/// Gap between depth columns.
pub const HGAP: f64 = 120.0;
/// Gap between nodes stacked in the same (lane, depth) slot.
pub const VGAP: f64 = 28.0;
/// Gap between lanes.
pub const LANE_GAP: f64 = 48.0;
pub const ARROW_LEN: f64 = 10.0;
pub const ARROW_HALF_WIDTH: f64 = 5.0;
pub const MIN_SCALE: f64 = 0.05;
pub const MAX_SCALE: f64 = 8.0;
pub const FIT_PADDING: f64 = 24.0;
/// One wheel notch multiplies the scale by this.
pub const ZOOM_STEP: f64 = 1.1;
/// Minimal back-edge arc drop below the lane.
pub const BACK_DROP_MIN: f64 = 56.0;
/// Space under a node for its title and the \"+N variants\" badge.
pub const TITLE_H: f64 = 22.0;
/// Corner radius of node cards and view backgrounds.
pub const CARD_RADIUS: f64 = 8.0;
/// Opacity of everything outside the focus (hovered / selected node and
/// its neighbours).
pub const DIM_OPACITY: f64 = 0.3;
/// Width of the "+N" variant chip in a node's top-right corner.
pub const BADGE_W: f64 = 40.0;
/// Edge labels longer than this are ellipsized.
pub const LABEL_MAX_W: f64 = 180.0;
/// Inner padding of label pills (edge labels, titles, badges).
pub const PILL_PAD: f64 = 3.0;
/// Height of one variant thumbnail in the stack under its node.
pub const VARIANT_H: f64 = 64.0;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const ZERO: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
    };

    pub fn max_x(&self) -> f64 {
        self.x + self.w
    }

    pub fn max_y(&self) -> f64 {
        self.y + self.h
    }

    pub fn cx(&self) -> f64 {
        self.x + self.w / 2.0
    }

    pub fn cy(&self) -> f64 {
        self.y + self.h / 2.0
    }

    /// True when the two rects overlap with positive area.
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.max_x() && o.x < self.max_x() && self.y < o.max_y() && o.y < self.max_y()
    }

    /// Borders are inclusive: a node owns its outline for hit testing.
    pub fn contains(&self, p: (f64, f64)) -> bool {
        p.0 >= self.x && p.0 <= self.max_x() && p.1 >= self.y && p.1 <= self.max_y()
    }
}

/// Screen = logical * scale + offset. `scale` is always in `[MIN_SCALE, MAX_SCALE]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewTransform {
    pub scale: f64,
    pub offset_x: f64,
    pub offset_y: f64,
}

impl ViewTransform {
    pub const IDENTITY: ViewTransform = ViewTransform {
        scale: 1.0,
        offset_x: 0.0,
        offset_y: 0.0,
    };

    pub fn logical_to_screen(&self, x: f64, y: f64) -> (f64, f64) {
        (
            x * self.scale + self.offset_x,
            y * self.scale + self.offset_y,
        )
    }

    /// Inverse of [`ViewTransform::logical_to_screen`]; used for hit testing.
    pub fn screen_to_logical(&self, x: f64, y: f64) -> (f64, f64) {
        (
            (x - self.offset_x) / self.scale,
            (y - self.offset_y) / self.scale,
        )
    }
}

/// Zoom by `factor` around screen point `(px, py)`: the logical point under
/// the pointer stays under the pointer. The scale is clamped to
/// `[MIN_SCALE, MAX_SCALE]`.
pub fn zoom_at(t: ViewTransform, px: f64, py: f64, factor: f64) -> ViewTransform {
    let new_scale = (t.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
    let k = new_scale / t.scale;
    ViewTransform {
        scale: new_scale,
        offset_x: px - (px - t.offset_x) * k,
        offset_y: py - (py - t.offset_y) * k,
    }
}

/// Transform that shows all of `bounds` inside `viewport` with
/// [`FIT_PADDING`] on every side, centered.
pub fn fit_transform(bounds: Rect, viewport: (f64, f64)) -> ViewTransform {
    let (vw, vh) = viewport;
    if bounds.w <= 0.0 || bounds.h <= 0.0 || vw <= 0.0 || vh <= 0.0 {
        return ViewTransform::IDENTITY;
    }
    let scale = ((vw - 2.0 * FIT_PADDING) / bounds.w)
        .min((vh - 2.0 * FIT_PADDING) / bounds.h)
        .clamp(MIN_SCALE, MAX_SCALE);
    ViewTransform {
        scale,
        offset_x: (vw - bounds.w * scale) / 2.0 - bounds.x * scale,
        offset_y: (vh - bounds.h * scale) / 2.0 - bounds.y * scale,
    }
}

/// Touchpad scroll distance (surface px) that counts as one wheel notch.
pub const SURFACE_PX_PER_NOTCH: f64 = 25.0;

/// A scroll delta in wheel notches: wheel deltas already are notches,
/// smooth touchpad deltas come in surface pixels (`GdkScrollUnit::Surface`)
/// and are scaled down by [`SURFACE_PX_PER_NOTCH`].
pub fn scroll_notches(dy: f64, surface_px: bool) -> f64 {
    if surface_px {
        dy / SURFACE_PX_PER_NOTCH
    } else {
        dy
    }
}

/// Scroll decision: the zoom factor for a vertical scroll of `dy` notches
/// (see [`scroll_notches`]; no modifier needed), or `None` when there is no
/// vertical motion. Scrolling up (`dy < 0`) zooms in; one notch is one
/// [`ZOOM_STEP`], fractions scale proportionally (clamped to three notches).
pub fn scroll_zoom(dy: f64) -> Option<f64> {
    if dy == 0.0 {
        None
    } else {
        Some(ZOOM_STEP.powf(-dy.clamp(-3.0, 3.0)))
    }
}

/// Canvas key-press decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// Zoom to fit everything ([`crate::canvas::Canvas::fit`], key `F`).
    Fit,
}

/// Key-press decision for a key's Unicode value: `f`/`F` fits, anything
/// else (including non-character keys) passes on.
pub fn key_action(key: Option<char>) -> Option<KeyAction> {
    match key {
        Some('f' | 'F') => Some(KeyAction::Fit),
        _ => None,
    }
}

/// Thumbnail size preserving the aspect `ratio` (width / height) inside a
/// `w x h` box (letterboxing); the whole box when `ratio` is unusable
/// (not finite or `<= 0`), e.g. before the paintable has a size.
pub fn fit_size(w: f64, h: f64, ratio: f64) -> (f64, f64) {
    if !ratio.is_finite() || ratio <= 0.0 {
        return (w, h);
    }
    let th = (w / ratio).min(h);
    (th * ratio, th)
}
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NodeGeometry {
    pub state_id: String,
    pub lane: u32,
    pub depth: u32,
    pub order: u32,
    /// Human title from `states/<view>.toml`; the draw code falls back to
    /// the classes part of the state id.
    pub title: Option<String>,
    /// How many data variants stack under this node (badge: \"+N variants\").
    pub variant_count: u32,
    pub rect: Rect,
}

/// One data variant, stacked under its flow node (`layout.json` variants).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VariantGeometry {
    pub state_id: String,
    pub variant_of: String,
    /// Authored variant title; the draw code falls back to the classes
    /// part of the state id.
    pub title: Option<String>,
    pub rect: Rect,
}

/// Deterministic geometry derived from `layout.json`: depth = columns; each
/// view (lane) is a horizontal segment spanning its depth range, and lanes
/// whose ranges do not overlap share a row. The journey spine (the longest
/// root-to-leaf path) is packed first, so it reads along the top row; nodes
/// sharing a (lane, depth) slot stack in `sbk expand` order (`order`).
/// Collapsed variant stacks reserve no space: expanded ones are drawn on
/// top with an opaque card.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CanvasPlan {
    pub nodes: Vec<NodeGeometry>,
    /// Data variants stacked under their nodes (drawn when expanded).
    pub variants: Vec<VariantGeometry>,
    /// lane index -> lane segment (header at its top-left; spans the
    /// lane's depth columns and its tallest stack plus titles).
    pub lanes: BTreeMap<u32, Rect>,
    /// View cluster frames (dot engine): drawn instead of lane headers.
    pub clusters: Vec<ClusterFrame>,
    pub bounds: Rect,
}

/// One view cluster of a `dot` layout: a rounded frame around the view's
/// nodes with the view name on top (references/canvas.md).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClusterFrame {
    pub view: String,
    pub rect: Rect,
}

/// A `dot` layout's cluster frame in canvas coordinates.
pub fn cluster_frame(cluster: &storybook_kit::model::ClusterPos) -> ClusterFrame {
    ClusterFrame {
        view: cluster.label.clone(),
        rect: Rect {
            x: cluster.rect.x,
            y: cluster.rect.y,
            w: cluster.rect.w,
            h: cluster.rect.h,
        },
    }
}

/// The stored Bezier chain of a `dot` edge: start point plus whole cubic
/// triples (`cubic_to` per triple); `None` for builtin edges.
pub fn chain_points(edge: &EdgeRoute) -> Option<&[storybook_kit::model::GeoPoint]> {
    let points = edge.points.as_ref()?;
    (!points.is_empty() && (points.len() - 1) % 3 == 0).then_some(points)
}

/// Geometry for a `dot` layout: nodes at their stored rects (`layout.json`
/// `rect`), variants stacked under their node — expanded stacks draw on
/// top as opaque cards, collapsed ones reserve no space. Lanes stay empty;
/// the cluster frames carry the view names.
pub fn plan_dot(
    nodes: &[NodePos],
    variants: &[VariantPos],
    clusters: &[storybook_kit::model::ClusterPos],
) -> CanvasPlan {
    let mut geo: Vec<NodeGeometry> = nodes
        .iter()
        .filter_map(|n| {
            n.rect.map(|r| NodeGeometry {
                state_id: n.state_id.clone(),
                lane: n.lane,
                depth: n.depth,
                order: n.order,
                title: n.title.clone(),
                variant_count: n.variant_count,
                rect: Rect {
                    x: r.x,
                    y: r.y,
                    w: r.w,
                    h: r.h,
                },
            })
        })
        .collect();
    geo.sort_by(|a, b| {
        (a.lane, a.depth, a.order, &a.state_id).cmp(&(b.lane, b.depth, b.order, &b.state_id))
    }); // ascending: like the builtin plan, later nodes win hit tests
    let by_id: BTreeMap<&str, &NodeGeometry> =
        geo.iter().map(|n| (n.state_id.as_str(), n)).collect();
    let mut stack: Vec<VariantGeometry> = Vec::new();
    for v in variants {
        let Some(node) = by_id.get(v.variant_of.as_str()) else {
            continue;
        };
        let r = node.rect;
        let y = r.max_y() + TITLE_H + 6.0 + v.order as f64 * (VARIANT_H + 8.0);
        stack.push(VariantGeometry {
            state_id: v.state_id.clone(),
            variant_of: v.variant_of.clone(),
            title: v.title.clone(),
            rect: Rect {
                x: r.x,
                y,
                w: r.w,
                h: VARIANT_H,
            },
        });
    }
    let mut bounds = Rect::ZERO;
    for (i, n) in geo.iter().enumerate() {
        let r = Rect {
            x: n.rect.x,
            y: n.rect.y,
            w: n.rect.w,
            h: n.rect.h + TITLE_H,
        };
        bounds = if i == 0 {
            r
        } else {
            Rect {
                x: bounds.x.min(r.x),
                y: bounds.y.min(r.y),
                w: bounds.max_x().max(r.max_x()) - bounds.x.min(r.x),
                h: bounds.max_y().max(r.max_y()) - bounds.y.min(r.y),
            }
        };
    }
    CanvasPlan {
        nodes: geo,
        variants: stack,
        lanes: BTreeMap::new(),
        clusters: clusters.iter().map(cluster_frame).collect(),
        bounds,
    }
}

pub fn plan(nodes: &[NodePos], variants: &[VariantPos]) -> CanvasPlan {
    plan_with_edges(nodes, variants, &[])
}

/// The journey spine: walk back from the deepest node through forward
/// edges whose source sits exactly one column earlier (longest-path
/// layering guarantees one exists), preferring the lowest lane, then id.
fn spine_lanes(nodes: &[NodePos], edges: &[EdgeRoute]) -> Vec<u32> {
    let by_id: BTreeMap<&str, &NodePos> = nodes.iter().map(|n| (n.state_id.as_str(), n)).collect();
    let Some(mut cur) = nodes.iter().min_by(|a, b| {
        (std::cmp::Reverse(a.depth), a.lane, &a.state_id).cmp(&(
            std::cmp::Reverse(b.depth),
            b.lane,
            &b.state_id,
        ))
    }) else {
        return Vec::new();
    };
    let mut lanes = vec![cur.lane];
    // Depth 0 has no predecessor one column earlier, so the walk ends there.
    loop {
        let prev = edges
            .iter()
            .filter(|e| !e.back && e.to == cur.state_id)
            .filter_map(|e| by_id.get(e.from.as_str()).copied())
            .filter(|p| p.depth + 1 == cur.depth)
            .min_by(|a, b| (a.lane, &a.state_id).cmp(&(b.lane, &b.state_id)));
        let Some(p) = prev else { break };
        if !lanes.contains(&p.lane) {
            lanes.push(p.lane);
        }
        cur = p;
    }
    lanes.reverse();
    lanes
}

pub fn plan_with_edges(
    nodes: &[NodePos],
    variants: &[VariantPos],
    edges: &[EdgeRoute],
) -> CanvasPlan {
    let mut sorted: Vec<&NodePos> = nodes.iter().collect();
    sorted.sort_by(|a, b| {
        (
            a.lane,
            a.depth,
            a.order,
            a.variants.len(),
            a.state_id.as_str(),
        )
            .cmp(&(
                b.lane,
                b.depth,
                b.order,
                b.variants.len(),
                b.state_id.as_str(),
            ))
    });

    // Per lane: depth range and tallest (lane, depth) stack.
    let mut counts: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    let mut span: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    for n in &sorted {
        *counts.entry((n.lane, n.depth)).or_insert(0) += 1;
        let e = span.entry(n.lane).or_insert((n.depth, n.depth));
        e.0 = e.0.min(n.depth);
        e.1 = e.1.max(n.depth);
    }
    let mut stack: BTreeMap<u32, usize> = BTreeMap::new();
    for (&(lane, _), &c) in &counts {
        let s = stack.entry(lane).or_insert(0);
        *s = (*s).max(c);
    }

    // Packing order: spine lanes first (journey order), then the rest by
    // first column, then lane index. First-fit into rows of non-overlapping
    // depth ranges.
    let spine = spine_lanes(nodes, edges);
    let mut order: Vec<u32> = spine
        .iter()
        .copied()
        .filter(|l| span.contains_key(l))
        .collect();
    let mut rest: Vec<u32> = span
        .keys()
        .copied()
        .filter(|l| !order.contains(l))
        .collect();
    rest.sort_by_key(|l| (span[l].0, *l));
    order.extend(rest);
    let mut rows: Vec<Vec<u32>> = Vec::new();
    for lane in order {
        let (lo, hi) = span[&lane];
        let free = |row: &Vec<u32>| row.iter().all(|o| span[o].1 < lo || span[o].0 > hi);
        match rows.iter_mut().find(|r| free(r)) {
            Some(r) => r.push(lane),
            None => rows.push(vec![lane]),
        }
    }
    let mut lanes: BTreeMap<u32, Rect> = BTreeMap::new();
    let mut y = MARGIN;
    for row in &rows {
        let tallest = row.iter().map(|l| stack[l]).max().unwrap_or(1) as f64;
        let height = LANE_HEADER_H + tallest * NODE_H + (tallest - 1.0) * VGAP + TITLE_H;
        for &lane in row {
            let (lo, hi) = span[&lane];
            let x = MARGIN + lo as f64 * (NODE_W + HGAP);
            let w = (hi - lo) as f64 * (NODE_W + HGAP) + NODE_W;
            lanes.insert(lane, Rect { x, y, w, h: height });
        }
        y += height + LANE_GAP;
    }

    // Stack rank: position within the slot, in sorted (expand) order.
    let mut seen: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    let geometry: Vec<NodeGeometry> = sorted
        .iter()
        .map(|n| {
            let rank = {
                let slot = seen.entry((n.lane, n.depth)).or_insert(0);
                let r = *slot;
                *slot += 1;
                r
            };
            let lane_y = lanes[&n.lane].y + LANE_HEADER_H;
            NodeGeometry {
                state_id: n.state_id.clone(),
                lane: n.lane,
                depth: n.depth,
                order: n.order,
                title: n.title.clone(),
                variant_count: n.variants.len() as u32,
                rect: Rect {
                    x: MARGIN + n.depth as f64 * (NODE_W + HGAP),
                    y: lane_y + rank as f64 * (NODE_H + VGAP),
                    w: NODE_W,
                    h: NODE_H,
                },
            }
        })
        .collect();
    // Variants stack under their node's title area, in stack order.
    let rect_of = |id: &str| geometry.iter().find(|n| n.state_id == id).map(|n| n.rect);
    let variant_geometry: Vec<VariantGeometry> = variants
        .iter()
        .filter_map(|v| {
            rect_of(&v.variant_of).map(|node_rect| VariantGeometry {
                state_id: v.state_id.clone(),
                variant_of: v.variant_of.clone(),
                title: v.title.clone(),
                rect: Rect {
                    x: node_rect.x,
                    y: node_rect.max_y() + TITLE_H + v.order as f64 * (VARIANT_H + VGAP),
                    w: NODE_W,
                    h: VARIANT_H,
                },
            })
        })
        .collect();
    let mut bounds = content_bounds(&geometry, edges);
    // Lane headers sit above their first node.
    let top = lanes.values().map(|r| r.y).fold(bounds.y, f64::min);
    bounds.h += bounds.y - top;
    bounds.y = top;

    CanvasPlan {
        nodes: geometry,
        variants: variant_geometry,
        lanes,
        clusters: Vec::new(),
        bounds,
    }
}

/// The variant grid cells of an expanded node: 2 columns of 112x75 px
/// thumbnails with their state titles under each, 8 px gaps
/// (`storybook_kit::dot::variant_grid_size` sizes the node box). Cells are
/// centred under the title strip; only those that fit inside the node rect
/// are returned, so the grid reveals top-down while the box animates open.
pub fn variant_cells(node: Rect, count: u32) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let gap = storybook_kit::dot::GRID_GAP;
    let cell_w = storybook_kit::dot::GRID_CELL_W;
    let cell_h = storybook_kit::dot::GRID_THUMB_H + storybook_kit::dot::GRID_TITLE_H;
    let cols = storybook_kit::dot::GRID_COLS;
    let grid_w = cols as f64 * cell_w + (cols - 1) as f64 * gap;
    let grid_top = node.y + NODE_H + TITLE_H + gap;
    let x0 = node.x + (node.w - grid_w) / 2.0;
    let mut cells = Vec::new();
    for i in 0..count {
        let cell = Rect {
            x: x0 + (i % cols) as f64 * (cell_w + gap),
            y: grid_top + (i / cols) as f64 * (cell_h + gap),
            w: cell_w,
            h: cell_h,
        };
        if cell.max_y() > node.max_y() + 0.5 {
            break;
        }
        cells.push(cell);
    }
    cells
}

/// The builtin plan with expanded variant grids: each expanded node's box
/// (`state_id` -> extra px height) grows to hold its grid, and everything
/// at or below the node's row shifts down by the expansions above it, so
/// no two node boxes ever overlap. Lane segments shift with their nodes.
pub fn plan_expanded(
    nodes: &[storybook_kit::model::NodePos],
    variants: &[storybook_kit::model::VariantPos],
    edges: &[EdgeRoute],
    expanded: &BTreeMap<String, f64>,
) -> CanvasPlan {
    let mut plan = plan_with_edges(nodes, variants, edges);
    // (state id, base y, extra height), applied position-independently
    let deltas: Vec<(String, f64, f64)> = plan
        .nodes
        .iter()
        .filter_map(|n| {
            expanded
                .get(&n.state_id)
                .map(|extra| (n.state_id.clone(), n.rect.y, *extra))
        })
        .collect();
    if deltas.is_empty() {
        return plan;
    }
    for n in &mut plan.nodes {
        if let Some((_, _, extra)) = deltas.iter().find(|(id, _, _)| *id == n.state_id) {
            n.rect.h += extra;
        }
        let shift: f64 = deltas
            .iter()
            .filter(|(id, y, _)| id != &n.state_id && n.rect.y >= y - 0.01)
            .map(|(_, _, e)| e)
            .sum();
        n.rect.y += shift;
    }
    for seg in plan.lanes.values_mut() {
        let shift: f64 = deltas
            .iter()
            .filter(|(_, y, _)| seg.y >= y - 0.01)
            .map(|(_, _, e)| e)
            .sum();
        seg.y += shift;
    }
    plan.bounds = content_bounds(&plan.nodes, edges);
    plan
}

/// Bounds of everything the canvas draws: the node rects and the edge
/// curves — a back edge's arc drops below its lane, so its control points
/// must extend the bounds or Fit would crop it. Cubic Beziers lie inside
/// the hull of their control points, so the four edge points bound each
/// curve. Edges naming unknown states are ignored, exactly like when
/// drawing.
pub fn content_bounds(nodes: &[NodeGeometry], edges: &[EdgeRoute]) -> Rect {
    let Some(first) = nodes.first() else {
        return Rect::ZERO;
    };
    let mut min_x = first.rect.x;
    let mut min_y = first.rect.y;
    let mut max_x = first.rect.max_x();
    let mut max_y = first.rect.max_y();
    let mut extend = |x: f64, y: f64| {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    };
    for n in &nodes[1..] {
        extend(n.rect.x, n.rect.y);
        extend(n.rect.max_x(), n.rect.max_y());
    }
    let rects: BTreeMap<&str, Rect> = nodes
        .iter()
        .map(|n| (n.state_id.as_str(), n.rect))
        .collect();
    for edge in edges {
        // dot edges carry their own chain: its control-point hull bounds
        // the whole curve (Cubic Beziers stay inside it)
        if let Some(points) = chain_points(edge) {
            for p in points {
                extend(p.x, p.y);
            }
            continue;
        }
        let (Some(&from), Some(&to)) = (rects.get(edge.from.as_str()), rects.get(edge.to.as_str()))
        else {
            continue;
        };
        let g = edge_geometry(from, to, edge.back);
        for (x, y) in [g.start, g.c1, g.c2, g.end] {
            extend(x, y);
        }
    }
    Rect {
        x: min_x,
        y: min_y,
        w: max_x - min_x,
        h: max_y - min_y,
    }
}

/// Cubic Bezier from node `from` to node `to`. Forward edges run from the
/// right edge of the source into the left edge of the target; back edges
/// (`back: true`, including `to == "back"` resolved by `sbk verify`) are side
/// arcs leaving and entering below both nodes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeGeometry {
    pub start: (f64, f64),
    pub c1: (f64, f64),
    pub c2: (f64, f64),
    pub end: (f64, f64),
    /// Bezier midpoint, where the label is drawn.
    pub label_at: (f64, f64),
}

pub fn edge_geometry(from: Rect, to: Rect, back: bool) -> EdgeGeometry {
    let (start, end, c1, c2) = if back {
        let s = (from.cx(), from.max_y());
        let e = (to.cx(), to.max_y());
        let drop = BACK_DROP_MIN + 0.25 * (from.cx() - to.cx()).abs();
        (s, e, (s.0, s.1 + drop), (e.0, e.1 + drop))
    } else {
        let s = (from.max_x(), from.cy());
        let e = (to.x, to.cy());
        let k = ((e.0 - s.0).abs() * 0.4).clamp(40.0, 160.0);
        (s, e, (s.0 + k, s.1), (e.0 - k, e.1))
    };
    let label_at = (
        (start.0 + 3.0 * c1.0 + 3.0 * c2.0 + end.0) / 8.0,
        (start.1 + 3.0 * c1.1 + 3.0 * c2.1 + end.1) / 8.0,
    );
    EdgeGeometry {
        start,
        c1,
        c2,
        end,
        label_at,
    }
}

/// Filled arrowhead triangle with its tip at `tip`, pointing along the
/// travel direction `incoming_from -> tip`.
pub fn arrow_triangle(tip: (f64, f64), incoming_from: (f64, f64)) -> [(f64, f64); 3] {
    let dx = tip.0 - incoming_from.0;
    let dy = tip.1 - incoming_from.1;
    let len = dx.hypot(dy);
    let (dx, dy) = if len < f64::EPSILON {
        (1.0, 0.0)
    } else {
        (dx / len, dy / len)
    };
    let base = (tip.0 - dx * ARROW_LEN, tip.1 - dy * ARROW_LEN);
    let perp = (-dy, dx);
    [
        tip,
        (
            base.0 + perp.0 * ARROW_HALF_WIDTH,
            base.1 + perp.1 * ARROW_HALF_WIDTH,
        ),
        (
            base.0 - perp.0 * ARROW_HALF_WIDTH,
            base.1 - perp.1 * ARROW_HALF_WIDTH,
        ),
    ]
}

/// Node under a logical point; later nodes win (drawn on top).
pub fn node_at(nodes: &[NodeGeometry], logical: (f64, f64)) -> Option<&NodeGeometry> {
    nodes.iter().rev().find(|n| n.rect.contains(logical))
}

/// Is `node`'s variant grid on screen? It must be expanded **and** its
/// rect already grown past the thumbnail — right after a `+N` click the
/// relayout is still running and the node is drawn collapsed, so its grid
/// cells must not be hit or highlighted yet.
pub fn grid_shown(node: &NodeGeometry, expanded: &BTreeSet<String>) -> bool {
    expanded.contains(&node.state_id) && node.rect.h > NODE_H + 0.5
}

/// State under a logical point: a variant cell of an expanded node wins
/// (the grown node rect also covers its grid), else the node itself
/// (thumbnail, title strip). `None` over empty canvas.
pub fn state_at(
    plan: &CanvasPlan,
    expanded: &BTreeSet<String>,
    logical: (f64, f64),
) -> Option<String> {
    for node in plan.nodes.iter().rev() {
        if !grid_shown(node, expanded) {
            continue;
        }
        let variants = plan
            .variants
            .iter()
            .filter(|v| v.variant_of == node.state_id);
        for (cell, variant) in variant_cells(node.rect, node.variant_count)
            .iter()
            .zip(variants)
        {
            if cell.contains(logical) {
                return Some(variant.state_id.clone());
            }
        }
    }
    plan.nodes
        .iter()
        .rev()
        .find(|n| {
            let r = n.rect;
            let card = Rect {
                h: r.h.max(r.h.min(NODE_H) + TITLE_H),
                ..r
            };
            card.contains(logical)
        })
        .map(|n| n.state_id.clone())
}

/// Badge rect under a node that has data variants (`"+N variants"`),
/// directly under the node frame; `None` when there is nothing to badge.
pub fn badge_rect(node: &NodeGeometry) -> Option<Rect> {
    // A pill inside the node's top-right corner, so it never shares the
    // title strip under the node.
    (node.variant_count > 0).then(|| Rect {
        x: node.rect.max_x() - BADGE_W - 6.0,
        y: node.rect.y + 6.0,
        w: BADGE_W,
        h: TITLE_H,
    })
}

/// The title strip under a node (where its ellipsized title is drawn).
pub fn title_rect(node: Rect) -> Rect {
    Rect {
        x: node.x,
        y: node.y + node.h.min(NODE_H),
        w: NODE_W,
        h: TITLE_H,
    }
}

/// Where the selection highlight goes for `selected` (synced from the
/// story list): a flow node -> its thumbnail + title strip; a variant of an
/// expanded node -> its grid cell; a variant of a collapsed node -> the
/// node. `None` when the state is not on the map.
pub fn highlight_rect(
    plan: &CanvasPlan,
    expanded: &BTreeSet<String>,
    selected: &str,
) -> Option<Rect> {
    let node_box = |n: &NodeGeometry| Rect {
        x: n.rect.x,
        y: n.rect.y,
        w: n.rect.w,
        h: n.rect.h.min(NODE_H) + TITLE_H,
    };
    if let Some(n) = plan.nodes.iter().find(|n| n.state_id == selected) {
        return Some(node_box(n));
    }
    let variant = plan.variants.iter().find(|v| v.state_id == selected)?;
    let node = plan
        .nodes
        .iter()
        .find(|n| n.state_id == variant.variant_of)?;
    if !grid_shown(node, expanded) {
        return Some(node_box(node));
    }
    let index = plan
        .variants
        .iter()
        .filter(|v| v.variant_of == node.state_id)
        .position(|v| v.state_id == selected)?;
    variant_cells(node.rect, node.variant_count)
        .get(index)
        .copied()
        .or(Some(node_box(node)))
}

/// Opaque card colour for pills and expanded variant cards, matching the
/// theme implied by the foreground colour.
fn card_color(fg: &gdk::RGBA) -> gdk::RGBA {
    if fg.red() + fg.green() + fg.blue() > 1.5 {
        gdk::RGBA::new(0.14, 0.14, 0.15, 1.0)
    } else {
        gdk::RGBA::new(0.98, 0.98, 0.98, 1.0)
    }
}

/// Rounded opaque pill, no border.
#[mutants::skip] // drawing glue
fn pill(snapshot: &gtk::Snapshot, fill: &gdk::RGBA, r: Rect) {
    let rr = rounded(r, r.h / 2.0);
    snapshot.push_rounded_clip(&rr);
    snapshot.append_color(fill, rr.bounds());
    snapshot.pop();
}

/// `r` as a GSK rounded rect with corner `radius`.
fn rounded(r: Rect, radius: f64) -> gsk::RoundedRect {
    gsk::RoundedRect::from_rect(
        graphene::Rect::new(r.x as f32, r.y as f32, r.w as f32, r.h as f32),
        radius as f32,
    )
}

/// Card under a node: soft shadow + opaque rounded fill, no outline.
#[mutants::skip] // drawing glue
fn node_card(snapshot: &gtk::Snapshot, fill: &gdk::RGBA, shadow: &gdk::RGBA, r: Rect) {
    let rr = rounded(r, CARD_RADIUS);
    snapshot.append_outset_shadow(&rr, shadow, 0.0, 1.0, 0.0, 6.0);
    snapshot.push_rounded_clip(&rr);
    snapshot.append_color(fill, rr.bounds());
    snapshot.pop();
}

/// Point on the cubic Bezier of `g` at parameter `t` in 0..=1.
pub fn bezier_point(g: &EdgeGeometry, t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (
        a * g.start.0 + b * g.c1.0 + c * g.c2.0 + d * g.end.0,
        a * g.start.1 + b * g.c1.1 + c * g.c2.1 + d * g.end.1,
    )
}

/// Parameters tried along each edge, most preferred first.
const LABEL_TS: [f64; 9] = [0.5, 0.4, 0.6, 0.3, 0.7, 0.2, 0.8, 0.12, 0.88];

/// Deterministic greedy label placement: every label is a rect of size
/// `sizes[i]` centred on a point of its edge; the first candidate (along
/// [`LABEL_TS`], then nudged above/below the curve) that hits no obstacle
/// and no already placed label wins. When nothing is free the midpoint is
/// used. Returns each label's rect, in input order.
pub fn place_labels(edges: &[(EdgeGeometry, (f64, f64))], obstacles: &[Rect]) -> Vec<Rect> {
    let mut placed: Vec<Rect> = Vec::with_capacity(edges.len());
    for (g, (w, h)) in edges {
        let at = |t: f64, dy: f64| {
            let (x, y) = bezier_point(g, t);
            Rect {
                x: x - w / 2.0,
                y: y - h / 2.0 + dy,
                w: *w,
                h: *h,
            }
        };
        let free = |r: &Rect| {
            !obstacles.iter().any(|o| o.intersects(r)) && !placed.iter().any(|o| o.intersects(r))
        };
        let nudges = [0.0, -(h + 2.0), h + 2.0];
        let chosen = nudges
            .iter()
            .flat_map(|&dy| LABEL_TS.iter().map(move |&t| (t, dy)))
            .map(|(t, dy)| at(t, dy))
            .find(|r| free(r))
            .unwrap_or_else(|| at(0.5, 0.0));
        placed.push(chosen);
    }
    placed
}

/// Badge text for a variant count: a compact chip, the tooltip-free
/// `+N` (the menu names the variants).
pub fn badge_label(count: u32) -> String {
    format!("+{count}")
}

/// Node whose variant badge sits under a logical point; later nodes win.
pub fn badge_at(nodes: &[NodeGeometry], logical: (f64, f64)) -> Option<&NodeGeometry> {
    nodes
        .iter()
        .rev()
        .find(|n| badge_rect(n).is_some_and(|b| b.contains(logical)))
}

/// Is an edge drawn at the current \"Back edges\" toggle? Back edges are
/// hidden by default (canvas.md).
pub fn edge_visible(back: bool, show_back: bool) -> bool {
    show_back || !back
}

/// Flow node the map focuses on: the hovered node wins, else the node of
/// the selected state (a variant focuses its node). `None` when neither
/// is on the map — then nothing is dimmed and no edge label is shown.
pub fn focus_node(
    plan: &CanvasPlan,
    hovered: Option<&str>,
    selected: Option<&str>,
) -> Option<String> {
    let on_map = |id: &str| plan.nodes.iter().any(|n| n.state_id == id);
    if let Some(h) = hovered.filter(|h| on_map(h)) {
        return Some(h.to_string());
    }
    let selected = selected?;
    if on_map(selected) {
        return Some(selected.to_string());
    }
    plan.variants
        .iter()
        .find(|v| v.state_id == selected)
        .map(|v| v.variant_of.clone())
        .filter(|n| on_map(n))
}

/// The focused node and its direct neighbours over the visible edges
/// (either direction); these stay at full strength, the rest is dimmed.
pub fn focus_set(edges: &[EdgeRoute], focus: &str, show_back: bool) -> BTreeSet<String> {
    let mut set = BTreeSet::from([focus.to_string()]);
    for e in edges.iter().filter(|e| edge_visible(e.back, show_back)) {
        if e.from == focus {
            set.insert(e.to.clone());
        }
        if e.to == focus {
            set.insert(e.from.clone());
        }
    }
    set
}

/// Does an edge touch the focused node? Only such edges show their action
/// label and stay at full strength; without focus no label is shown.
pub fn edge_in_focus(from: &str, to: &str, focus: Option<&str>) -> bool {
    focus.is_some_and(|f| f == from || f == to)
}

// ---------------------------------------------------------------------------
// GTK widget
// ---------------------------------------------------------------------------

/// Everything the canvas draws, set at once by the application.
#[derive(Debug, Default)]
pub struct CanvasContent {
    pub plan: CanvasPlan,
    pub lane_names: Vec<String>,
    pub edges: Vec<EdgeRoute>,
    /// View cluster frames (dot layouts): drawn instead of lane headers.
    pub clusters: Vec<ClusterFrame>,
    pub paintables: BTreeMap<String, gtk::WidgetPaintable>,
}

mod imp {
    use super::*;

    pub struct Canvas {
        pub content: RefCell<CanvasContent>,
        pub transform: RefCell<ViewTransform>,
        pub viewport: Cell<(i32, i32)>,
        pub fitted: Cell<bool>,
        pub drag_start: Cell<Option<ViewTransform>>,
        pub zoom_last: Cell<f64>,
        pub pointer: Cell<(f64, f64)>,
        pub open_story: RefCell<Option<crate::StoryHook>>,
        /// Header toggle: back edges are hidden until it is on.
        pub show_back: Cell<bool>,
        /// Nodes whose variant grid is expanded.
        pub expanded: RefCell<BTreeSet<String>>,
        /// Handler run when a variant badge toggles a node: it must
        /// recompute the layout (off the main loop) and call
        /// [`Canvas::animate_to`] with the result.
        #[allow(clippy::type_complexity)] // same shape as StoryHook
        pub toggle_hook: RefCell<Option<Rc<dyn Fn(&str)>>>,
        /// Animation source geometry and clock; `None` when idle.
        pub anim_old_nodes: RefCell<Vec<NodeGeometry>>,
        pub anim_old_clusters: RefCell<Vec<ClusterFrame>>,
        pub anim_target_edges: RefCell<Vec<EdgeRoute>>,
        pub anim_start: Cell<Option<std::time::Instant>>,
        pub anim_ticking: Cell<bool>,
        /// State selected in the story list, highlighted when on the map.
        pub selected: RefCell<Option<String>>,
        /// Flow node under the pointer (variants map to their node); it
        /// takes the focus from the selection while hovered.
        pub hovered: RefCell<Option<String>>,
        /// Whether the pointer is over the canvas (hover is only
        /// recomputed then).
        pub pointer_inside: Cell<bool>,
    }

    impl Default for Canvas {
        fn default() -> Self {
            Self {
                content: RefCell::default(),
                transform: RefCell::new(ViewTransform::IDENTITY),
                viewport: Cell::new((0, 0)),
                fitted: Cell::new(false),
                drag_start: Cell::new(None),
                zoom_last: Cell::new(1.0),
                pointer: Cell::new((0.0, 0.0)),
                open_story: RefCell::default(),
                show_back: Cell::new(false),
                expanded: RefCell::default(),
                toggle_hook: RefCell::default(),
                anim_old_nodes: RefCell::default(),
                anim_old_clusters: RefCell::default(),
                anim_target_edges: RefCell::default(),
                anim_start: Cell::new(None),
                anim_ticking: Cell::new(false),
                selected: RefCell::default(),
                hovered: RefCell::default(),
                pointer_inside: Cell::new(false),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Canvas {
        const NAME: &'static str = "SbkCanvas";
        type Type = super::Canvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Canvas {}

    impl WidgetImpl for Canvas {
        #[mutants::skip] // size bookkeeping only; manual GUI check deferred to user
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            self.obj().imp().viewport.set((width, height));
            if !self.obj().imp().fitted.get() && width > 1 && height > 1 {
                self.obj().imp().fitted.set(true);
                self.obj().initial_show();
            }
        }

        #[mutants::skip] // one-line delegate to the skipped draw(); manual GUI check deferred
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.obj().draw(snapshot);
        }
    }
}

glib::wrapper! {
    /// Endless flow-map canvas: logical coordinates, live thumbnails, pan/zoom.
    pub struct Canvas(ObjectSubclass<imp::Canvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// `GDK_BUTTON_MIDDLE` (bindings do not expose the constant).
const GDK_BUTTON_MIDDLE: u32 = 2;

impl Canvas {
    pub fn new() -> Self {
        let obj: Self = glib::Object::new();
        // Drawing never spills outside the canvas area.
        obj.set_overflow(gtk::Overflow::Hidden);
        obj.setup_controllers();
        obj
    }

    /// Replace geometry, lane names, edges and live thumbnails in one go.
    #[mutants::skip] // state handoff to the imp struct; manual GUI check deferred
    pub fn set_content(
        &self,
        plan: CanvasPlan,
        lane_names: Vec<String>,
        edges: Vec<EdgeRoute>,
        paintables: BTreeMap<String, gtk::WidgetPaintable>,
    ) {
        let clusters = plan.clusters.clone();
        self.imp().expanded.borrow_mut().clear();
        *self.imp().content.borrow_mut() = CanvasContent {
            plan,
            lane_names,
            edges,
            clusters,
            paintables,
        };
        self.imp().fitted.set(false);
        self.imp().anim_start.set(None);
        self.refresh_hover();
        self.queue_draw();
    }

    /// Highlight `state_id` (selected in the story list) when it is on the
    /// map; `None` clears the highlight.
    #[mutants::skip] // state handoff + redraw; geometry is highlight_rect
    pub fn set_selected(&self, state_id: Option<&str>) {
        *self.imp().selected.borrow_mut() = state_id.map(str::to_string);
        self.queue_draw();
    }

    /// Header toggle \"Back edges\": back edges draw only while it is on.
    #[mutants::skip] // flag flip + redraw; the filter is the tested edge_visible
    pub fn set_show_back_edges(&self, show: bool) {
        self.imp().show_back.set(show);
        self.queue_draw();
    }

    /// Expand/collapse the variant grid of `state_id` (badge click) and
    /// notify the toggle handler, which recomputes the layout and calls
    /// [`Canvas::animate_to`].
    #[mutants::skip] // set toggle + callback; the relayout is the tested kit code
    pub fn toggle_variants(&self, state_id: &str) {
        if !self.imp().expanded.borrow_mut().remove(state_id) {
            self.imp()
                .expanded
                .borrow_mut()
                .insert(state_id.to_string());
        }
        if let Some(on_toggle) = self.imp().toggle_hook.borrow().clone() {
            on_toggle(state_id);
        }
        self.queue_draw();
    }

    /// Register the handler run when a variant badge toggles a node.
    #[mutants::skip] // callback registration; manual GUI check deferred
    pub fn connect_toggle_variants(&self, f: impl Fn(&str) + 'static) {
        *self.imp().toggle_hook.borrow_mut() = Some(Rc::new(f));
    }

    /// The nodes whose variant grid is expanded right now.
    #[mutants::skip] // state read for the relayout wiring
    pub fn expanded_ids(&self) -> BTreeSet<String> {
        self.imp().expanded.borrow().clone()
    }

    /// Animate from the geometry currently on screen to `plan`/`edges`
    /// over [`crate::anim::ANIM_MS`] with ease-out: node rects and cluster
    /// rects interpolate; edges draw with the builtin geometry between the
    /// interpolated rects until the animation ends and the dot splines
    /// return. A call while animating restarts from the blended geometry
    /// on screen.
    #[mutants::skip] // GTK clock glue over the tested anim:: blending
    pub fn animate_to(&self, plan: CanvasPlan, edges: Vec<EdgeRoute>) {
        let imp = self.imp();
        {
            let mut content = imp.content.borrow_mut();
            *imp.anim_old_nodes.borrow_mut() = content.plan.nodes.clone();
            *imp.anim_old_clusters.borrow_mut() = content.clusters.clone();
            *imp.anim_target_edges.borrow_mut() = edges;
            content.clusters = plan.clusters.clone();
            content.edges = crate::anim::plain_edges(imp.anim_target_edges.borrow().as_slice());
            content.plan = plan;
        }
        imp.anim_start.set(Some(std::time::Instant::now()));
        if !imp.anim_ticking.get() {
            imp.anim_ticking.set(true);
            let weak = self.downgrade();
            self.add_tick_callback(move |_, _| {
                let Some(canvas) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                if canvas.anim_frame() {
                    glib::ControlFlow::Continue
                } else {
                    glib::ControlFlow::Break
                }
            });
        }
        self.queue_draw();
    }

    /// One animation frame: blend old → target at the eased progress and
    /// end the animation with the target's full geometry. Returns whether
    /// to keep ticking.
    #[mutants::skip] // GTK clock glue over the tested anim:: blending
    fn anim_frame(&self) -> bool {
        let imp = self.imp();
        let Some(start) = imp.anim_start.get() else {
            imp.anim_ticking.set(false);
            return false;
        };
        let t = start.elapsed().as_millis() as f64 / crate::anim::ANIM_MS as f64;
        if t >= 1.0 {
            let edges = imp.anim_target_edges.borrow().clone();
            imp.anim_start.set(None);
            imp.anim_ticking.set(false);
            imp.content.borrow_mut().edges = edges;
            self.refresh_hover();
        } else {
            let e = crate::anim::ease_out(t);
            let mut content = imp.content.borrow_mut();
            content.plan.nodes =
                crate::anim::blend_nodes(&imp.anim_old_nodes.borrow(), &content.plan.nodes, e);
            content.clusters =
                crate::anim::blend_clusters(&imp.anim_old_clusters.borrow(), &content.clusters, e);
        }
        self.queue_draw();
        true
    }

    /// The edges drawn at the current \"Back edges\" toggle.
    fn visible_edges(content: &CanvasContent, show_back: bool) -> Vec<EdgeRoute> {
        content
            .edges
            .iter()
            .filter(|e| edge_visible(e.back, show_back))
            .cloned()
            .collect()
    }

    /// Register the handler run when a node is double-clicked: it must open
    /// that story full size and interactive in the story list.
    #[mutants::skip] // callback registration; manual GUI check deferred
    pub fn connect_open_story(&self, f: impl Fn(&str) + 'static) {
        *self.imp().open_story.borrow_mut() = Some(Rc::new(f));
    }

    /// Zoom to fit everything drawn — nodes, lane headers, edge arcs (`F`).
    #[mutants::skip] // delegates to the tested content_bounds/fit_transform; GUI-only
    pub fn fit(&self) {
        let imp = self.imp();
        let (w, h) = imp.viewport.get();
        let bounds = {
            let content = imp.content.borrow();
            content_bounds(
                &content.plan.nodes,
                &Self::visible_edges(&content, imp.show_back.get()),
            )
        };
        *imp.transform.borrow_mut() = fit_transform(bounds, (w as f64, h as f64));
        self.refresh_hover();
        self.queue_draw();
    }

    /// First show: the map always opens fitted, the whole journey in view.
    #[mutants::skip] // one-line delegate to fit()
    fn initial_show(&self) {
        self.fit();
    }

    #[mutants::skip] // delegates to the tested zoom_at; GUI-only
    fn zoom_around(&self, px: f64, py: f64, factor: f64) {
        let imp = self.imp();
        let t = *imp.transform.borrow();
        *imp.transform.borrow_mut() = zoom_at(t, px, py, factor);
        self.refresh_hover();
        self.queue_draw();
    }

    /// Recompute the hovered node and the cursor from the last pointer
    /// position: the map can move under a still pointer (zoom, fit,
    /// relayout animation, new content). Returns whether the hover changed.
    #[mutants::skip] // GTK glue over the tested state_at/focus_node
    fn refresh_hover(&self) -> bool {
        let imp = self.imp();
        if !imp.pointer_inside.get() {
            return imp.hovered.replace(None).is_some();
        }
        let (x, y) = imp.pointer.get();
        let (over, hovered) = {
            let logical = imp.transform.borrow().screen_to_logical(x, y);
            let content = imp.content.borrow();
            let state = state_at(&content.plan, &imp.expanded.borrow(), logical);
            let hovered = state
                .as_deref()
                .and_then(|s| focus_node(&content.plan, None, Some(s)));
            (
                badge_at(&content.plan.nodes, logical).is_some() || state.is_some(),
                hovered,
            )
        };
        self.set_cursor_from_name(Some(if over { "pointer" } else { "default" }));
        if *imp.hovered.borrow() != hovered {
            imp.hovered.replace(hovered);
            true
        } else {
            false
        }
    }

    #[mutants::skip] // GTK input wiring (pan/zoom/pinch/fit/dblclick); manual GUI check deferred
    fn setup_controllers(&self) {
        self.set_focusable(true);

        for button in [1, GDK_BUTTON_MIDDLE] {
            let drag = gtk::GestureDrag::new();
            drag.set_button(button);
            drag.connect_drag_begin(clone!(
                #[weak(rename_to = canvas)]
                self,
                move |_, _, _| {
                    let imp = canvas.imp();
                    imp.drag_start.set(Some(*imp.transform.borrow()));
                }
            ));
            drag.connect_drag_update(clone!(
                #[weak(rename_to = canvas)]
                self,
                move |_, dx, dy| {
                    let imp = canvas.imp();
                    if let Some(start) = imp.drag_start.get() {
                        *imp.transform.borrow_mut() = ViewTransform {
                            offset_x: start.offset_x + dx,
                            offset_y: start.offset_y + dy,
                            ..start
                        };
                        canvas.queue_draw();
                    }
                }
            ));
            self.add_controller(drag);
        }

        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(clone!(
            #[weak(rename_to = canvas)]
            self,
            move |_, x, y| {
                canvas.imp().pointer.set((x, y));
                canvas.imp().pointer_inside.set(true);
                // clickable things (states, badges) show the pointer cursor;
                // the node under the pointer takes the focus
                if canvas.refresh_hover() {
                    canvas.queue_draw();
                }
            }
        ));
        motion.connect_leave(clone!(
            #[weak(rename_to = canvas)]
            self,
            move |_| {
                canvas.imp().pointer_inside.set(false);
                if canvas.imp().hovered.replace(None).is_some() {
                    canvas.queue_draw();
                }
            }
        ));
        self.add_controller(motion);

        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.connect_scroll(clone!(
            #[weak(rename_to = canvas)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |ctrl, _dx, dy| {
                let surface = ctrl.unit() == gdk::ScrollUnit::Surface;
                match scroll_zoom(scroll_notches(dy, surface)) {
                    Some(factor) => {
                        let (px, py) = canvas.imp().pointer.get();
                        canvas.zoom_around(px, py, factor);
                        glib::Propagation::Stop
                    }
                    None => glib::Propagation::Proceed,
                }
            }
        ));
        self.add_controller(scroll);
        let zoom = gtk::GestureZoom::new();
        zoom.connect_begin(clone!(
            #[weak(rename_to = canvas)]
            self,
            move |_, _| {
                canvas.imp().zoom_last.set(1.0);
            }
        ));
        zoom.connect_scale_changed(clone!(
            #[weak(rename_to = canvas)]
            self,
            move |ctrl, scale| {
                let last = canvas.imp().zoom_last.replace(scale);
                let factor = if last > 0.0 { scale / last } else { 1.0 };
                if let Some((cx, cy)) = ctrl.bounding_box_center() {
                    canvas.zoom_around(cx, cy, factor);
                }
            }
        ));
        self.add_controller(zoom);

        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(clone!(
            #[weak(rename_to = canvas)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _code, _mod| match key_action(key.to_unicode()) {
                Some(KeyAction::Fit) => {
                    canvas.fit();
                    glib::Propagation::Stop
                }
                None => glib::Propagation::Proceed,
            }
        ));
        self.add_controller(keys);

        let click = gtk::GestureClick::new();
        click.set_button(1);
        click.connect_pressed(clone!(
            #[weak(rename_to = canvas)]
            self,
            move |_, _, _, _| {
                canvas.grab_focus();
            }
        ));
        click.connect_released(clone!(
            #[weak(rename_to = canvas)]
            self,
            move |_, n_press, x, y| {
                let imp = canvas.imp();
                let t = *imp.transform.borrow();
                let logical = t.screen_to_logical(x, y);
                // A "+N variants" pill expands/collapses its node.
                let badge = {
                    let content = imp.content.borrow();
                    badge_at(&content.plan.nodes, logical).map(|n| n.state_id.clone())
                };
                if let Some(state_id) = badge {
                    if n_press == 1 {
                        canvas.toggle_variants(&state_id);
                    }
                    return;
                }
                // Any other click on a state (node or expanded variant cell)
                // selects it: the menu walks to it, the preview shows it.
                let hit = {
                    let content = imp.content.borrow();
                    state_at(&content.plan, &imp.expanded.borrow(), logical)
                };
                if let Some(state_id) = hit
                    && let Some(open) = imp.open_story.borrow().clone()
                {
                    open(&state_id);
                }
            }
        ));
        self.add_controller(click);
    }

    /// Draw lanes, edges (GskPath curves + arrowheads + labels) and live
    /// thumbnail nodes. Pure rendering glue over the tested geometry.
    #[mutants::skip] // drawing glue over tested geometry; manual GUI check deferred
    fn draw(&self, snapshot: &gtk::Snapshot) {
        let imp = self.imp();

        let content = imp.content.borrow();
        let t = *imp.transform.borrow();
        if content.plan.nodes.is_empty() {
            return;
        }
        let fg = self.color();
        let alpha = |a: f64| gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), a as f32);
        let line = alpha(0.45);
        let dim = alpha(0.6);
        let faint = alpha(0.12);
        let accent = gdk::RGBA::new(0.21, 0.52, 0.89, 1.0);
        let shadow = gdk::RGBA::new(0.0, 0.0, 0.0, 0.25);
        let show_back = imp.show_back.get();
        // Focus: hovered node, else the selected state's node. Its edges
        // turn accent and show their labels; non-neighbours are dimmed.
        let focus = focus_node(
            &content.plan,
            imp.hovered.borrow().as_deref(),
            imp.selected.borrow().as_deref(),
        );
        let near = focus
            .as_deref()
            .map(|f| focus_set(&content.edges, f, show_back));
        let dimmed = |id: &str| near.as_ref().is_some_and(|n| !n.contains(id));

        snapshot.save();
        snapshot.translate(&graphene::Point::new(t.offset_x as f32, t.offset_y as f32));
        snapshot.scale(t.scale as f32, t.scale as f32);

        let card = card_color(&fg);
        let card_fill = card;
        // Everything an edge label must not cover: headers, nodes, titles.
        let mut obstacles: Vec<Rect> = Vec::new();
        // Lane headers: one per view segment, above its first node.
        for (lane, seg) in content.plan.lanes.iter() {
            let name = content
                .lane_names
                .get(*lane as usize)
                .cloned()
                .unwrap_or_else(|| format!("lane {lane}"));
            let layout = self.create_pango_layout(Some(&name));
            let (w, h) = layout.pixel_size();
            obstacles.push(Rect {
                x: seg.x,
                y: seg.y,
                w: w as f64,
                h: h as f64,
            });
            snapshot.save();
            snapshot.translate(&graphene::Point::new(seg.x as f32, seg.y as f32));
            snapshot.append_layout(&layout, &fg);
            snapshot.restore();
        }

        // Dot layouts: one softly tinted background per view (no outline),
        // view name on top.
        for cluster in &content.clusters {
            let frame = rounded(cluster.rect, 2.0 * CARD_RADIUS);
            snapshot.push_rounded_clip(&frame);
            snapshot.append_color(&alpha(0.05), frame.bounds());
            snapshot.pop();
            let layout = self.create_pango_layout(Some(&cluster.view));
            let mut bold = gtk::pango::FontDescription::new();
            bold.set_weight(gtk::pango::Weight::Bold);
            layout.set_font_description(Some(&bold));
            obstacles.push(Rect {
                x: cluster.rect.x,
                y: cluster.rect.y,
                w: cluster.rect.w,
                h: 24.0,
            });
            snapshot.save();
            snapshot.translate(&graphene::Point::new(
                (cluster.rect.x + 8.0) as f32,
                (cluster.rect.y + 4.0) as f32,
            ));
            snapshot.append_layout(&layout, &dim);
            snapshot.restore();
        }

        // Edges: cubic curves, arrowheads, action labels.
        let rects: BTreeMap<&str, Rect> = content
            .plan
            .nodes
            .iter()
            .map(|n| (n.state_id.as_str(), n.rect))
            .collect();
        let mut label_jobs: Vec<(EdgeGeometry, gtk::pango::Layout)> = Vec::new();
        for edge in content
            .edges
            .iter()
            .filter(|e| edge_visible(e.back, show_back))
        {
            let (Some(from), Some(to)) =
                (rects.get(edge.from.as_str()), rects.get(edge.to.as_str()))
            else {
                continue;
            };
            let in_focus = edge_in_focus(&edge.from, &edge.to, focus.as_deref());
            let (line, width) = match (&focus, in_focus) {
                (_, true) => (accent, 2.5),
                (Some(_), false) => (faint, 1.5),
                (None, _) => (line, 1.5),
            };
            let mut label_at: Option<(storybook_kit::model::GeoPoint, gtk::pango::Layout)> = None;
            if let Some(points) = chain_points(edge) {
                // dot engine: the stored Bezier chain, arrowhead and label
                let b = gsk::PathBuilder::new();
                b.move_to(points[0].x as f32, points[0].y as f32);
                for triple in points[1..].as_chunks::<3>().0 {
                    b.cubic_to(
                        triple[0].x as f32,
                        triple[0].y as f32,
                        triple[1].x as f32,
                        triple[1].y as f32,
                        triple[2].x as f32,
                        triple[2].y as f32,
                    );
                }
                snapshot.append_stroke(&b.to_path(), &gsk::Stroke::new(width), &line);
                let ab = gsk::PathBuilder::new();
                if let Some(arrow) = &edge.arrow {
                    let mut it = arrow.iter();
                    if let Some(first) = it.next() {
                        ab.move_to(first.x as f32, first.y as f32);
                        for p in it {
                            ab.line_to(p.x as f32, p.y as f32);
                        }
                        ab.close();
                        snapshot.append_fill(&ab.to_path(), gsk::FillRule::Winding, &line);
                    }
                } else {
                    let n = points.len();
                    let tip = (points[n - 1].x, points[n - 1].y);
                    let from = (points[n - 2].x, points[n - 2].y);
                    for p in arrow_triangle(tip, from) {
                        ab.line_to(p.0 as f32, p.1 as f32);
                    }
                    ab.close();
                    snapshot.append_fill(&ab.to_path(), gsk::FillRule::Winding, &line);
                }
                if let Some(label) = edge.label.as_ref().filter(|_| in_focus)
                    && let Some(lp) = edge.label_pos
                {
                    let layout = self.create_pango_layout(Some(label));
                    layout.set_width((LABEL_MAX_W as i32) * gtk::pango::SCALE);
                    layout.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    label_at = Some((lp, layout));
                }
            } else {
                let g = edge_geometry(*from, *to, edge.back);
                let b = gsk::PathBuilder::new();
                b.move_to(g.start.0 as f32, g.start.1 as f32);
                b.cubic_to(
                    g.c1.0 as f32,
                    g.c1.1 as f32,
                    g.c2.0 as f32,
                    g.c2.1 as f32,
                    g.end.0 as f32,
                    g.end.1 as f32,
                );
                snapshot.append_stroke(&b.to_path(), &gsk::Stroke::new(width), &line);

                let tri = arrow_triangle(g.end, g.c2);
                let ab = gsk::PathBuilder::new();
                ab.move_to(tri[0].0 as f32, tri[0].1 as f32);
                ab.line_to(tri[1].0 as f32, tri[1].1 as f32);
                ab.line_to(tri[2].0 as f32, tri[2].1 as f32);
                ab.close();
                snapshot.append_fill(&ab.to_path(), gsk::FillRule::Winding, &line);

                if let Some(label) = edge.label.as_ref().filter(|_| in_focus) {
                    // Placed after the nodes (collision-free), drawn last.
                    let layout = self.create_pango_layout(Some(label));
                    layout.set_width((LABEL_MAX_W as i32) * gtk::pango::SCALE);
                    layout.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    label_jobs.push((g, layout));
                }
            }
            if let Some((lp, layout)) = label_at {
                // dot engine: the label sits where dot placed it, as an
                // opaque pill centred on the stored position
                let (w, h) = layout.pixel_size();
                pill(
                    snapshot,
                    &card,
                    Rect {
                        x: lp.x - (w as f64) / 2.0 - PILL_PAD,
                        y: lp.y - (h as f64) / 2.0 - PILL_PAD,
                        w: w as f64 + 2.0 * PILL_PAD,
                        h: h as f64 + 2.0 * PILL_PAD,
                    },
                );
                snapshot.save();
                snapshot.translate(&graphene::Point::new(
                    (lp.x - (w as f64) / 2.0) as f32,
                    (lp.y - (h as f64) / 2.0) as f32,
                ));
                snapshot.append_layout(&layout, &fg);
                snapshot.restore();
            }
        }

        // Nodes: card (shadow, no outline) + live thumbnail + state label.
        for n in &content.plan.nodes {
            let r = n.rect;
            let is_dimmed = dimmed(&n.state_id);
            if is_dimmed {
                snapshot.push_opacity(DIM_OPACITY);
            }
            node_card(snapshot, &card_fill, &shadow, r);

            // An expanded node's rect is taller than the thumbnail: the
            // extra height is the inline variant grid under the title strip.
            let expanded = r.h > NODE_H + 0.5;
            let thumb = Rect {
                h: NODE_H.min(r.h),
                ..r
            };
            if let Some(paintable) = content.paintables.get(&n.state_id) {
                // Letterbox: keep the story's aspect ratio inside the thumb.
                let (tw, th) = fit_size(thumb.w, thumb.h, paintable.intrinsic_aspect_ratio());
                snapshot.push_rounded_clip(&rounded(thumb, CARD_RADIUS));
                snapshot.save();
                snapshot.translate(&graphene::Point::new(
                    (thumb.x + (thumb.w - tw) / 2.0) as f32,
                    (thumb.y + (thumb.h - th) / 2.0) as f32,
                ));
                paintable.snapshot(snapshot, tw, th);
                snapshot.restore();
                snapshot.pop();
            }

            let label = n
                .title
                .as_deref()
                .unwrap_or_else(|| n.state_id.rsplit('/').next().unwrap_or(n.state_id.as_str()));
            let layout = self.create_pango_layout(Some(label));
            layout.set_width((NODE_W as i32) * gtk::pango::SCALE);
            layout.set_ellipsize(gtk::pango::EllipsizeMode::End);
            obstacles.push(r);
            obstacles.push(title_rect(r));
            let title_y = if expanded { thumb.max_y() } else { r.max_y() };
            snapshot.save();
            snapshot.translate(&graphene::Point::new(r.x as f32, (title_y + 4.0) as f32));
            snapshot.append_layout(&layout, &fg);
            snapshot.restore();

            if expanded {
                // the card under the whole grown rect backs the grid
                let variant_geometry: Vec<&VariantGeometry> = content
                    .plan
                    .variants
                    .iter()
                    .filter(|v| v.variant_of == n.state_id)
                    .collect();
                for (cell, variant) in variant_cells(r, n.variant_count)
                    .iter()
                    .zip(variant_geometry)
                {
                    obstacles.push(*cell);
                    if let Some(paintable) = content.paintables.get(&variant.state_id) {
                        let (tw, th) = fit_size(
                            cell.w,
                            storybook_kit::dot::GRID_THUMB_H,
                            paintable.intrinsic_aspect_ratio(),
                        );
                        snapshot.save();
                        snapshot.translate(&graphene::Point::new(
                            (cell.x + (cell.w - tw) / 2.0) as f32,
                            (cell.y + (storybook_kit::dot::GRID_THUMB_H - th) / 2.0) as f32,
                        ));
                        paintable.snapshot(snapshot, tw, th);
                        snapshot.restore();
                    }
                    // authored variant title, else the classes part
                    let vlabel = variant.title.as_deref().unwrap_or_else(|| {
                        variant
                            .state_id
                            .rsplit('/')
                            .next()
                            .unwrap_or(variant.state_id.as_str())
                    });
                    let layout = self.create_pango_layout(Some(vlabel));
                    layout.set_width((cell.w as i32) * gtk::pango::SCALE);
                    layout.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    snapshot.save();
                    snapshot.translate(&graphene::Point::new(
                        cell.x as f32,
                        (cell.y + storybook_kit::dot::GRID_THUMB_H) as f32,
                    ));
                    snapshot.append_layout(&layout, &dim);
                    snapshot.restore();
                }
            }

            if let Some(badge) = badge_rect(n) {
                let layout = self.create_pango_layout(Some(&badge_label(n.variant_count)));
                let (w, h) = layout.pixel_size();
                pill(
                    snapshot,
                    &accent,
                    Rect {
                        x: badge.max_x() - w as f64 - 2.0 * PILL_PAD,
                        y: badge.y,
                        w: w as f64 + 2.0 * PILL_PAD,
                        h: h as f64 + 2.0 * PILL_PAD,
                    },
                );
                snapshot.save();
                snapshot.translate(&graphene::Point::new(
                    (badge.max_x() - w as f64 - PILL_PAD) as f32,
                    (badge.y + PILL_PAD) as f32,
                ));
                snapshot.append_layout(&layout, &gdk::RGBA::WHITE);
                snapshot.restore();
            }
            if is_dimmed {
                snapshot.pop();
            }
        }

        // Selection synced from the story list: accent frame on top of nodes.
        if let Some(selected) = self.imp().selected.borrow().as_deref()
            && let Some(h) = highlight_rect(&content.plan, &self.imp().expanded.borrow(), selected)
        {
            let frame = gsk::RoundedRect::from_rect(
                graphene::Rect::new(
                    (h.x - 4.0) as f32,
                    (h.y - 4.0) as f32,
                    (h.w + 8.0) as f32,
                    (h.h + 8.0) as f32,
                ),
                (CARD_RADIUS + 4.0) as f32,
            );
            snapshot.append_border(&frame, &[3.0; 4], &[accent; 4]);
        }

        // Edge labels: collision-free placement, opaque pills, on top.
        let sized: Vec<(EdgeGeometry, (f64, f64))> = label_jobs
            .iter()
            .map(|(g, l)| {
                let (w, h) = l.pixel_size();
                (*g, (w as f64 + 2.0 * PILL_PAD, h as f64 + 2.0 * PILL_PAD))
            })
            .collect();
        for (rect, (_, layout)) in place_labels(&sized, &obstacles).iter().zip(&label_jobs) {
            pill(snapshot, &card, *rect);
            snapshot.save();
            snapshot.translate(&graphene::Point::new(
                (rect.x + PILL_PAD) as f32,
                (rect.y + PILL_PAD) as f32,
            ));
            snapshot.append_layout(layout, &fg);
            snapshot.restore();
        }

        snapshot.restore();
    }
}

impl Default for Canvas {
    fn default() -> Self {
        Self::new()
    }
}
