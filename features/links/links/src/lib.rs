// architect's rpc/HasDispatcher derives emit cfg-gated blocks.
#![allow(unexpected_cfgs)]

//! `links` — backing store for the generalized typed-link primitive.
//!
//! Sits on top of the wasm-clean [`links_proto`] contract (same split as
//! `locations` over `locations-proto`). [`Store`] is a JSONL-file-backed
//! [`links_proto::LinksService`] impl.

#![cfg(not(target_arch = "wasm32"))]
// Moved from Task as it was (#333), under Task's lints. Architect's
// house lints (no panics, no unchecked arithmetic, indexing or casts,
// pedantic) are relaxed here by name until the crate is brought up to
// them; new code in it meets them.
#![allow(
    clippy::expect_used,
    clippy::significant_drop_tightening,
    clippy::too_long_first_doc_paragraph
)]

pub mod store;

#[cfg(feature = "actions")]
pub mod actions;

pub use links_proto::{
    Confidence, LinksError, LinksService, NodeKind, NodeRef, Provenance, Reach, Relation,
    ResolvedNode, TypedLink, Visibility,
};
pub use store::{NoFederation, NodeHomes, Store};

// architect-emitted vox bits, re-exported for the server mount.
#[cfg(feature = "vox")]
pub use links_proto::{links_service_descriptor, serve_links_service};
