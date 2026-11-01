use crate::gtk::{Button, Widget};
use serde::{Deserialize, Serialize};
use std::rc::Rc;

/// Covers: text axis, bool axis, enum axis, an unread field and two flavors
/// of widget-local state (a `let mut` counter and a `Cell` captured by a
/// `connect_*` handler).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillDetailVm {
    pub name: String,
    pub dirty: bool,
    pub filter: Filter,
    pub internal_note: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum Filter {
    All,
    Installed,
    Modified,
}

pub struct SkillDetailActions {
    pub on_save: Rc<dyn Fn()>,
    pub on_copy_path: Rc<dyn Fn()>,
}

pub fn build(vm: &SkillDetailVm, actions: &SkillDetailActions) -> Widget {
    let title = render_title(vm);
    let _ = vm.dirty;
    let shown = match vm.filter {
        Filter::All => 0,
        Filter::Installed => 1,
        Filter::Modified => 2,
    };
    let mut clicks = 0u32;
    let clicks_cell = std::cell::Cell::new(0u32);
    let button = Button;
    button.connect_clicked(move || {
        clicks += 1;
    });
    button.connect_clicked(move || {
        clicks_cell.set(clicks_cell.get() + 1);
    });
    (actions.on_save)();
    let _ = (title, shown);
    let _ = &actions.on_copy_path;
    button.widget()
}

/// Helper in the same module taking the whole VM: its reads project too.
fn render_title(vm: &SkillDetailVm) -> String {
    vm.name.clone()
}
