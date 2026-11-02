//! Story list (references/canvas.md): an `AdwNavigationSplitView` whose
//! sidebar is an `AdwNavigationView` drill-down (views -> states,
//! GNOME HIG "browsing": back button, optional search), and whose content
//! shows the live story inside an `AdwBreakpointBin` at the chosen width
//! preset (exact width via a `GtkConstraintGuide` with min = max) plus an
//! Events pane.

use adw::prelude::*;

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
    /// Left column: the drill-down menu (nav pages carry their header bars).
    pub sidebar: adw::ToolbarView,
    /// Right column: live preview of the selected state + Events pane.
    pub preview: adw::ToolbarView,
    /// Sidebar drill-down: level 1 "Views", one level-2 page per view,
    /// plus the flat search-results page.
    pub nav: adw::NavigationView,
    /// Drill-down pages (crate::sidebar::pages): spec, page widget and row
    /// list share one index; page 0 is the root "Widoki".
    pub specs: Vec<crate::sidebar::PageSpec>,
    pub pages: Vec<adw::NavigationPage>,
    pub lists: Vec<gtk::ListBox>,
    pub search_page: adw::NavigationPage,
    pub search_list: gtk::ListBox,
    pub search_entry: gtk::SearchEntry,
    pub content_slot: gtk::Box,
    pub events: gtk::ListBox,
    pub width_choice: gtk::DropDown,
    pub theme_choice: gtk::DropDown,
    /// Hides the preview column.
    pub preview_close: gtk::Button,
}

/// The row list of one drill-down page: a row per spec row; rows that
/// lead to a further page carry `go-next-symbolic` and are not selectable,
/// leaf rows select a state (state id in the tooltip, never as text).
fn page_list(spec: &crate::sidebar::PageSpec) -> gtk::ListBox {
    let list = gtk::ListBox::new();
    list.set_css_classes(&["navigation-sidebar"]);
    for row_spec in &spec.rows {
        let row = adw::ActionRow::builder()
            .title(&row_spec.title)
            .activatable(true)
            .build();
        if let Some(subtitle) = &row_spec.subtitle {
            row.set_subtitle(subtitle);
        }
        match &row_spec.target {
            crate::sidebar::Target::Page(_) => {
                row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
                row.set_selectable(false);
            }
            crate::sidebar::Target::State(state_id) => {
                row.set_tooltip_text(Some(state_id));
            }
        }
        list.append(&row);
    }
    list
}

/// A non-root drill-down page: header bar (back button) over the list.
fn sub_page(spec: &crate::sidebar::PageSpec, list: &gtk::ListBox) -> adw::NavigationPage {
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_child(Some(list));
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&scroll));
    adw::NavigationPage::builder()
        .title(&spec.title)
        .child(&toolbar)
        .build()
}

/// Build the story list: sidebar drill-down from the pure sidebar model
/// ([`crate::sidebar`]), content slot hosts the live story, Events pane is
/// fed by the application from [`crate::Events`].
#[mutants::skip] // widget construction; manual GUI check deferred to user
pub(crate) fn build(specs: &[crate::sidebar::PageSpec]) -> StoryListView {
    // --- sidebar: drill-down pages (root = views) ----------------------------
    let lists: Vec<gtk::ListBox> = specs.iter().map(page_list).collect();
    let views_list = lists[0].clone();
    let views_scroll = gtk::ScrolledWindow::new();
    views_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    views_scroll.set_child(Some(&views_list));

    let search_entry = gtk::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Szukaj stanów"));
    search_entry.set_hexpand(true);
    let search_bar = gtk::SearchBar::builder().child(&search_entry).build();
    let search_button = gtk::ToggleButton::new();
    search_button.set_icon_name("system-search-symbolic");
    search_button.set_tooltip_text(Some("Szukaj (Ctrl+F)"));
    search_bar.connect_entry(&search_entry);

    let header = adw::HeaderBar::new();
    header.pack_end(&search_button);
    search_button
        .bind_property("active", &search_bar, "search-mode-enabled")
        .sync_create()
        .bidirectional()
        .build();
    let views_toolbar = adw::ToolbarView::new();
    views_toolbar.add_top_bar(&header);
    views_toolbar.add_top_bar(&search_bar);
    views_toolbar.set_content(Some(&views_scroll));
    let views_page = adw::NavigationPage::builder()
        .title("Widoki")
        .child(&views_toolbar)
        .build();

    // --- sidebar: search results (flat, while the query is non-empty) -----
    let search_list = gtk::ListBox::new();
    search_list.set_css_classes(&["navigation-sidebar"]);
    let search_scroll = gtk::ScrolledWindow::new();
    search_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    search_scroll.set_child(Some(&search_list));
    let search_toolbar = adw::ToolbarView::new();
    search_toolbar.add_top_bar(&adw::HeaderBar::new());
    search_toolbar.set_content(Some(&search_scroll));
    let search_page = adw::NavigationPage::builder()
        .title("Wyniki")
        .child(&search_toolbar)
        .build();

    let nav = adw::NavigationView::new();
    let mut pages = vec![views_page];
    for (spec, list) in specs.iter().zip(&lists).skip(1) {
        pages.push(sub_page(spec, list));
    }
    for page in &pages {
        nav.add(page);
    }
    nav.add(&search_page);
    // Ctrl+F / typing finds the search entry while the sidebar has focus.
    search_bar.set_key_capture_widget(Some(&views_toolbar));

    // The nav pages carry their own header bars; no extra outer bar.
    let sidebar_view = adw::ToolbarView::new();
    sidebar_view.set_content(Some(&nav));

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

    let preview_close = gtk::Button::from_icon_name("window-close-symbolic");
    preview_close.set_tooltip_text(Some("Ukryj podgląd"));
    let width_labels: Vec<String> = WIDTH_PRESETS.iter().map(|w| format!("{w} px")).collect();
    let width_strs: Vec<&str> = width_labels.iter().map(String::as_str).collect();
    let width_choice = gtk::DropDown::from_strings(&width_strs);
    let theme_choice = gtk::DropDown::from_strings(&THEMES);
    width_choice.set_tooltip_text(Some("Preview width"));
    theme_choice.set_tooltip_text(Some("Theme"));
    theme_choice.set_selected(0);
    width_choice.set_selected(0);

    let content_bar = adw::HeaderBar::new();
    // window controls live on the map's bar; this one only closes the column
    content_bar.set_show_start_title_buttons(false);
    content_bar.set_show_end_title_buttons(false);
    content_bar.set_title_widget(Some(&gtk::Label::new(Some("Podgląd"))));
    content_bar.pack_start(&width_choice);
    content_bar.pack_start(&theme_choice);
    content_bar.pack_end(&preview_close);
    let content_view = adw::ToolbarView::new();
    content_view.add_top_bar(&content_bar);
    content_view.set_content(Some(&content_scroll));

    StoryListView {
        sidebar: sidebar_view,
        preview: content_view,
        nav,
        specs: specs.to_vec(),
        pages,
        lists,
        search_page,
        search_list,
        search_entry,
        content_slot,
        events,
        width_choice,
        theme_choice,
        preview_close,
    }
}
