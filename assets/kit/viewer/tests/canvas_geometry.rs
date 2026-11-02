//! Pure flow-map geometry: transform math, layout planning, edge shapes,
//! arrowheads and hit testing (references/canvas.md).

use storybook_kit::model::{EdgeRoute, NodePos, VariantPos};
use storybook_viewer::canvas::{
    ARROW_HALF_WIDTH, ARROW_LEN, BACK_DROP_MIN, FIT_PADDING, HGAP, KeyAction, LANE_GAP,
    LANE_HEADER_H, MARGIN, MAX_SCALE, MIN_SCALE, NODE_H, NODE_W, NodeGeometry, Rect, TITLE_H,
    VARIANT_H, VGAP, ViewTransform, arrow_triangle, badge_at, badge_label, badge_rect,
    content_bounds, edge_geometry, edge_visible, fit_size, fit_transform, initial_view, key_action,
    node_at, plan, scroll_zoom, zoom_at,
};
fn node(state_id: &str, lane: u32, depth: u32, order: u32) -> NodePos {
    NodePos {
        state_id: state_id.to_string(),
        lane,
        depth,
        order,
        title: None,
        self_actions: vec![],
        variants: vec![],
        variant_count: 0,
        rect: None,
    }
}

#[test]
fn logical_screen_roundtrip() {
    let t = ViewTransform {
        scale: 2.5,
        offset_x: 10.0,
        offset_y: -4.0,
    };
    for (x, y) in [(0.0, 0.0), (12.5, -300.25), (1e6, -1e6)] {
        let (sx, sy) = t.logical_to_screen(x, y);
        let (bx, by) = t.screen_to_logical(sx, sy);
        assert!((bx - x).abs() < 1e-9, "x mismatch for ({x},{y}): {bx}");
        assert!((by - y).abs() < 1e-9, "y mismatch for ({x},{y}): {by}");
    }
}

#[test]
fn screen_to_logical_inverts_translation_first() {
    // screen = logical * scale + offset, so the inverse must divide before subtracting.
    let t = ViewTransform {
        scale: 2.0,
        offset_x: 100.0,
        offset_y: 50.0,
    };
    assert_eq!(t.screen_to_logical(104.0, 54.0), (2.0, 2.0));
}

#[test]
fn zoom_at_keeps_the_point_under_the_pointer_fixed() {
    let t = ViewTransform {
        scale: 1.0,
        offset_x: 20.0,
        offset_y: 30.0,
    };
    let (px, py) = (300.0, 200.0);
    let before = t.screen_to_logical(px, py);
    let zipped = zoom_at(t, px, py, 1.75);
    let after = zipped.screen_to_logical(px, py);
    assert!(
        (before.0 - after.0).abs() < 1e-9,
        "logical x moved: {} -> {}",
        before.0,
        after.0
    );
    assert!(
        (before.1 - after.1).abs() < 1e-9,
        "logical y moved: {} -> {}",
        before.1,
        after.1
    );
    assert!((zipped.scale - 1.75).abs() < 1e-12);
}
#[test]
fn zoom_at_clamps_to_bounds() {
    let t = ViewTransform {
        scale: MAX_SCALE,
        offset_x: 0.0,
        offset_y: 0.0,
    };
    assert_eq!(zoom_at(t, 5.0, 5.0, 2.0).scale, MAX_SCALE);
    let t = ViewTransform {
        scale: MIN_SCALE,
        offset_x: 0.0,
        offset_y: 0.0,
    };
    assert_eq!(zoom_at(t, 5.0, 5.0, 0.25).scale, MIN_SCALE);
}

#[test]
fn fit_transform_fits_and_centers_with_padding() {
    let bounds = Rect {
        x: 100.0,
        y: 200.0,
        w: 500.0,
        h: 300.0,
    };
    let viewport = (1000.0, 800.0);
    let t = fit_transform(bounds, viewport);
    let expected_scale = ((1000.0 - 2.0 * FIT_PADDING) / 500.0)
        .min((800.0 - 2.0 * FIT_PADDING) / 300.0)
        .clamp(MIN_SCALE, MAX_SCALE);
    assert!((t.scale - expected_scale).abs() < 1e-9);
    let on_screen = Rect {
        x: bounds.x * t.scale + t.offset_x,
        y: bounds.y * t.scale + t.offset_y,
        w: bounds.w * t.scale,
        h: bounds.h * t.scale,
    };
    // Fully visible with the requested padding on every side, centered.
    assert!(on_screen.x >= FIT_PADDING - 1e-6);
    assert!(on_screen.y >= FIT_PADDING - 1e-6);
    assert!(on_screen.max_x() <= 1000.0 - FIT_PADDING + 1e-6);
    assert!(on_screen.max_y() <= 800.0 - FIT_PADDING + 1e-6);
    let hslack = 1000.0 - FIT_PADDING - on_screen.max_x();
    let hmargin = on_screen.x - FIT_PADDING;
    assert!(
        (hslack - hmargin).abs() < 1e-6,
        "not centered horizontally: {hslack} vs {hmargin}"
    );
}

#[test]
fn fit_transform_is_identity_for_degenerate_input() {
    assert_eq!(
        fit_transform(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 100.0
            },
            (500.0, 500.0)
        ),
        ViewTransform::IDENTITY
    );
    assert_eq!(
        fit_transform(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 0.0
            },
            (500.0, 500.0)
        ),
        ViewTransform::IDENTITY
    );
    assert_eq!(
        fit_transform(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 100.0
            },
            (0.0, 0.0)
        ),
        ViewTransform::IDENTITY
    );
}

