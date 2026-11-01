//! `Events`: the action recorder handed to `Registry::build`.

use storybook_viewer::Events;

#[test]
fn new_events_starts_empty() {
    assert_eq!(Events::new().log(), Vec::<String>::new());
}

#[test]
fn record_appends_in_order() {
    let events = Events::new();
    events.record("on_save");
    events.record("on_cancel");
    assert_eq!(
        events.log(),
        vec!["on_save".to_string(), "on_cancel".to_string()]
    );
}

#[test]
fn log_returns_a_snapshot_not_a_live_view() {
    let events = Events::new();
    events.record("on_save");
    let snapshot = events.log();
    events.record("on_delete");
    assert_eq!(snapshot, vec!["on_save".to_string()]);
    assert_eq!(events.log().len(), 2);
}

#[test]
fn hook_is_called_with_the_recorded_action() {
    let events = Events::new();
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let sink = seen.clone();
    events.set_hook(Some(std::rc::Rc::new(move |action: &str| {
        sink.borrow_mut().push(action.to_string());
    })));
    events.record("on_save");
    assert_eq!(*seen.borrow(), vec!["on_save".to_string()]);
}

#[test]
fn setting_no_hook_stops_notifications() {
    let events = Events::new();
    let fired = std::rc::Rc::new(std::cell::Cell::new(false));
    let flag = fired.clone();
    events.set_hook(Some(std::rc::Rc::new(move |_action: &str| {
        flag.set(true);
    })));
    events.set_hook(None);
    events.record("on_save");
    assert!(!fired.get());
    // The log is unaffected by the hook state.
    assert_eq!(events.log().len(), 1);
}

#[test]
fn clone_shares_the_underlying_log() {
    let events = Events::new();
    let clone = events.clone();
    clone.record("on_save");
    assert_eq!(events.log(), vec!["on_save".to_string()]);
}
