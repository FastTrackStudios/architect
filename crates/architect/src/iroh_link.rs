//! iroh p2p ↔ vox transport adapter.
//!
//! The QUIC sibling of [`crate::axum_ws`]: serve a vox router over an
//! [iroh](https://docs.rs/iroh) endpoint so remotes dial the backend by
//! bare [`iroh::EndpointId`] — across NATs and networks, no port
//! forwarding, no addresses to configure. Each accepted bi-directional
//! QUIC stream is one vox connection (mirroring "one WebSocket = one vox
//! connection"); the client opens one bi-stream per vox connection.
//!
//! QUIC streams are byte streams, so vox frames travel length-prefixed
//! (u32 big-endian length + payload; frames over [`MAX_FRAME_LEN`] are
//! rejected).
//!
//! ## Usage
//!
//! ```ignore
//! // server
//! let sk = iroh_link::load_or_create_secret_key(&key_path)?;
//! let endpoint = iroh_link::bind_endpoint(sk).await?;
//! tracing::info!("iroh endpoint id: {}", endpoint.id());
//! let acceptor = iroh_link::lane_acceptor_fn(move |_req, connection| {
//!     connection.handle_with(router.clone());
//!     Ok(())
//! });
//! iroh_link::serve_endpoint(&endpoint, acceptor).await;
//!
//! // client
//! let link = iroh_link::connect(&endpoint, remote_id).await?;
//! let client = vox_core::initiator_on(link).establish::<RigClient>().await?;
//! ```
//!
//! Gated behind `--features iroh`. The client side (`connect`,
//! [`IrohLink`], [`iroh_link_source`]) also compiles on
//! wasm32-unknown-unknown — browsers dial relay-only (no UDP in the
//! sandbox; traffic rides WebSockets to the relays, still
//! e2e-encrypted). Serving (`serve_endpoint` / `serve_link`) and
//! on-disk key persistence are native only; browser callers persist
//! the key themselves via [`secret_key_to_hex`] /
//! [`secret_key_from_hex`] (e.g. in localStorage).

use std::io;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

#[cfg(not(target_arch = "wasm32"))]
use tracing::{debug, warn};

// Re-export so consumers can write `architect::iroh_link::lane_acceptor_fn`
// without importing vox-core, same as `axum_ws`.
pub use vox_core::{acceptor_on, lane_acceptor_fn};

// Re-export the crate so consumers reach iroh types (EndpointId,
// SecretKey, Endpoint) through architect on every platform — wasm needs
// a defaults-off iroh, and this saves each consumer re-declaring that
// target split.
pub use iroh;

/// ALPN identifying the vox protocol on an architect iroh endpoint.
pub const VOX_ALPN: &[u8] = b"architect/vox/1";

/// Hard ceiling on a single vox frame travelling the QUIC stream.
pub const MAX_FRAME_LEN: u32 = 64 * 1024 * 1024;

/// Read a 32-byte iroh secret key from `path`, or generate one and
/// persist it (0600 on unix) when the file doesn't exist yet.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_or_create_secret_key(path: &Path) -> io::Result<iroh::SecretKey> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let bytes: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}: expected exactly 32 key bytes", path.display()),
                )
            })?;
            Ok(iroh::SecretKey::from_bytes(&bytes))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let key = iroh::SecretKey::generate();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            #[cfg(unix)]
            {
                use std::io::Write as _;
                use std::os::unix::fs::OpenOptionsExt as _;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(path)?;
                file.write_all(&key.to_bytes())?;
            }
            #[cfg(not(unix))]
            std::fs::write(path, key.to_bytes())?;
            Ok(key)
        }
        Err(e) => Err(e),
    }
}

/// Parse a secret key from its hex encoding (as produced by
/// [`secret_key_to_hex`]) — how platforms without a filesystem
/// (browsers: localStorage) persist an endpoint identity.
pub fn secret_key_from_hex(s: &str) -> Result<iroh::SecretKey, iroh::KeyParsingError> {
    s.parse()
}

