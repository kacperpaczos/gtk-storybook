//! Story list (references/canvas.md): an `AdwNavigationSplitView` whose
//! sidebar groups states by view, and whose content shows the live story
//! inside an `AdwBreakpointBin` at the chosen width preset (exact width via
//! a `GtkConstraintGuide` with min = max) plus an Events pane.

use gtk::prelude::*;

/// Width presets (px) the story can be shown at.
pub const WIDTH_PRESETS: [i32; 4] = [360, 600, 900, 1200];

/// Theme preset labels (System / Light / Dark), applied via `AdwStyleManager`.
pub const THEMES: [&str; 3] = ["System", "Light", "Dark"];

/// Map a theme-choice index onto an `AdwColorScheme`; unknown -> System.
pub fn theme_scheme(index: usize) -> adw::ColorScheme {
    match index {
        1 => adw::ColorScheme::ForceLight,
        2 => adw::ColorScheme::ForceDark,
        _ => adw::ColorScheme::Default,
    }
}

/// Human row title for a state id: the classes of `view/classes`.
pub fn story_title(state_id: &str) -> String {
    match state_id.split_once('/') {
        Some((_view, classes)) => classes.replace(['-', '_'], " "),
        None => state_id.to_string(),
    }
}

/// Owning view of a state id (the part before `/`).
pub fn view_of(state_id: &str) -> String {
    state_id
        .split_once('/')
        .map_or_else(|| state_id.to_string(), |(view, _)| view.to_string())
}

/// `GTK_CONSTRAINT_STRENGTH_REQUIRED` (bindings take the raw value).
const STRENGTH_REQUIRED: i32 = 1001001000;

/// Container that allocates its child at exactly `width` px wide — a
/// `GtkConstraintGuide` with min = max ("exact width via min = max") —
/// centered in whatever space the parent offers. Height stays natural.
#[mutants::skip] // GTK layout wiring; manual GUI check deferred to user
pub(crate) fn preset_width_container(child: &impl IsA<gtk::Widget>, width: i32) -> gtk::Widget {
    let bin = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bin.set_halign(gtk::Align::Center);
    bin.set_valign(gtk::Align::Start);

    let layout = gtk::ConstraintLayout::new();
    let guide = gtk::ConstraintGuide::new();
    guide.set_name(Some("sbk-preset-width"));
    guide.set_min_size(width, 0);
    guide.set_max_size(width, i32::MAX);
    layout.add_guide(guide.clone());

    let eq = |a: gtk::ConstraintAttribute,
              b: gtk::ConstraintAttribute,
              target: Option<&gtk::Widget>,
              source: Option<&gtk::ConstraintTarget>| {
        gtk::Constraint::new(
            target,
            a,
            gtk::ConstraintRelation::Eq,
            source,
            b,
            1.0,
            0.0,
            STRENGTH_REQUIRED,
        )
    };
    use gtk::ConstraintAttribute as A;
    let child = child.upcast_ref();
    let guide_target = guide.upcast_ref::<gtk::ConstraintTarget>();
    layout.add_constraint(eq(A::Width, A::Width, Some(child), Some(guide_target)));
    layout.add_constraint(eq(A::CenterX, A::CenterX, Some(child), None));
    layout.add_constraint(eq(A::Top, A::Top, Some(child), None));

    bin.set_layout_manager(Some(layout));
    bin.append(child);
    bin.into()
}

/// Widgets of the story-list page; the application wires behavior.
pub(crate) struct StoryListView {
    pub split: adw::NavigationSplitView,
    pub sidebar: gtk::ListBox,
    pub content_slot: gtk::Box,
    pub events: gtk::ListBox,
    pub width_choice: gtk::DropDown,
    pub theme_choice: gtk::DropDown,
    pub map_button: gtk::Button,
}

