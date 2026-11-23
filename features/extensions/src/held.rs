//! A project's files the org's node holds, for extensions.
//!
//! `file_list` and `file_url`, with `reads = ["files"]`: what an engine in
//! the app (Studio's, in a browser) lists and range-reads to stream a
//! member's own project.
//!
//! The app gives the host a [`HeldFilesSource`] ([`Host::set_held_files`])
//! over the node's `architect_files::held` face, signed in as the person:
//! the web app's, with the bearer its org lane holds. The host checks the
//! capability (`crate::hostcall::check`) and hands the call over; the node
//! decides what the person may read, so an extension never reads past
//! them. A host with none (a share link's guest, the demo, a desk app
//! that serves its own files) answers `unsupported`: a guest reads only
//! what its link lists, through the link.
//!
//! Answers are the node's JSON as it is: a listing
//! `{place, folder, files: [{path, size, sha256, url}], expires_us}` and a
//! ticket `{url, expires_us}`, each `url` absolute and read with `Range`,
//! no header needed, until `expires_us` (then ask again).
//!
//! [`Host::set_held_files`]: crate::host::Host::set_held_files

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::hostcall::HostError;

/// What a [`HeldFilesSource`] answers with.
#[cfg(not(target_family = "wasm"))]
pub type HeldFuture = Pin<Box<dyn Future<Output = Result<Value, HostError>> + Send>>;
/// What a [`HeldFilesSource`] answers with (a browser's: not `Send`).
#[cfg(target_family = "wasm")]
pub type HeldFuture = Pin<Box<dyn Future<Output = Result<Value, HostError>>>>;

/// The node's files of a project, as the person reaches them.
pub trait HeldFilesSource: Send + Sync {
    /// Every file under `folder` (all of it without one) of `place`
    /// (`vault:<name>`, `root:<id>`) the person may read.
    fn list(&self, place: String, folder: Option<String>) -> HeldFuture;

    /// A URL that reads the file at `path` of `place`, by range, until
    /// it runs out.
    fn url(&self, place: String, path: String) -> HeldFuture;
}

/// Why a host answers no project's files.
#[must_use]
pub fn unsupported() -> HostError {
    HostError::Unsupported {
        what: "a project's files: this host has none for extensions (no signed-in org here, or a share link's guest, whose files are the link's)".to_owned(),
    }
}
