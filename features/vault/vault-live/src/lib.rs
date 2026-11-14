//! Filesystem-backed vault.
//!
//! Native-only crate. `.md` files on disk are authoritative; this
//! crate provides:
//!
//! - [`Vault`]: an in-memory snapshot of a vault root, rebuildable
//!   from disk in one pass.
//! - [`BlockIndex`]: a `uuid → (page, byte offset)` map covering
//!   every `id:: <uuid>` block-id property line across the vault.
//!   Enables cross-page block refs (`((uuid))`) and embeds.
//! - [`watch`]: a debounced filesystem watcher that emits change
//!   events the editor can use to refresh state.
//!
//! Architectural shape mirrors [`vault-obsidian`]: small
//! focused modules, native-only `cfg`, vault snapshot +
//! walker + watcher + a thin mutate layer for create /
//! append / save. The difference is block-awareness:
//! every vault page is parsed for its `id:: <uuid>` lines,
//! and the [`BlockIndex`] gives O(1) cross-vault lookup of
//! a block's containing page and offset.
//!
//! File content is canonical: there is no separate block table beside
//! the markdown, the block index points back into the file, pages can
//! live anywhere under the root (Obsidian's free-form folders), and
//! block properties (`id:: <uuid>`, `key:: value`) are the file itself.
//! Raw file bytes are the document; rewrites touch only the byte ranges
//! they identified.

#![cfg(not(target_arch = "wasm32"))]
// Format-agnostic file parsers, owned here with the files they read:
//   bases           — `.base` parser + evaluator (filters, formulas,
//                      link functions, link-follow)
//   refs            — wikilink / blockref / tag value types
//   property_schema — frontmatter type coercion (Obsidian's
//                      `.obsidian/types.json` shape)
//   lexorank        — fractional-index sort keys

// Moved from Task as it was (#333), under Task's lints. Architect's
// house lints (no panics, no unchecked arithmetic, indexing or casts,
// pedantic) are relaxed here by name until the crate is brought up to
// them; new code in it meets them.
#![allow(
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
pub mod bases;
pub mod blocks;
pub mod cook;
/// Grouping rows by a value, `vault_proto::grouping` (wasm-clean, so the
/// navigator's model can use it on every target).
pub use vault_proto::grouping;
pub mod lexorank;
pub mod lookup;
pub mod mutate;
pub mod own;
pub mod property_schema;
pub mod refs;
/// Rewriting the wikilinks that named a page after it is renamed.
pub mod relink;
/// `VaultSync` backend — canonical filesystem impl of the
/// [`vault_proto::VaultSync`] wire trait. Consumers use
/// `vault::sync::Backend` (also re-exported as
/// [`Backend`]).
pub mod sync;
pub mod write_guard;

/// Where one registered root stops and a nested one begins — the
/// prune that lets a shelf hold a shelf.
pub mod shelf_boundary;
/// The cached [`Vault`] snapshot per vault root, re-read only where the
/// disk changed (zero-copy audit V12).
pub mod snapshot;
pub mod vault;
pub mod walker;
pub mod watcher;

pub use blocks::{BlockIndex, BlockLocation};
pub use lookup::VaultLookupView;
pub use mutate::{
    MutateError, PageSink, append_to_page, bind_sink, create_page, delete_page, delete_page_at,
    move_page_at, save_page, save_page_at, sink_for, unbind_sink, write_atomic,
};
pub use sync::Backend;
pub use vault::{LoadError, SaveError, Vault, VaultPage};
pub use vault::{PropertyTypes, VaultBase};
pub use walker::{VaultEntry, VaultEntryKind, walk_vault};
pub use watcher::{FsEvent, VaultEvent, WatchError, watch, watch_any};
pub use write_guard::WriteGuard;

/// Node-private directories (`vault.private.node-only`): a directory
/// holding [`private::PRIVATE_MARKER`] stays on this machine — out of
/// the manifest, the page walk and every replica — and the names of a
/// file root's own internals, which replication skips. Defined in
/// [`vault_proto::consts`] (wasm-clean) so every crate that walks a
/// vault agrees on the same names.
pub mod private {
    pub use vault_proto::consts::{
        MARKER_FILE, PRIVATE_MARKER, STORE_DIR, is_private_dir, is_within_private,
    };
}