#[test]
fn plan_stacks_same_lane_and_depth_by_order() {
    let nodes = [
        node("v/a", 0, 0, 0),
        node("v/d", 0, 0, 3), // same slot as v/a, later expand order -> stacked below
        node("v/b", 0, 1, 1),
        node("w/c", 1, 0, 2),
    ];
    let p = plan(&nodes, &[]);
    assert_eq!(p.nodes.len(), 4);
    let a = p.nodes.iter().find(|n| n.state_id == "v/a").unwrap();
    let d = p.nodes.iter().find(|n| n.state_id == "v/d").unwrap();
    let b = p.nodes.iter().find(|n| n.state_id == "v/b").unwrap();
    let c = p.nodes.iter().find(|n| n.state_id == "w/c").unwrap();
    // Same depth => same x; stacked with the documented gap, earlier order on top.
    assert_eq!(a.rect.x, d.rect.x);
    assert_eq!(a.rect.y, d.rect.y - (NODE_H + VGAP));
    // One depth step right.
    assert_eq!(b.rect.x, a.rect.x + NODE_W + HGAP);
    assert_eq!(b.rect.y, a.rect.y);
    // Lane 1 overlaps lane 0 in depth 0, so it gets the next row: below
    // lane 0's header, tallest stack (2) and title strip.
    let row0 = LANE_HEADER_H + 2.0 * NODE_H + VGAP + TITLE_H;
    assert_eq!(c.rect.y, MARGIN + row0 + LANE_GAP + LANE_HEADER_H);
    assert_eq!(c.rect.x, a.rect.x);
    assert_eq!(
        p.lanes[&0],
        Rect {
            x: MARGIN,
            y: MARGIN,
            w: 2.0 * NODE_W + HGAP,
            h: row0
        }
    );
    assert_eq!(p.lanes[&1].y + LANE_HEADER_H, c.rect.y);
    // Bounds contain every node.
    for n in &p.nodes {
        assert!(p.bounds.x <= n.rect.x && p.bounds.y <= n.rect.y);
        assert!(p.bounds.max_x() >= n.rect.max_x() && p.bounds.max_y() >= n.rect.max_y());
    }
    assert_eq!(p.bounds.max_x(), MARGIN + NODE_W + HGAP + NODE_W);
}

#[test]
fn plan_is_deterministic_regardless_of_input_order() {
    let a = [node("v/a", 0, 0, 0), node("v/b", 0, 1, 1)];
    let b = [node("v/b", 0, 1, 1), node("v/a", 0, 0, 0)];
    assert_eq!(plan(&a, &[]).nodes, plan(&b, &[]).nodes);
}

#[test]
fn plan_on_empty_input_is_empty() {
    let p = plan(&[], &[]);
    assert!(p.nodes.is_empty());
    assert!(p.lanes.is_empty());
    assert_eq!(
        p.bounds,
        Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0
        }
    );
}

#[test]
fn edge_geometry_forward_runs_left_to_right() {
    let from = Rect {
        x: 0.0,
        y: 0.0,
        w: NODE_W,
        h: NODE_H,
    };
    let to = Rect {
        x: 416.0,
        y: 0.0,
        w: NODE_W,
        h: NODE_H,
    };
    let g = edge_geometry(from, to, false);
    // Exits the right edge center of `from`, enters the left edge of `to`.
    assert_eq!(g.start, (from.max_x(), from.cy()));
    assert_eq!(g.end, (to.x, to.cy()));
    assert!(g.c1.0 > g.start.0);
    assert!(g.c2.0 < g.end.0);
    // Cubic control points stay on the lane line.
    assert_eq!(g.c1.1, g.start.1);
    assert_eq!(g.c2.1, g.end.1);
    assert!(g.label_at.0 > g.start.0 && g.label_at.0 < g.end.0);
}

#[test]
fn rect_centers_are_midpoints() {
    let r = Rect {
        x: 1.0,
        y: 2.0,
        w: 6.0,
        h: 10.0,
    };
    assert_eq!(r.cx(), 4.0);
    assert_eq!(r.cy(), 7.0);
}

#[test]
fn zoom_at_keeps_the_point_fixed_from_a_non_identity_scale() {
    let t = ViewTransform {
        scale: 2.0,
        offset_x: 40.0,
        offset_y: -20.0,
    };
    let (px, py) = (120.0, 80.0);
    let before = t.screen_to_logical(px, py);
    let z = zoom_at(t, px, py, 4.0);
    assert_eq!(z.scale, 8.0);
    let after = z.screen_to_logical(px, py);
    assert!(
        (before.0 - after.0).abs() < 1e-9,
        "x moved: {} -> {}",
        before.0,
        after.0
    );
    assert!(
        (before.1 - after.1).abs() < 1e-9,
        "y moved: {} -> {}",
        before.1,
        after.1
    );
}

#[test]
fn fit_transform_width_binding_case_is_exact() {
    // w ratio 1.904 < h ratio 2.48: width binds.
    let bounds = Rect {
        x: 100.0,
        y: 200.0,
        w: 500.0,
        h: 300.0,
    };
    let (vw, vh) = (1000.0, 800.0);
    let t = fit_transform(bounds, (vw, vh));
    let s = (vw - 2.0 * FIT_PADDING) / 500.0;
    assert!((t.scale - s).abs() < 1e-12);
    assert!((t.offset_x - ((vw - 500.0 * s) / 2.0 - 100.0 * s)).abs() < 1e-9);
    assert!((t.offset_y - ((vh - 300.0 * s) / 2.0 - 200.0 * s)).abs() < 1e-9);
}

#[test]
fn fit_transform_height_binding_case_is_exact() {
    // h ratio 552/700 < w ratio 4.76: height binds.
    let bounds = Rect {
        x: 10.0,
        y: 20.0,
        w: 200.0,
        h: 700.0,
    };
    let (vw, vh) = (1000.0, 600.0);
    let t = fit_transform(bounds, (vw, vh));
    let s = (vh - 2.0 * FIT_PADDING) / 700.0;
    assert!((t.scale - s).abs() < 1e-12);
    assert!((t.offset_x - ((vw - 200.0 * s) / 2.0 - 10.0 * s)).abs() < 1e-9);
    assert!((t.offset_y - ((vh - 700.0 * s) / 2.0 - 20.0 * s)).abs() < 1e-9);
}

#[test]
fn fit_transform_is_identity_when_either_viewport_dimension_is_zero() {
    let ok = Rect {
        x: 0.0,
        y: 0.0,
        w: 100.0,
        h: 100.0,
    };
    assert_eq!(fit_transform(ok, (0.0, 400.0)), ViewTransform::IDENTITY);
    assert_eq!(fit_transform(ok, (400.0, 0.0)), ViewTransform::IDENTITY);
}

