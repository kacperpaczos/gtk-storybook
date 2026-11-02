//! Dialog mode (references/canvas.md): some widgets must never be parented
//! into a story mount — `AdwDialog` aborts with "Use adw_dialog_present()",
//! windows need a toplevel, popovers need to be attached. The viewer hosts
//! their *content* instead, wrapped in a dialog frame; the shell widget is
//! kept alive so its callbacks stay valid.

use gtk4::prelude::*;
use libadwaita::prelude::*;
/// Widgets that must never be parented, by GType name.
const DIALOG_TYPES: &[&str] = &["AdwDialog"];
const WINDOW_TYPES: &[&str] = &[
    "GtkWindow",
    "AdwWindow",
    "GtkApplicationWindow",
    "AdwApplicationWindow",
];
const POPOVER_TYPES: &[&str] = &["GtkPopover"];

/// How a built story widget is hosted in a story mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostingMode {
    /// Parent the widget itself.
    Direct,
    /// `AdwDialog`: host its child, keep the dialog alive.
    DialogContent,
    /// `GtkWindow`/`AdwWindow` (and application windows): host their child.
    WindowContent,
    /// `GtkPopover`: host its child.
    PopoverContent,
}

/// Pure decision from a GType name (e.g. `"AdwDialog"`): how to host a story
/// widget. Unknown types are hosted directly — a custom host widget is an
/// ordinary widget.
pub fn hosting_mode(type_name: &str) -> HostingMode {
    if DIALOG_TYPES.contains(&type_name) {
        HostingMode::DialogContent
    } else if WINDOW_TYPES.contains(&type_name) {
        HostingMode::WindowContent
    } else if POPOVER_TYPES.contains(&type_name) {
        HostingMode::PopoverContent
    } else {
        HostingMode::Direct
    }
}

/// A story widget ready to be placed in a mount: `widget` is what the mount
/// parents, `keep_alive` holds unparented shells (the `AdwDialog` whose child
/// is hosted) so their callbacks stay valid.
pub struct Hosted {
    pub widget: gtk::Widget,
    pub keep_alive: Vec<gtk::Widget>,
}

/// Host `built` per its [`hosting_mode`]. Dialog-frame: a titled card
/// (`dialog-sheet`) standing in for the dialog chrome. Popovers have no
/// title of their own; they keep the frame for consistent styling.
#[mutants::skip] // GTK widget surgery; manual GUI check deferred to user
pub fn host(built: &gtk::Widget) -> Hosted {
    let direct = || Hosted {
        widget: built.clone(),
        keep_alive: Vec::new(),
    };
    if hosting_mode(built.type_().name()) == HostingMode::Direct {
        return direct();
    }
    // Dialogs, windows and popovers are all bins: take their child and title.
    let (child, title) = if let Some(dialog) = built.downcast_ref::<adw::Dialog>() {
        (dialog.child(), Some(dialog.title()))
    } else if let Some(window) = built.downcast_ref::<gtk::Window>() {
        (window.child(), window.title())
    } else if let Some(popover) = built.downcast_ref::<gtk::Popover>() {
        (popover.child(), None)
    } else {
        return direct();
    };
    let Some(child) = child else {
        return Hosted {
            widget: built.clone(),
            keep_alive: vec![built.clone()],
        };
    };
    // Unparent the content from the shell; the shell stays alive below.
    if let Some(dialog) = built.downcast_ref::<adw::Dialog>() {
        dialog.set_child(gtk::Widget::NONE);
    } else if let Some(window) = built.downcast_ref::<gtk::Window>() {
        window.set_child(gtk::Widget::NONE);
    } else if let Some(popover) = built.downcast_ref::<gtk::Popover>() {
        popover.set_child(gtk::Widget::NONE);
    }

    let frame = gtk::Box::new(gtk::Orientation::Vertical, 12);
    frame.set_css_classes(&["dialog-sheet", "card"]);
    if title.as_deref().is_some_and(|t| !t.is_empty()) {
        let heading = gtk::Label::new(title.as_deref());
        heading.set_halign(gtk::Align::Start);
        heading.add_css_class("title");
        frame.append(&heading);
    }
    frame.append(&child);
    Hosted {
        widget: frame.upcast(),
        keep_alive: vec![built.clone()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GTK-init test, gated behind `GTK_STORYBOOK_GTK_TESTS=1` and skipped when
    /// no display is available — it never creates one, never presents.
    #[test]
    #[allow(clippy::disallowed_methods)] // env gate is mandated by the kit harness rules
    fn host_unparents_dialog_content_into_a_sheet() {
        if std::env::var_os("GTK_STORYBOOK_GTK_TESTS").as_deref() != Some(std::ffi::OsStr::new("1"))
        {
            eprintln!("skipping: GTK_STORYBOOK_GTK_TESTS is not set to 1");
            return;
        }
        if gtk::init().is_err() {
            eprintln!("skipping: no display available for gtk::init");
            return;
        }

        let dialog = adw::Dialog::new();
        dialog.set_title("Nowy skill");
        let label = gtk::Label::new(Some("form"));
        dialog.set_child(Some(&label));
        let hosted = host(dialog.upcast_ref());

        assert!(dialog.child().is_none(), "content must be unparented");
        assert_eq!(hosted.widget.type_().name(), "GtkBox");
        assert!(hosted.widget.has_css_class("dialog-sheet"));
        assert_eq!(hosted.keep_alive.len(), 1);

        // Direct hosting leaves plain widgets untouched.
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let direct = host(box_.upcast_ref());
        assert_eq!(direct.widget.type_().name(), "GtkBox");
        assert!(direct.keep_alive.is_empty());
    }
}
