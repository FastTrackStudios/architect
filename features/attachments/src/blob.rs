//! A device's side (feature `blob`, native), over `architect::blob`:
//! attach from the local store, push what the server lacks, and fetch
//! another device's attachments by hash.
//!
//! ```ignore
//! // Offline: the bytes into this device's store, the entry into the doc.
//! let a = attach_local(&store, doc.loro(), "brief.pdf", "application/pdf", &pdf, now, by).await?;
//! // Online again: send what the server lacks (resumes where it stood).
//! client.push_doc(&store, doc_id, doc.loro()).await?;
//! // Another device: the doc arrived by sync; its attachments by hash.
//! let fetcher = Fetcher::new(store).with_remote(Arc::new(AttachmentsRemote::new(client, Parent::doc(doc_id))));
//! crdt::blobs::keep_offline(&kind, doc.loro(), &fetcher).await?;
//! ```

use std::io;

use architect::blob::{LocalFs, Reader, Remote, Sha256, StreamReader};
use architect::vox;
use futures_util::future::BoxFuture;
use loro::LoroDoc;
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};

use crate::client::{AttachmentsClient, CHUNK, ClientError, Result};
use crate::doc;
use crate::model::{AttachFault, Attachment, Parent, ReadRequest, Upload};
use crate::service::AttachmentsServiceClient;

/// Attach `bytes` to `doc` on this device: into `store` (pinned, so a
/// collection keeps them until they are pushed and after), then into the
/// document. Needs no server.
///
/// # Errors
///
/// The store, or an attachment the document refuses.
pub async fn attach_local(
    store: &LocalFs,
    doc: &LoroDoc,
    name: &str,
    mime: &str,
    bytes: &[u8],
    added_us: i64,
    added_by: Option<String>,
) -> Result<Attachment> {
    let hash = store.put_bytes(bytes).await.map_err(local)?;
    store.pin(hash).await.map_err(local)?;
    let attachment = Attachment {
        hash: hash.to_hex(),
        name: name.to_owned(),
        mime: mime.to_owned(),
        size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        added_us,
        added_by,
    };
    doc::attach(doc, &attachment)?;
    Ok(attachment)
}

fn local(e: impl std::fmt::Display) -> ClientError {
    ClientError::Local(e.to_string())
}

impl AttachmentsClient {
    /// Make sure the server holds the blob `hash` names, from `store`:
    /// send what it lacks, read off disk a chunk at a time.
    ///
    /// # Errors
    ///
    /// The store does not hold it, the server's refusal, or the link.
    pub async fn push_from(&self, store: &LocalFs, parent: &Parent, hash: &str) -> Result<Upload> {
        let blob = Sha256::from_hex(hash).map_err(local)?;
        let path = store.path_for(&blob);
        let size = tokio::fs::metadata(&path).await.map_err(local)?.len();
        let mut upload = self
            .service
            .begin(parent.clone(), hash.to_owned(), size)
            .await?;
        if upload.is_done() {
            return Ok(upload);
        }
        let mut file = tokio::fs::File::open(&path).await.map_err(local)?;
        let mut buf = vec![0u8; CHUNK];
        while !upload.is_done() {
            file.seek(io::SeekFrom::Start(upload.received))
                .await
                .map_err(local)?;
            let n = file.read(&mut buf).await.map_err(local)?;
            let chunk = buf.get(..n).filter(|c| !c.is_empty()).ok_or_else(|| {
                ClientError::Local(format!("{hash} is shorter than the server says"))
            })?;
            upload = self
                .send(parent, hash, size, upload.received, chunk.to_vec())
                .await?;
        }
        Ok(upload)
    }

    /// Push every attachment of document `doc_id` whose bytes `store`
    /// holds: what a device runs when it is back online. Returns the
    /// attachments the server now holds.
    ///
    /// # Errors
    ///
    /// The first that fails; run again to finish (each resumes).
    pub async fn push_doc(
        &self,
        store: &LocalFs,
        doc_id: uuid::Uuid,
        doc: &LoroDoc,
    ) -> Result<Vec<Attachment>> {
        let parent = Parent::doc(doc_id);
        let mut pushed = Vec::new();
        for attachment in doc::list(doc) {
            let blob = Sha256::from_hex(&attachment.hash).map_err(local)?;
            if !architect::blob::BlobStore::has(store, blob)
                .await
                .map_err(local)?
            {
                continue;
            }
            self.push_from(store, &parent, &attachment.hash).await?;
            pushed.push(attachment);
        }
        Ok(pushed)
    }
}

/// A parent's attachments on the org's server, as a [`Remote`]: what a
/// [`architect::blob::Fetcher`] reads another device's attachments
/// through, by hash, with the parent's permissions.
#[derive(Clone)]
pub struct AttachmentsRemote {
    service: AttachmentsServiceClient,
    parent: Parent,
}

impl std::fmt::Debug for AttachmentsRemote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttachmentsRemote")
            .field("parent", &self.parent)
            .finish_non_exhaustive()
    }
}

impl AttachmentsRemote {
    #[must_use]
    pub fn new(client: &AttachmentsClient, parent: Parent) -> Self {
        Self {
            service: client.service.clone(),
            parent,
        }
    }
}

fn call_err(e: &vox::VoxError<AttachFault>) -> io::Error {
    match e {
        vox::VoxError::User(fault) => match **fault {
            AttachFault::Denied { .. } | AttachFault::Unauthenticated => {
                io::Error::new(io::ErrorKind::PermissionDenied, fault.to_string())
            }
            AttachFault::NotFound { .. } | AttachFault::NoDoc(_) => {
                io::Error::new(io::ErrorKind::NotFound, fault.to_string())
            }
            _ => io::Error::other(fault.to_string()),
        },
        other => io::Error::new(io::ErrorKind::ConnectionAborted, format!("{other:?}")),
    }
}

impl Remote for AttachmentsRemote {
    fn size(&self, hash: Sha256) -> BoxFuture<'_, io::Result<Option<u64>>> {
        Box::pin(async move {
            self.service
                .size(self.parent.clone(), hash.to_hex())
                .await
                .map_err(|e| call_err(&e))
        })
    }

    fn read_from(&self, hash: Sha256, offset: u64) -> BoxFuture<'_, io::Result<Reader>> {
        Box::pin(async move {
            let (tx, rx) = vox::channel::<Vec<u8>>();
            let service = self.service.clone();
            let parent = self.parent.clone();
            // The call must run for the request (which binds `tx`) to go
            // out, and stays in flight while the frames arrive.
            let call = tokio::spawn(async move {
                service
                    .read(ReadRequest::bytes(parent, hash.to_hex(), offset), tx)
                    .await
            });
            let frames =
                futures_util::stream::unfold((rx, Some(call)), |(mut rx, call)| async move {
                    let call = call?;
                    match rx.recv().await {
                        Ok(Some(frame)) => {
                            let mut owned = None;
                            let _ = frame.map(|f| owned = Some(f));
                            Some((Ok(owned.unwrap_or_default()), (rx, Some(call))))
                        }
                        // The stream closed: the call says whether that was
                        // the end or a failure.
                        _ => match call.await {
                            Ok(Ok(_)) => None,
                            Ok(Err(e)) => Some((Err(call_err(&e)), (rx, None))),
                            Err(e) => Some((Err(io::Error::other(e)), (rx, None))),
                        },
                    }
                });
            let reader: Reader = Box::pin(StreamReader::new(Box::pin(frames)));
            Ok(reader)
        })
    }
}
