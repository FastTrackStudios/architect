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
//! ## Finding peers
//!
//! [`bind_endpoint`] uses n0's relays and DNS, which need the internet.
//! With `--features iroh-lan`, [`bind_endpoint_with`] /
//! [`endpoint_builder`] take a [`PeerLookup`]: `N0AndLan` adds
//! mDNS-style lookup on the local network beside n0 (peers on one LAN
//! dial by bare id with the internet down), `Lan` uses only the local
//! network. `EngineHost::iroh_lookup` picks it for a host.
//!
//! ## Dialling a restarted peer
//!
//! [`connect`] is bounded ([`DIAL_TIMEOUT`]) and survives the peer
//! restarting on new addresses. If the first attempt fails, the dial
//! closes the stale connections to that peer, looks it up again and
//! retries once (see [`connect_with`]). A failure carries a
//! [`DialError`]: [`DialErrorKind::PeerUnreachable`] means the retry
//! failed too.
//!
//! Gated behind `--features iroh`. The client side (`connect`,
//! [`IrohLink`], [`iroh_link_source`]) also compiles on
//! wasm32-unknown-unknown — browsers dial relay-only (no UDP in the
//! sandbox; traffic rides `WebSockets` to the relays, still
//! e2e-encrypted). Serving (`serve_endpoint` / `serve_link`) and
//! on-disk key persistence are native only; browser callers persist
//! the key themselves via [`secret_key_to_hex`] /
//! [`secret_key_from_hex`] (e.g. in localStorage).

