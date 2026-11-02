//! The service, for one principal in one org: what every call does.

use std::sync::Arc;

use architect::blob::{BlobError, BlobStore as _, Reader, Sha256};
use architect::vox::Tx;
use architect_files::server::{TICKET_TTL_US, ticket_subject};
use architect_files::{ContentId, RenditionKind, RenditionSet, Ticket};
use architect_orgs::OrgId;
use architect_permissions::{Action, Decision, Principal, Resource};
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _, AsyncWriteExt as _};

use super::ledger::OrgLedger;
use super::{AttachmentsServer, io, now_us};
use crate::client::CHUNK;
use crate::doc;
use crate::model::{
    AttachFault, AttachUsage, Attachment, Parent, ReadDone, ReadRequest, Upload, check_hash,
};
use crate::service::AttachmentsService;

/// Why a server with no renditions has none.
const NO_RENDITIONS: &str = "this server makes no renditions";

/// What a ticket for a parent's attachments says.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub(super) struct Claims {
    pub org: String,
    /// The parent's text.
    pub parent: String,
    /// `user:<id>` or `service:<name>`.
    pub who: String,
    pub expires_us: i64,
}

/// Tickets of this feature: apart from the files feature's, under the
/// same kind of key.
pub(super) const TICKET_PREFIX: &str = "fta1.";

/// [`AttachmentsService`] for one principal in one org
/// ([`AttachmentsServer::acting`]).
#[derive(Clone)]
pub struct Acting {
    server: AttachmentsServer,
    org: OrgId,
    who: Principal,
}

impl Acting {
    pub(super) const fn new(server: AttachmentsServer, org: OrgId, who: Principal) -> Self {
        Self { server, org, who }
    }

    /// Whether the caller may `action` the parent, by the org's engine.
    fn need(&self, parent: &Parent, action: &str) -> Result<(), AttachFault> {
        parent.validate()?;
        if matches!(self.who, Principal::Anonymous) {
            return Err(AttachFault::Unauthenticated);
        }
        let resource = Resource::new(self.org.path(&parent.resource()));
        match self
            .server
            .engine
            .check(&self.who, &resource, &Action::new(action))
        {
            Decision::Allow => Ok(()),
            Decision::Deny { .. } => Err(AttachFault::denied(action, parent)),
        }
    }

    async fn ledger(&self) -> Result<Arc<OrgLedger>, AttachFault> {
        self.server.ledger(&self.org).await
    }

    /// The attachment `hash` names on `parent`, if the parent points at
    /// it: a document's own list, a record's on the server.
    pub async fn locate(&self, parent: &Parent, hash: &str) -> Result<Attachment, AttachFault> {
        self.need(parent, Action::READ)?;
        check_hash(hash)?;
        let found = match parent {
            Parent::Doc { id } => doc::get(&self.document(*id).await?, hash),
            Parent::Record { .. } => {
                let ledger = self.ledger().await?;
                let state = ledger.state.lock().await;
                state
                    .record(&parent.to_string())
                    .iter()
                    .find(|a| a.hash == hash)
                    .cloned()
            }
        };
        found.ok_or_else(|| AttachFault::not_found(parent, hash))
    }

    async fn document(&self, id: uuid::Uuid) -> Result<loro::LoroDoc, AttachFault> {
        let docs = self
            .server
            .docs
            .as_ref()
            .ok_or_else(|| AttachFault::NoDoc(id.to_string()))?;
        docs.doc(&self.org, id)
            .await
            .ok_or_else(|| AttachFault::NoDoc(id.to_string()))
    }

    /// Where the bytes of `hash` (or its `rendition`) are on disk, and
    /// their length, once `parent` is known to point at them.
    pub async fn blob(
        &self,
        parent: &Parent,
        hash: &str,
        rendition: Option<RenditionKind>,
    ) -> Result<(std::path::PathBuf, Attachment, u64), AttachFault> {
        let attachment = self.locate(parent, hash).await?;
        let (blob, size) = if let Some(kind) = rendition {
            let renditions =
                self.server
                    .renditions
                    .as_ref()
                    .ok_or_else(|| AttachFault::NoRendition {
                        kind,
                        reason: NO_RENDITIONS.into(),
                    })?;
            renditions
                .ready(&ContentId::new(attachment.hash.clone()), kind)
                .await?
        } else {
            let blob = sha(hash)?;
            let size = self
                .stored(blob)
                .await?
                .ok_or_else(|| AttachFault::not_found(parent, hash))?;
            (blob, size)
        };
        Ok((self.server.inner.blobs.path_for(&blob), attachment, size))
    }

