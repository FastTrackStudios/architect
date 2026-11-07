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

mod agent;
pub mod assets;
pub mod dav;
pub mod engine;
pub mod lane;
pub mod layout;
pub mod manifest;
pub mod scan;
pub mod store;

pub use engine::{Docs, Engine, EngineConfig, EngineError, KEEP_INLINE, RootStatus};
pub use lane::{BLOB_PERMITS, HeldManifest};
pub use manifest::{Checkpoint, FileVersion, Manifest, RootMeta, Section, Settled, Tree};
pub use store::TreeStore;
