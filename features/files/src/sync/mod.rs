//! Synced roots: an org's projects and asset libraries.
//!
//! Held in place by the headless node, pinned per machine, browsed on demand (feature
//! `sync`, native). The spec: `docs/content/architecture/project-files.md`.
//!
//! - [`manifest`]: a root's versions and checkpoints in one Loro document,
//!   which travels the peer lanes like any note (#412).
//! - [`layout`]: where roots are on disk, their ids, what is never synced.
//! - [`scan`]: a working copy's index, and the settled changes in it.
//! - [`store`]: the bytes this device can serve, its working files too.
//! - [`engine`]: each pass, scan then apply, forks, checkpoints, pins.
//! - [`dav`] and `s3` (feature `s3`): the roots over `WebDAV` and over S3.
//! - [`registry`]: every live file of every root, searched offline.
//! - [`ledger`]: each holder's signed attestations, and policy.
//! - [`verify`]: Merkle roots and possession proofs.

mod agent;
pub mod assets;
#[cfg(feature = "s3")]
mod bucket;
pub mod dav;
pub mod engine;
pub mod lane;
pub mod layout;
pub mod ledger;
pub mod manifest;
pub mod registry;
mod replicas;
#[cfg(feature = "s3")]
pub mod s3;
pub mod scan;
pub mod store;
pub mod verify;

#[cfg(feature = "s3")]
pub use bucket::BucketHolder;
pub use engine::{Docs, Engine, EngineConfig, EngineError, KEEP_INLINE, RootStatus, VERIFY_WINDOW};
pub use lane::{BLOB_PERMITS, HeldManifest, POSSESSION_PERMITS};
pub use ledger::{Identity, LedgerDoc};
pub use manifest::{Checkpoint, FileVersion, Manifest, RootMeta, Section, Settled, Tree};
pub use replicas::PossessionHost;
pub use store::TreeStore;
pub use verify::{
    Possession, PossessionClient, PossessionError, Proof, ProveError, Prover, VoxProver,
};
