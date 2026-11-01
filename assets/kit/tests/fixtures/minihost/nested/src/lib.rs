//! Second crate inside the host workspace: nested crates and subdirectory
//! modules are part of the scan.

pub mod gtk {
    #[derive(Clone)]
    pub struct Widget;
}

pub mod inner;