use std::io;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::time::Duration;

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
#[must_use]
pub fn secret_key_to_hex(key: &iroh::SecretKey) -> String {
    use std::fmt::Write as _;
    key.to_bytes().iter().fold(String::new(), |mut out, b| {
        // Writing to a `String` is infallible.
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// Re-export of the LAN address-lookup crate (feature `iroh-lan`, native
/// only), for consumers that subscribe to discovery events or tune the
/// service themselves.
#[cfg(all(feature = "iroh-lan", not(target_arch = "wasm32")))]
pub use iroh_mdns_address_lookup;

/// How an endpoint turns a bare [`iroh::EndpointId`] into addresses it
/// can dial, and where it publishes its own.
///
/// ADR 0006: a directory (n0's DNS, a relay) must never be the only way
/// to find a peer. `PeerLookup::N0AndLan` keeps the internet path and
/// adds the local network; `PeerLookup::Lan` never leaves it (both need
/// the `iroh-lan` feature).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum PeerLookup {
    /// The n0 preset: relay servers, pkarr publishing and DNS lookup on
    /// n0's servers. Dials from anywhere, but needs the internet.
    #[default]
    N0,
    /// [`PeerLookup::N0`] plus mDNS-style lookup on the local network:
    /// peers on the same LAN still find each other when the internet is
    /// down. Feature `iroh-lan`, native only.
    #[cfg(all(feature = "iroh-lan", not(target_arch = "wasm32")))]
    N0AndLan,
    /// Local network only: mDNS-style lookup, relays off, nothing
    /// published to n0. Feature `iroh-lan`, native only.
    #[cfg(all(feature = "iroh-lan", not(target_arch = "wasm32")))]
    Lan,
}

/// An endpoint builder speaking [`VOX_ALPN`] that resolves peers the
/// way `lookup` says. Callers add a secret key, extra lookups (iroh's
/// `MemoryLookup`) or a bind address, then `.bind()`.
#[must_use]
pub fn endpoint_builder(lookup: PeerLookup) -> iroh::endpoint::Builder {
    let builder = match lookup {
        PeerLookup::N0 => iroh::Endpoint::builder(iroh::endpoint::presets::N0),
        #[cfg(all(feature = "iroh-lan", not(target_arch = "wasm32")))]
        PeerLookup::N0AndLan => iroh::Endpoint::builder(iroh::endpoint::presets::N0)
            .address_lookup(iroh_mdns_address_lookup::MdnsAddressLookup::builder()),
        #[cfg(all(feature = "iroh-lan", not(target_arch = "wasm32")))]
        PeerLookup::Lan => iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .relay_mode(iroh::RelayMode::Disabled)
            .address_lookup(iroh_mdns_address_lookup::MdnsAddressLookup::builder()),
    };
    builder.alpns(vec![VOX_ALPN.to_vec()])
}

/// Bind an iroh endpoint speaking [`VOX_ALPN`] with the n0 defaults
/// (relay servers + DNS address lookup + pkarr publishing), so remotes
/// can dial it by bare [`iroh::EndpointId`] from anywhere.
pub async fn bind_endpoint(
    secret_key: iroh::SecretKey,
) -> Result<iroh::Endpoint, iroh::endpoint::BindError> {
    bind_endpoint_with(secret_key, PeerLookup::N0).await
}

/// [`bind_endpoint`] with an explicit [`PeerLookup`] — e.g.
/// `PeerLookup::N0AndLan` (feature `iroh-lan`) so two machines on one LAN dial each other
/// by bare id with no route to the internet.
pub async fn bind_endpoint_with(
    secret_key: iroh::SecretKey,
    lookup: PeerLookup,
) -> Result<iroh::Endpoint, iroh::endpoint::BindError> {
    endpoint_builder(lookup).secret_key(secret_key).bind().await
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
    let local = endpoint.id();
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
            // An inbound connection pins iroh's path to the peer just
            // like an outbound one; a redial may need to drop it.
            tracked::insert(local, &connection);
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

/// Serve a whole router over an iroh endpoint.
///
/// The iroh analogue of [`crate::axum_ws::serve_router`]: accepts any vox
/// [`vox::Handler`] and collapses the
/// `lane_acceptor_fn(|_, conn| conn.handle_with(router.clone()))` +
/// [`serve_endpoint`] boilerplate to one call.
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
///
/// [`connect_with`] with the default [`DialOptions`]: the whole dial,
/// one retry included, is bounded by [`DIAL_TIMEOUT`]. A failure carries
/// a [`DialError`] (see [`DialError::of`]).
pub async fn connect(
    endpoint: &iroh::Endpoint,
    remote: impl Into<iroh::EndpointAddr>,
) -> io::Result<IrohLink> {
    connect_with(endpoint, remote, DialOptions::default()).await
}

/// [`connect`] with explicit [`DialOptions`].
///
/// The first attempt gets half the budget. If it times out or loses the
/// path (reset, closed, no address), the dial drops this endpoint's
/// stale state for the peer and tries once more with what is left:
///
/// - It closes every connection to the peer that `iroh_link` opened or
///   accepted on this endpoint. While any connection to a peer is open,
///   iroh keeps sending new handshakes down that connection's path and
///   skips address lookup. After the peer restarts on new addresses, that
///   path leads nowhere. Those connections are already dead: nothing
///   reached the peer on that path for the whole first attempt.
/// - It runs the endpoint's address lookup again and dials the addresses
///   it returns as well as any given in `remote`.
///
/// If the retry fails too, the error is [`DialErrorKind::PeerUnreachable`].
pub async fn connect_with(
    endpoint: &iroh::Endpoint,
    remote: impl Into<iroh::EndpointAddr>,
    options: DialOptions,
) -> io::Result<IrohLink> {
    dial(endpoint, remote.into(), options)
        .await
        .map_err(io::Error::from)
}

/// The default budget for a whole [`connect`], retry included.
pub const DIAL_TIMEOUT: Duration = Duration::from_secs(10);

/// How [`connect_with`] dials: the time budget and whether to retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DialOptions {
    timeout: Duration,
    retry: bool,
}

impl Default for DialOptions {
    fn default() -> Self {
        Self {
            timeout: DIAL_TIMEOUT,
            retry: true,
        }
    }
}

impl DialOptions {
    /// Budget for the whole dial, retry included (default
    /// [`DIAL_TIMEOUT`]). With retry on, the first attempt gets half.
    #[must_use]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Whether a failed attempt is retried once after dropping stale
    /// state for the peer (default on). With it off, an attempt that runs
    /// out of time fails with [`DialErrorKind::Timeout`].
    #[must_use]
    pub const fn retry(mut self, retry: bool) -> Self {
        self.retry = retry;
        self
    }
}

/// Why a dial failed. See [`DialError::kind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DialErrorKind {
    /// One attempt ran out of time and was not retried
    /// ([`DialOptions::retry`] off). The peer may still be up, behind
    /// stale path state on this endpoint. `io::ErrorKind::TimedOut`.
    Timeout,
    /// Nothing reached the peer, even on the retry after dropping stale
    /// connections and re-running address lookup. Either the peer is
    /// down, or none of its published addresses reach it from here.
    /// `io::ErrorKind::HostUnreachable`.
    PeerUnreachable,
    /// The dial could not start, or the peer refused it: endpoint closed,
    /// own id, TLS or ALPN rejected. Not retried. `io::ErrorKind::Other`.
    Failed,
}

/// A failed [`connect`] / [`connect_with`] / [`IrohLinkSource`] dial,
/// carried inside the returned `io::Error` — get it back with
/// [`DialError::of`].
#[derive(Debug)]
pub struct DialError {
    kind: DialErrorKind,
    remote: iroh::EndpointId,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl DialError {
    /// What went wrong.
    #[must_use]
    pub const fn kind(&self) -> DialErrorKind {
        self.kind
    }

    /// The peer that was dialled.
    #[must_use]
    pub const fn remote(&self) -> iroh::EndpointId {
        self.remote
    }

    /// The `DialError` inside an `io::Error` a dial returned, if any.
    #[must_use]
    pub fn of(error: &io::Error) -> Option<&Self> {
        error.get_ref()?.downcast_ref()
    }
}

impl std::fmt::Display for DialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = match self.kind {
            DialErrorKind::Timeout => "timed out",
            DialErrorKind::PeerUnreachable => "peer unreachable",
            DialErrorKind::Failed => "failed",
        };
        write!(f, "dial {}: {what}", self.remote.fmt_short())?;
        if let Some(source) = &self.source {
            write!(f, ": {source}")?;
        }
        Ok(())
    }
}

