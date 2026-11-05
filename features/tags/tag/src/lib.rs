//! Tag feature backend.
//!
//! The per-org tag registry — name → icon/color decorations — persisted
//! as a single `Records/tags.json` file in the vault and served over the
//! [`tag_proto::TagService`] vox RPC. Native-only: the round-trip touches
//! disk. The web UI binds `tag-proto` directly on wasm. Mirrors the
//! `inbox` crate (minus per-item files — the registry is small, so one
//! JSON document is simpler than a file per tag).

// Moved from Task as it was (#333), under Task's lints. Architect's
// house lints (no panics, no unchecked arithmetic, indexing or casts,
// pedantic) are relaxed here by name until the crate is brought up to
// them; new code in it meets them.
#![allow(clippy::must_use_candidate, clippy::too_long_first_doc_paragraph)]

mod vault_tags;

#[cfg(feature = "actions")]
pub mod actions;

pub use vault_tags::{VaultTags, VaultTagsError};
