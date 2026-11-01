//! Pure story-list helpers: presets, theme mapping, row titles (canvas.md).

use storybook_viewer::adw;
use storybook_viewer::story_list::{THEMES, WIDTH_PRESETS, story_title, theme_scheme, view_of};

#[test]
fn width_presets_match_the_canvas_contract() {
    assert_eq!(WIDTH_PRESETS, [360, 600, 900, 1200]);
}

#[test]
fn themes_cover_system_light_dark() {
    assert_eq!(THEMES, ["System", "Light", "Dark"]);
}
#[test]
fn theme_scheme_maps_index_to_adw_scheme() {
    assert_eq!(theme_scheme(0), adw::ColorScheme::Default);
    assert_eq!(theme_scheme(1), adw::ColorScheme::ForceLight);
    assert_eq!(theme_scheme(2), adw::ColorScheme::ForceDark);
    // Out-of-range falls back to the system theme instead of panicking.
    assert_eq!(theme_scheme(usize::MAX), adw::ColorScheme::Default);
}

#[test]
fn story_title_strips_the_view_and_humanizes_classes() {
    assert_eq!(story_title("skill_detail/dirty-empty"), "dirty empty");
    assert_eq!(story_title("create/valid_some_value"), "valid some value");
    // A state id without a view part is shown as-is.
    assert_eq!(story_title("standalone"), "standalone");
}

#[test]
fn view_of_returns_the_owning_view() {
    assert_eq!(view_of("skill_detail/dirty"), "skill_detail");
    assert_eq!(view_of("standalone"), "standalone");
}
