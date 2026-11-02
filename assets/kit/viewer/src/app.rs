//! Viewer application (references/canvas.md "The viewer").
//!
//! Pure input handling — argument parsing, storybook loading, signature
//! grouping — is unit-tested headless; GTK application wiring is marked for
//! the manual GUI check (deferred to the user).

use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use gtk::glib::clone;
use serde::de::DeserializeOwned;
use storybook_kit::model::{Flows, Inventory, Layout};

use crate::canvas::{self, Canvas};
use crate::story_list::{self, preset_width_container};
use crate::{Events, Registry};

/// Parsed viewer arguments (`--story <id>`, `--check-render`, `--start`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ViewerArgs {
    pub story: Option<String>,
    pub check_render: bool,
    pub start: StartPage,
}

/// Legacy `--start list|map` flag: still parsed so old launch commands keep
/// working, but it has no effect — the viewer is one window with the menu,
/// the map and the preview side by side.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum StartPage {
    #[default]
    Map,
    List,
}

impl StartPage {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "map" => Some(Self::Map),
            "list" => Some(Self::List),
            _ => None,
        }
    }
}

/// Parse viewer flags; unknown arguments are left to GTK.
pub fn parse_view_args(args: impl IntoIterator<Item = String>) -> ViewerArgs {
    let mut out = ViewerArgs::default();
    let mut it = args.into_iter().peekable();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--check-render" => out.check_render = true,
            "--start" => {
                // A value that is itself a flag is not a page: skip it.
                if let Some(start) = it
                    .next_if(|v| !v.starts_with('-'))
                    .and_then(|v| StartPage::parse(&v))
                {
                    out.start = start;
                }
            }
            other if other.starts_with("--story=") => {
                out.story = Some(other["--story=".len()..].to_string());
            }
            other if other.starts_with("--start=") => {
                if let Some(start) = StartPage::parse(&other["--start=".len()..]) {
                    out.start = start;
                }
            }
            "--story" => {
                // A value that is itself a flag is not a story id: leave it
                // for the next loop turn (or GTK).
                if let Some(story) = it.next_if(|v| !v.starts_with('-')) {
                    out.story = Some(story);
                }
            }
            _ => {}
        }
    }
    out
}

/// The storybook inputs the viewer reads: `inventory.json`, `layout.json`,
/// `flows.toml` and `fixtures/<view>/<state>.json` (formats.md).
#[derive(Debug, Clone)]
pub struct Storyboard {
    pub inventory: Inventory,
    pub layout: Layout,
    pub flows: Flows,
    fixtures: BTreeMap<String, String>,
    /// Story-menu dimension tree inputs (axes, cell assignments, labels).
    pub facets: crate::sidebar::Facets,
}

impl Storyboard {
    /// Fixture JSON of a cell, by state id (`view/classes`).
    pub fn fixture_json(&self, state_id: &str) -> Result<&str, String> {
        self.fixtures
            .get(state_id)
            .map(String::as_str)
            .ok_or_else(|| format!("missing fixture for state {state_id}"))
    }

    /// View id for a lane index (lanes follow the inventory order).
    pub fn view_name(&self, lane: u32) -> Option<&str> {
        self.inventory
            .views
            .get(lane as usize)
            .map(|v| v.id.as_str())
    }

    /// Human title of a cell from `layout.json` (authored in
    /// `states/<view>.toml` `[titles]`); `None` falls back to the state id.
    pub fn title_of(&self, state_id: &str) -> Option<&str> {
        self.layout
            .nodes
            .iter()
            .find(|n| n.state_id == state_id)
            .and_then(|n| n.title.as_deref())
    }

    /// State ids in (lane, depth, order), each node followed by its
    /// variants (stack order): sidebar and expand order.
    pub fn state_ids(&self) -> Vec<String> {
        let mut keyed: Vec<((u32, u32, u32, String), String)> = self
            .layout
            .nodes
            .iter()
            .map(|n| {
                (
                    (n.lane, n.depth, n.order, n.state_id.clone()),
                    n.state_id.clone(),
                )
            })
            .collect();
        for v in &self.layout.variants {
            let node = self
                .layout
                .nodes
                .iter()
                .find(|n| n.state_id == v.variant_of);
            if let Some(node) = node {
                // Variants sort right after their node: order + 1 keeps the
                // node first even when the node shares (lane, depth, order).
                keyed.push((
                    (v.lane, v.depth, node.order + 1, v.state_id.clone()),
                    v.state_id.clone(),
                ));
            }
        }
        keyed.sort();
        keyed.into_iter().map(|(_, id)| id).collect()
    }
}

