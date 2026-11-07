//! The engine on a node's peer lane: its bytes as `BlobTransfer`, and its
//! manifests as the documents the node's catalogue lists.

use std::sync::Arc;

use architect::blob::{BlobTransferHost, Sha256};
use architect_permissions::{MethodPermit, ServicePermits};
use uuid::Uuid;

use super::engine::Engine;
use super::manifest::{Manifest, lane_name};

/// `BlobTransfer` for the node's gate: an admitted device reads bytes.
pub const BLOB_PERMITS: ServicePermits = ServicePermits {
    service: "BlobTransfer",
    methods: &[
        MethodPermit::new("size", "read", "files/**"),
        MethodPermit::new("read", "read", "files/**"),
    ],
};

/// A manifest as the peer lane lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldManifest {
    pub doc_id: Uuid,
    /// `files/<org>/<section>/<name>`.
    pub name: String,
    /// Changes when the document does: two nodes that agree skip it.
    pub digest: String,
}

impl Engine {
    /// `BlobTransfer` over this device's bytes, sending only what some
    /// manifest it holds names (a hash is not a key to anything else).
    #[must_use]
    pub fn blob_host(&self) -> BlobTransferHost {
        let served = self.served.clone();
        BlobTransferHost::new(Arc::new(self.store.clone())).allow(move |hash: Sha256| {
            served
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains(&hash)
        })
    }

    /// Every hash some manifest this device holds names, as of the last
    /// pass: what a blob backend replicates.
    #[must_use]
    pub fn named(&self) -> Vec<Sha256> {
        self.served
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .copied()
            .collect()
    }

    /// The manifests this device holds, as of the last pass.
    #[must_use]
    pub fn held(&self) -> Vec<HeldManifest> {
        self.held
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Refresh [`Self::held`] and what [`Self::blob_host`] serves.
    pub(super) async fn refresh_lane(&self) {
        let mut held = Vec::new();
        let mut served = std::collections::HashSet::new();
        for doc_id in self.known() {
            let Some(doc) = self.doc(doc_id).await else {
                continue;
            };
            let manifest = Manifest::new(&doc);
            let Some(meta) = manifest.meta() else {
                continue;
            };
            for v in manifest.versions() {
                if let Some(hash) = v.content.as_deref().and_then(|c| Sha256::from_hex(c).ok()) {
                    served.insert(hash);
                }
            }
            held.push(HeldManifest {
                doc_id,
                name: lane_name(&meta.place()),
                digest: super::scan::hex(&sha2_of(&doc.oplog_vv().encode())),
            });
        }
        *self
            .held
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = held;
        *self
            .served
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = served;
    }
}

fn sha2_of(bytes: &[u8]) -> Vec<u8> {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes).to_vec()
}