    /// The length of a blob the store holds.
    async fn stored(&self, blob: Sha256) -> Result<Option<u64>, AttachFault> {
        match tokio::fs::metadata(self.server.inner.blobs.path_for(&blob)).await {
            Ok(meta) => Ok(Some(meta.len())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io(e)),
        }
    }

    /// Land a staged upload whose bytes are all here: checked against
    /// `hash` as the store takes them.
    async fn land(&self, ledger: &OrgLedger, hash: &str) -> Result<(), AttachFault> {
        let staged = ledger.staging(hash);
        let file = tokio::fs::File::open(&staged).await.map_err(io)?;
        let body: Reader = Box::pin(file);
        let stored = self.server.inner.blobs.put(Some(sha(hash)?), body).await;
        let _ = tokio::fs::remove_file(&staged).await;
        match stored {
            Ok(_) => {
                if let Some(renditions) = &self.server.renditions {
                    renditions.schedule(ContentId::new(hash));
                }
                Ok(())
            }
            Err(BlobError::HashMismatch { actual, .. }) => Err(AttachFault::Mismatch {
                expected: hash.to_owned(),
                got: actual.to_hex(),
            }),
            Err(e) => Err(io(e)),
        }
    }
}

/// How many bytes of an upload are staged (none: nothing yet).
async fn staged_len(staged: &std::path::Path) -> u64 {
    tokio::fs::metadata(staged)
        .await
        .map_or(0, |meta| meta.len())
}

fn sha(hash: &str) -> Result<Sha256, AttachFault> {
    check_hash(hash)?;
    Sha256::from_hex(hash).map_err(|e| AttachFault::invalid(e.to_string()))
}

impl AttachmentsService for Acting {
    async fn begin(&self, parent: Parent, hash: String, size: u64) -> Result<Upload, AttachFault> {
        self.need(&parent, Action::WRITE)?;
        let blob = sha(&hash)?;
        let ledger = self.ledger().await?;
        let mut state = ledger.state.lock().await;
        if let Some(held) = state.held(&hash) {
            return Ok(Upload {
                received: held.size,
                size: held.size,
            });
        }
        if let Some(stored) = self.stored(blob).await? {
            // The store has these bytes (another org's, a file's): the org
            // holds them now, nothing is sent.
            let mut next = state.clone();
            next.room_for(stored)?;
            next.hold(&hash, stored, now_us());
            ledger.save(&next).await?;
            *state = next;
            return Ok(Upload {
                received: stored,
                size: stored,
            });
        }
        if state.pending(&hash).is_none_or(|p| p.size != size) {
            let mut next = state.clone();
            next.pending.retain(|p| p.hash != hash);
            next.room_for(size)?;
            next.pending.push(super::ledger::Pending {
                hash: hash.clone(),
                size,
            });
            ledger.save(&next).await?;
            *state = next;
            let _ = tokio::fs::remove_file(ledger.staging(&hash)).await;
        }
        let received = staged_len(&ledger.staging(&hash)).await;
        drop(state);
        Ok(Upload { received, size })
    }