/// Load the viewer inputs from a storybook directory.
pub fn load_storyboard(dir: &Path) -> Result<Storyboard, String> {
    let inventory: Inventory = read_json(&dir.join("inventory.json"))?;
    let layout: Layout = read_json(&dir.join("layout.json"))
        .map_err(|e| format!("{e}; run `sbk verify` to generate layout.json"))?;
    let flows: Flows = toml::from_str(&read_text(&dir.join("flows.toml"))?)
        .map_err(|e| format!("flows.toml: {e}"))?;

    let mut fixtures = BTreeMap::new();
    let fixtures_dir = dir.join("fixtures");
    if let Ok(views) = std::fs::read_dir(&fixtures_dir) {
        for view in views.flatten() {
            let Ok(files) = std::fs::read_dir(view.path()) else {
                continue;
            };
            let view_name = view.file_name().to_string_lossy().into_owned();
            for file in files.flatten() {
                let is_json = file.path().extension().is_some_and(|e| e == "json");
                if is_json {
                    let stem = file
                        .path()
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned());
                    if let Some(stem) = stem {
                        let key = format!("{view_name}/{stem}");
                        let text = read_text(&file.path())?;
                        fixtures.insert(key, text);
                    }
                }
            }
        }
    }

    let facets = load_facets(dir, &inventory);
    Ok(Storyboard {
        inventory,
        layout,
        flows,
        fixtures,
        facets,
    })
}

/// Dimension-tree inputs for the story menu: axes from the inventory,
/// assignments from `cells.json` and `[labels]` from `states/*.toml`.
/// Missing or unreadable files only flatten the menu, never fail the load.
fn load_facets(dir: &Path, inventory: &Inventory) -> crate::sidebar::Facets {
    let mut facets = crate::sidebar::Facets::default();
    for view in &inventory.views {
        facets.axes.insert(
            view.id.clone(),
            view.axes
                .iter()
                .map(|a| (a.path.clone(), a.classes.clone()))
                .collect(),
        );
    }
    if let Ok(cells) = read_json::<storybook_kit::model::CellsFile>(&dir.join("cells.json")) {
        for cell in cells.cells {
            facets.assignment.insert(cell.state_id, cell.assignment);
        }
    }
    if let Ok(entries) = std::fs::read_dir(dir.join("states")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "toml")
                && let Ok(text) = read_text(&path)
                && let Ok(file) = toml::from_str::<storybook_kit::model::StatesFile>(&text)
            {
                facets.labels.insert(file.view, file.labels);
            }
        }
    }
    facets
}

fn read_text(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = read_text(path)?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Groups of states (>1) sharing a render signature, sorted; each group is
/// sorted itself. Empty when every cell renders distinctly.
pub fn duplicate_groups(signatures: &BTreeMap<String, String>) -> Vec<Vec<String>> {
    let mut by_signature: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (state, signature) in signatures {
        by_signature
            .entry(signature.as_str())
            .or_default()
            .push(state.as_str());
    }
    let mut groups: Vec<Vec<String>> = by_signature
        .into_iter()
        .filter(|(_, states)| states.len() > 1)
        .map(|(_, states)| states.into_iter().map(str::to_string).collect())
        .collect();
    groups.sort();
    groups
}

/// Where a story mount is placed by a [`MountState`] transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountPlace {
    /// The off-view container: rooted and allocated at the preset width, so
    /// the `WidgetPaintable` thumbnails keep painting.
    Offview,
    /// The story-list content slot: visible and interactive.
    ContentView,
}

/// One widget operation emitted by a [`MountState`] transition; the GTK
/// layer applies them in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountOp {
    /// Detach the story's mount from its current preset container and drop
    /// the now-empty container — `gtk::Box::append` refuses a child that is
    /// still parented elsewhere.
    Unwrap { state: String },
    /// Wrap the story's mount in a fresh preset-width container and place it
    /// ([`MountPlace::Offview`] keeps the thumbnail rooted at the current
    /// preset width).
    Place { state: String, place: MountPlace },
}

/// Mount bookkeeping for the story mounts: exactly one story may live in the
/// content slot, every other story stays in the off-view container — an
/// unrooted mount paints a blank thumbnail. Pure state machine: transitions
/// return the operations to apply, keeping the GTK wiring thin.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MountState {
    in_view: Option<String>,
}

