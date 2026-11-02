// `#[architect::rpc]` writes `async fn` into its trait.
#![allow(async_fn_in_trait)]

//! Attachments (`docs/content/architecture/documents.md`, "Attachments"):
//! files attached to a document or a record, on the files feature's
//! store and architect's permissions.
//!
//! An attachment is a blob, named by its sha256, hung off a [`Parent`]:
//! a CRDT document, which lists its attachments itself in a Loro map
//! ([`doc`]) — so attaching works offline and reaches every replica with
//! the document — or a record of the app's, whose list the server keeps.
//! The bytes travel apart, by hash: a device pushes what the server
//! lacks (resumably, and nothing the org already holds), another fetches
//! them when it opens the document. They count against the org's
//! attachments quota once however many parents hold them, previews come
//! from the files feature's renditions, and a blob nothing points at any
//! more is collected.
//!
//! **Access follows the parent**: reading needs `read` on it, attaching
//! `write`, as the org's permission engine (its roles, its spaces) says
//! for the parent's resource — and a parent reads only the bytes it
//! points at.
//!
//! - default (wasm-clean): [`Parent`], [`Attachment`], [`AttachFault`],
//!   the document side ([`doc`]: `attach`, `detach`, `list`, and `refs`
//!   for `DocKind::blobs`), and the byte face's URLs ([`web`]).
//! - `vox`: [`AttachmentsService`], its gate table ([`PERMITS`]) and
//!   [`AttachmentsClient`] (push bytes, read them back).
//! - `blob` (native): a device's side over `architect::blob` —
//!   `attach_local`, `AttachmentsClient::push_doc`, and
//!   `AttachmentsRemote`, the `Remote` a `Fetcher` reads another device's
//!   attachments through.
//! - `server` (native): [`server::AttachmentsServer`].
//! - `http` (native): `server::WebAttachments`, the [`web`] byte face a
//!   browser previews from.

#[cfg(all(feature = "blob", not(target_arch = "wasm32")))]
mod blob;
#[cfg(feature = "vox")]
mod client;
pub mod doc;
mod model;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub mod server;
#[cfg(feature = "vox")]
mod service;
pub mod web;

#[cfg(all(feature = "blob", not(target_arch = "wasm32")))]
pub use blob::{AttachmentsRemote, attach_local};
#[cfg(feature = "vox")]
pub use client::{AttachmentsClient, CHUNK, ClientError, attachment_of, hash_of};
pub use model::{
    AttachFault, AttachUsage, Attachment, Parent, ReadDone, ReadRequest, Upload, check_hash,
};
#[cfg(feature = "vox")]
pub use service::{
    AttachmentsService, AttachmentsServiceClient, PERMITS,
    attachments_service_rpc_service_descriptor, layer, serve,
};
