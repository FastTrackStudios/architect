//! The node's vaults as documents another node can ask for.
//!
//! Every markdown file of every vault the node holds is one document,
//! `vault_proto::collab_doc_id(vault, path)`, named `<vault>/<path>` on
//! the peer lane (`crdt::peer::HeldDoc::name`) and digested by the file's
//! sha-256, so two nodes whose files agree skip it. A name this node does
//! not hold yet is registered on first ask (`resolve`), so the session
//! that follows makes the file here: `vault_collab` writes it behind.
//!
//! A vault file of a kind an extension declares (`super::kinds`) is
//! listed the same way, `<vault>/<path>`, by the id its own header
//! carries and the file's sha-256 (`vault_collab::VaultCollab::document_of`);
//! one another node names is made here on first open, empty and of its
//! lineage (`VaultCollab::register_document`).

use std::collections::HashMap;
use std::sync::Arc;

use architect_files::sync::{Engine, LedgerDoc, manifest};
use crdt::peer::{DocAt, DocCatalog, HeldDoc};
use crdt::sync::SyncError;
use uuid::Uuid;
use vault_live::Backend;
use vault_proto::{VaultSync as _, collab_doc_id};

/// [`DocCatalog`] over a vault [`Backend`], and the files engine's
/// manifests (`files/<org>/<section>/<name>`) when the node runs one.
#[derive(Clone)]
pub struct VaultCatalog {
    backend: Backend,
    files: Option<Engine>,
    documents: Option<Documents>,
}

/// The vaults' documents of a kind: the reconciler that keeps them, and
/// what keeps its kinds up to the extensions installed.
#[derive(Clone)]
struct Documents {
    collab: vault_collab::VaultCollab,
    kinds: Arc<super::kinds::ExtensionKinds>,
}

impl VaultCatalog {
    #[must_use]
    pub const fn new(backend: Backend) -> Self {
        Self {
            backend,
            files: None,
            documents: None,
        }
    }

    /// The same, listing the vault files of the kinds `kinds` declares as
    /// the documents `collab` keeps.
    #[must_use]
    pub fn with_documents(
        mut self,
        collab: vault_collab::VaultCollab,
        kinds: Arc<super::kinds::ExtensionKinds>,
    ) -> Self {
        self.documents = Some(Documents { collab, kinds });
        self
    }

    /// The same, listing `files`' manifests too.
    #[must_use]
    pub fn with_files(mut self, files: Option<Engine>) -> Self {
        self.files = files;
        self
    }

    /// Every note this node holds, every document of an extension's
    /// kind, and every manifest, as the peer lane lists them.
    #[must_use]
    pub fn notes(&self) -> Vec<HeldDoc> {
        if let Some(documents) = &self.documents {
            documents.kinds.refresh();
        }
        let mut out: Vec<HeldDoc> = self
            .files
            .iter()
            .flat_map(Engine::held)
            .map(|m| HeldDoc {
                doc_id: m.doc_id,
                name: m.name,
                digest: m.digest,
            })
            .collect();
        for vault in self.backend.vault_ids() {
            let Ok(manifest) = self.backend.manifest(&vault) else {
                continue;
            };
            for file in manifest.files {
                let doc_id = if is_note(&file.path) {
                    collab_doc_id(&vault, &file.path)
                } else if let Some(id) = self
                    .documents
                    .as_ref()
                    .and_then(|d| d.collab.document_of(&vault, &file.path))
                {
                    id
                } else {
                    continue;
                };
                out.push(HeldDoc {
                    doc_id,
                    name: name(&vault, &file.path),
                    digest: file.sha256,
                });
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
    /// `at` names no vault this node holds, is neither a note nor a file
    /// of a registered kind, or its id is not the one its name derives to
    /// (a note's) or the one its file carries (a document's).
    pub fn register(&self, at: &DocAt) -> Result<Uuid, SyncError> {
        if let Some(which) = LedgerDoc::parse(&at.name) {
            let files = self.files.as_ref().ok_or(SyncError::UnknownDoc)?;
            if which.doc_id() != at.doc_id {
                return Err(SyncError::UnknownDoc);
            }
            return Ok(files.learn_ledger(which));
        }
        if let Some(place) = manifest::place_of(&at.name) {
            let files = self.files.as_ref().ok_or(SyncError::UnknownDoc)?;
            if manifest::doc_id_for(place) != at.doc_id {
                return Err(SyncError::UnknownDoc);
            }
            files.learn(at.doc_id);
            return Ok(at.doc_id);
        }
        let (vault, path) = at.name.split_once('/').ok_or(SyncError::UnknownDoc)?;
        if !self.backend.vault_ids().iter().any(|v| v == vault) {
            return Err(SyncError::UnknownDoc);
        }
        if !is_note(path) {
            let documents = self.documents.as_ref().ok_or(SyncError::UnknownDoc)?;
            if documents.collab.kind_of(path).is_none() {
                // Perhaps installed since the last look.
                documents.kinds.refresh();
            }
            documents.collab.register_document(vault, path, at.doc_id)?;
            return Ok(at.doc_id);
        }
        if collab_doc_id(vault, path) != at.doc_id {
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

impl VaultCatalog {
    /// `peer` listed `theirs`: hand the engine the roots and ledgers in it,
    /// so it forgets those the peer no longer lets this node open.
    pub fn listed_by(&self, peer: &str, theirs: &[HeldDoc]) {
        let Some(files) = &self.files else { return };
        let ids: Vec<Uuid> = theirs
            .iter()
            .filter(|d| {
                manifest::place_of(&d.name).is_some() || LedgerDoc::parse(&d.name).is_some()
            })
            .map(|d| d.doc_id)
            .collect();
        for id in files.listed_by(peer, &ids) {
            tracing::info!(doc = %id, peer, "files: a root or ledger this peer no longer shares is forgotten");
        }
    }
}

/// A note's name on the peer lane.
#[must_use]
pub fn name(vault: &str, path: &str) -> String {
    format!("{vault}/{path}")
}

fn is_note(path: &str) -> bool {
    vault_proto::note_types::is_note_path(path)
}