impl MountState {
    /// The state id currently placed in the content slot, if any.
    pub fn in_view(&self) -> Option<&str> {
        self.in_view.as_deref()
    }

    /// Show `state` in the content slot. The previously shown story — if any
    /// — is unwrapped and moved back off-view so it stays rooted and its
    /// thumbnail keeps painting. A no-op (no operations) when `state` is
    /// already in view.
    pub fn show(&mut self, state: &str) -> Vec<MountOp> {
        if self.in_view.as_deref() == Some(state) {
            return Vec::new();
        }
        let mut ops = Vec::new();
        if let Some(prev) = self.in_view.clone() {
            ops.push(MountOp::Unwrap {
                state: prev.clone(),
            });
            ops.push(MountOp::Place {
                state: prev,
                place: MountPlace::Offview,
            });
        }
        ops.push(MountOp::Unwrap {
            state: state.to_string(),
        });
        ops.push(MountOp::Place {
            state: state.to_string(),
            place: MountPlace::ContentView,
        });
        self.in_view = Some(state.to_string());
        ops
    }

    /// Re-wrap every story at the current preset width (initial placement
    /// and width-preset changes): the story in view returns to the content
    /// slot, all others go off-view, and every old preset container is
    /// dropped.
    pub fn rewrap(&mut self, states: &[String]) -> Vec<MountOp> {
        states
            .iter()
            .flat_map(|state| {
                [
                    MountOp::Unwrap {
                        state: state.clone(),
                    },
                    MountOp::Place {
                        state: state.clone(),
                        place: if self.in_view.as_deref() == Some(state.as_str()) {
                            MountPlace::ContentView
                        } else {
                            MountPlace::Offview
                        },
                    },
                ]
            })
            .collect()
    }
}

/// Open the storybook viewer for `storybook_dir`.
pub fn run(registry: Box<dyn Registry>, storybook_dir: &Path) -> glib::ExitCode {
    let args = parse_view_args(
        std::env::args_os()
            .skip(1)
            .map(|a| a.to_string_lossy().into_owned()),
    );
    let storyboard = match load_storyboard(storybook_dir) {
        Ok(sb) => sb,
        Err(e) => {
            eprintln!("storybook-viewer: {e}");
            return glib::ExitCode::from(1);
        }
    };
    if args.check_render {
        return check_render(registry.as_ref(), &storyboard);
    }
    launch(registry, storyboard, args.story, args.start)
}

/// `--check-render`: build every story and compare render signatures; equal
/// signatures across distinct cells are reported (canvas.md).
#[mutants::skip] // needs a display (gtk::init); manual GUI check deferred to user
fn check_render(registry: &dyn Registry, sb: &Storyboard) -> glib::ExitCode {
    if gtk::init().is_err() {
        eprintln!("storybook-viewer: --check-render requires a display");
        return glib::ExitCode::from(1);
    }
    let mut signatures: BTreeMap<String, String> = BTreeMap::new();
    for state_id in sb.state_ids() {
        let fixture = match sb.fixture_json(&state_id) {
            Ok(f) => f,
            Err(e) => {
                println!("check-render: FAIL {state_id}: {e}");
                return glib::ExitCode::from(1);
            }
        };
        let widget = match registry.build(&state_id, fixture, &Events::new()) {
            Ok(w) => w,
            Err(e) => {
                println!("check-render: FAIL {state_id}: {e}");
                return glib::ExitCode::from(1);
            }
        };
        signatures.insert(state_id, crate::signature::render_signature(&widget));
    }
    let groups = duplicate_groups(&signatures);
    for group in &groups {
        println!(
            "check-render: duplicate render signature for {} states: {}",
            group.len(),
            group.join(", ")
        );
    }
    if groups.is_empty() {
        println!(
            "check-render: OK ({} states, signatures unique)",
            signatures.len()
        );
        glib::ExitCode::SUCCESS
    } else {
        println!(
            "check-render: FAIL ({} duplicate-signature groups)",
            groups.len()
        );
        glib::ExitCode::from(1)
    }
}

/// Thumbnails live in this overlay child: rooted and allocated at their
/// preset width, but placed far outside the visible area.
const OFFVIEW_MARGIN: i32 = 8192;

#[derive(Clone)]
struct StoryHandle {
    /// `AdwBreakpointBin` holding the story; its parent is a preset container.
    mount: adw::BreakpointBin,
    paintable: gtk::WidgetPaintable,
    events: Events,
    /// Unparented shells (e.g. the `AdwDialog` whose content is hosted);
    /// held so their callbacks stay valid. Never read, only kept alive.
    _keep_alive: Vec<gtk::Widget>,
}

