//! Filesystem-backed Obsidian vault.
//!
//! Native-only. **Disk is the source of truth at runtime.** Reads
//! produce plain in-memory snapshots (`Vault`, `VaultPage`,
//! `VaultBase`). Writes go straight back to the same `.md` /
//! `.base` files. No Loro / no CRDT in this layer — that's a
//! separate opt-in bridge crate when block-level collaborative
//! editing is wanted.
//!
//! Entry points:
//! - [`Vault::open`] — one-shot scan a directory into memory.
//! - [`Vault::save_page`] — write a single page back to disk.
//! - [`watch`] — start a debounced FS watcher that pushes change
//!   events into a channel.

#![cfg(not(target_arch = "wasm32"))]
// Moved from Task as it was (#333), under Task's lints. Architect's
// house lints (no panics, no unchecked arithmetic, indexing or casts,
// pedantic) are relaxed here by name until the crate is brought up to
// them; new code in it meets them.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::case_sensitive_file_extension_comparisons,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::derive_partial_eq_without_eq,
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::items_after_statements,
    clippy::manual_let_else,
    clippy::map_unwrap_or,
    clippy::missing_const_for_fn,
    clippy::missing_panics_doc,
    clippy::needless_collect,
    clippy::needless_pass_by_value,
    clippy::option_if_let_else,
    clippy::redundant_clone,
    clippy::string_slice,
    clippy::too_long_first_doc_paragraph,
    clippy::unwrap_used
)]

pub mod config;
pub mod guard;
pub mod links;
// Migrated out of the deleted `knowledge-proto` crate. Holds
// the obsidian-flavored markdown parser (frontmatter,
// wikilink / blockref / tag extraction, block-id stripping,
// callout / list-item / heading parsers, code-fence handling).
// The entity-coupled fns (resolve_linkpath, serialize_page,
// translate_logseq_block_refs, format_link) were stripped
// during migration — they depended on the deleted
// `knowledge_proto::{Page, Block, Vault}` entity types. Add
// them back against `vault_obsidian::Vault` if needed.
pub mod obsidian_parse;
pub mod vault;
pub mod walker;
pub mod watcher;

// `VaultGraph` RPC backend over `LinkIndex` — the wire face of
// the link queries below, mounted next to the sync backend.
pub mod graph;

pub use config::{ObsidianConfig, read_obsidian_config};
pub use graph::GraphBackend;
pub use guard::SelfWriteGuard;
pub use links::{LinkIndex, OutgoingLink, UnresolvedLink};
pub use vault::{LoadError, SaveError, Vault, VaultBase, VaultEntryRef, VaultPage};
pub use walker::{VaultEntry, VaultEntryKind, walk_vault};
pub use watcher::{VaultEvent, WatchError, watch};

pub mod aliases;
pub mod outline;
pub mod properties;
pub use aliases::{AliasEntry, list_aliases, total_aliases};
pub use outline::{Heading, PageOutline, outline};
pub use properties::{list_property_keys, property_values, read_property};

pub mod tasks;
pub mod wordcount;
pub use tasks::{TaskItem, list_tasks, page_tasks};
pub use wordcount::{WordCount, page_wordcount, vault_wordcount};

pub mod base_query;
pub use base_query::{
    QueryError, find_base, query_all_views, query_view, rows_from_markdown, run_base, vault_to_rows,
};

pub mod mutate;
pub use mutate::{
    MutateError, append_to_page, create_page, delete_page, edit_frontmatter, move_page,
    prepend_to_page, remove_property, set_property,
};

/// "Open an Obsidian vault as a vault" translation tool —
/// produces a `vault::Vault` snapshot or a single-vault
/// `vault::Backend` from an Obsidian-shaped directory, honoring
/// the skip set + `userIgnoreFilters`.
pub mod translate;
pub use translate::{open_as_backend, open_as_vault};
