//! The node's vaults as documents another node can ask for.
//!
//! Every markdown file of every vault the node holds is one document,
//! `vault_proto::collab_doc_id(vault, path)`, named `<vault>/<path>` on
//! the peer lane (`crdt::peer::HeldDoc::name`) and digested by the file's
//! sha-256, so two nodes whose files agree skip it. A name this node does
//! not hold yet is registered on first ask (`resolve`), so the session
//! that follows makes the file here: `vault_collab` writes it behind.

use std::collections::HashMap;

use crdt::peer::{DocAt, DocCatalog, HeldDoc};
use crdt::sync::SyncError;
use uuid::Uuid;
use vault_live::Backend;
use vault_proto::{VaultSync as _, collab_doc_id};

/// [`DocCatalog`] over a vault [`Backend`].
#[derive(Clone)]
pub struct VaultCatalog {
    backend: Backend,
}

impl VaultCatalog {
    #[must_use]
    pub const fn new(backend: Backend) -> Self {
        Self { backend }
    }

    /// Every note this node holds, as the peer lane lists it.
    #[must_use]
    pub fn notes(&self) -> Vec<HeldDoc> {
        let mut out = Vec::new();
        for vault in self.backend.vault_ids() {
            let Ok(manifest) = self.backend.manifest(&vault) else {
                continue;
            };
            for file in manifest.files {
                if is_note(&file.path) {
                    out.push(HeldDoc {
                        doc_id: collab_doc_id(&vault, &file.path),
                        name: name(&vault, &file.path),
                        digest: file.sha256,
                    });
                }
            }
        }
        out
    }

    /// What [`crdt::peer::pull`] compares a peer's list against.
    #[must_use]
    pub fn digests(&self) -> HashMap<Uuid, String> {
        self.notes()
            .into_iter()
            .map(|d| (d.doc_id, d.digest))
            .collect()
    }

    /// Register `at` with the vault, so this node's registry opens it.
    ///
    /// # Errors
    ///
    /// `at` names no vault this node holds, is not a note, or its id is
    /// not the one its name derives to.
    pub fn register(&self, at: &DocAt) -> Result<Uuid, SyncError> {
        let (vault, path) = at.name.split_once('/').ok_or(SyncError::UnknownDoc)?;
        if !is_note(path) || collab_doc_id(vault, path) != at.doc_id {
            return Err(SyncError::UnknownDoc);
        }
        if !self.backend.vault_ids().iter().any(|v| v == vault) {
            return Err(SyncError::UnknownDoc);
        }
        self.backend
            .register_collab(vault, path)
            .map_err(|e| SyncError::Internal(e.to_string()))
    }
}

impl DocCatalog for VaultCatalog {
    async fn held(&self) -> Result<Vec<HeldDoc>, SyncError> {
        Ok(self.notes())
    }

    async fn resolve(&self, at: &DocAt) -> Result<Uuid, SyncError> {
        self.register(at)
    }
}

/// A note's name on the peer lane.
#[must_use]
pub fn name(vault: &str, path: &str) -> String {
    format!("{vault}/{path}")
}

fn is_note(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}