#[test]
fn plan_bounds_wrap_headers_and_every_node_exactly() {
    let nodes = [
        node("v/a", 0, 0, 0),
        node("v/d", 0, 0, 3),
        node("v/b", 0, 1, 1),
        node("w/c", 1, 0, 2),
    ];
    let p = plan(&nodes, &[]);
    // Lane headers sit above their nodes; the first one starts at the margin.
    assert_eq!(p.bounds.x, MARGIN);
    assert_eq!(p.bounds.y, MARGIN);
    assert_eq!(p.bounds.w, 2.0 * NODE_W + HGAP);
    // row 0 (header + 2-stack + title strip) + lane gap + lane 1 header + node.
    assert_eq!(
        p.bounds.h,
        2.0 * LANE_HEADER_H + 3.0 * NODE_H + VGAP + TITLE_H + LANE_GAP
    );
}

#[test]
fn content_bounds_extends_to_the_back_edge_arc_drop() {
    // Regression (review Important 5): the back-edge arc dips below the
    // lane, so `F` must fit it in as well.
    let p = plan(&[node("v/a", 0, 0, 0), node("v/b", 0, 1, 1)], &[]);
    let (a, b) = (
        p.nodes.iter().find(|n| n.state_id == "v/a").unwrap().rect,
        p.nodes.iter().find(|n| n.state_id == "v/b").unwrap().rect,
    );
    let drop = BACK_DROP_MIN + 0.25 * (b.cx() - a.cx()).abs();
    let back = EdgeRoute {
        from: "v/b".to_string(),
        to: "v/a".to_string(),
        back: true,
        label: None,
        points: None,
        label_pos: None,
        arrow: None,
    };
    let bounds = content_bounds(&p.nodes, &[back]);
    assert_eq!(bounds.x, MARGIN);
    assert_eq!(bounds.w, p.bounds.w);
    assert_eq!(bounds.max_y(), p.bounds.max_y() + drop);
}

#[test]
fn content_bounds_keeps_forward_edges_inside_the_plan_bounds() {
    let p = plan(&[node("v/a", 0, 0, 0), node("v/b", 0, 1, 1)], &[]);
    let forward = EdgeRoute {
        from: "v/a".to_string(),
        to: "v/b".to_string(),
        back: false,
        label: Some("save".to_string()),
        points: None,
        label_pos: None,
        arrow: None,
    };
    assert_eq!(
        content_bounds(&p.nodes, &[forward]),
        content_bounds(&p.nodes, &[])
    );
}

#[test]
fn content_bounds_ignores_edges_to_unknown_states_and_empty_input() {
    let p = plan(&[node("v/a", 0, 0, 0)], &[]);
    let ghost = EdgeRoute {
        from: "v/a".to_string(),
        to: "v/ghost".to_string(),
        back: true,
        label: None,
        points: None,
        label_pos: None,
        arrow: None,
    };
    assert_eq!(
        content_bounds(&p.nodes, &[ghost]),
        content_bounds(&p.nodes, &[])
    );
    assert_eq!(content_bounds(&[], &[]), Rect::ZERO);
}

#[test]
fn scroll_zooms_without_modifier_and_follows_the_wheel_direction() {
    // user: zoom with the plain wheel, no Ctrl
    let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
    assert!(close(scroll_zoom(-1.0).unwrap(), 1.1));
    assert!(close(scroll_zoom(1.0).unwrap(), 1.0 / 1.1));
    // smooth touchpad deltas scale, big jumps are clamped to 3 notches
    assert!(close(scroll_zoom(-0.5).unwrap(), 1.1f64.powf(0.5)));
    assert!(close(scroll_zoom(-10.0).unwrap(), 1.1f64.powi(3)));
    assert!(close(scroll_zoom(10.0).unwrap(), 1.1f64.powi(-3)));
    // no vertical motion, no zoom
    assert_eq!(scroll_zoom(0.0), None);
}

#[test]
fn key_action_fits_on_f_and_passes_everything_else() {
    assert_eq!(key_action(Some('f')), Some(KeyAction::Fit));
    assert_eq!(key_action(Some('F')), Some(KeyAction::Fit));
    assert_eq!(key_action(Some('x')), None);
    assert_eq!(key_action(None), None);
}

#[test]
fn fit_size_letterboxes_inside_the_node_box() {
    assert_eq!(fit_size(160.0, 100.0, 2.0), (160.0, 80.0));
    assert_eq!(fit_size(160.0, 100.0, 0.5), (50.0, 100.0));
    assert_eq!(fit_size(160.0, 100.0, 1.6), (160.0, 100.0));
    // Unusable ratios fall back to filling the whole box.
    assert_eq!(fit_size(160.0, 100.0, 0.0), (160.0, 100.0));
    assert_eq!(fit_size(160.0, 100.0, -2.0), (160.0, 100.0));
    assert_eq!(fit_size(160.0, 100.0, f64::NAN), (160.0, 100.0));
}

#[test]
fn edge_geometry_forward_control_points_are_exact_when_unclamped() {
    // Horizontal distance 200 => k = 0.4 * 200 = 80, inside [40, 160].
    let from = Rect {
        x: 0.0,
        y: 0.0,
        w: 100.0,
        h: 40.0,
    };
    let to = Rect {
        x: 300.0,
        y: 100.0,
        w: 100.0,
        h: 40.0,
    };
    let g = edge_geometry(from, to, false);
    assert_eq!(g.start, (100.0, 20.0));
    assert_eq!(g.end, (300.0, 120.0));
    assert_eq!(g.c1, (180.0, 20.0));
    assert_eq!(g.c2, (220.0, 120.0));
    // Cubic Bezier midpoint at t = 0.5.
    assert_eq!(g.label_at, (200.0, 70.0));
}

#[test]
fn edge_geometry_back_arc_drop_is_exact() {
    let from = Rect {
        x: 0.0,
        y: 0.0,
        w: 100.0,
        h: 40.0,
    };
    let to = Rect {
        x: 300.0,
        y: 0.0,
        w: 100.0,
        h: 40.0,
    };
    let g = edge_geometry(from, to, true);
    let drop = BACK_DROP_MIN + 0.25 * (50.0f64 - 350.0).abs();
    assert_eq!(g.start, (50.0, 40.0));
    assert_eq!(g.end, (350.0, 40.0));
    assert_eq!(g.c1, (50.0, 40.0 + drop));
    assert_eq!(g.c2, (350.0, 40.0 + drop));
    assert_eq!(
        g.label_at,
        (200.0, (40.0 + 3.0 * (40.0 + drop) * 2.0 + 40.0) / 8.0)
    );
}

