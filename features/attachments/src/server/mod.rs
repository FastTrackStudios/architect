//! [`AttachmentsServer`] (feature `server`, native): every org's
//! attachments on a server.
//!
//! Mounted behind the permissions gate on each org's lane, beside the
//! files feature and the documents:
//!
//! ```ignore
//! let attachments = AttachmentsServer::new(&data, blobs.clone(), engine.clone())
//!     .with_docs(docs)                                   // the documents it may read
//!     .with_renditions(files.rendition_engine().cloned()); // previews, shared with files
//! let router = router.with(attachments_service_rpc_service_descriptor(), serve(attachments.clone()));
//! gate.permit(attachments_service_rpc_service_descriptor(), &architect_attachments::PERMITS);
//! ```
//!
//! **Access follows the parent.** Every call is checked against `engine`
//! (the gate's: the org's roles and, through `SpacesEngine`, its spaces)
//! as the parent's resource (`org/<org>/doc/<id>`, the resource
//! `crdt::registry::DocPermissions` checks too, or
//! `org/<org>/<kind>/<id>`): `read` to list and read, `write` to send,
//! attach and detach. And a parent reads only the bytes it points at:
//! for a document what its own `attachments` map lists (in the server's
//! copy, through [`DocSource`]), for a record what the server keeps for
//! it. So knowing a hash reads nothing.
//!
//! **Bytes.** Uploads are staged under `<data>/org/<id>/attachments/` by
//! hash, resume from where they stand, are checked against the hash and
//! land in the blob store the files feature and documents share; bytes
//! the store holds are not sent again. The org's ledger there records
//! what its attachments hold (its quota counts the distinct blobs) and
//! each record's list, so they survive a restart.
//!
//! **Collection.** [`AttachmentsServer::release_unreferenced`] drops from
//! an org's ledger what no document (the live set the app computes with
//! `crdt::blobs::live`) and no record points at any more;
//! [`AttachmentsServer::held`] is what the org's attachments still keep,
//! for `LocalFs::collect`'s live set.

mod acting;
mod ledger;
#[cfg(feature = "http")]
mod web;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use architect::blob::{LocalFs, Sha256};
use architect_files::server::{Renditions, TicketKey};
use architect_orgs::OrgId;
use architect_permissions::{PermissionEngine, Principal};
use futures_util::future::BoxFuture;
use loro::LoroDoc;
use uuid::Uuid;

pub use acting::Acting;
use ledger::{Ledger, OrgLedger};
#[cfg(feature = "http")]
pub use web::WebAttachments;

use crate::model::{AttachFault, AttachUsage};

/// Where the server reads a document's attachments: its copy of the
/// document (`DocRegistry::doc`, a store of the app's), or `None` when it
/// has none.
///
/// ```ignore
/// let registry = registry.clone();
/// let docs = move |_org: &OrgId, id: Uuid| -> BoxFuture<'static, Option<LoroDoc>> {
///     let registry = registry.clone();
///     Box::pin(async move { registry.doc(id).await.ok().map(|d| d.loro().clone()) })
/// };
/// ```
pub trait DocSource: Send + Sync {
    fn doc(&self, org: &OrgId, id: Uuid) -> BoxFuture<'static, Option<LoroDoc>>;
}

impl<F> DocSource for F
where
    F: Fn(&OrgId, Uuid) -> BoxFuture<'static, Option<LoroDoc>> + Send + Sync,
{
    fn doc(&self, org: &OrgId, id: Uuid) -> BoxFuture<'static, Option<LoroDoc>> {
        self(org, id)
    }
}

/// Every org's attachments on a server. Cheap to clone; configure
/// (`with_*`) before mounting.
#[derive(Clone)]
pub struct AttachmentsServer {
    inner: Arc<Inner>,
    engine: Arc<dyn PermissionEngine>,
    docs: Option<Arc<dyn DocSource>>,
    renditions: Option<Renditions>,
    tickets: TicketKey,
    default_org: Option<OrgId>,
}

struct Inner {
    data: PathBuf,
    blobs: LocalFs,
    orgs: Mutex<HashMap<OrgId, Arc<OrgLedger>>>,
}