impl std::error::Error for DialError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}

impl From<DialError> for io::Error {
    fn from(error: DialError) -> Self {
        let kind = match error.kind {
            DialErrorKind::Timeout => io::ErrorKind::TimedOut,
            DialErrorKind::PeerUnreachable => io::ErrorKind::HostUnreachable,
            DialErrorKind::Failed => io::ErrorKind::Other,
        };
        Self::new(kind, error)
    }
}

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// How one dial attempt failed.
enum Attempt {
    /// Ran out of its time budget.
    Elapsed,
    /// Lost the path: no address, reset, closed or timed out by QUIC.
    Lost(BoxError),
    /// Could not start, or the peer refused.
    Refused(BoxError),
}

impl Attempt {
    fn from_connect(error: iroh::endpoint::ConnectError) -> Self {
        use iroh::endpoint::{ConnectError, ConnectWithOptsError, ConnectingError};
        let lost = match &error {
            ConnectError::Connect {
                source: ConnectWithOptsError::NoAddress { .. },
                ..
            } => true,
            ConnectError::Connecting {
                source: ConnectingError::ConnectionError { source, .. },
                ..
            }
            | ConnectError::Connection { source, .. } => path_lost(source),
            _ => false,
        };
        if lost {
            Self::Lost(Box::new(error))
        } else {
            Self::Refused(Box::new(error))
        }
    }

    fn from_connection(error: iroh::endpoint::ConnectionError) -> Self {
        if path_lost(&error) {
            Self::Lost(Box::new(error))
        } else {
            Self::Refused(Box::new(error))
        }
    }

    fn into_error(self, remote: iroh::EndpointId, retried: bool) -> DialError {
        let (kind, source) = match self {
            Self::Elapsed if retried => (DialErrorKind::PeerUnreachable, None),
            Self::Elapsed => (DialErrorKind::Timeout, None),
            Self::Lost(e) => (DialErrorKind::PeerUnreachable, Some(e)),
            Self::Refused(e) => (DialErrorKind::Failed, Some(e)),
        };
        DialError {
            kind,
            remote,
            source,
        }
    }
}

