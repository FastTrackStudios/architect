//! The vault index and what queries read it with, wasm-clean (`docs/content/architecture/extensions.md`,
//! "The vault's query service").
//!
//! - [`index`]: the vault index, a compact entry per note (title,
//!   properties, tags, links, headings, task lines; no body) and per
//!   `.base` file, kept a note at a time by [`index::Builder`] wherever
//!   the vault lives, synced to browsers as a snapshot and deltas.
//! - [`bases`]: the `.base` parser and evaluator; [`obsidian_parse`]: a
//!   note parsed; [`rows`]: a parsed note as a base's row;
//!   [`base_json`]: a base as JSON.

// `bases`, `refs`, `obsidian_parse` and `rows` (`base_query`'s) moved here from `vault-live` and
// `vault-obsidian` as they were, under the lints those crates relax (see
// `vault-live`'s root); new code here meets the house's.
#[allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::assigning_clones,
    clippy::case_sensitive_file_extension_comparisons,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::default_trait_access,
    clippy::derive_partial_eq_without_eq,
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::format_push_string,
    clippy::indexing_slicing,
    clippy::manual_let_else,
    clippy::map_unwrap_or,
    clippy::match_same_arms,
    clippy::missing_const_for_fn,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::needless_continue,
    clippy::needless_pass_by_value,
    clippy::option_if_let_else,
    clippy::redundant_clone,
    clippy::significant_drop_tightening,
    clippy::similar_names,
    clippy::single_match_else,
    clippy::string_slice,
    clippy::too_long_first_doc_paragraph,
    clippy::unnecessary_wraps,
    clippy::unsafe_derive_deserialize,
    clippy::unwrap_used,
    clippy::use_self
)]
mod moved {
    pub mod bases;
    pub mod obsidian_parse;
    pub mod refs;
    pub mod rows;
}
pub use moved::{bases, obsidian_parse, refs, rows};

pub mod base_json;
pub mod index;

pub use index::{Builder, Built, Entry, IndexDelta, Note, NoteSource, Text, VaultIndex};
