//! Animated re-layout (references/canvas.md): when a node's variant grid
//! expands or collapses, the canvas animates from the old geometry to the
//! new one. Everything here is pure math — the GTK tick callback in
//! `canvas.rs` only clocks it.
//!
//! During the animation edges are drawn with the builtin `edge_geometry`
//! between the interpolated rects; the dot splines (and their stored label
//! positions) return when it ends.

use crate::canvas::{ClusterFrame, NodeGeometry};
use storybook_kit::model::EdgeRoute;

/// Relayout animation duration (ms).
pub const ANIM_MS: u64 = 250;

/// Ease-out cubic: fast start, gentle landing.
pub fn ease_out(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn lerp_rect(a: crate::canvas::Rect, b: crate::canvas::Rect, t: f64) -> crate::canvas::Rect {
    crate::canvas::Rect {
        x: lerp(a.x, b.x, t),
        y: lerp(a.y, b.y, t),
        w: lerp(a.w, b.w, t),
        h: lerp(a.h, b.h, t),
    }
}

/// Nodes interpolated between `old` and `new` at eased progress `t`:
/// matched by state id, rects lerp. Nodes without an old twin start at
/// their new position; the remaining fields come from `new`.
pub fn blend_nodes(old: &[NodeGeometry], new: &[NodeGeometry], t: f64) -> Vec<NodeGeometry> {
    new.iter()
        .map(|n| {
            let rect = old
                .iter()
                .find(|o| o.state_id == n.state_id)
                .map_or(n.rect, |o| lerp_rect(o.rect, n.rect, t));
            NodeGeometry { rect, ..n.clone() }
        })
        .collect()
}

/// Cluster frames interpolated between `old` and `new` at eased progress
/// `t`, matched by view.
pub fn blend_clusters(old: &[ClusterFrame], new: &[ClusterFrame], t: f64) -> Vec<ClusterFrame> {
    new.iter()
        .map(|c| {
            let rect = old
                .iter()
                .find(|o| o.view == c.view)
                .map_or(c.rect, |o| lerp_rect(o.rect, c.rect, t));
            ClusterFrame { rect, ..c.clone() }
        })
        .collect()
}

/// A copy of `edges` without dot geometry, so the animation draws them
/// with the builtin `edge_geometry` between the interpolated rects.
pub fn plain_edges(edges: &[EdgeRoute]) -> Vec<EdgeRoute> {
    edges
        .iter()
        .map(|e| EdgeRoute {
            points: None,
            label_pos: None,
            arrow: None,
            ..e.clone()
        })
        .collect()
}

/// Generation counter for relayout results: every toggle issues a ticket,
/// and only the newest ticket's result is still current — stale results
/// from superseded toggles are dropped, which is what keeps a burst of
/// badge clicks race-free.
#[derive(Debug, Default)]
pub struct Relay {
    current: std::sync::atomic::AtomicU64,
}

impl Relay {
    /// Issue the ticket for a new relayout, superseding older ones.
    pub fn issue(&self) -> u64 {
        use std::sync::atomic::Ordering;
        self.current.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Is `ticket` still the newest relayout?
    pub fn is_current(&self, ticket: u64) -> bool {
        use std::sync::atomic::Ordering;
        ticket == self.current.load(Ordering::Relaxed)
    }
}