#[test]
fn arrow_triangle_diagonal_direction_is_exact() {
    // Travel (2,-2) -> (10,4): unit (0.8, 0.6), base (2,-2), perp (-0.6, 0.8).
    let [tip, b1, b2] = arrow_triangle((10.0, 4.0), (2.0, -2.0));
    let close = |a: (f64, f64), b: (f64, f64), what: &str| {
        assert!((a.0 - b.0).abs() < 1e-9, "{what}.0: {a:?} != {b:?}");
        assert!((a.1 - b.1).abs() < 1e-9, "{what}.1: {a:?} != {b:?}");
    };
    close(tip, (10.0, 4.0), "tip");
    close(b1, (-1.0, 2.0), "b1");
    close(b2, (5.0, -6.0), "b2");
}

#[test]
fn arrow_triangle_zero_length_falls_back_to_plus_x() {
    let [tip, b1, b2] = arrow_triangle((5.0, 5.0), (5.0, 5.0));
    assert_eq!(tip, (5.0, 5.0));
    assert_eq!(b1, (-5.0, 10.0));
    assert_eq!(b2, (-5.0, 0.0));
}

#[test]
fn arrow_triangle_epsilon_length_is_a_real_direction() {
    // len == f64::EPSILON is NOT the degenerate case: only strictly smaller is.
    let [tip, b1, b2] = arrow_triangle((0.0, 0.0), (f64::EPSILON, 0.0));
    assert_eq!(tip, (0.0, 0.0));
    assert_eq!(b1, (10.0, -5.0));
    assert_eq!(b2, (10.0, 5.0));
}

#[test]
fn edge_geometry_back_is_a_side_arc_below_both_nodes() {
    let from = Rect {
        x: 0.0,
        y: 0.0,
        w: NODE_W,
        h: NODE_H,
    };
    let to = Rect {
        x: 416.0,
        y: 0.0,
        w: NODE_W,
        h: NODE_H,
    };
    let g = edge_geometry(from, to, true);
    assert_eq!(g.start, (from.cx(), from.max_y()));
    assert_eq!(g.end, (to.cx(), to.max_y()));
    assert!(g.c1.1 > g.start.1, "arc must bulge below the source");
    assert!(g.c2.1 > g.end.1, "arc must bulge below the target");
    assert!(g.label_at.1 > from.max_y());
}

#[test]
fn arrow_triangle_points_along_travel_direction() {
    // Travel along +x, tip at (10, 0): base sits ARROW_LEN behind, symmetric.
    let [tip, b1, b2] = arrow_triangle((10.0, 0.0), (0.0, 0.0));
    assert_eq!(tip, (10.0, 0.0));
    let base_x = 10.0 - ARROW_LEN;
    let mut ys = [b1.1, b2.1];
    ys.sort_by(|x, y| x.partial_cmp(y).unwrap());
    assert_eq!(ys, [-ARROW_HALF_WIDTH, ARROW_HALF_WIDTH]);
    assert!((b1.0 - base_x).abs() < 1e-9 && (b2.0 - base_x).abs() < 1e-9);
}

#[test]
fn arrow_triangle_with_no_direction_does_not_panic() {
    let tri = arrow_triangle((5.0, 5.0), (5.0, 5.0));
    assert_eq!(tri[0], (5.0, 5.0));
}

#[test]
fn node_at_returns_the_last_drawn_overlapping_node() {
    let nodes = [
        NodeGeometry {
            state_id: "v/a".into(),
            lane: 0,
            depth: 0,
            order: 0,
            title: None,
            variant_count: 0,
            rect: Rect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
        },
        NodeGeometry {
            state_id: "v/b".into(),
            lane: 0,
            depth: 1,
            order: 1,
            title: None,
            variant_count: 0,
            rect: Rect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
        },
    ];
    assert_eq!(node_at(&nodes, (5.0, 5.0)).unwrap().state_id, "v/b");
    assert!(node_at(&nodes, (50.0, 50.0)).is_none());
    // Borders are inclusive.
    assert_eq!(node_at(&nodes, (0.0, 0.0)).unwrap().state_id, "v/b");
    assert_eq!(node_at(&nodes, (10.0, 10.0)).unwrap().state_id, "v/b");
    assert!(node_at(&nodes, (10.0001, 5.0)).is_none());
}

#[test]
fn plan_carries_node_titles_for_the_canvas_labels() {
    let titled = NodePos {
        state_id: "v/root".to_string(),
        lane: 0,
        depth: 0,
        order: 0,
        title: Some("Pierwsze uruchomienie".to_string()),
        self_actions: vec![],
        variants: vec![],
        variant_count: 0,
        rect: None,
    };
    let p = plan(&[titled, node("v/other", 0, 1, 1)], &[]);
    let root = p.nodes.iter().find(|n| n.state_id == "v/root").unwrap();
    let other = p.nodes.iter().find(|n| n.state_id == "v/other").unwrap();
    assert_eq!(root.title.as_deref(), Some("Pierwsze uruchomienie"));
    assert_eq!(other.title, None);
}

#[test]
fn initial_view_zooms_the_root_to_80_percent_and_centers_it() {
    // 0.8 * 800 / NODE_W binds (width-limited); the height ratio is larger.
    let root = Rect {
        x: 100.0,
        y: 50.0,
        w: NODE_W,
        h: NODE_H,
    };
    let t = initial_view(root, (800.0, 600.0));
    const { assert!(0.8 * 600.0 / NODE_H > 0.8 * 800.0 / NODE_W) };
    assert!(
        (t.scale - 0.8 * 800.0 / NODE_W).abs() < 1e-9,
        "scale {}",
        t.scale
    );
    let (sx, sy) = t.logical_to_screen(100.0 + NODE_W / 2.0, 50.0 + NODE_H / 2.0);
    assert!((sx - 400.0).abs() < 1e-9, "center x {sx}");
    assert!((sy - 300.0).abs() < 1e-9, "center y {sy}");
    // the node covers 80% of the limiting dimension on screen
    assert!((t.scale * NODE_W - 0.8 * 800.0).abs() < 1e-9);

    // height binds when the node is tall
    let tall = Rect {
        x: 0.0,
        y: 0.0,
        w: 400.0,
        h: 500.0,
    };
    let t = initial_view(tall, (800.0, 600.0));
    assert!(
        (t.scale - (0.8 * 600.0 / 500.0)).abs() < 1e-9,
        "scale {}",
        t.scale
    );

    // degenerate viewport or node: identity, never a NaN
    assert_eq!(initial_view(tall, (0.0, 0.0)), ViewTransform::IDENTITY);
    assert_eq!(
        initial_view(Rect::ZERO, (800.0, 600.0)),
        ViewTransform::IDENTITY
    );
}

