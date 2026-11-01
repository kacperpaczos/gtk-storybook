//! A view one directory below the crate root.

use crate::gtk::Widget;
use serde::{Deserialize, Serialize};
use std::rc::Rc;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct DeepVm {
    pub note: String,
}

pub struct DeepActions {
    pub on_leave: Rc<dyn Fn()>,
}

pub fn build(vm: &DeepVm, actions: &DeepActions) -> Widget {
    let _ = vm.note.len();
    let _ = &actions.on_leave;
    Widget
}
