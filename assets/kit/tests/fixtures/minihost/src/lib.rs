//! Minimal fake host used as a scan/probe fixture: three views in the
//! VM/build/actions style described by the storybook host contract, plus
//! `corners` (scanner syntax corners) and its `alt` module (by-value and
//! decoy views).

pub mod gtk {
    /// Stand-in for the real `gtk::Widget` so the fixture builds without GTK.
    #[derive(Clone)]
    pub struct Widget;

    impl Widget {
        pub fn show(&self) {}
    }

    pub struct Button;

    impl Button {
        pub fn connect_clicked(&self, _handler: impl FnMut()) {}
        pub fn widget(&self) -> Widget {
            Widget
        }
    }
}

pub mod corners;
pub mod create;
pub mod settings;
pub mod skill_detail;
