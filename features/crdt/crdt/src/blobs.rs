//! The blobs documents point at (`docs/content/architecture/documents.md`,
//! "The file": large media are blobs).
//!
//! A document holds its audio and images by reference: their sha256s, as
//! its kind's [`DocKind::blobs`] reads them out of it. The bytes live in a
//! [`LocalFs`] beside the documents and travel separately, by hash:
//!
//! - **On demand.** Opening a document on another device fetches nothing;
//!   a blob is fetched when it is read ([`Fetcher::open`] streams it as it
//!   arrives, so playback starts before the end), from whichever
//!   [`Remote`](architect::blob::Remote)s the [`Fetcher`] has: the peer
//!   the document came from, the org's server.
//! - **Offline.** [`keep_offline`] fetches every blob a document points
//!   at and pins it; [`release`] unpins them.
//! - **Collected.** [`live`] is every blob the device's documents point
//!   at; [`LocalFs::collect`] with it removes the rest, pins aside.
//!
//! Where the bytes sit is the store's business, not the document file's:
//! a file format that carries small blobs inline (`crdt::file`) is one
//! more place to read them from, and its hashes are the same names.

use std::collections::HashSet;

use architect::blob::{BlobError, Fetcher, LocalFs, Sha256};
use loro::LoroDoc;

use crate::kinds::{DocKind, DocKinds, kind_of};

/// The blobs `doc` points at, as `kind` reads them. References that are
/// not 64 hex digits are skipped (a kind's bug, not the store's).
#[must_use]
pub fn referenced(kind: &DocKind, doc: &LoroDoc) -> Vec<Sha256> {
    let mut seen = HashSet::new();
    kind.blobs_of(doc)
        .iter()
        .filter_map(|hex| Sha256::from_hex(hex).ok())
        .filter(|hash| seen.insert(*hash))
        .collect()
}

/// Every blob these documents point at, each read by the kind its
/// `meta` names (documents of a kind `kinds` does not know point at
/// nothing): the live set for [`LocalFs::collect`].
pub fn live<'a>(kinds: &DocKinds, docs: impl IntoIterator<Item = &'a LoroDoc>) -> HashSet<Sha256> {
    docs.into_iter()
        .filter_map(|doc| Some((kinds.get(&kind_of(doc)?)?, doc)))
        .flat_map(|(kind, doc)| referenced(kind, doc))
        .collect()
}

/// The blobs `doc` points at that `store` does not hold yet.
///
/// # Errors
///
/// The store could not be read.
pub async fn missing(
    kind: &DocKind,
    doc: &LoroDoc,
    store: &LocalFs,
) -> Result<Vec<Sha256>, BlobError> {
    let mut out = Vec::new();
    for hash in referenced(kind, doc) {
        if !architect::blob::BlobStore::has(store, hash).await? {
            out.push(hash);
        }
    }
    Ok(out)
}

/// Keep `doc`'s blobs on this device: pin every one, then fetch those
/// not here. Pinned first, so a collection running meanwhile keeps
/// them; the pins stay if a fetch fails (fetch again to finish).
///
/// # Errors
///
/// The first blob no remote has or could send whole.
pub async fn keep_offline(
    kind: &DocKind,
    doc: &LoroDoc,
    fetcher: &Fetcher,
) -> Result<Vec<Sha256>, BlobError> {
    let hashes = referenced(kind, doc);
    for hash in &hashes {
        fetcher.store().pin(*hash).await?;
    }
    for hash in &hashes {
        fetcher.fetch(*hash).await?;
    }
    Ok(hashes)
}

/// Stop keeping `doc`'s blobs offline. They stay while any document
/// points at them; the next collection after that removes them.
///
/// # Errors
///
/// A pin could not be removed.
pub async fn release(kind: &DocKind, doc: &LoroDoc, store: &LocalFs) -> Result<(), BlobError> {
    for hash in referenced(kind, doc) {
        store.unpin(hash).await?;
    }
    Ok(())
}