/// Build the story list: sidebar rows are the states (grouped by view, in
/// the given order), content slot hosts the live story, Events pane is fed
/// by the application from [`crate::Events`].
#[mutants::skip] // widget construction; manual GUI check deferred to user
pub(crate) fn build(state_ids: &[String]) -> StoryListView {
    // --- sidebar: states grouped by view -----------------------------------
    let sidebar = gtk::ListBox::new();
    sidebar.set_css_classes(&["navigation-sidebar"]);
    for state_id in state_ids {
        let row = adw::ActionRow::builder()
            .title(story_title(state_id))
            .subtitle(state_id)
            .activatable(true)
            .build();
        sidebar.append(&row);
    }
    // Group headers: a view label above its first state.
    {
        let ids = state_ids.to_vec();
        sidebar.set_header_func(
            move |row: &gtk::ListBoxRow, before: Option<&gtk::ListBoxRow>| {
                let view_of_row =
                    |r: &gtk::ListBoxRow| ids.get(r.index() as usize).map(|id| view_of(id));
                let new_group = match before {
                    None => true,
                    Some(prev) => view_of_row(prev).as_deref() != view_of_row(row).as_deref(),
                };
                if new_group {
                    let header = gtk::Label::new(view_of_row(row).as_deref());
                    header.set_halign(gtk::Align::Start);
                    row.set_header(Some(&header));
                } else {
                    row.set_header(None::<&gtk::Label>);
                }
            },
        );
    }
    let sidebar_scroll = gtk::ScrolledWindow::new();
    sidebar_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    sidebar_scroll.set_child(Some(&sidebar));
    let sidebar_view = adw::ToolbarView::new();
    sidebar_view.add_top_bar(&adw::HeaderBar::new());
    sidebar_view.set_content(Some(&sidebar_scroll));

    // --- content: live story at a preset width + events pane ---------------
    let content_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content_slot.set_hexpand(true);
    content_slot.set_vexpand(true);

    let events = gtk::ListBox::new();
    events.set_css_classes(&["boxed-list"]);
    let events_title = gtk::Label::new(Some("Events"));
    events_title.set_halign(gtk::Align::Start);
    let events_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    events_box.append(&events_title);
    events_box.append(&events);

    let content_column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content_column.set_margin_top(12);
    content_column.set_margin_bottom(12);
    content_column.set_margin_start(12);
    content_column.set_margin_end(12);
    content_column.append(&content_slot);
    content_column.append(&events_box);
    let content_scroll = gtk::ScrolledWindow::new();
    content_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    content_scroll.set_child(Some(&content_column));

    let map_button = gtk::Button::with_label("Flow map");
    let width_labels: Vec<String> = WIDTH_PRESETS.iter().map(|w| format!("{w} px")).collect();
    let width_strs: Vec<&str> = width_labels.iter().map(String::as_str).collect();
    let width_choice = gtk::DropDown::from_strings(&width_strs);
    let theme_choice = gtk::DropDown::from_strings(&THEMES);
    width_choice.set_tooltip_text(Some("Preview width"));
    theme_choice.set_tooltip_text(Some("Theme"));
    theme_choice.set_selected(0);
    width_choice.set_selected(0);

    let content_bar = adw::HeaderBar::new();
    content_bar.pack_start(&map_button);
    content_bar.pack_end(&theme_choice);
    content_bar.pack_end(&width_choice);
    let content_view = adw::ToolbarView::new();
    content_view.add_top_bar(&content_bar);
    content_view.set_content(Some(&content_scroll));

    // --- split view ----------------------------------------------------------
    let sidebar_page = adw::NavigationPage::builder()
        .title("Views")
        .child(&sidebar_view)
        .build();
    let content_page = adw::NavigationPage::builder()
        .title("Preview")
        .child(&content_view)
        .build();
    let split = adw::NavigationSplitView::new();
    split.set_sidebar(Some(&sidebar_page));
    split.set_content(Some(&content_page));
    split.set_show_content(false);

    StoryListView {
        split,
        sidebar,
        content_slot,
        events,
        width_choice,
        theme_choice,
        map_button,
    }
}
