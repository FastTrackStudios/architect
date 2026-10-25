//! [`DriverHost`]: a device's own docs, served to the owner's other
//! devices.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use architect::PubSub;
use architect_permissions::Principal;
use uuid::Uuid;

use super::driver::{DocsDriver, LocalDoc, peer_view};
use super::{
    CreateDoc, DocChange, DocMeta, DocPage, DocQuery, Docs, DocsError, DocsStreamSource, Lineage,
    check_create, matches, now_us,
};
use crate::sync::{DocSync, SyncDown, SyncError};

type Admit = dyn Fn(Option<&Principal>) -> bool + Send + Sync;

/// The device's own lane, for the owner's other devices.
///
/// [`Docs`], its `changes` stream and [`DocSync`] over the driver's
/// index and open docs, for the owner's other devices to run
/// [`DocsDriver::run_peer`] against (mount it on the device's iroh
/// router, behind a gate whose resolver yields the peer's
/// `Principal::Host`).
///
/// There is no second writer: a peer's edits land in the same open doc
/// the app edits (through a [`DocSyncHost`](crate::sync::DocSyncHost) over
/// it) and are relayed to the driver's other lanes, the hub included.
/// The catalog's `rev` here is the index's own `seq`. A doc a peer
/// registers is held as pending, for the hub session to register; rename
/// and delete go through the hub.
#[derive(Clone)]
pub struct DriverHost {
    driver: DocsDriver,
    admit: Arc<Admit>,
}

impl DocsDriver {
    /// This device's docs as a host for its peers. `admit` decides, on
    /// every call, whether the caller may use it (the principal the
    /// router's permissions gate resolved; `None` with no gate): a
    /// device of the same owner, say, by its endpoint id. It is asked
    /// per call, so a device taken off the account is refused at once.
    pub async fn host<F>(&self, admit: F) -> Result<DriverHost, DocsError>
    where
        F: Fn(Option<&Principal>) -> bool + Send + Sync + 'static,
    {
        let head = self
            .inner
            .index
            .docs()
            .await?
            .iter()
            .map(|d| d.seq)
            .max()
            .unwrap_or(0);
        self.inner.head_seq.fetch_max(head, Ordering::SeqCst);
        Ok(DriverHost {
            driver: self.clone(),
            admit: Arc::new(admit),
        })
    }
}

impl DriverHost {
    fn admitted(&self) -> bool {
        let who = architect::permissions_gate::caller();
        (self.admit)(who.as_ref())
    }

    fn check(&self) -> Result<(), DocsError> {
        if self.admitted() {
            Ok(())
        } else {
            tracing::debug!("docs: a peer the device does not admit");
            Err(DocsError::Forbidden)
        }
    }

    fn head(&self) -> u64 {
        self.driver.inner.head_seq.load(Ordering::SeqCst)
    }
}

/// Who registered a doc here, from the caller.
fn creator(who: Option<&Principal>) -> (String, Option<String>) {
    match who {
        Some(Principal::Host { endpoint }) => (format!("host:{endpoint}"), Some(endpoint.clone())),
        Some(Principal::User { user_id }) => (user_id.clone(), None),
        Some(other) => (other.describe(), None),
        None => (String::new(), None),
    }
}

impl Docs for DriverHost {
    async fn create(&self, req: CreateDoc) -> Result<DocMeta, DocsError> {
        self.check()?;
        check_create(&req)?;
        if let Some(held) = self.driver.inner.index.get(req.doc_id).await? {
            if held.meta.kind != req.kind {
                return Err(DocsError::KindMismatch(held.meta.kind));
            }
            if held.meta.lineage != req.lineage {
                return Err(DocsError::LineageMismatch(held.meta.lineage));
            }
            if held.meta.deleted {
                return Err(DocsError::Deleted);
            }
            return Ok(peer_view(&held));
        }
        let who = architect::permissions_gate::caller();
        let (created_by, origin) = creator(who.as_ref());
        let now = now_us();
        let meta = DocMeta {
            doc_id: req.doc_id,
            kind: req.kind,
            name: req.name,
            parent: req.parent,
            created_by,
            origin,
            created_us: now,
            updated_us: now,
            rev: 0,
            deleted: false,
            lineage: req.lineage,
        };
        let stored = self
            .driver
            .put(
                LocalDoc {
                    meta: meta.clone(),
                    pending: true,
                    seq: 0,
                },
                DocChange::Created(meta),
            )
            .await?;
        // The hub session registers it.
        self.driver.kick_changed();
        Ok(peer_view(&stored))
    }

    async fn get(&self, doc_id: Uuid) -> Result<DocMeta, DocsError> {
        self.check()?;
        self.driver
            .inner
            .index
            .get(doc_id)
            .await?
            .map(|d| peer_view(&d))
            .ok_or(DocsError::NotFound)
    }

    async fn list(&self, query: DocQuery) -> Result<DocPage, DocsError> {
        self.check()?;
        let head = self.head();
        let mut rows: Vec<LocalDoc> = self
            .driver
            .inner
            .index
            .docs()
            .await?
            .into_iter()
            .filter(|d| d.seq > query.since_rev && d.seq <= head && matches(&query, &d.meta))
            .collect();
        rows.sort_by_key(|d| d.seq);
        let limit = usize::try_from(query.limit).unwrap_or(usize::MAX);
        if query.limit > 0 && rows.len() > limit {
            rows.truncate(limit);
            let cursor = rows.last().map_or(query.since_rev, |d| d.seq);
            return Ok(DocPage {
                docs: rows.iter().map(peer_view).collect(),
                cursor,
                more: true,
            });
        }
        Ok(DocPage {
            docs: rows.iter().map(peer_view).collect(),
            cursor: head.max(query.since_rev),
            more: false,
        })
    }

    async fn rename(&self, _: Uuid, _: String, _: Option<u64>) -> Result<DocMeta, DocsError> {
        self.check()?;
        Err(DocsError::Invalid("rename a doc through its hub".into()))
    }

    async fn delete(&self, _: Uuid) -> Result<DocMeta, DocsError> {
        self.check()?;
        Err(DocsError::Invalid("delete a doc through its hub".into()))
    }
}

impl DocsStreamSource for DriverHost {
    fn changes_hub(&self) -> &PubSub<DocChange> {
        &self.driver.inner.host_hub
    }

    fn changes_intro(&self) -> Option<DocChange> {
        Some(DocChange::Attached {
            head_rev: self.head(),
        })
    }
}

impl DocSync for DriverHost {
    async fn sync(
        &self,
        doc_id: Uuid,
        from: Vec<u8>,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<SyncDown>,
    ) -> Result<(), SyncError> {
        if !self.admitted() {
            return Err(SyncError::UnknownDoc);
        }
        let held = self
            .driver
            .inner
            .index
            .get(doc_id)
            .await
            .map_err(|e| SyncError::Internal(e.to_string()))?
            .filter(|d| !d.meta.deleted)
            .ok_or(SyncError::UnknownDoc)?;
        let slot = self
            .driver
            .slot(&held.meta)
            .await
            .map_err(|e| SyncError::Internal(e.to_string()))?;
        if held.meta.lineage == Lineage::Strict && !from.is_empty() {
            if let Ok(client) = loro::VersionVector::decode(&from) {
                if crate::peer::is_foreign_lineage(&client, &slot.doc.loro().oplog_vv()) {
                    return Err(SyncError::BadVersion(format!(
                        "{}: this copy of {doc_id} is not a version of this device's",
                        crate::peer::FOREIGN_LINEAGE
                    )));
                }
            }
        }
        slot.host(doc_id).sync(doc_id, from, up, down).await
    }
}