    async fn send(
        &self,
        parent: Parent,
        hash: String,
        offset: u64,
        bytes: Vec<u8>,
    ) -> Result<Upload, AttachFault> {
        self.need(&parent, Action::WRITE)?;
        check_hash(&hash)?;
        let ledger = self.ledger().await?;
        let mut state = ledger.state.lock().await;
        if let Some(held) = state.held(&hash) {
            return Ok(Upload {
                received: held.size,
                size: held.size,
            });
        }
        let size = state
            .pending(&hash)
            .map(|p| p.size)
            .ok_or_else(|| AttachFault::invalid(format!("no upload of {hash}: begin it")))?;
        let staged = ledger.staging(&hash);
        let received = staged_len(&staged).await;
        if offset != received {
            return Err(AttachFault::Offset { received, offset });
        }
        let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let received = received.saturating_add(len);
        if received > size {
            return Err(AttachFault::invalid(format!(
                "{hash} is {size} bytes, and {received} were sent"
            )));
        }
        if let Some(dir) = staged.parent() {
            tokio::fs::create_dir_all(dir).await.map_err(io)?;
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&staged)
            .await
            .map_err(io)?;
        file.write_all(&bytes).await.map_err(io)?;
        file.sync_data().await.map_err(io)?;
        drop(file);
        if received < size {
            return Ok(Upload { received, size });
        }
        let mut next = state.clone();
        next.pending.retain(|p| p.hash != hash);
        let landed = self.land(&ledger, &hash).await;
        if landed.is_ok() {
            next.hold(&hash, size, now_us());
        }
        ledger.save(&next).await?;
        *state = next;
        drop(state);
        landed.map(|()| Upload { received, size })
    }

    async fn attach(
        &self,
        parent: Parent,
        mut attachment: Attachment,
    ) -> Result<Attachment, AttachFault> {
        self.need(&parent, Action::WRITE)?;
        if matches!(parent, Parent::Doc { .. }) {
            return Err(AttachFault::invalid(
                "a document lists its attachments itself (architect_attachments::doc::attach)",
            ));
        }
        attachment.validate()?;
        let ledger = self.ledger().await?;
        let mut state = ledger.state.lock().await;
        let held = state.held(&attachment.hash).ok_or_else(|| {
            AttachFault::invalid(format!("send the bytes of {} first", attachment.hash))
        })?;
        attachment.size = held.size;
        if attachment.added_us == 0 {
            attachment.added_us = now_us();
        }
        if attachment.added_by.is_none() {
            attachment.added_by = ticket_subject(&self.who);
        }
        let key = parent.to_string();
        let mut list = state.record(&key).to_vec();
        list.retain(|a| a.hash != attachment.hash);
        list.push(attachment.clone());
        let mut next = state.clone();
        next.set_record(&key, list);
        ledger.save(&next).await?;
        *state = next;
        drop(state);
        Ok(attachment)
    }

    async fn detach(&self, parent: Parent, hash: String) -> Result<bool, AttachFault> {
        self.need(&parent, Action::WRITE)?;
        if matches!(parent, Parent::Doc { .. }) {
            return Err(AttachFault::invalid(
                "a document lists its attachments itself (architect_attachments::doc::detach)",
            ));
        }
        let ledger = self.ledger().await?;
        let mut state = ledger.state.lock().await;
        let key = parent.to_string();
        let mut list = state.record(&key).to_vec();
        let before = list.len();
        list.retain(|a| a.hash != hash);
        if list.len() == before {
            return Ok(false);
        }
        let mut next = state.clone();
        next.set_record(&key, list);
        ledger.save(&next).await?;
        *state = next;
        drop(state);
        Ok(true)
    }

    async fn list(&self, parent: Parent) -> Result<Vec<Attachment>, AttachFault> {
        self.need(&parent, Action::READ)?;
        match &parent {
            Parent::Doc { id } => Ok(doc::list(&self.document(*id).await?)),
            Parent::Record { .. } => {
                let ledger = self.ledger().await?;
                let state = ledger.state.lock().await;
                Ok(state.record(&parent.to_string()).to_vec())
            }
        }
    }

    async fn size(&self, parent: Parent, hash: String) -> Result<Option<u64>, AttachFault> {
        self.locate(&parent, &hash).await?;
        self.stored(sha(&hash)?).await
    }