#[mutants::skip] // GTK wiring; manual GUI check deferred to user
fn build_story(
    registry: &dyn Registry,
    state_id: &str,
    fixture: Result<&str, String>,
) -> StoryHandle {
    let events = Events::new();
    let built = fixture.and_then(|json| registry.build(state_id, json, &events));
    let mount = adw::BreakpointBin::new();
    mount.set_hexpand(true);
    // a breakpoint bin needs a minimum size; the narrowest preset is it
    mount.set_size_request(story_list::WIDTH_PRESETS[0], 1);
    let (paintable, keep_alive) = match built {
        Ok(widget) => {
            // Dialogs/windows/popovers must never be parented (dialog mode,
            // references/canvas.md): host their content instead.
            let hosted = crate::hosting::host(&widget);
            mount.set_child(Some(&hosted.widget));
            (
                gtk::WidgetPaintable::new(Some(&hosted.widget)),
                hosted.keep_alive,
            )
        }
        Err(error) => {
            let label = gtk::Label::new(Some(&error));
            label.set_wrap(true);
            mount.set_child(Some(&label));
            (gtk::WidgetPaintable::new(Some(&label)), Vec::new())
        }
    };
    StoryHandle {
        mount,
        paintable,
        events,
        _keep_alive: keep_alive,
    }
}

#[mutants::skip] // GTK application wiring; manual GUI check deferred to user
fn launch(
    registry: Box<dyn Registry>,
    sb: Storyboard,
    story: Option<String>,
    start: StartPage,
) -> glib::ExitCode {
    let app = adw::Application::new(None, gtk::gio::ApplicationFlags::default());
    let sb = Rc::new(sb);
    let registry = Rc::new(registry);
    let story = Rc::new(story);
    let start = Rc::new(start);
    app.connect_activate(move |app| {
        activate(app, sb.clone(), registry.clone(), story.clone(), *start);
    });
    app.run_with_args::<&str>(&[])
}