/// A QUIC error that means "the path to the peer is gone", not "the peer
/// said no".
const fn path_lost(error: &iroh::endpoint::ConnectionError) -> bool {
    use iroh::endpoint::ConnectionError;
    matches!(
        error,
        ConnectionError::TimedOut | ConnectionError::Reset | ConnectionError::ApplicationClosed(_)
    )
}

async fn dial(
    endpoint: &iroh::Endpoint,
    remote: iroh::EndpointAddr,
    options: DialOptions,
) -> Result<IrohLink, DialError> {
    use n0_future::time::{Instant, timeout};

    let id = remote.id;
    let started = Instant::now();
    let first_budget = if options.retry {
        options.timeout.checked_div(2).unwrap_or(options.timeout)
    } else {
        options.timeout
    };
    let first = match timeout(first_budget, dial_once(endpoint, remote.clone())).await {
        Ok(Ok(link)) => return Ok(link),
        Ok(Err(attempt)) => attempt,
        Err(_) => Attempt::Elapsed,
    };
    if !options.retry || matches!(first, Attempt::Refused(_)) {
        return Err(first.into_error(id, false));
    }

    let closed = close_tracked(endpoint, id).await;
    let left = options.timeout.saturating_sub(started.elapsed());
    let remote = with_fresh_lookup(endpoint, remote, left.checked_div(4).unwrap_or(left)).await;
    tracing::debug!(
        remote = %id.fmt_short(),
        closed,
        addrs = remote.addrs.len(),
        "iroh dial failed; redialling after dropping stale connections"
    );
    let left = options.timeout.saturating_sub(started.elapsed());
    match timeout(left, dial_once(endpoint, remote)).await {
        Ok(Ok(link)) => Ok(link),
        Ok(Err(attempt)) => Err(attempt.into_error(id, true)),
        Err(_) => Err(Attempt::Elapsed.into_error(id, true)),
    }
}

async fn dial_once(endpoint: &iroh::Endpoint, remote: iroh::EndpointAddr) -> Result<IrohLink, Attempt> {
    let connection = endpoint
        .connect(remote, VOX_ALPN)
        .await
        .map_err(Attempt::from_connect)?;
    tracked::insert(endpoint.id(), &connection);
    let (send, recv) = connection
        .open_bi()
        .await
        .map_err(Attempt::from_connection)?;
    Ok(IrohLink::new(connection, send, recv))
}

/// Close every tracked connection from `endpoint` to `remote`, so iroh
/// drops the path it selected for the peer and looks it up again.
/// Returns how many were closed.
async fn close_tracked(endpoint: &iroh::Endpoint, remote: iroh::EndpointId) -> usize {
    let stale = tracked::take(endpoint.id(), remote);
    for connection in &stale {
        connection.close(
            iroh::endpoint::VarInt::from_u32(0),
            b"architect: redialling a stale peer",
        );
    }
    // A local close completes at once; bounded all the same.
    let _ = n0_future::time::timeout(Duration::from_millis(250), async {
        for connection in &stale {
            connection.closed().await;
        }
    })
    .await;
    stale.len()
}

/// `remote` plus the first answer the endpoint's address lookup gives
/// within `budget`.
async fn with_fresh_lookup(
    endpoint: &iroh::Endpoint,
    mut remote: iroh::EndpointAddr,
    budget: Duration,
) -> iroh::EndpointAddr {
    use n0_future::StreamExt as _;

    let Ok(lookup) = endpoint.address_lookup() else {
        return remote;
    };
    let mut answers = lookup.resolve(remote.id);
    let _ = n0_future::time::timeout(budget, async {
        while let Some(answer) = answers.next().await {
            if let Ok(Ok(item)) = answer {
                remote.addrs.extend(item.into_endpoint_addr().addrs);
                return;
            }
        }
    })
    .await;
    remote
}

