// `#[architect::rpc]` writes `async fn` into its trait.
#![allow(async_fn_in_trait)]

//! Files (`docs/content/architecture/documents.md`, "Files"): the file
//! layer every app needs, on architect's tenancy and permissions.
//!
//! An org keeps **roots** — trees of files and folders an app finds by a
//! name (`signal/impulse-responses`, `session/<song>`). Every save of a
//! file is a **version**; a delete is one too, so history replaces trash.
//! A person names the versions worth keeping (**labels**). Bytes live in
//! the content-addressed blob store documents use, so the same bytes are
//! stored once however many files hold them, and an upload of content the
//! org already has sends nothing.
//!
//! **Who may do what** is the org's: a member's [`Role`] there gives them
//! capabilities everywhere in its files ([`role_capabilities`]), a
//! [`Grant`] adds capabilities over one folder for one person, and
//! outside what a person holds a path is absent ([`resolve`]). An org's
//! **quota** bounds the bytes its live files hold.
//!
//! - default (wasm-clean): the ids, [`RootPath`], the model, the
//!   [`FilesFault`], capabilities and their resolution.
//! - `vox`: [`FilesService`] (and its events stream), its gate tables
//!   ([`PERMITS`], [`STREAM_PERMITS`]) and [`FilesClient`], the store an
//!   app talks to — save safely, read back whole or by range, follow a
//!   pinned reference, hear what changed.
//! - `server` (native): [`server::FilesServer`], every org's files kept
//!   under [`OrgId::path`](architect_orgs::OrgId::path), and its
//!   renditions, made off the request path by a pluggable
//!   [`server::Transcoder`].
//! - `http` (native): `server::WebFiles`, the [`web`] byte face — ranged
//!   reads a browser's `<video>` seeks against, renditions, resumable
//!   `PUT` uploads — on the same access rules.
//! - `ffmpeg` (native): `server::FfmpegTranscoder`, renditions by the
//!   `ffmpeg` and `ffprobe` on `PATH`.
//!
//! Every version of a media file gets its renditions ([`rendition`]): a
//! video a streaming proxy, a poster and peaks, an audio file peaks and
//! an AAC transcode. They are keyed by the version's content and kept on
//! disk beside the blobs, so they survive a restart, and one found
//! missing is made again.
//!
//! [`Role`]: architect_orgs::Role

pub mod access;
#[cfg(feature = "vox")]
mod client;
mod error;
mod id;
mod model;
mod path;
pub mod rendition;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub mod server;
#[cfg(feature = "vox")]
mod service;
pub mod web;

pub use access::{Access, Capability, Effective, Grant, resolve, role_capabilities, stewards};
#[cfg(feature = "vox")]
pub use client::{ClientError, FRAME, FilesClient, Resolved, Save, content_of};
pub use error::FilesFault;
pub use id::{ContentId, GrantId, LabelId, RootId, UploadId, VersionId};
pub use model::{
    ByteRange, Entry, EntryKind, Expect, FilesEvent, OnConflict, Outcome, ReadDone, ReadRequest,
    Received, Relocation, Root, UploadFrame, UploadPlan, UploadProgress, UploadSpec, Usage,
    Version, VersionLabel, WriteReceipt,
};
pub use path::{PathError, RootPath};
pub use rendition::{
    MediaClass, PEAKS_PER_SECOND, PeakReducer, Peaks, RECIPE, RenditionKind, RenditionSet,
    RenditionState, RenditionStatus, Ticket,
};
#[cfg(feature = "vox")]
pub use service::{
    FilesService, FilesServiceClient, FilesServiceStream, FilesServiceStreamClient,
    FilesServiceStreamSource, PERMITS, STREAM_PERMITS, files_service_rpc_service_descriptor,
    files_service_stream_service_descriptor, layer, serve, stream_layer, stream_serve,
};
