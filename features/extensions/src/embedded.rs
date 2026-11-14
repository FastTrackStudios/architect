//! Another extension's view inside a remote view (#710, for Bases).
//!
//! A remote view (a process's, a WASI component's) can't run another
//! extension's code, but it can hold its view: it renders a placeholder
//! naming `<extension>/<view>` ([`VIEW_ATTR`]) and the input that view is
//! given ([`INPUT_ATTR`], an [`EmbedInput`] as JSON: a base view's is
//! `linked::BASE_VIEW_CLAIM` with its rows). The host mounts the named
//! view over the placeholder's box, sized by it, clipped to it and to the
//! pane, in front of it so it takes its own input: a linked view in the
//! app's document, a process's or a component's as a remote view of its
//! own (`remote::RemoteDocument::embedded`). It runs as its own extension,
//! with its own capabilities; the view that holds it reaches nothing of
//! it but its place and its input.
//!
//! In the guest: [`EmbeddedView`] (feature `guest`) renders the
//! placeholder.

use crate::contrib::EmbedInput;

/// The attribute naming the view a placeholder holds: `<extension>/<view>`.
pub const VIEW_ATTR: &str = "data-architect-embed-view";

/// The attribute carrying its input: an [`EmbedInput`] as JSON.
pub const INPUT_ATTR: &str = "data-architect-embed-input";

/// The most views one remote view holds at once; past it the view fails.
pub const MAX_EMBEDDED: usize = 64;

/// The most bytes of one held view's input.
pub const MAX_INPUT_BYTES: usize = 1024 * 1024;

/// A held view: whose, which, with what, and where (CSS pixels, in the
/// holding view's box, scrolled).
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    /// Unique in its document while it lives.
    pub key: u64,
    pub extension: String,
    pub view: String,
    pub input: EmbedInput,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// `<extension>/<view>` read: both parts, the extension id with its dots.
#[must_use]
pub fn split_view(value: &str) -> Option<(&str, &str)> {
    let (extension, view) = value.rsplit_once('/')?;
    (!extension.is_empty() && !view.is_empty()).then_some((extension, view))
}

/// The guest's side.
#[cfg(feature = "guest")]
mod guest {
    use dioxus::prelude::*;

    use crate::contrib::EmbedInput;

    /// Another extension's view, held here: `extension`'s view `view`,
    /// given `input`, filling this element (sized as `style` says; it
    /// fills its parent by default). The host draws it.
    #[component]
    pub fn EmbeddedView(
        extension: String,
        view: String,
        #[props(default)] input: EmbedInput,
        #[props(default)] style: String,
    ) -> Element {
        let name = format!("{extension}/{view}");
        let input = serde_json::to_string(&input).unwrap_or_default();
        rsx! {
            div {
                style: "display: block; width: 100%; height: 100%; {style}",
                "data-architect-embed-view": "{name}",
                "data-architect-embed-input": "{input}",
            }
        }
    }
}

#[cfg(feature = "guest")]
pub use guest::EmbeddedView;

#[cfg(test)]
mod tests {
    use super::split_view;

    #[test]
    fn a_held_view_is_named_extension_slash_view() {
        assert_eq!(
            split_view("fts.bases/base.table"),
            Some(("fts.bases", "base.table"))
        );
        assert_eq!(
            split_view("lab.kanban/board"),
            Some(("lab.kanban", "board"))
        );
        assert_eq!(split_view("board"), None);
        assert_eq!(split_view("/board"), None);
        assert_eq!(split_view("a/"), None);
    }
}
