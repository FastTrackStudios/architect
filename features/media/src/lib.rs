//! Core media primitives (#668): what any extension's audio or video work
//! builds on, with nothing domain-specific in it.
//!
//! - A **media item** ([`Item`]) is a note with a `title`, a `subtitle`
//!   (or `artist`), a `duration`, its media (`media`, or `recording`: an
//!   audio or video file of the vault) and its `cover` (or `art`).
//! - A **collection** ([`Collection`]) is a note listing items in order,
//!   in named sections: `sets: [{name, songs: [links]}]` and/or a flat
//!   `songs: [links]` (the shape Studio's setlists write, studio#406), or
//!   the generic `sections: [{name, items}]` / `items`. Any note that
//!   carries those fields is one; `type: playlist` is the generic name.
//!
//! Both are read from a note's frontmatter as JSON ([`front::fields`])
//! and resolved against the paths a caller can see ([`link::resolve`]):
//! a member's whole vault, a share's guest only the notes the link lists,
//! so a guest's list never reaches a note outside the share.
//!
//! Key, tempo, meter and anything else music-specific are an extension's
//! (Studio's), never read here. An extension that claims the note's type
//! draws over the core view; [`is_listing_fence`] names the fences such an
//! extension writes as its richer layer of the same list.

pub mod collection;
pub mod front;
pub mod item;
pub mod link;
#[cfg(feature = "ui")]
pub mod ui;

pub use collection::{Collection, Entry, Resolved, Row, Section};
pub use item::Item;

/// The note type a generic collection takes.
pub const PLAYLIST: &str = "playlist";

/// Whether a fence names an extension's richer layer of a collection.
///
/// Its info string (```` ```studio-setlist ```` → `studio-setlist`) is
/// `playlist`, or a name ending `-playlist` or `-setlist`. A reading view
/// that draws the core list folds such a fence away: the list says it
/// already.
#[must_use]
pub fn is_listing_fence(info: &str) -> bool {
    let lang = info
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    lang == PLAYLIST || lang.ends_with("-playlist") || lang.ends_with("-setlist")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_fence_is_a_playlist_or_an_extensions_list() {
        assert!(is_listing_fence("studio-setlist"));
        assert!(is_listing_fence("playlist"));
        assert!(is_listing_fence("Acme-Playlist compact"));
        assert!(!is_listing_fence("setlist-notes"));
        assert!(!is_listing_fence("kf"));
        assert!(!is_listing_fence(""));
    }
}