#[test]
fn initial_view_clamps_the_scale_into_the_canvas_bounds() {
    // a tiny root in a huge viewport would zoom past MAX_SCALE
    let t = initial_view(
        Rect {
            x: 0.0,
            y: 0.0,
            w: 4.0,
            h: 4.0,
        },
        (4000.0, 4000.0),
    );
    assert_eq!(t.scale, MAX_SCALE);
    let t = initial_view(
        Rect {
            x: 0.0,
            y: 0.0,
            w: 1e6,
            h: 1e6,
        },
        (100.0, 100.0),
    );
    assert_eq!(t.scale, MIN_SCALE);
}

fn variant(state_id: &str, variant_of: &str, lane: u32, depth: u32, order: u32) -> VariantPos {
    VariantPos {
        state_id: state_id.to_string(),
        variant_of: variant_of.to_string(),
        title: None,
        lane,
        depth,
        order,
    }
}

#[test]
fn badge_exists_only_for_nodes_with_variants() {
    let mut many = node("v/a", 0, 0, 0);
    many.variants = vec!["v/x".to_string(), "v/y".to_string()];
    let bare = node("v/b", 0, 1, 1);
    let nodes = [
        NodeGeometry {
            state_id: many.state_id.clone(),
            lane: 0,
            depth: 0,
            order: 0,
            title: None,
            variant_count: 2,
            rect: Rect {
                x: 0.0,
                y: 0.0,
                w: NODE_W,
                h: NODE_H,
            },
        },
        NodeGeometry {
            state_id: bare.state_id.clone(),
            lane: 0,
            depth: 1,
            order: 1,
            title: None,
            variant_count: 0,
            rect: Rect {
                x: 300.0,
                y: 0.0,
                w: NODE_W,
                h: NODE_H,
            },
        },
    ];
    let badge = badge_rect(&nodes[0]).unwrap();
    // A pill inside the node's top-right corner, clear of the title strip.
    let r = nodes[0].rect;
    assert_eq!(badge.max_x(), r.max_x() - 6.0);
    assert_eq!(badge.y, r.y + 6.0);
    assert!(badge.max_y() < r.max_y());
    assert!(!badge.intersects(&storybook_viewer::canvas::title_rect(r)));
    assert!(badge_rect(&nodes[1]).is_none());
    // hit testing: only the badged node answers inside its pill
    let inside = (badge.cx(), badge.cy());
    assert_eq!(badge_at(&nodes, inside).unwrap().state_id, "v/a");
    assert!(badge_at(&nodes, (r.x + 2.0, r.y + 2.0)).is_none());
    assert!(badge_at(&nodes, (300.0 + inside.0, inside.1)).is_none());
    assert_eq!(badge_label(2), "+2");
    assert_eq!(badge_label(12), "+12");
}

#[test]
fn variants_stack_under_their_node() {
    let mut root = node("v/a", 0, 0, 0);
    root.variants = vec!["v/x".to_string(), "v/y".to_string()];
    let variants = [
        variant("v/x", "v/a", 0, 0, 0),
        variant("v/y", "v/a", 0, 0, 1),
    ];
    let p = plan(&[root], &variants);
    let node = &p.nodes[0];
    assert_eq!(node.variant_count, 2);
    let x = &p.variants[0];
    let y = &p.variants[1];
    assert_eq!(x.rect.x, node.rect.x);
    assert_eq!(x.rect.y, node.rect.max_y() + TITLE_H);
    assert_eq!(y.rect.y, x.rect.max_y() + VGAP);
    assert_eq!(x.rect.w, NODE_W);
    assert_eq!(x.rect.h, VARIANT_H);
    // a variant naming an unknown node is dropped, not a panic
    let p2 = plan(
        &[node_state("v/a")],
        &[variant("v/z", "v/missing", 0, 0, 0)],
    );
    assert!(p2.variants.is_empty());
}

fn node_state(state_id: &str) -> NodePos {
    node(state_id, 0, 0, 0)
}

#[test]
fn back_edges_are_hidden_until_the_toggle() {
    assert!(!edge_visible(true, false));
    assert!(edge_visible(true, true));
    assert!(edge_visible(false, false));
    assert!(edge_visible(false, true));
}

#[test]
fn lanes_with_disjoint_depth_ranges_share_a_row() {
    // lane 0 at depth 0, lane 1 at depth 1, lane 2 at depth 0..1 (overlaps both).
    let nodes = [
        node("a/x", 0, 0, 0),
        node("b/x", 1, 1, 0),
        node("c/x", 2, 0, 0),
        node("c/y", 2, 1, 1),
    ];
    let p = plan(&nodes, &[]);
    assert_eq!(p.lanes[&0].y, p.lanes[&1].y);
    assert!(p.lanes[&2].y > p.lanes[&0].y);
    let b = p.nodes.iter().find(|n| n.state_id == "b/x").unwrap();
    assert_eq!(b.rect.x, MARGIN + NODE_W + HGAP);
}

#[test]
fn journey_spine_takes_the_top_row() {
    use storybook_kit::model::EdgeRoute;
    // Spine: c/x (lane 2, depth 0) -> a/x (lane 0, depth 1). Lane 1 also
    // starts at depth 0 but is not on the longest path.
    let nodes = [
        node("c/x", 2, 0, 0),
        node("a/x", 0, 1, 0),
        node("b/x", 1, 0, 0),
    ];
    let e = |f: &str, t: &str, back: bool| EdgeRoute {
        from: f.into(),
        to: t.into(),
        back,
        label: None,
        points: None,
        label_pos: None,
        arrow: None,
    };
    let edges = [e("c/x", "a/x", false), e("b/x", "c/x", true)];
    let p = storybook_viewer::canvas::plan_with_edges(&nodes, &[], &edges);
    assert_eq!(p.lanes[&2].y, MARGIN);
    assert_eq!(p.lanes[&0].y, MARGIN);
    assert!(p.lanes[&1].y > MARGIN);
    // Without edges the lower lane index wins the first row at depth 0.
    let q = plan(&nodes, &[]);
    assert_eq!(q.lanes[&1].y, MARGIN);
    assert!(q.lanes[&2].y > MARGIN);
}