/// Weak handles to every connection `iroh_link` dialled or accepted,
/// keyed by (local endpoint, remote endpoint): the only per-peer state a
/// redial can drop, since iroh 1.0 has no call to make an endpoint
/// forget one remote's paths.
mod tracked {
    use std::collections::HashMap;
    use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

    use iroh::EndpointId;
    use iroh::endpoint::{Connection, WeakConnectionHandle};

    type Tracked = HashMap<(EndpointId, EndpointId), Vec<WeakConnectionHandle>>;

    static TRACKED: LazyLock<Mutex<Tracked>> = LazyLock::new(Mutex::default);

    fn lock() -> MutexGuard<'static, Tracked> {
        TRACKED.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn is_open(handle: &WeakConnectionHandle) -> bool {
        handle
            .upgrade()
            .is_some_and(|connection| connection.close_reason().is_none())
    }

    pub(super) fn insert(local: EndpointId, connection: &Connection) {
        let mut tracked = lock();
        tracked.retain(|_, handles| {
            handles.retain(is_open);
            !handles.is_empty()
        });
        tracked
            .entry((local, connection.remote_id()))
            .or_default()
            .push(connection.weak_handle());
    }

    pub(super) fn take(local: EndpointId, remote: EndpointId) -> Vec<Connection> {
        let handles = lock().remove(&(local, remote));
        handles
            .into_iter()
            .flatten()
            .filter_map(|handle| handle.upgrade())
            .collect()
    }
}

/// A [`vox_core::LinkSource`] that dials the same remote afresh on every
/// `next_link` — what reconnect-capable clients hand to the connection
/// supervisor. Each dial is a [`connect_with`].
pub struct IrohLinkSource {
    endpoint: iroh::Endpoint,
    remote: iroh::EndpointAddr,
    options: DialOptions,
}

/// Create an [`IrohLinkSource`] dialing `remote` from `endpoint`.
pub fn iroh_link_source(
    endpoint: iroh::Endpoint,
    remote: impl Into<iroh::EndpointAddr>,
) -> IrohLinkSource {
    IrohLinkSource {
        endpoint,
        remote: remote.into(),
        options: DialOptions::default(),
    }
}

impl IrohLinkSource {
    /// Dial with `options` instead of the defaults.
    #[must_use]
    pub const fn with_dial_options(mut self, options: DialOptions) -> Self {
        self.options = options;
        self
    }
}

impl vox_core::LinkSource for IrohLinkSource {
    type Link = IrohLink;

    async fn next_link(&mut self) -> io::Result<vox_core::Attachment<Self::Link>> {
        let link = connect_with(&self.endpoint, self.remote.clone(), self.options).await?;
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
    #[must_use]
    pub const fn new(
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
    #[must_use]
    pub fn with_closed_notifier(mut self, closed: tokio::sync::oneshot::Sender<()>) -> Self {
        self.closed = Some(closed);
        self
    }

    /// The remote peer's endpoint id.
    #[must_use]
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
            Err(
                ReadExactError::FinishedEarly(0)
                | ReadExactError::ReadError(ReadError::ConnectionLost(
                    ConnectionError::ApplicationClosed(_) | ConnectionError::LocallyClosed,
                )),
            ) => return Ok(None),
            // Peer closed the whole QUIC connection: also a close.
            Err(e) => return Err(IrohLinkError(e.to_string())),
        }
        let len = u32::from_be_bytes(len_bytes);
        if len > MAX_FRAME_LEN {
            return Err(IrohLinkError(format!(
                "vox frame of {len} bytes exceeds {MAX_FRAME_LEN}"
            )));
        }
        // `MAX_FRAME_LEN` was already enforced above, so this fits;
        // `try_from` keeps it total on a 16-bit target too.
        let len = usize::try_from(len)
            .map_err(|_| IrohLinkError(format!("vox frame of {len} bytes does not fit usize")))?;
        let mut bytes = vec![0u8; len];
        self.recv
            .read_exact(&mut bytes)
            .await
            .map_err(|e| IrohLinkError(format!("truncated vox frame: {e}")))?;
        Ok(Some(vox_types::Backing::Boxed(bytes.into_boxed_slice())))
    }
}
