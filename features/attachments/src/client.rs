//! [`AttachmentsClient`] (feature `vox`): what a device does over the
//! org's lane — send an attachment's bytes, resumably and only once, and
//! read them back. Wasm-clean; the local store's side is `crate::blob`.

use architect::vox;
use futures_util::future::join;
use sha2::Digest as _;

use crate::model::{AttachFault, Attachment, Parent, ReadRequest, Upload};
use crate::service::AttachmentsServiceClient;

/// Bytes per [`AttachmentsServiceClient::send`] call.
pub const CHUNK: usize = 256 * 1024;

/// Why a client call failed.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The server's answer.
    #[error("{0}")]
    Fault(AttachFault),
    /// The link.
    #[error("transport: {0}")]
    Transport(String),
    /// A read that sent other than it promised.
    #[error("stream: {0}")]
    Stream(String),
    /// This device's own store.
    #[error("local store: {0}")]
    Local(String),
}

impl ClientError {
    #[must_use]
    pub const fn fault(&self) -> Option<&AttachFault> {
        match self {
            Self::Fault(f) => Some(f),
            _ => None,
        }
    }
}

impl From<vox::VoxError<AttachFault>> for ClientError {
    fn from(e: vox::VoxError<AttachFault>) -> Self {
        match e {
            vox::VoxError::User(fault) => Self::Fault(*fault),
            other => Self::Transport(format!("{other:?}")),
        }
    }
}

impl From<AttachFault> for ClientError {
    fn from(fault: AttachFault) -> Self {
        Self::Fault(fault)
    }
}

pub type Result<T> = std::result::Result<T, ClientError>;

/// The sha256 of `bytes`, as an attachment names them.
#[must_use]
pub fn hash_of(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    sha2::Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        })
}

/// An attachment of `bytes` named `name`: what a device writes into a
/// document ([`crate::doc::attach`]) before, or without, any server.
#[must_use]
pub fn attachment_of(
    bytes: &[u8],
    name: &str,
    mime: &str,
    added_us: i64,
    added_by: Option<String>,
) -> Attachment {
    Attachment {
        hash: hash_of(bytes),
        name: name.to_owned(),
        mime: mime.to_owned(),
        size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        added_us,
        added_by,
    }
}

/// The org's attachments, over its lane (`/org/{slug}/vox`).
#[derive(Clone)]
pub struct AttachmentsClient {
    pub service: AttachmentsServiceClient,
}

impl AttachmentsClient {
    #[must_use]
    pub fn new(caller: vox::Caller) -> Self {
        Self {
            service: AttachmentsServiceClient::new(caller),
        }
    }

    /// Make sure the server holds `bytes` for `parent`: send what it
    /// lacks, from where it stands (nothing, when the org has them).
    ///
    /// # Errors
    ///
    /// The server's refusal, or the link.
    pub async fn push(&self, parent: &Parent, bytes: &[u8]) -> Result<Upload> {
        let hash = hash_of(bytes);
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let mut upload = self
            .service
            .begin(parent.clone(), hash.clone(), size)
            .await?;
        while !upload.is_done() {
            let start = usize::try_from(upload.received).unwrap_or(usize::MAX);
            let chunk = bytes
                .get(start..)
                .and_then(|rest| rest.get(..CHUNK.min(rest.len())))
                .ok_or_else(|| ClientError::Stream("the server is past the end".into()))?;
            upload = self
                .send(parent, &hash, size, upload.received, chunk.to_vec())
                .await?;
        }
        Ok(upload)
    }

    /// One chunk of an upload of `size` bytes. A server that stands
    /// elsewhere (an earlier attempt's bytes arrived after all) says where,
    /// and the upload carries on from there.
    pub(crate) async fn send(
        &self,
        parent: &Parent,
        hash: &str,
        size: u64,
        offset: u64,
        bytes: Vec<u8>,
    ) -> Result<Upload> {
        let sent = self
            .service
            .send(parent.clone(), hash.to_owned(), offset, bytes)
            .await;
        match sent {
            Err(vox::VoxError::User(fault)) => match *fault {
                AttachFault::Offset { received, .. } => Ok(Upload { received, size }),
                other => Err(ClientError::Fault(other)),
            },
            other => Ok(other?),
        }
    }

    /// An attachment's bytes, whole.
    ///
    /// # Errors
    ///
    /// The server's refusal, the link, or a short read.
    pub async fn read(&self, parent: &Parent, hash: &str) -> Result<Vec<u8>> {
        let (tx, mut rx) = vox::channel::<Vec<u8>>();
        let collecting = async move {
            let mut out = Vec::new();
            loop {
                match rx.recv().await {
                    Ok(Some(chunk)) => {
                        let _ = chunk.map(|c| out.extend_from_slice(&c));
                    }
                    Ok(None) => return Ok(out),
                    Err(e) => return Err(ClientError::Transport(format!("{e:?}"))),
                }
            }
        };
        let call = self
            .service
            .read(ReadRequest::bytes(parent.clone(), hash, 0), tx);
        let (done, out) = join(call, collecting).await;
        let (done, out) = (done?, out?);
        if u64::try_from(out.len()).ok() != Some(done.sent) {
            return Err(ClientError::Stream(format!(
                "the read promised {} bytes and sent {}",
                done.sent,
                out.len()
            )));
        }
        Ok(out)
    }
}