mod labels {
    use storybook_viewer::canvas::{Rect, bezier_point, edge_geometry, place_labels};

    fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn bezier_point_hits_the_endpoints() {
        let g = edge_geometry(
            r(0.0, 0.0, 100.0, 50.0),
            r(300.0, 200.0, 100.0, 50.0),
            false,
        );
        assert_eq!(bezier_point(&g, 0.0), g.start);
        assert_eq!(bezier_point(&g, 1.0), g.end);
        let m = bezier_point(&g, 0.5);
        assert!((m.0 - g.label_at.0).abs() < 1e-9 && (m.1 - g.label_at.1).abs() < 1e-9);
    }

    #[test]
    fn free_midpoint_is_used_as_is() {
        let g = edge_geometry(r(0.0, 0.0, 100.0, 50.0), r(400.0, 0.0, 100.0, 50.0), false);
        let out = place_labels(&[(g, (40.0, 16.0))], &[]);
        let m = bezier_point(&g, 0.5);
        assert_eq!(out[0], r(m.0 - 20.0, m.1 - 8.0, 40.0, 16.0));
    }

    #[test]
    fn labels_avoid_obstacles_and_each_other() {
        let a = r(0.0, 0.0, 100.0, 50.0);
        let b = r(600.0, 0.0, 100.0, 50.0);
        let g = edge_geometry(a, b, false);
        let m = bezier_point(&g, 0.5);
        // A node sits right on the midpoint.
        let blocker = r(m.0 - 50.0, m.1 - 25.0, 100.0, 50.0);
        let out = place_labels(&[(g, (40.0, 16.0)), (g, (40.0, 16.0))], &[blocker, a, b]);
        for l in &out {
            assert!(!l.intersects(&blocker) && !l.intersects(&a) && !l.intersects(&b));
        }
        assert!(!out[0].intersects(&out[1]));
        // Deterministic.
        assert_eq!(
            out,
            place_labels(&[(g, (40.0, 16.0)), (g, (40.0, 16.0))], &[blocker, a, b])
        );
    }

    #[test]
    fn crowded_label_falls_back_to_the_midpoint() {
        let g = edge_geometry(r(0.0, 0.0, 100.0, 50.0), r(400.0, 0.0, 100.0, 50.0), false);
        let wall = r(-1000.0, -1000.0, 3000.0, 3000.0);
        let out = place_labels(&[(g, (40.0, 16.0))], &[wall]);
        let m = bezier_point(&g, 0.5);
        assert_eq!(out[0], r(m.0 - 20.0, m.1 - 8.0, 40.0, 16.0));
    }

    #[test]
    fn title_rect_is_the_strip_under_the_node() {
        use storybook_viewer::canvas::{NODE_W, TITLE_H, title_rect};
        assert_eq!(
            title_rect(r(5.0, 7.0, NODE_W, 50.0)),
            r(5.0, 57.0, NODE_W, TITLE_H)
        );
    }

    #[test]
    fn blocked_curve_nudges_the_label_above_then_below() {
        // Two nodes on one row: the edge is a horizontal line at cy = 25.
        let g = edge_geometry(r(0.0, 0.0, 100.0, 50.0), r(500.0, 0.0, 100.0, 50.0), false);
        let (w, h) = (40.0, 16.0);
        let m = bezier_point(&g, 0.5);
        let band = r(-1000.0, m.1 - h / 2.0 - 1.0, 3000.0, h + 2.0);
        let out = place_labels(&[(g, (w, h))], &[band]);
        assert_eq!(out[0], r(m.0 - w / 2.0, m.1 - h / 2.0 - (h + 2.0), w, h));
        let above = r(-1000.0, m.1 - 3.0 * h, 3000.0, 2.0 * h);
        let out = place_labels(&[(g, (w, h))], &[band, above]);
        assert_eq!(out[0], r(m.0 - w / 2.0, m.1 - h / 2.0 + (h + 2.0), w, h));
    }

    #[test]
    fn intersects_needs_positive_overlap() {
        let a = r(0.0, 0.0, 10.0, 10.0);
        // touching from every side, both argument orders
        for o in [
            r(10.0, 0.0, 5.0, 10.0),
            r(-5.0, 0.0, 5.0, 10.0),
            r(0.0, 10.0, 10.0, 5.0),
            r(0.0, -5.0, 10.0, 5.0),
        ] {
            assert!(!a.intersects(&o) && !o.intersects(&a), "{o:?}");
        }
        assert!(a.intersects(&r(5.0, 5.0, 10.0, 10.0)));
        assert!(!a.intersects(&r(10.0, 0.0, 10.0, 10.0))); // touching edge
        assert!(!a.intersects(&r(0.0, 10.0, 10.0, 10.0)));
        assert!(!a.intersects(&r(20.0, 20.0, 1.0, 1.0)));
    }
}

#[test]
fn lane_segment_spans_its_columns() {
    let nodes = [node("a/x", 0, 1, 0), node("a/y", 0, 2, 1)];
    let p = plan(&nodes, &[]);
    assert_eq!(p.lanes[&0].x, MARGIN + NODE_W + HGAP);
    assert_eq!(p.lanes[&0].w, 2.0 * NODE_W + HGAP);
    assert_eq!(p.lanes[&0].h, LANE_HEADER_H + NODE_H + TITLE_H);
}

#[test]
fn spine_follows_only_edges_into_the_current_node() {
    use storybook_kit::model::EdgeRoute;
    let e = |f: &str, t: &str, back: bool| EdgeRoute {
        from: f.into(),
        to: t.into(),
        back,
        label: None,
        points: None,
        label_pos: None,
        arrow: None,
    };
    // spine: a (lane 0, d0) -> b (lane 4, d1) -> d (lane 3, d2); c (lane 2, d1)
    // leads elsewhere (forward to e, back to a) and must not join the spine.
    let nodes = [
        node("a/x", 0, 0, 0),
        node("b/x", 4, 1, 0),
        node("d/x", 3, 2, 0),
        node("c/x", 2, 1, 0),
        node("e/x", 5, 2, 0),
    ];
    let edges = [
        e("a/x", "b/x", false),
        e("b/x", "d/x", false),
        e("a/x", "c/x", false),
        e("c/x", "a/x", true),
        e("c/x", "e/x", false),
    ];
    let p = storybook_viewer::canvas::plan_with_edges(&nodes, &[], &edges);
    assert_eq!(p.lanes[&4].y, MARGIN);
    assert_eq!(p.lanes[&3].y, MARGIN);
    assert!(p.lanes[&2].y > MARGIN);
}

