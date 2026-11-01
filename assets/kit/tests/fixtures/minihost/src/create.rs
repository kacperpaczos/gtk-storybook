use crate::gtk::Widget;
use serde::{Deserialize, Serialize};
use std::rc::Rc;

/// Covers: phase-like enum axis, Vec axis, Option axis, nested struct axis,
/// an unread nested field, a macro finding and a dialog edge candidate.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateVm {
    pub phase: Phase,
    pub tags: Vec<String>,
    pub desc: Option<String>,
    pub owner: OwnerInfo,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum Phase {
    Idle,
    Pending,
    Ok,
    Err(String),
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnerInfo {
    pub name: String,
    pub email: String,
}

pub struct CreateActions {
    pub on_confirm: Rc<dyn Fn()>,
    pub on_pick_dir: Rc<dyn Fn()>,
}

/// Dialog builder exercising the edge-candidate rule (`present`).
pub struct DirDialog;

impl DirDialog {
    pub fn new() -> Self {
        DirDialog
    }

    pub fn present(&self) {}
}

macro_rules! trace_view {
    () => {};
}

pub fn build(vm: &CreateVm, actions: &CreateActions) -> Widget {
    trace_view!();
    let _ = &vm.phase;
    let _ = vm.tags.len();
    if let Some(d) = &vm.desc {
        let _ = d.len();
    }
    let _ = vm.owner.name.clone();
    (actions.on_pick_dir)();
    DirDialog::new().present();
    let _ = &actions.on_confirm;
    Widget
}