    async fn read(&self, request: ReadRequest, out: Tx<Vec<u8>>) -> Result<ReadDone, AttachFault> {
        let ReadRequest {
            parent,
            hash,
            rendition,
            offset,
        } = request;
        let (path, _, total) = self.blob(&parent, &hash, rendition).await?;
        let mut file = tokio::fs::File::open(&path).await.map_err(io)?;
        let start = offset.min(total);
        file.seek(std::io::SeekFrom::Start(start))
            .await
            .map_err(io)?;
        let mut left = total.saturating_sub(start);
        let mut sent = 0u64;
        let mut buf = vec![0u8; CHUNK];
        while left > 0 {
            let want = usize::try_from(left).unwrap_or(CHUNK).min(CHUNK);
            let chunk = buf.get_mut(..want).unwrap_or_default();
            file.read_exact(chunk).await.map_err(io)?;
            if out.send(chunk.to_vec()).await.is_err() {
                return Err(AttachFault::Io("the reader went away".into()));
            }
            let n = u64::try_from(want).unwrap_or(left);
            left = left.saturating_sub(n);
            sent = sent.saturating_add(n);
        }
        Ok(ReadDone { total, sent })
    }

    async fn renditions(&self, parent: Parent, hash: String) -> Result<RenditionSet, AttachFault> {
        let attachment = self.locate(&parent, &hash).await?;
        let content = ContentId::new(attachment.hash);
        match &self.server.renditions {
            Some(renditions) => Ok(renditions.set(&content).await?),
            None => Ok(RenditionSet {
                content,
                class: None,
                kinds: Vec::new(),
                problem: Some(NO_RENDITIONS.into()),
            }),
        }
    }

    async fn ticket(&self, parent: Parent) -> Result<Ticket, AttachFault> {
        self.need(&parent, Action::READ)?;
        let who = ticket_subject(&self.who).ok_or(AttachFault::Unauthenticated)?;
        let expires_us = now_us().saturating_add(TICKET_TTL_US);
        let token = self
            .server
            .tickets
            .seal(
                TICKET_PREFIX,
                &Claims {
                    org: self.org.0.clone(),
                    parent: parent.to_string(),
                    who,
                    expires_us,
                },
            )
            .ok_or_else(|| AttachFault::Internal("could not sign a ticket".into()))?;
        Ok(Ticket { token, expires_us })
    }

    async fn usage(&self) -> Result<AttachUsage, AttachFault> {
        if matches!(self.who, Principal::Anonymous) {
            return Err(AttachFault::Unauthenticated);
        }
        let ledger = self.ledger().await?;
        let state = ledger.state.lock().await;
        Ok(state.usage())
    }
}

// ── The server, for the call running now ───────────────────────────

/// Every method for the gate's principal in the lane's org.
impl AttachmentsService for AttachmentsServer {
    async fn begin(&self, parent: Parent, hash: String, size: u64) -> Result<Upload, AttachFault> {
        self.for_call()?.begin(parent, hash, size).await
    }
    async fn send(
        &self,
        parent: Parent,
        hash: String,
        offset: u64,
        bytes: Vec<u8>,
    ) -> Result<Upload, AttachFault> {
        self.for_call()?.send(parent, hash, offset, bytes).await
    }
    async fn attach(
        &self,
        parent: Parent,
        attachment: Attachment,
    ) -> Result<Attachment, AttachFault> {
        self.for_call()?.attach(parent, attachment).await
    }
    async fn detach(&self, parent: Parent, hash: String) -> Result<bool, AttachFault> {
        self.for_call()?.detach(parent, hash).await
    }
    async fn list(&self, parent: Parent) -> Result<Vec<Attachment>, AttachFault> {
        self.for_call()?.list(parent).await
    }
    async fn size(&self, parent: Parent, hash: String) -> Result<Option<u64>, AttachFault> {
        self.for_call()?.size(parent, hash).await
    }
    async fn read(&self, request: ReadRequest, out: Tx<Vec<u8>>) -> Result<ReadDone, AttachFault> {
        AttachmentsService::read(&self.for_call()?, request, out).await
    }
    async fn renditions(&self, parent: Parent, hash: String) -> Result<RenditionSet, AttachFault> {
        self.for_call()?.renditions(parent, hash).await
    }
    async fn ticket(&self, parent: Parent) -> Result<Ticket, AttachFault> {
        self.for_call()?.ticket(parent).await
    }
    async fn usage(&self) -> Result<AttachUsage, AttachFault> {
        self.for_call()?.usage().await
    }
}