// --- dot engine: geometry straight from layout.json (references/canvas.md) --

use storybook_kit::model::{ClusterPos, GeoPoint, GeoRect};

fn dot_node(state_id: &str, rect: GeoRect, variant_count: u32) -> NodePos {
    NodePos {
        state_id: state_id.to_string(),
        lane: 0,
        depth: 0,
        order: 0,
        title: None,
        self_actions: vec![],
        variants: vec![],
        variant_count,
        rect: Some(rect),
    }
}

#[test]
fn plan_dot_draws_nodes_at_stored_rects() {
    let nodes = vec![
        dot_node(
            "a/s1",
            GeoRect {
                x: 10.0,
                y: 20.0,
                w: 240.0,
                h: 160.0,
            },
            0,
        ),
        dot_node(
            "b/s3",
            GeoRect {
                x: 500.0,
                y: 400.0,
                w: 240.0,
                h: 160.0,
            },
            2,
        ),
    ];
    let variants = vec![
        storybook_kit::model::VariantPos {
            state_id: "b/one".to_string(),
            variant_of: "b/s3".to_string(),
            title: None,
            lane: 1,
            depth: 2,
            order: 0,
        },
        storybook_kit::model::VariantPos {
            state_id: "b/two".to_string(),
            variant_of: "b/s3".to_string(),
            title: None,
            lane: 1,
            depth: 2,
            order: 1,
        },
    ];
    let p = storybook_viewer::canvas::plan_dot(&nodes, &variants, &[]);

    // hit testing uses the stored rects, wherever dot put the nodes
    assert_eq!(
        node_at(&p.nodes, (130.0, 100.0)).map(|n| n.state_id.as_str()),
        Some("a/s1")
    );
    assert_eq!(
        node_at(&p.nodes, (620.0, 480.0)).map(|n| n.state_id.as_str()),
        Some("b/s3")
    );
    assert_eq!(node_at(&p.nodes, (300.0, 100.0)), None);
    // badge pill sits inside the stored rect's top-right corner
    let badge = badge_rect(&p.nodes[1]).unwrap();
    assert!((badge.y - (400.0 + 6.0)).abs() < 0.001, "{badge:?}");
    assert!(badge.max_x() <= 740.0);
    // variants stack under their node: same x, increasing y, VARIANT_H tall
    assert_eq!(p.variants.len(), 2);
    assert!((p.variants[0].rect.x - 500.0).abs() < 0.001);
    assert!(p.variants[1].rect.y > p.variants[0].rect.y);
    assert!((p.variants[0].rect.h - VARIANT_H).abs() < 0.001);
    // bounds cover both stored rects (Fit centres on everything drawn)
    assert!((p.bounds.x - 10.0).abs() < 0.001);
    assert!(p.bounds.max_x() >= 740.0);
}

#[test]
fn content_bounds_covers_dot_edge_chains() {
    let nodes = vec![dot_node(
        "a/s1",
        GeoRect {
            x: 0.0,
            y: 0.0,
            w: 240.0,
            h: 160.0,
        },
        0,
    )];
    // a chain dipping below every node rect: Fit must not crop it
    let chain = EdgeRoute {
        from: "a/s1".into(),
        to: "a/s1".into(),
        back: true,
        label: None,
        points: Some(vec![
            GeoPoint { x: 10.0, y: 150.0 },
            GeoPoint { x: 5.0, y: 400.0 },
            GeoPoint { x: 235.0, y: 400.0 },
            GeoPoint { x: 230.0, y: 150.0 },
        ]),
        label_pos: None,
        arrow: None,
    };
    let g: Vec<NodeGeometry> = storybook_viewer::canvas::plan_dot(&nodes, &[], &[])
        .nodes
        .clone();
    let b = content_bounds(&g, &[chain]);
    assert!(b.max_y() >= 400.0, "{b:?}");
}

#[test]
fn dot_edge_chains_and_cluster_frames_carry_geometry() {
    // the pure accessors the draw code uses: a stored chain is a start
    // point plus whole cubic triples; clusters carry a frame rect + label
    let edge = EdgeRoute {
        from: "a/s1".into(),
        to: "a/s2".into(),
        back: false,
        label: None,
        points: Some(vec![
            GeoPoint { x: 1.0, y: 2.0 },
            GeoPoint { x: 3.0, y: 4.0 },
            GeoPoint { x: 5.0, y: 6.0 },
            GeoPoint { x: 7.0, y: 8.0 },
        ]),
        label_pos: Some(GeoPoint { x: 4.0, y: 5.0 }),
        arrow: Some(vec![
            GeoPoint { x: 7.0, y: 8.0 },
            GeoPoint { x: 6.0, y: 9.0 },
            GeoPoint { x: 8.0, y: 9.0 },
        ]),
    };
    let pts = storybook_viewer::canvas::chain_points(&edge).unwrap();
    assert_eq!(pts.len(), 4);
    assert!((pts[0].x - 1.0).abs() < f64::EPSILON);
    assert_eq!(
        storybook_viewer::canvas::chain_points(&EdgeRoute {
            from: "a".into(),
            to: "b".into(),
            back: false,
            label: None,
            points: None,
            label_pos: None,
            arrow: None,
        }),
        None
    );

    let cluster = ClusterPos {
        view: "a".to_string(),
        label: "a".to_string(),
        rect: GeoRect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 50.0,
        },
    };
    let frame = storybook_viewer::canvas::cluster_frame(&cluster);
    assert!((frame.rect.w - 100.0).abs() < 0.001);
    assert_eq!(cluster.label, "a");
}

// --- selection highlight (story list -> map sync) ---------------------------

