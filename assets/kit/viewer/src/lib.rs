//! Storybook viewer library: the generic GTK shell every host storybook runs
//! (references/canvas.md). The host provides a [`Registry`] mapping view ids
//! to its real `build` functions; the viewer reads `inventory.json`,
//! `fixtures/`, `flows.toml` and `layout.json` from the storybook directory
//! and renders the story list plus the flow-map canvas.
//!
//! Nothing in this crate knows a specific host: SkillCraft is just the first
//! consumer of the kit.

pub mod anim;
pub mod app;
pub mod canvas;
pub mod hosting;
pub mod sidebar;
pub mod signature;
pub mod story_list;

pub extern crate gtk4 as gtk;
pub extern crate libadwaita as adw;

/// Hook notified on every recorded action (the Events pane installs one).
pub(crate) type StoryHook = Rc<dyn Fn(&str)>;

use std::cell::RefCell;
use std::rc::Rc;

pub use app::run;
pub use signature::render_signature;

/// Records every `Actions` callback fired inside a live story: appended to
/// the per-story log and forwarded to the hook the Events pane installs.
/// Clones share the log so closures captured at build time stay connected.
#[derive(Clone, Default)]
pub struct Events {
    log: Rc<RefCell<Vec<String>>>,
    hook: Rc<RefCell<Option<StoryHook>>>,
}

impl Events {
    pub fn new() -> Self {
        Self::default()
    }

    /// Log that `action` fired (e.g. `on_save`).
    pub fn record(&self, action: &str) {
        self.log.borrow_mut().push(action.to_owned());
        if let Some(hook) = self.hook.borrow().clone() {
            hook(action);
        }
    }

    /// Snapshot of the actions recorded so far.
    pub fn log(&self) -> Vec<String> {
        self.log.borrow().clone()
    }

    /// Install (or remove) the live notification hook.
    pub fn set_hook(&self, hook: Option<StoryHook>) {
        *self.hook.borrow_mut() = hook;
    }
}

/// Host-provided mapping from storybook view ids to real view constructors.
pub trait Registry {
    /// View ids the host registered, in manifest (inventory) order.
    fn views(&self) -> Vec<String>;

    /// Build the live widget for `state_id` (`view/classes`) from its
    /// fixture JSON; `events` records every action the story fires.
    fn build(
        &self,
        state_id: &str,
        fixture_json: &str,
        events: &Events,
    ) -> Result<gtk::Widget, String>;
}