#[mutants::skip] // GTK application wiring; manual GUI check deferred to user
fn activate(
    app: &adw::Application,
    sb: Rc<Storyboard>,
    registry: Rc<Box<dyn Registry>>,
    initial: Rc<Option<String>>,
    _start: StartPage, // legacy flag, no effect (see StartPage)
) {
    use std::cell::{Cell, RefCell};

    let state_ids = sb.state_ids();
    let current_width = Rc::new(Cell::new(story_list::WIDTH_PRESETS[0]));
    let mounts = Rc::new(RefCell::new(MountState::default()));

    // --- stories: live widgets, rooted off-view, one paintable each --------
    let offview = gtk::Box::new(gtk::Orientation::Vertical, 8);
    offview.set_halign(gtk::Align::Start);
    offview.set_valign(gtk::Align::Start);
    offview.set_margin_start(OFFVIEW_MARGIN);
    // The overlay child would cover the whole window and swallow pointer
    // input: it must never take input or focus itself (its mounts do).
    offview.set_can_target(false);
    offview.set_can_focus(false);
    offview.set_focusable(false);

    let canvas_widget = Canvas::new();
    let mut stories: BTreeMap<String, StoryHandle> = BTreeMap::new();
    let mut paintables: BTreeMap<String, gtk::WidgetPaintable> = BTreeMap::new();
    for state_id in &state_ids {
        let handle = build_story(&**registry, state_id, sb.fixture_json(state_id));
        handle.paintable.connect_invalidate_contents(clone!(
            #[weak]
            canvas_widget,
            move |_| {
                canvas_widget.queue_draw();
            }
        ));
        paintables.insert(state_id.clone(), handle.paintable.clone());
        stories.insert(state_id.clone(), handle);
    }

    // --- canvas: lanes = views (manifest order), depth = BFS distance -------
    let lane_count = sb
        .layout
        .nodes
        .iter()
        .map(|n| n.lane)
        .max()
        .map_or(0, |l| l + 1);
    let lane_names: Vec<String> = (0..lane_count)
        .map(|l| {
            sb.view_name(l).map_or_else(
                || format!("lane {l}"),
                |id| {
                    // lane header: authored view display name, else the
                    // humanized view id (same fallback as the sidebar)
                    sb.layout
                        .view_titles
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| crate::sidebar::humanize(id))
                },
            )
        })
        .collect();
    canvas_widget.set_content(
        if sb.layout.engine == storybook_kit::model::ENGINE_DOT {
            // dot engine: geometry comes straight from layout.json
            canvas::plan_dot(&sb.layout.nodes, &sb.layout.variants, &sb.layout.clusters)
        } else {
            canvas::plan_with_edges(&sb.layout.nodes, &sb.layout.variants, &sb.layout.edges)
        },
        lane_names,
        sb.layout.edges.clone(),
        paintables,
    );

    // --- variant expansion: badge click -> relayout off the main loop ----
    // and an animated transition on the way back (references/canvas.md).
    #[allow(clippy::type_complexity)]
    fn relayout_target(
        views: Vec<String>,
        layout: storybook_kit::model::Layout,
        expanded: std::collections::BTreeSet<String>,
        program: Option<std::path::PathBuf>,
    ) -> (canvas::CanvasPlan, Vec<storybook_kit::model::EdgeRoute>) {
        let variant_count = |id: &str| {
            layout
                .nodes
                .iter()
                .find(|n| n.state_id == id)
                .map_or(0, |n| n.variant_count)
        };
        if let Some(program) = &program {
            let sizes: BTreeMap<String, (f64, f64)> = expanded
                .iter()
                .filter(|id| variant_count(id) > 0)
                .map(|id| {
                    (
                        id.clone(),
                        storybook_kit::dot::expanded_size(variant_count(id)),
                    )
                })
                .collect();
            if let Ok(relaid) =
                storybook_kit::dot::relayout_with_dot(program, &views, &layout, &sizes)
            {
                return (
                    canvas::plan_dot(&relaid.nodes, &relaid.variants, &relaid.clusters),
                    relaid.edges,
                );
            }
        }
        // builtin engine (or dot unavailable): the expanded node's grid
        // height is added to its row; rows below shift down.
        let extras: BTreeMap<String, f64> = expanded
            .iter()
            .filter(|id| variant_count(id) > 0)
            .map(|id| {
                (
                    id.clone(),
                    canvas::TITLE_H
                        + storybook_kit::dot::GRID_GAP
                        + storybook_kit::dot::variant_grid_size(variant_count(id)).1,
                )
            })
            .collect();
        (
            canvas::plan_expanded(&layout.nodes, &layout.variants, &layout.edges, &extras),
            layout.edges.clone(),
        )
    }

    let relay = std::sync::Arc::new(crate::anim::Relay::default());
    let views: Vec<String> = (0..lane_count)
        .filter_map(|l| sb.view_name(l).map(str::to_string))
        .collect();
    let layout0 = sb.layout.clone();
    let dot_engine = sb.layout.engine == storybook_kit::model::ENGINE_DOT;
    let program = storybook_kit::dot::dot_program();
    canvas_widget.connect_toggle_variants(clone!(
        #[weak]
        canvas_widget,
        #[strong]
        relay,
        move |_state_id| {
            let ticket = relay.issue();
            let expanded = canvas_widget.expanded_ids();
            let views = views.clone();
            let layout = layout0.clone();
            let program = if dot_engine { program.clone() } else { None };
            let relay = relay.clone();
            let canvas_ref: glib::SendWeakRef<Canvas> = canvas_widget.downgrade().into();
            std::thread::spawn(move || {
                let (plan, edges) = relayout_target(views, layout, expanded, program);
                // hop back to the main loop; a newer toggle's ticket makes
                // this result stale, and it is dropped
                let _ = glib::idle_add_once(move || {
                    if relay.is_current(ticket)
                        && let Some(canvas) = canvas_ref.upgrade()
                    {
                        canvas.animate_to(plan, edges);
                    }
                });
            });
        }
    ));
    // Sidebar drill-down model: views in inventory order, nodes in journey
    // order, variants behind their node (crate::sidebar).
    let view_ids: Vec<String> = sb.inventory.views.iter().map(|v| v.id.clone()).collect();
    let model = crate::sidebar::build_model(&view_ids, &sb.layout);
    let specs = crate::sidebar::pages(&model, &sb.facets);
    let list = story_list::build(&specs);

    // Root every story off-view at the initial preset width (initial
    // MountState transition; showing a story later reparents its mount).
    apply_mount_ops(
        &mounts.borrow_mut().rewrap(&state_ids),
        &stories,
        current_width.get(),
        &list.content_slot,
        &offview,
    );
    let canvas_page_content = adw::ToolbarView::new();
    let canvas_bar = adw::HeaderBar::new();
    let fit_button = gtk::Button::with_label("Fit");
    let menu_button = gtk::ToggleButton::new();
    menu_button.set_icon_name("sidebar-show-symbolic");
    menu_button.set_tooltip_text(Some("State menu"));
    menu_button.set_active(true);
    let preview_button = gtk::ToggleButton::new();
    preview_button.set_icon_name("sidebar-show-right-symbolic");
    preview_button.set_tooltip_text(Some("State preview"));
    preview_button.set_active(true);
    // Back edges are hidden by default; the header toggle shows them.
    let back_button = gtk::ToggleButton::with_label("Back edges");
    // The map's controls sit centred in the header bar (title widget).
    let canvas_controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    canvas_controls.append(&fit_button);
    canvas_controls.append(&back_button);
    canvas_bar.set_title_widget(Some(&canvas_controls));
    canvas_bar.pack_start(&menu_button);
    canvas_bar.pack_end(&preview_button);
    canvas_page_content.add_top_bar(&canvas_bar);
    canvas_page_content.set_content(Some(&canvas_widget));
    let hint = gtk::Label::new(Some(
        "Click a state: select it · +N: variants · drag: pan · scroll / pinch: zoom · F: fit",
    ));
    hint.set_wrap(true);
    canvas_page_content.add_bottom_bar(&hint);
    // One window, three columns: drill-down menu | flow map | live preview.
    // Selecting a state in the menu highlights it on the map and shows it
    // in the preview column.
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&canvas_page_content));
    overlay.add_overlay(&offview);
    // Right column: a collapsible end sidebar (animated show/hide; it
    // overlays the map on narrow windows).
    let inner = adw::OverlaySplitView::new();
    inner.set_content(Some(&overlay));
    inner.set_sidebar(Some(&list.preview));
    inner.set_sidebar_position(gtk::PackType::End);
    inner.set_min_sidebar_width(380.0);
    inner.set_max_sidebar_width(560.0);
    inner.set_sidebar_width_fraction(0.33);
    let outer = adw::OverlaySplitView::new();
    outer.set_sidebar(Some(&list.sidebar));
    outer.set_content(Some(&inner));
    outer.set_min_sidebar_width(260.0);
    let window = adw::ApplicationWindow::new(app);
    window.set_title(Some("Storybook"));
    window.set_default_size(1500, 860);
    window.set_content(Some(&outer));
    window.set_size_request(360, 294);
    // Narrow windows: the preview overlays the map first, then the menu too
    // (the header toggles keep working on the overlays).
    for (max_width, collapse_menu) in [(1100.0, false), (720.0, true)] {
        let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            max_width,
            adw::LengthUnit::Sp,
        ));
        bp.add_setter(&inner, "collapsed", Some(&true.to_value()));
        if collapse_menu {
            bp.add_setter(&outer, "collapsed", Some(&true.to_value()));
        }
        window.add_breakpoint(bp);
    }
    // Header toggles: menu column (start), preview column (end).
    menu_button
        .bind_property("active", &outer, "show-sidebar")
        .sync_create()
        .bidirectional()
        .build();
    preview_button
        .bind_property("active", &inner, "show-sidebar")
        .sync_create()
        .bidirectional()
        .build();

    // --- selection: sidebar row -> live story in the content slot ----------
    let show_story: Rc<dyn Fn(&str)> = {
        let mounts = mounts.clone();
        let stories = stories.clone();
        let content_slot = list.content_slot.clone();
        let offview = offview.clone();
        let events_pane = list.events.clone();
        let preview_column = inner.clone();
        let current_width = current_width.clone();
        let canvas_sel = canvas_widget.clone();
        Rc::new(move |state_id: &str| {
            apply_mount_ops(
                &mounts.borrow_mut().show(state_id),
                &stories,
                current_width.get(),
                &content_slot,
                &offview,
            );
            clear_list_box(&events_pane);
            if let Some(handle) = stories.get(state_id) {
                let pane = events_pane.clone();
                handle.events.set_hook(Some(Rc::new(move |action: &str| {
                    pane.prepend(&gtk::Label::new(Some(action)));
                })));
            }
            preview_column.set_show_sidebar(true);
            canvas_sel.set_selected(Some(state_id));
        })
    };

    // Exactly one selected leaf across all menu pages.
    let unselect_others: Rc<dyn Fn(usize)> = {
        let lists = list.lists.clone();
        Rc::new(move |keep: usize| {
            for (index, other) in lists.iter().enumerate() {
                if index != keep
                    && let Some(selected) = other.selected_row()
                {
                    other.unselect_row(&selected);
                }
            }
        })
    };

    // Drill-down rows: a leaf row shows its story (on selection, and on
    // activation, so re-clicking the selected leaf reopens the preview); a
    // row that leads further pushes its page.
    for (index, page_list) in list.lists.iter().enumerate() {
        let rows = list.specs[index].rows.clone();
        let nav = list.nav.clone();
        let pages = list.pages.clone();
        let show = show_story.clone();
        page_list.connect_row_activated(move |_, row| {
            match rows.get(row.index() as usize).map(|r| &r.target) {
                Some(crate::sidebar::Target::Page(child)) => {
                    if let Some(page) = pages.get(*child) {
                        nav.push(page);
                    }
                }
                Some(crate::sidebar::Target::State(state_id)) => show(state_id),
                None => {}
            }
        });
        let rows = list.specs[index].rows.clone();
        page_list.connect_row_selected(clone!(
            #[strong]
            show_story,
            #[strong]
            unselect_others,
            move |_, row| {
                let Some(row) = row else { return };
                if let Some(crate::sidebar::Target::State(state_id)) =
                    rows.get(row.index() as usize).map(|r| &r.target)
                {
                    unselect_others(index);
                    show_story(state_id);
                }
            }
        ));
    }

    // Programmatic selection by state id (--story, map click, search hit):
    // walk the drill-down to the leaf's page, select its row and show the
    // story (explicitly: selecting an already selected row emits nothing).
    let navigate: Rc<dyn Fn(&str)> = {
        let specs = list.specs.clone();
        let nav = list.nav.clone();
        let pages = list.pages.clone();
        let lists = list.lists.clone();
        let show_story = show_story.clone();
        let unselect_others = unselect_others.clone();
        Rc::new(move |state_id: &str| {
            let Some((page, row)) = crate::sidebar::locate_leaf(&specs, state_id) else {
                return;
            };
            if nav.visible_page().as_ref() != pages.get(page) {
                nav.pop_to_page(&pages[0]);
                for step in crate::sidebar::page_path(&specs, page).into_iter().skip(1) {
                    nav.push(&pages[step]);
                }
            }
            unselect_others(page);
            if let Some(row_widget) = lists[page].row_at_index(row as i32) {
                lists[page].select_row(Some(&row_widget));
            }
            show_story(state_id);
        })
    };

    // Search: while the query is non-empty the root page shows flat
    // results (title + owning view) in place of the view list; Escape / an
    // empty query brings the view list back.
    let search_ids = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    {
        let model = model.clone();
        let nav = list.nav.clone();
        let root_page = list.pages[0].clone();
        let root_stack = list.root_stack.clone();
        let search_list = list.search_list.clone();
        let results = search_ids.clone();
        list.search_entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            let hits = crate::sidebar::search(&model, &text);
            *results.borrow_mut() = hits.clone();
            clear_list_box(&search_list);
            for state_id in &hits {
                // row title: the state's display title (node or variant)
                let (title, view_title) = model
                    .iter()
                    .find_map(|view| {
                        view.nodes.iter().find_map(|node| {
                            if node.state_id == *state_id {
                                Some((node.title.clone(), view.title.clone()))
                            } else {
                                node.variants
                                    .iter()
                                    .find(|(id, _)| id == state_id)
                                    .map(|(_, title)| (title.clone(), view.title.clone()))
                            }
                        })
                    })
                    .unwrap_or_else(|| (state_id.clone(), String::new()));
                let row = adw::ActionRow::builder()
                    .title(title)
                    .subtitle(view_title)
                    .activatable(true)
                    .build();
                row.set_tooltip_text(Some(state_id));
                search_list.append(&row);
            }
            root_stack.set_visible_child_name(if text.is_empty() { "pages" } else { "results" });
            if !text.is_empty() && nav.visible_page().as_ref() != Some(&root_page) {
                nav.pop_to_page(&root_page);
            }
        });
        // Escape stops the search: back to the pages.
        {
            let entry = list.search_entry.clone();
            list.search_entry.connect_stop_search(move |_| {
                entry.set_text("");
            });
        }
        // Activating a result opens the story on its view page.
        {
            let search_ids = search_ids.clone();
            let navigate = navigate.clone();
            let entry = list.search_entry.clone();
            list.search_list.connect_row_activated(move |_, row| {
                let hit = search_ids.borrow().get(row.index() as usize).cloned();
                if let Some(state_id) = hit {
                    // clears the query first: the view list returns
                    entry.set_text("");
                    navigate(&state_id);
                }
            });
        }
    }

    // A click on a map state selects it in the menu and the preview.
    canvas_widget.connect_open_story(clone!(
        #[strong]
        navigate,
        move |state_id| navigate(state_id)
    ));

    // Ctrl+F anywhere: back to the menu root with the search entry focused
    // (the menu column is shown if it was hidden).
    {
        let nav = list.nav.clone();
        let root_page = list.pages[0].clone();
        let search_bar = list.search_bar.clone();
        let entry = list.search_entry.clone();
        let outer = outer.clone();
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_scope(gtk::ShortcutScope::Managed);
        shortcuts.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string("<Control>f"),
            Some(gtk::CallbackAction::new(move |_, _| {
                outer.set_show_sidebar(true);
                nav.pop_to_page(&root_page);
                search_bar.set_search_mode(true);
                entry.grab_focus();
                glib::Propagation::Stop
            })),
        ));
        window.add_controller(shortcuts);
    }
    list.preview_close.connect_clicked(clone!(
        #[weak]
        inner,
        move |_| {
            inner.set_show_sidebar(false);
        }
    ));
    back_button.connect_toggled(clone!(
        #[weak]
        canvas_widget,
        move |button| {
            canvas_widget.set_show_back_edges(button.is_active());
        }
    ));
    fit_button.connect_clicked(clone!(
        #[weak]
        canvas_widget,
        move |_| {
            canvas_widget.fit();
        }
    ));

    // Width presets re-wrap every story at its new exact width; the story in
    // view returns to the content slot, all others stay rooted off-view.
    {
        let mounts = mounts.clone();
        let stories = stories.clone();
        let offview = offview.clone();
        let content_slot = list.content_slot.clone();
        let state_ids = state_ids.clone();
        let canvas_widget = canvas_widget.clone();
        list.width_choice.connect_selected_notify(move |choice| {
            let Some(preset) = story_list::WIDTH_PRESETS.get(choice.selected() as usize) else {
                return;
            };
            current_width.set(*preset);
            apply_mount_ops(
                &mounts.borrow_mut().rewrap(&state_ids),
                &stories,
                *preset,
                &content_slot,
                &offview,
            );
            canvas_widget.queue_draw();
        });
    }

    // Light / Dark / System theme.
    list.theme_choice.connect_selected_notify(move |choice| {
        adw::StyleManager::default()
            .set_color_scheme(story_list::theme_scheme(choice.selected() as usize));
    });

    // Initial state: the menu starts on its first level ("Views") with
    // nothing selected and the preview hidden — unless --story names a
    // known state, which is then selected (menu walks to it, preview shown).
    match initial
        .as_deref()
        .filter(|s| crate::sidebar::locate_leaf(&specs, s).is_some())
    {
        Some(state_id) => navigate(state_id),
        None => inner.set_show_sidebar(false),
    }

    window.present();
}