fn hl_plan() -> storybook_viewer::canvas::CanvasPlan {
    use storybook_viewer::canvas::{CanvasPlan, VariantGeometry};
    let rect = Rect {
        x: 100.0,
        y: 50.0,
        w: NODE_W,
        h: NODE_H,
    };
    let node = |id: &str, count: u32, rect: Rect| NodeGeometry {
        state_id: id.to_string(),
        lane: 0,
        depth: 0,
        order: 0,
        title: None,
        variant_count: count,
        rect,
    };
    let variant = |id: &str| VariantGeometry {
        state_id: id.to_string(),
        variant_of: "v/a".to_string(),
        title: None,
        rect,
    };
    CanvasPlan {
        nodes: vec![
            node("v/a", 2, rect),
            node(
                "v/b",
                0,
                Rect {
                    x: 500.0,
                    ..rect
                },
            ),
        ],
        variants: vec![variant("v/a1"), variant("v/a2")],
        lanes: Default::default(),
        clusters: vec![],
        bounds: rect,
    }
}

#[test]
fn highlight_covers_node_thumbnail_and_title_strip() {
    use storybook_viewer::canvas::highlight_rect;
    let plan = hl_plan();
    let h = highlight_rect(&plan, &Default::default(), "v/b").unwrap();
    assert_eq!(
        h,
        Rect {
            x: 500.0,
            y: 50.0,
            w: NODE_W,
            h: NODE_H + TITLE_H
        }
    );
}

#[test]
fn highlight_of_collapsed_variant_falls_back_to_its_node() {
    use storybook_viewer::canvas::highlight_rect;
    let plan = hl_plan();
    let h = highlight_rect(&plan, &Default::default(), "v/a2").unwrap();
    assert_eq!(h.x, 100.0);
    assert_eq!(h.h, NODE_H + TITLE_H);
}

#[test]
fn highlight_of_expanded_variant_is_its_grid_cell() {
    use storybook_viewer::canvas::{highlight_rect, variant_cells};
    let mut plan = hl_plan();
    // an expanded node's rect grows around its grid
    plan.nodes[0].rect.h = NODE_H + 200.0;
    let expanded: std::collections::BTreeSet<String> = ["v/a".to_string()].into();
    let cells = variant_cells(plan.nodes[0].rect, 2);
    assert_eq!(highlight_rect(&plan, &expanded, "v/a1"), Some(cells[0]));
    assert_eq!(highlight_rect(&plan, &expanded, "v/a2"), Some(cells[1]));
    // the node itself is still thumbnail + title, not the grown box
    assert_eq!(
        highlight_rect(&plan, &expanded, "v/a").unwrap().h,
        NODE_H + TITLE_H
    );
}

#[test]
fn highlight_is_none_for_states_off_the_map() {
    use storybook_viewer::canvas::highlight_rect;
    assert_eq!(
        highlight_rect(&hl_plan(), &Default::default(), "v/zzz"),
        None
    );
}

// --- clicking states on the map ---------------------------------------------

#[test]
fn state_at_hits_node_thumbnail_and_title_strip() {
    use storybook_viewer::canvas::state_at;
    let plan = hl_plan();
    let none = Default::default();
    assert_eq!(state_at(&plan, &none, (110.0, 60.0)).as_deref(), Some("v/a"));
    // the title strip under the thumbnail belongs to the node too
    assert_eq!(
        state_at(&plan, &none, (110.0, 50.0 + NODE_H + TITLE_H / 2.0)).as_deref(),
        Some("v/a")
    );
    assert_eq!(state_at(&plan, &none, (510.0, 60.0)).as_deref(), Some("v/b"));
    assert_eq!(state_at(&plan, &none, (5.0, 5.0)), None);
}

#[test]
fn state_at_prefers_variant_cells_of_expanded_nodes() {
    use storybook_viewer::canvas::{state_at, variant_cells};
    let mut plan = hl_plan();
    plan.nodes[0].rect.h = NODE_H + 200.0;
    let expanded: std::collections::BTreeSet<String> = ["v/a".to_string()].into();
    let cells = variant_cells(plan.nodes[0].rect, 2);
    let centre = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
    assert_eq!(state_at(&plan, &expanded, centre(cells[0])).as_deref(), Some("v/a1"));
    assert_eq!(state_at(&plan, &expanded, centre(cells[1])).as_deref(), Some("v/a2"));
    // collapsed: the same point is not a variant
    assert_ne!(
        state_at(&plan, &Default::default(), centre(cells[1])).as_deref(),
        Some("v/a2")
    );
}

// --- focus mode (hover / selection dims the rest of the map) ----------------

fn route(f: &str, t: &str, back: bool) -> EdgeRoute {
    EdgeRoute {
        from: f.into(),
        to: t.into(),
        back,
        label: None,
        points: None,
        label_pos: None,
        arrow: None,
    }
}

#[test]
fn focus_prefers_hover_then_selection_and_maps_variants_to_their_node() {
    use storybook_viewer::canvas::focus_node;
    let plan = hl_plan();
    assert_eq!(focus_node(&plan, None, None), None);
    assert_eq!(focus_node(&plan, None, Some("v/b")).as_deref(), Some("v/b"));
    assert_eq!(focus_node(&plan, None, Some("v/a2")).as_deref(), Some("v/a"));
    assert_eq!(focus_node(&plan, Some("v/b"), Some("v/a")).as_deref(), Some("v/b"));
    // off-map ids never focus; a stale hover falls back to the selection
    assert_eq!(focus_node(&plan, None, Some("v/zz")), None);
    assert_eq!(focus_node(&plan, Some("v/zz"), Some("v/a1")).as_deref(), Some("v/a"));
}

#[test]
fn focus_set_is_the_node_and_its_visible_neighbours() {
    use storybook_viewer::canvas::focus_set;
    let edges = [
        route("a", "b", false),
        route("c", "a", false),
        route("b", "d", false),
        route("e", "a", true),
    ];
    let set: Vec<String> = focus_set(&edges, "a", false).into_iter().collect();
    assert_eq!(set, ["a", "b", "c"]);
    let with_back: Vec<String> = focus_set(&edges, "a", true).into_iter().collect();
    assert_eq!(with_back, ["a", "b", "c", "e"]);
    let alone: Vec<String> = focus_set(&[], "a", false).into_iter().collect();
    assert_eq!(alone, ["a"]);
}

#[test]
fn only_edges_touching_the_focus_are_in_focus() {
    use storybook_viewer::canvas::edge_in_focus;
    assert!(edge_in_focus("a", "b", Some("a")));
    assert!(edge_in_focus("a", "b", Some("b")));
    assert!(!edge_in_focus("a", "b", Some("c")));
    assert!(!edge_in_focus("a", "b", None));
}
