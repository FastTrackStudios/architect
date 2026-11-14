//! Core media primitives (#668): what any extension's audio or video work
//! builds on, with nothing domain-specific in it.
//!
//! - A **media item** ([`Item`]) is a note with a `title`, a `subtitle`
//!   (or `artist`), a `duration`, its media (`media`, or `recording`: an
//!   audio or video file of the vault) and its `art` (or `cover`).
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
//! extension writes as its richer layer of the same list, and
//! [`fence::within`] cuts one to the rows a reader may see.

pub mod collection;
pub mod fence;
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

/// The fences of `text` that [`is_listing_fence`] names.
///
/// Each one's info string and its byte range, from its opening ```` ``` ```` to the
/// end of its closing line (without the newline after it). An unclosed
/// one runs to the end. What an editor folds while the core list draws.
#[must_use]
pub fn listing_fences(text: &str) -> Vec<(String, std::ops::Range<usize>)> {
    let mut out = Vec::new();
    let mut open: Option<(String, usize)> = None;
    let mut at = 0_usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        let end = at.saturating_add(line.trim_end_matches(['\n', '\r']).len());
        match &open {
            Some((info, start)) if trimmed.starts_with("```") => {
                if is_listing_fence(info) {
                    out.push((info.clone(), *start..end));
                }
                open = None;
            }
            Some(_) => {}
            None => {
                if let Some(info) = trimmed.strip_prefix("```") {
                    open = Some((info.trim().to_owned(), at));
                }
            }
        }
        at = at.saturating_add(line.len());
    }
    if let Some((info, start)) = open
        && is_listing_fence(&info)
    {
        out.push((info, start..text.len()));
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn listing_fences_are_found_with_their_ranges() {
        let text = "# Set\n\n```studio-setlist\n1 | A\n```\n\n```kf\n| C |\n```\n```playlist\nx";
        let found = listing_fences(text);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].0, "studio-setlist");
        assert_eq!(
            text.get(found[0].1.clone()).unwrap(),
            "```studio-setlist\n1 | A\n```"
        );
        assert_eq!(text.get(found[1].1.clone()).unwrap(), "```playlist\nx");
    }

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