/// Apply [`MountOp`]s to the widgets: detach a mount from its preset
/// container (dropping the now-empty bin from its parent, too), then re-wrap
/// and place it as told. The transitions themselves are the tested
/// [`MountState`]; ids without a built story are skipped defensively.
#[mutants::skip] // GTK reparenting; the transitions are the tested MountState
fn apply_mount_ops(
    ops: &[MountOp],
    stories: &BTreeMap<String, StoryHandle>,
    width: i32,
    content_slot: &gtk::Box,
    offview: &gtk::Box,
) {
    for op in ops {
        match op {
            MountOp::Unwrap { state } => {
                let Some(handle) = stories.get(state) else {
                    continue;
                };
                if let Some(container) = handle.mount.parent() {
                    handle.mount.unparent();
                    if container.parent().is_some() {
                        container.unparent();
                    }
                }
            }
            MountOp::Place { state, place } => {
                let Some(handle) = stories.get(state) else {
                    continue;
                };
                let container = preset_width_container(&handle.mount, width);
                match place {
                    MountPlace::ContentView => content_slot.append(&container),
                    MountPlace::Offview => offview.append(&container),
                }
            }
        }
    }
}

#[mutants::skip] // GTK helper; manual GUI check deferred to user
fn clear_list_box(list_box: &gtk::ListBox) {
    while let Some(child) = list_box.first_child() {
        list_box.remove(&child);
    }
}
