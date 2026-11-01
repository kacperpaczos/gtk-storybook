//! Which widgets must never be parented into a story mount, and how the
//! viewer hosts them instead (references/canvas.md "dialog mode").

use storybook_viewer::hosting::{HostingMode, hosting_mode};

fn mode(type_name: &str) -> HostingMode {
    hosting_mode(type_name)
}

#[test]
fn plain_widgets_are_hosted_directly() {
    assert_eq!(mode("GtkBox"), HostingMode::Direct);
    assert_eq!(mode("GtkLabel"), HostingMode::Direct);
    assert_eq!(mode("AdwPreferencesPage"), HostingMode::Direct);
    assert_eq!(mode("AdwBreakpointBin"), HostingMode::Direct);
    assert_eq!(mode("GtkListBox"), HostingMode::Direct);
}

#[test]
fn dialogs_host_their_content() {
    assert_eq!(mode("AdwDialog"), HostingMode::DialogContent);
}

#[test]
fn windows_host_their_content() {
    assert_eq!(mode("GtkWindow"), HostingMode::WindowContent);
    assert_eq!(mode("AdwWindow"), HostingMode::WindowContent);
    assert_eq!(mode("GtkApplicationWindow"), HostingMode::WindowContent);
    assert_eq!(mode("AdwApplicationWindow"), HostingMode::WindowContent);
}

#[test]
fn popovers_host_their_content() {
    assert_eq!(mode("GtkPopover"), HostingMode::PopoverContent);
}

#[test]
fn unknown_types_default_to_direct() {
    assert_eq!(mode("SomeHostCustomWidget"), HostingMode::Direct);
    assert_eq!(mode(""), HostingMode::Direct);
}
