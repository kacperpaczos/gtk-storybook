//! Render signatures (references/canvas.md `--check-render`).
//!
//! A signature is a SHA-256 hex digest of a walked widget-tree description:
//! per node the type name, visibility, sensitivity, label/text property, css
//! classes and child order. Two distinct cells with an identical signature
//! mean a wrong axis or a missing guard. The walk is a pure function of a
//! built tree (no rendering, no allocation needed).

use gtk::glib;
use gtk::prelude::*;
use sha2::{Digest, Sha256};

/// Signature of a built story tree: walk it, hash the description.
#[mutants::skip] // widget-tree walk needs gtk::init; covered by the GTK_STORYBOOK_GTK_TESTS-gated test below
pub fn render_signature(w: &gtk::Widget) -> String {
    let mut description = String::new();
    walk(w, 0, &mut description);
    hash_lines(&description)
}

/// SHA-256 hex digest of a tree description. Pure; distinct trees must give
/// distinct digests, equal trees equal digests.
pub(crate) fn hash_lines(description: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(description.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn walk(w: &gtk::Widget, depth: usize, acc: &mut String) {
    let type_name = w.type_().name();
    let text = text_property(w, "label").or_else(|| text_property(w, "text"));
    let css_classes = w.css_classes();
    acc.push_str(&format!(
        "{}{} visible={} sensitive={} text={text:?} css={css_classes:?}\n",
        "  ".repeat(depth),
        type_name,
        w.is_visible(),
        w.is_sensitive(),
    ));
    let children = w.observe_children();
    for i in 0..children.n_items() {
        let Some(child) = children
            .item(i)
            .and_then(|obj| obj.downcast::<gtk::Widget>().ok())
        else {
            continue;
        };
        walk(&child, depth + 1, acc);
    }
}

/// Value of a string property (`label`, `text`) if the widget has one.
fn text_property(w: &gtk::Widget, name: &str) -> Option<String> {
    let spec = w.find_property(name)?;
    if spec.value_type() != glib::Type::STRING {
        return None;
    }
    w.property_value(name).get::<String>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_lines_is_deterministic_and_sensitive() {
        let a = hash_lines("GtkLabel hello\n");
        assert_eq!(a, hash_lines("GtkLabel hello\n"));
        assert_eq!(a.len(), 64);
        assert_ne!(a, hash_lines("GtkLabel hello \n"));
        assert_ne!(a, hash_lines("GtkLabel hello\nGtkBox\n"));
    }

    /// The single GTK-init test of this crate (harness rule: gated behind
    /// `GTK_STORYBOOK_GTK_TESTS=1` and skipped when no display is available —
    /// it never creates one and never calls `present()`).
    #[test]
    #[allow(clippy::disallowed_methods)] // env gate is mandated by the kit harness rules
    fn render_signature_walks_a_built_tree() {
        if std::env::var_os("GTK_STORYBOOK_GTK_TESTS").as_deref() != Some(std::ffi::OsStr::new("1"))
        {
            eprintln!("skipping: GTK_STORYBOOK_GTK_TESTS is not set to 1");
            return;
        }
        if gtk::init().is_err() {
            eprintln!("skipping: no display available for gtk::init");
            return;
        }

        let tree = || {
            let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
            let l1 = gtk::Label::new(Some("Zapisz"));
            let l2 = gtk::Label::new(Some("Anuluj"));
            l2.set_sensitive(false);
            b.append(&l1);
            b.append(&l2);
            b
        };

        let sig1 = render_signature(&tree().upcast::<gtk::Widget>());
        let sig2 = render_signature(&tree().upcast::<gtk::Widget>());
        assert_eq!(sig1, sig2, "identical trees must have identical signatures");

        let changed = tree();
        changed.first_child().unwrap().set_visible(false);
        let sig3 = render_signature(&changed.upcast::<gtk::Widget>());
        assert_ne!(sig1, sig3, "visibility must be part of the signature");

        let relabeled = {
            let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
            let l1 = gtk::Label::new(Some("Zapisz jako"));
            let l2 = gtk::Label::new(Some("Anuluj"));
            l2.set_sensitive(false);
            b.append(&l1);
            b.append(&l2);
            b
        };
        assert_ne!(
            sig1,
            render_signature(&relabeled.upcast::<gtk::Widget>()),
            "label text must be part of the signature"
        );

        // Same preorder widget sequence, different nesting: indentation (depth)
        // is what tells them apart, so it must be part of the signature.
        let nested = |deep: bool| {
            let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
            let mid = gtk::Box::new(gtk::Orientation::Vertical, 6);
            let label = gtk::Label::new(Some("X"));
            if deep {
                mid.append(&label);
                root.append(&mid);
            } else {
                root.append(&mid);
                root.append(&label);
            }
            root
        };
        assert_ne!(
            render_signature(&nested(true).upcast::<gtk::Widget>()),
            render_signature(&nested(false).upcast::<gtk::Widget>()),
            "nesting depth must be part of the signature"
        );

        let other = gtk::Label::new(Some("Zapisz"));
        assert_ne!(sig1, render_signature(&other.upcast::<gtk::Widget>()));
    }
}