/// Hex-encode a secret key for persistence outside the filesystem —
/// the inverse of [`secret_key_from_hex`].
pub fn secret_key_to_hex(key: &iroh::SecretKey) -> String {
    key.to_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

/// Bind an iroh endpoint speaking [`VOX_ALPN`] with the n0 defaults
/// (relay servers + DNS address lookup + pkarr publishing), so remotes
/// can dial it by bare [`iroh::EndpointId`] from anywhere.
pub async fn bind_endpoint(
    secret_key: iroh::SecretKey,
) -> Result<iroh::Endpoint, iroh::endpoint::BindError> {
    iroh::Endpoint::builder(iroh::endpoint::presets::N0)
        .secret_key(secret_key)
        .alpns(vec![VOX_ALPN.to_vec()])
        .bind()
        .await
}

/// Accept vox connections on an iroh endpoint until the endpoint closes.
///
/// Every accepted QUIC connection may carry any number of bi-streams;
/// each bi-stream is served as its own vox connection through `acceptor`
/// — the exact `acceptor_on(...).establish_connection()` dance
/// [`crate::axum_ws::serve`] does per WebSocket.
#[cfg(not(target_arch = "wasm32"))]
pub async fn serve_endpoint<A>(endpoint: &iroh::Endpoint, acceptor: A)
where
    A: vox_core::LaneAcceptor,
{
    let acceptor = SharedAcceptor(std::sync::Arc::new(acceptor));
    while let Some(incoming) = endpoint.accept().await {
        let acceptor = acceptor.clone();
        tokio::task::spawn(async move {
            let connection = match incoming.await {
                Ok(c) => c,
                Err(e) => {
                    warn!(error = %e, "iroh incoming connection failed");
                    return;
                }
            };
            loop {
                match connection.accept_bi().await {
                    Ok((send, recv)) => {
                        let link = IrohLink::new(connection.clone(), send, recv);
                        let acceptor = acceptor.clone();
                        tokio::task::spawn(serve_link(link, acceptor));
                    }
                    Err(e) => {
                        debug!(error = %e, "iroh connection closed");
                        return;
                    }
                }
            }
        });
    }
}

/// Serve a whole router over an iroh endpoint — the iroh analogue of
/// [`crate::axum_ws::serve_router`], accepting any vox [`vox::Handler`] and
/// collapsing the `lane_acceptor_fn(|_, conn| conn.handle_with(router.clone()))`
/// + [`serve_endpoint`] boilerplate to one call.
#[cfg(not(target_arch = "wasm32"))]
pub async fn serve_router<H>(endpoint: &iroh::Endpoint, router: H)
where
    H: vox::Handler<vox::DriverReplySink> + Clone + Send + Sync + 'static,
{
    serve_endpoint(endpoint, crate::layer::handler_acceptor(router)).await;
}

/// One acceptor shared by every connection/stream the endpoint accepts
/// — `on_lane` wants an owned acceptor per vox connection, so hand each
/// one a clone of the same `Arc`.
#[cfg(not(target_arch = "wasm32"))]
struct SharedAcceptor<A>(std::sync::Arc<A>);

#[cfg(not(target_arch = "wasm32"))]
impl<A> Clone for SharedAcceptor<A> {
    fn clone(&self) -> Self {
        Self(std::sync::Arc::clone(&self.0))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl<A: vox_core::LaneAcceptor> vox_core::LaneAcceptor for SharedAcceptor<A> {
    fn accept(
        &self,
        request: &vox_core::LaneRequest,
        connection: vox_core::PendingLane,
    ) -> Result<(), vox_core::LaneRejection> {
        self.0.accept(request, connection)
    }
}

/// Drive a single vox connection on an already-established [`IrohLink`].
///
/// Completes the vox handshake, then parks until the peer's stream
/// closes (the notifier fires from `Drop` on the link's receive half).
#[cfg(not(target_arch = "wasm32"))]
pub async fn serve_link<A>(link: IrohLink, acceptor: A)
where
    A: vox_core::LaneAcceptor,
{
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
    let connection = match acceptor_on(link.with_closed_notifier(closed_tx))
        .on_lane(acceptor)
        .establish_connection()
        .await
    {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "vox iroh session failed");
            return;
        }
    };
    let _connection = connection;
    let _ = closed_rx.await;
}

/// Dial a remote vox endpoint and open one bi-stream — one vox
/// connection. Accepts a bare [`iroh::EndpointId`] (resolved through the
/// endpoint's address lookup) or a full [`iroh::EndpointAddr`].
pub async fn connect(
    endpoint: &iroh::Endpoint,
    remote: impl Into<iroh::EndpointAddr>,
) -> io::Result<IrohLink> {
    let connection = endpoint
        .connect(remote, VOX_ALPN)
        .await
        .map_err(io::Error::other)?;
    let (send, recv) = connection.open_bi().await.map_err(io::Error::other)?;
    Ok(IrohLink::new(connection, send, recv))
}

/// A [`vox_core::LinkSource`] that dials the same remote afresh on every
/// `next_link` — what reconnect-capable clients hand to the connection
/// supervisor.
pub struct IrohLinkSource {
    endpoint: iroh::Endpoint,
    remote: iroh::EndpointAddr,
}

/// Create an [`IrohLinkSource`] dialing `remote` from `endpoint`.
pub fn iroh_link_source(
    endpoint: iroh::Endpoint,
    remote: impl Into<iroh::EndpointAddr>,
) -> IrohLinkSource {
    IrohLinkSource {
        endpoint,
        remote: remote.into(),
    }
}

impl vox_core::LinkSource for IrohLinkSource {
    type Link = IrohLink;

    async fn next_link(&mut self) -> io::Result<vox_core::Attachment<Self::Link>> {
        let link = connect(&self.endpoint, self.remote.clone()).await?;
        Ok(vox_core::Attachment::initiator(link))
    }
}

// ── Link adapter ──────────────────────────────────────────────────────

/// One vox connection over one iroh bi-directional QUIC stream.
///
/// Holds the owning [`iroh::endpoint::Connection`] so the QUIC
/// connection outlives the link's stream halves.
pub struct IrohLink {
    connection: iroh::endpoint::Connection,
    send: iroh::endpoint::SendStream,
    recv: iroh::endpoint::RecvStream,
    closed: Option<tokio::sync::oneshot::Sender<()>>,
}

impl IrohLink {
    pub fn new(
        connection: iroh::endpoint::Connection,
        send: iroh::endpoint::SendStream,
        recv: iroh::endpoint::RecvStream,
    ) -> Self {
        Self {
            connection,
            send,
            recv,
            closed: None,
        }
    }

    /// Fire `closed` when the receive half is dropped (peer gone, stream
    /// finished, or vox tearing the session down) — how [`serve_link`]
    /// knows to release its `ConnectionHandle`.
    pub fn with_closed_notifier(mut self, closed: tokio::sync::oneshot::Sender<()>) -> Self {
        self.closed = Some(closed);
        self
    }

    /// The remote peer's endpoint id.
    pub fn remote_id(&self) -> iroh::EndpointId {
        self.connection.remote_id()
    }
}

impl vox_types::Link for IrohLink {
    type Tx = IrohLinkTx;
    type Rx = IrohLinkRx;

    fn split(self) -> (Self::Tx, Self::Rx) {
        (
            IrohLinkTx {
                send: tokio::sync::Mutex::new(self.send),
                _connection: self.connection.clone(),
            },
            IrohLinkRx {
                recv: self.recv,
                _connection: self.connection,
                _notify: NotifyOnDrop(self.closed),
            },
        )
    }
}

struct NotifyOnDrop(Option<tokio::sync::oneshot::Sender<()>>);
impl Drop for NotifyOnDrop {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}

pub struct IrohLinkTx {
    send: tokio::sync::Mutex<iroh::endpoint::SendStream>,
    _connection: iroh::endpoint::Connection,
}

impl vox_types::LinkTx for IrohLinkTx {
    async fn send(&self, bytes: Vec<u8>) -> io::Result<()> {
        let len = u32::try_from(bytes.len())
            .ok()
            .filter(|len| *len <= MAX_FRAME_LEN)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("vox frame of {} bytes exceeds {MAX_FRAME_LEN}", bytes.len()),
                )
            })?;
        let mut send = self.send.lock().await;
        send.write_all(&len.to_be_bytes())
            .await
            .map_err(io::Error::other)?;
        send.write_all(&bytes).await.map_err(io::Error::other)
    }

    async fn close(self) -> io::Result<()> {
        let mut send = self.send.into_inner();
        // ClosedStream just means the peer already stopped the stream.
        let _ = send.finish();
        Ok(())
    }
}

