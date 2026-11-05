// architect's rpc macro emits cfg-gated blocks; allow at crate scope.
#![allow(unexpected_cfgs)]

//! Wasm-clean wire contract for the generalized **typed-link** primitive.
//!
//! A link is a *first-class object* between any two [`NodeRef`]s — a
//! verse, a note, a wiki page, a topic, an entity — carrying its own
//! [`Relation`], [`Confidence`], [`Visibility`], and [`Provenance`]. This
//! is the model the whole knowledge graph hangs on: verses↔verses,
//! notes↔verses, ideas↔wiki entries. Confidence + visibility are what
//! enable quality filtering and private-vs-public publishing.
//!
//! The sibling `links` crate owns the backing store. See the
//! knowledge-primitives design.

// Moved from Task as it was (#333), under Task's lints. Architect's
// house lints (no panics, no unchecked arithmetic, indexing or casts,
// pedantic) are relaxed here by name until the crate is brought up to
// them; new code in it meets them.
#![allow(
    clippy::derive_partial_eq_without_eq,
    clippy::doc_markdown,
    clippy::missing_const_for_fn,
    clippy::redundant_clone,
    clippy::too_long_first_doc_paragraph,
    clippy::unsafe_derive_deserialize
)]

pub mod link;
pub mod node;
pub mod service;

pub use link::{Confidence, Provenance, Relation, TypedLink, Visibility};
pub use node::{Anchor, NodeKind, NodeRef, format_timecode, parse_timecode};
pub use service::{LinksError, LinksService, Reach, ResolvedNode};

#[cfg(feature = "vox")]
pub use service::{
    LinksServiceClient, LinksServiceRpcDispatcher as LinksDispatcher,
    Service as LinksServiceBridge, layer as links_service_layer,
    links_service_rpc_service_descriptor as links_service_descriptor, serve as serve_links_service,
};
