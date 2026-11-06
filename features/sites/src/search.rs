//! The search index: every page's title and its text by heading, in
//! reading order, written once as JSON under a fingerprinted name and
//! fetched the first time a reader searches.
//!
//! The scoring lives with the reader's script (`assets/site.js`) and is
//! Task's guide search, kept: a title match beats a heading match beats a
//! text match, and reading order breaks ties, so the page that introduces
//! a word comes first.

use serde::Serialize;

use crate::site::Site;

/// Most text kept per page section, in characters: enough to match and
/// quote, without shipping the docs twice.
const SECTION_TEXT: usize = 1_500;

/// One page in the index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchEntry {
    /// Title.
    pub t: String,
    /// URL.
    pub u: String,
    /// Description.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub d: String,
    /// Sections: `[id, heading, text]`, the first with an empty id.
    pub s: Vec<(String, String, String)>,
}

/// The index of `site`.
#[must_use]
pub fn entries(site: &Site) -> Vec<SearchEntry> {
    site.pages
        .iter()
        .map(|page| SearchEntry {
            t: page.title.clone(),
            u: page.url.clone(),
            d: page.description.clone().unwrap_or_default(),
            s: page
                .sections
                .iter()
                .map(|s| {
                    (
                        s.id.clone().unwrap_or_default(),
                        s.heading.clone(),
                        s.text.chars().take(SECTION_TEXT).collect(),
                    )
                })
                .collect(),
        })
        .collect()
}