impl AttachmentsServer {
    /// Attachments kept under `data` (`<data>/org/<id>/attachments/`),
    /// their bytes in `blobs` (the store the files feature and documents
    /// use), each call checked by `engine` (the gate's).
    pub fn new(
        data: impl Into<PathBuf>,
        blobs: LocalFs,
        engine: Arc<dyn PermissionEngine>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                data: data.into(),
                blobs,
                orgs: Mutex::new(HashMap::new()),
            }),
            engine,
            docs: None,
            renditions: None,
            tickets: TicketKey::random(),
            default_org: None,
        }
    }

    /// Read documents' attachments from `docs`. Without it, a document's
    /// attachments can be sent but not listed or read
    /// ([`AttachFault::NoDoc`]).
    #[must_use]
    pub fn with_docs(mut self, docs: impl DocSource + 'static) -> Self {
        self.docs = Some(Arc::new(docs));
        self
    }

    /// Previews by `renditions` (the files server's,
    /// `FilesServer::rendition_engine`, so the same bytes share them):
    /// each upload that lands asks for its renditions.
    #[must_use]
    pub fn with_renditions(mut self, renditions: Option<Renditions>) -> Self {
        self.renditions = renditions;
        self
    }

    /// Sign read tickets with `key` (the files server's, or one of its
    /// own), so servers sharing it read each other's, and this one after
    /// a restart. Without it the key is random.
    #[must_use]
    pub const fn ticket_key(mut self, key: [u8; 32]) -> Self {
        self.tickets = TicketKey::new(key);
        self
    }

    /// The org a call on a lane in no org is in: a server that serves one
    /// org.
    #[must_use]
    pub fn default_org(mut self, org: OrgId) -> Self {
        self.default_org = Some(org);
        self
    }

    /// The service for `who` in `org`, in process.
    #[must_use]
    pub fn acting(&self, org: OrgId, who: Principal) -> Acting {
        Acting::new(self.clone(), org, who)
    }

    fn for_call(&self) -> Result<Acting, AttachFault> {
        let org = architect_orgs::server::current_org()
            .or_else(|| self.default_org.clone())
            .ok_or(AttachFault::NoOrg)?;
        let who = architect::permissions_gate::caller().ok_or(AttachFault::Unauthenticated)?;
        Ok(self.acting(org, who))
    }

    /// Bound the bytes `org`'s attachments hold (`None`: unbounded). The
    /// deployment's call, not a member's.
    ///
    /// # Errors
    ///
    /// The ledger could not be written.
    pub async fn set_quota(
        &self,
        org: &OrgId,
        quota: Option<u64>,
    ) -> Result<AttachUsage, AttachFault> {
        let ledger = self.ledger(org).await?;
        let mut state = ledger.state.lock().await;
        let mut next = state.clone();
        next.quota = quota;
        ledger.save(&next).await?;
        *state = next;
        let usage = state.usage();
        drop(state);
        Ok(usage)
    }

    /// The blobs `org`'s attachments keep: part of the live set a
    /// collection of the shared store spares.
    ///
    /// # Errors
    ///
    /// The ledger could not be read.
    pub async fn held(&self, org: &OrgId) -> Result<HashSet<Sha256>, AttachFault> {
        let ledger = self.ledger(org).await?;
        let state = ledger.state.lock().await;
        Ok(state.held_hashes())
    }

    /// Forget what `org`'s attachments hold that nothing points at any
    /// more: not in `live` (every blob the org's documents point at,
    /// `crdt::blobs::live`), not on any record, and in the ledger since
    /// before `before_us` (so bytes sent ahead of the document that
    /// lists them are kept a while). Returns what was released; a
    /// collection of the store then removes those no one else keeps.
    ///
    /// # Errors
    ///
    /// The ledger could not be written.
    pub async fn release_unreferenced(
        &self,
        org: &OrgId,
        live: &HashSet<Sha256>,
        before_us: i64,
    ) -> Result<Vec<Sha256>, AttachFault> {
        let ledger = self.ledger(org).await?;
        let mut state = ledger.state.lock().await;
        let mut next = state.clone();
        let released = next.release(live, before_us);
        if !released.is_empty() {
            ledger.save(&next).await?;
            *state = next;
        }
        drop(state);
        Ok(released)
    }

    async fn ledger(&self, org: &OrgId) -> Result<Arc<OrgLedger>, AttachFault> {
        if let Some(ledger) = self.loaded(org) {
            return Ok(ledger);
        }
        let dir = self.inner.data.join(org.path("attachments"));
        let state = Ledger::load(&dir).await?;
        let mut orgs = self
            .inner
            .orgs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(Arc::clone(
            orgs.entry(org.clone())
                .or_insert_with(|| Arc::new(OrgLedger::new(dir, state))),
        ))
    }

    fn loaded(&self, org: &OrgId) -> Option<Arc<OrgLedger>> {
        self.inner
            .orgs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(org)
            .cloned()
    }
}

/// Microseconds since the Unix epoch.
pub(crate) fn now_us() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_micros()).ok())
        .unwrap_or_default()
}

pub(crate) fn io(e: impl std::fmt::Display) -> AttachFault {
    AttachFault::Io(e.to_string())
}