pub struct IrohLinkRx {
    recv: iroh::endpoint::RecvStream,
    _connection: iroh::endpoint::Connection,
    _notify: NotifyOnDrop,
}

#[derive(Debug)]
pub struct IrohLinkError(String);
impl std::fmt::Display for IrohLinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "iroh link: {}", self.0)
    }
}
impl std::error::Error for IrohLinkError {}

impl vox_types::LinkRx for IrohLinkRx {
    type Error = IrohLinkError;

    async fn recv(&mut self) -> Result<Option<vox_types::Backing>, Self::Error> {
        use iroh::endpoint::{ConnectionError, ReadError, ReadExactError};

        let mut len_bytes = [0u8; 4];
        match self.recv.read_exact(&mut len_bytes).await {
            Ok(()) => {}
            // Peer finished the stream at a frame boundary: clean close.
            Err(ReadExactError::FinishedEarly(0)) => return Ok(None),
            // Peer closed the whole QUIC connection: also a close.
            Err(ReadExactError::ReadError(ReadError::ConnectionLost(
                ConnectionError::ApplicationClosed(_) | ConnectionError::LocallyClosed,
            ))) => return Ok(None),
            Err(e) => return Err(IrohLinkError(e.to_string())),
        }
        let len = u32::from_be_bytes(len_bytes);
        if len > MAX_FRAME_LEN {
            return Err(IrohLinkError(format!(
                "vox frame of {len} bytes exceeds {MAX_FRAME_LEN}"
            )));
        }
        let mut bytes = vec![0u8; len as usize];
        self.recv
            .read_exact(&mut bytes)
            .await
            .map_err(|e| IrohLinkError(format!("truncated vox frame: {e}")))?;
        Ok(Some(vox_types::Backing::Boxed(bytes.into_boxed_slice())))
    }
}
