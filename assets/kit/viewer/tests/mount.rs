//! Mount lifecycle: exactly one story mount may live in the story-list
//! content slot; every other mount must stay in the off-view container
//! (rooted and allocated, so its `WidgetPaintable` thumbnail keeps painting).
//! The pure [`MountState`] transitions are applied by the GTK layer.

use storybook_viewer::app::{MountOp, MountPlace, MountState};

fn unwrap(state: &str) -> MountOp {
    MountOp::Unwrap {
        state: state.to_string(),
    }
}

fn place(state: &str, place: MountPlace) -> MountOp {
    MountOp::Place {
        state: state.to_string(),
        place,
    }
}

fn states(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

#[test]
fn rewrap_from_scratch_places_every_story_offview() {
    let mut mounts = MountState::default();
    assert_eq!(
        mounts.rewrap(&states(&["v/a", "v/b"])),
        vec![
            unwrap("v/a"),
            place("v/a", MountPlace::Offview),
            unwrap("v/b"),
            place("v/b", MountPlace::Offview),
        ]
    );
    assert_eq!(mounts.in_view(), None);
}

#[test]
fn show_moves_the_story_into_the_content_view() {
    let mut mounts = MountState::default();
    let ops = mounts.show("v/a");
    assert_eq!(
        ops,
        vec![unwrap("v/a"), place("v/a", MountPlace::ContentView)]
    );
    assert_eq!(mounts.in_view(), Some("v/a"));
}

#[test]
fn show_returns_the_previous_story_offview() {
    // Regression (review Critical 1): leaving a story must move it back into
    // the off-view container so its thumbnail stays rooted and allocated.
    let mut mounts = MountState::default();
    mounts.show("v/a");
    let ops = mounts.show("v/b");
    assert_eq!(
        ops,
        vec![
            unwrap("v/a"),
            place("v/a", MountPlace::Offview),
            unwrap("v/b"),
            place("v/b", MountPlace::ContentView),
        ]
    );
    assert_eq!(mounts.in_view(), Some("v/b"));
}

#[test]
fn show_of_the_story_already_in_view_does_nothing() {
    let mut mounts = MountState::default();
    mounts.show("v/a");
    assert_eq!(mounts.show("v/a"), Vec::<MountOp>::new());
    assert_eq!(mounts.in_view(), Some("v/a"));
}

#[test]
fn rewrap_keeps_the_shown_story_in_the_content_view() {
    // A width-preset change re-wraps every story at the new width; the story
    // in view must return to the content slot, all others off-view.
    let mut mounts = MountState::default();
    mounts.show("v/b");
    assert_eq!(
        mounts.rewrap(&states(&["v/a", "v/b", "v/c"])),
        vec![
            unwrap("v/a"),
            place("v/a", MountPlace::Offview),
            unwrap("v/b"),
            place("v/b", MountPlace::ContentView),
            unwrap("v/c"),
            place("v/c", MountPlace::Offview),
        ]
    );
    assert_eq!(mounts.in_view(), Some("v/b"));
}
