//! Syntax corners for the scanner: reference/paren/slice/array types, unit
//! and pair tuples, `Option<bool>` read structurally through all three
//! binding forms, an enum-in-Option with tuple and struct payloads, a
//! phase-named enum that is not a phase, `Cow`, a non-std `fs` module,
//! expression macros, Cell initializers and edge candidates with literal
//! and computed targets.

use crate::gtk::Widget;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::rc::Rc;

/// Non-std module whose name collides with an I/O module name.
pub mod fs {
    pub const PATHS: u8 = 0;
}
use self::fs as net;

macro_rules! trace_val {
    () => {
        0u32
    };
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct CornersVm {
    pub suffix: String,
    pub parens: (String),
    pub raw: Vec<u8>,
    pub unit: (),
    pub autosave: Option<bool>,
    pub mode: Option<Mode>,
    pub otp: OnlyPending,
    pub debug_trace: bool,
    pub cow: Cow<'static, str>,
    pub duo: (u8, u8),
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum Mode {
    On(bool),
    Custom { loud: bool },
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum OnlyPending {
    Pending,
}

pub struct CornersActions {
    pub on_open: Rc<dyn Fn()>,
}

#[derive(Clone)]
pub struct Nav;

impl Nav {
    pub fn push(&self, _target: &str) {}
    pub fn pop(&self) {}
}

pub struct Page {
    pub stack: Nav,
}

impl Page {
    pub fn push(&self, _target: &str) {}
    pub fn pop(&self) {}
}

pub fn build(vm: &CornersVm, actions: &CornersActions) -> Widget {
    let _ = vm.suffix;
    let _ = &vm.parens;
    let _ = vm.raw.len();
    let _ = vm.unit;
    // three binding forms all mark the Option<bool> inner as read
    if let Some(flag) = &vm.autosave {
        let _ = *flag;
    }
    match &vm.autosave {
        Some(flag) => {
            let _ = flag;
        }
        None => {}
    }
    for flag in &vm.autosave {
        let _ = *flag;
    }
    if let Some(mode) = &vm.mode {
        match mode {
            Mode::On(power) => {
                let _ = *power;
            }
            Mode::Custom { loud } => {
                let _ = loud;
            }
        }
    }
    let _ = &vm.otp;
    let _ = vm.debug_trace;
    let _ = vm.cow.len();
    let _ = vm.duo;
    let _ = self::fs::PATHS;

    let mut edited = 0u32;
    let counter = std::cell::Cell::new(0u32);
    let stale = std::cell::Cell::replace(&counter, 1);
    let clicks = std::cell::Cell::new(0u32);
    let page = Page { stack: Nav };
    let nav = Nav;
    (actions.on_open)();
    page.push("corners/base");
    nav.clone().push("settings/any-any-none-ok-any");
    page.stack.push("skill_detail/any-true-All");
    (&page).pop();
    let target = String::new();
    nav.clone().push(&target);
    page.stack.pop();
    let _ = (edited, stale, clicks);
    let _ = trace_val!();
    Widget
}

pub fn update(_widget: &Widget, vm: &CornersVm) {
    let mut passes = 0u32;
    passes += u32::from(vm.debug_trace);
    // no actions parameter: navigation effects here are not edge candidates
    let nav = Nav;
    nav.pop();
}

/// No VM or actions parameter: never scanned as a view function.
fn noise(x: u8) -> u8 {
    let mut acc = x;
    acc += 1;
    acc
}

/// Array, Box and a two-field tuple-variant enum.
pub mod pairy {
    use crate::gtk::Widget;
    use serde::{Deserialize, Serialize};
    use std::rc::Rc;

    #[derive(Clone, PartialEq, Serialize, Deserialize)]
    pub enum Decision {
        Yes(bool),
        No(u8, u8),
    }

    #[derive(Clone, PartialEq, Serialize, Deserialize)]
    pub struct PairVm {
        pub pick: Decision,
        pub fixed: [u8; 4],
        pub boxed: Box<bool>,
        /// cfg without a feature gate is not feature-gating
        #[cfg(test)]
        pub experimental: bool,
    }

    pub struct PairActions {
        pub on_decide: Rc<dyn Fn()>,
    }

    pub fn build(vm: &PairVm, actions: &PairActions) -> Widget {
        let _ = &vm.pick;
        let _ = vm.fixed.len();
        let _ = *vm.boxed;
        #[cfg(test)]
        let _ = vm.experimental;
        let _ = &actions.on_decide;
        Widget
    }
}
/// The Option is read only through a match with a reference pattern.
pub mod pm {
    use crate::gtk::Widget;
    use serde::{Deserialize, Serialize};
    use std::rc::Rc;

    #[derive(Clone, PartialEq, Serialize, Deserialize)]
    pub struct PmVm {
        pub opt: Option<bool>,
        /// read only through a parenthesized receiver
        pub hint: String,
    }

    pub struct PmActions {
        pub on_go: Rc<dyn Fn()>,
    }

    pub fn build(vm: &PmVm, actions: &PmActions) -> Widget {
        match &vm.opt {
            &Some(flag) => {
                let _ = flag;
            }
            _ => {}
        }
        let _ = (vm).hint.len();
        let _ = &actions.on_go;
        Widget
    }
}

/// One Option read only through a for loop, one Result-in-Option.
pub mod pf {
    use crate::gtk::Widget;
    use serde::{Deserialize, Serialize};
    use std::rc::Rc;

    #[derive(Clone, PartialEq, Serialize, Deserialize)]
    pub struct PfVm {
        pub opt: Option<bool>,
        pub job: Option<Result<String, String>>,
    }

    pub struct PfActions {
        pub on_run: Rc<dyn Fn()>,
    }

    pub fn build(vm: &PfVm, actions: &PfActions) -> Widget {
        for flag in &vm.opt {
            let _ = *flag;
        }
        if let Some(result) = &vm.job {
            let _ = result.is_ok();
        }
        let _ = &actions.on_run;
        Widget
    }
}

/// Options whose inner enums are read only through struct, tuple and or
/// patterns nested inside the binding itself.
pub mod pv {
    use crate::gtk::Widget;
    use serde::{Deserialize, Serialize};
    use std::rc::Rc;

    #[derive(Clone, PartialEq, Serialize, Deserialize)]
    pub enum Holder {
        One(bool),
        Named { loud: bool },
    }

    #[derive(Clone, PartialEq, Serialize, Deserialize)]
    pub enum Holder2 {
        Pair((bool, bool)),
    }

    #[derive(Clone, PartialEq, Serialize, Deserialize)]
    pub enum Holder4 {
        Y(bool),
        Z(bool),
    }

    #[derive(Clone, PartialEq, Serialize, Deserialize)]
    pub struct PvVm {
        pub pick: Option<Holder>,
        pub pick2: Option<Holder2>,
        pub pick3: Option<Holder4>,
    }

    pub struct PvActions {
        pub on_pick: Rc<dyn Fn()>,
    }

    pub fn build(vm: &PvVm, actions: &PvActions) -> Widget {
        if let Some(Holder::Named { loud }) = &vm.pick {
            let _ = loud;
        }
        if let Some(Holder2::Pair((first, second))) = &vm.pick2 {
            let _ = (first, second);
        }
        if let Some(Holder4::Y(first) | Holder4::Z(first)) = &vm.pick3 {
            let _ = first;
        }
        let _ = &actions.on_pick;
        Widget
    }
}

pub mod alt {
    use crate::corners::fs;
    use crate::gtk::Widget;
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;
    use std::rc::Rc;

    pub struct DecoyVm;
    pub struct DecoyActions;
    pub struct HiddenVm;

    #[derive(Clone, PartialEq, Serialize, Deserialize)]
    pub struct AltVm {
        pub greeting: String,
        pub table: HashMap<String, u32>,
    }

    pub struct AltActions {
        pub on_done: Rc<dyn Fn()>,
    }

    // private build with a different VM: never a view, and its locals must
    // not leak findings into the alt view
    fn build(vm: &HiddenVm, actions: &AltActions) -> Widget {
        let mut shadowed = 0u32;
        let _ = (vm, actions, shadowed);
        Widget
    }

    // decoy that must NOT be a view: tuple without a widget
    pub fn build_with_handle(vm: &DecoyVm, actions: &DecoyActions) -> (u8, u8) {
        let mut tick = 0u32;
        let _ = (vm, actions, tick);
        (0, 0)
    }

    // by-value params and a tuple-with-widget return are a real view
    pub fn build_content(vm: AltVm, actions: AltActions) -> (Widget, u8) {
        let _ = vm.greeting.len();
        let _ = vm.table.len();
        let _ = &actions.on_done;
        let _ = fs::PATHS;
        (Widget, 0)
    }
}
