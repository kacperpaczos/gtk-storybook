use crate::gtk::Widget;
use serde::{Deserialize, Serialize};
use std::rc::Rc;

/// Covers: int and float number axes, char, text axis, Option-with-enum-inner
/// axis, Result phase axis, struct-read-as-a-whole axis and a feature-gated
/// field.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingsVm {
    pub poll_seconds: u32,
    pub ratio: f64,
    pub marker: char,
    pub prompt: String,
    pub sort: Option<SortOrder>,
    pub load: Result<String, String>,
    pub meta: ViewMeta,
    #[cfg(feature = "extra")]
    pub experimental: bool,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum SortOrder {
    Asc,
    Desc,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewMeta {
    pub revision: u32,
}

pub struct SettingsActions {
    pub on_reset: Rc<dyn Fn()>,
    pub on_close: Rc<dyn Fn()>,
}

pub fn build(vm: &SettingsVm, _actions: &SettingsActions) -> Widget {
    let _ = vm.poll_seconds;
    let _ = vm.ratio;
    let _ = vm.marker;
    let _ = &vm.prompt;
    if let Some(order) = &vm.sort {
        match order {
            SortOrder::Asc => {}
            SortOrder::Desc => {}
        }
    }
    let _ = &vm.load;
    let _ = &vm.meta;
    Widget
}

pub fn update(_widget: &Widget, vm: &SettingsVm) {
    let _ = vm.poll_seconds;
}
