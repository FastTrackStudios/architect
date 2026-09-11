//! Client-side connection establishment — the twin of [`axum_ws::serve`].
//!
//! [`axum_ws`](crate::axum_ws) opens with "plain boilerplate that every
//! architect-using server reuses… so a server's `main.rs` only writes the
//! *interesting* part". The client side never got that treatment: the
//! house idiom was `vox_core::initiator_on(link).establish::<Client>()`,
//! and every app wrote the surrounding machinery for itself.
//!
//! Four of them did, independently, and only one learned the lessons:
//!
//! - A page load fans out to dozens of services at once. `establish` per
//!   typed client and that is dozens of `WebSocket`s to one endpoint —
//!   **72 sockets on one page load**, measured in production. One
//!   connection can carry every service on it
//!   ([`LayerRouter`](crate::layer::LayerRouter) dispatches them all), so
//!   typed clients must be cheap views over a shared caller.
//! - A cache alone does not fix that, because an entry can only be
//!   inserted *after* its dial completes: every caller arriving during
//!   the first dial also misses and dials. Concurrent callers have to
//!   await the **same** dial.
//! - A cached connection can die (server restart, dropped socket), so
//!   every access has to check liveness and evict.
//! - Identity belongs to the *connection*, not the call: the server reads
//!   the credential once, at the handshake. A root established
//!   anonymously can never become authenticated, so the credential is
//!   part of the cache key — not doing that hands a signed-in user a
//!   silently anonymous socket.
//!
//! ## Shape
//!
//! [`Pool`] is transport-agnostic: it owns the cache, the single-flight
//! and the liveness check, and takes the dial as a closure. [`ws`] is the
//! cross-target WebSocket dial. [`endpoint`](Endpoint) names what to dial
//! and as whom.
//!
//! ```ignore
//! use architect::connect::{pool, Endpoint};
//!
//! // One connection per (url, identity) — shared by every typed client.
//! let endpoint = Endpoint::url(&vox_url).with_bearer(session_token);
//! let caller = pool().caller(&endpoint).await?;
//! let tasks = TaskServiceClient::new(caller.clone());
//! let files = FilesServiceClient::new(caller);
//! ```

use std::collections::HashMap;
use std::fmt;

pub mod ws;

// The dial future crosses threads on native and never does on wasm (whose
// vox callers are `!Send` by construction). One bound seam, cfg-split, so
// the pool itself stays target-agnostic — the same shape `resource.rs`
// uses.
#[cfg(not(target_arch = "wasm32"))]
mod bounds {
    pub trait MaybeSend: Send {}
    impl<T: Send> MaybeSend for T {}
}
#[cfg(target_arch = "wasm32")]
mod bounds {
    pub trait MaybeSend {}
    impl<T> MaybeSend for T {}
}
use bounds::MaybeSend;

/// What to dial, and as whom.
///
/// The credential is part of the identity, not a per-call header: vox
/// middleware is per *typed client*, so there is no per-call choke point
/// to hang a token on — every client construction would have to remember,
/// and a forgotten one fails **open** and silently. Presenting it once at
/// establish is the only shape that can't be forgotten.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Endpoint {
    /// Where to dial. A `ws://` / `wss://` URL for [`ws`], or any string
    /// a custom dial understands (task keys iroh peers as
    /// `iroh://<endpoint-id>`).
    pub target: String,
    /// The bearer credential to present at the handshake, if any.
    pub bearer: Option<String>,
}

impl Endpoint {
    /// An anonymous endpoint.
    #[must_use]
    pub fn url(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            bearer: None,
        }
    }

    /// Present `bearer` at the handshake.
    ///
    /// An empty token is treated as no token, so a caller can thread an
    /// `Option<String>` straight through without branching.
    #[must_use]
    pub fn with_bearer(mut self, bearer: impl Into<Option<String>>) -> Self {
        self.bearer = bearer.into().filter(|t| !t.is_empty());
        self
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never render the credential — this lands in logs.
        let identity = if self.bearer.is_some() {
            "authenticated"
        } else {
            "anonymous"
        };
        write!(f, "{} ({identity})", self.target)
    }
}

/// Why a connection could not be established.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum ConnectError {
    /// No target configured — an empty URL, usually a missing env var.
    #[error("no endpoint configured")]
    NoEndpoint,
    /// The transport refused or failed to open.
    #[error("dial {endpoint}: {reason}")]
    Dial {
        /// The endpoint that failed, rendered without its credential.
        endpoint: String,
        /// Transport-level detail.
        reason: String,
    },
    /// The socket opened but the vox handshake did not complete.
    #[error("handshake {endpoint}: {reason}")]
    Handshake {
        /// The endpoint that failed, rendered without its credential.
        endpoint: String,
        /// vox-level detail.
        reason: String,
    },
}

/// An established root connection: the caller, plus the handle that keeps
/// it open.
///
/// Dropping the [`vox_core::ConnectionHandle`] tears the connection down,
/// so the pool keeps it alongside the caller for the entry's lifetime.
#[derive(Clone)]
pub struct RootLane {
    /// The established lane's caller. Typed clients are cheap views over
    /// this — build as many as you like, they share one socket.
    pub caller: vox_core::Caller,
    connection: Option<vox_core::ConnectionHandle>,
}

impl RootLane {
    /// The underlying connection handle, if the transport provided one.
    #[must_use]
    pub const fn connection(&self) -> Option<&vox_core::ConnectionHandle> {
        self.connection.as_ref()
    }
}

impl vox_core::FromVoxLane for RootLane {
    // The root lane carries no service of its own; typed clients attach
    // their own descriptors as views over `caller`.
    const SERVICE_NAME: &'static str = "Noop";

    fn from_vox_lane(
        caller: vox_core::Caller,
        connection: Option<vox_core::ConnectionHandle>,
    ) -> Self {
        Self { caller, connection }
    }
}

// ── Pool ────────────────────────────────────────────────────────────────

/// One connection per `(target, identity)`, shared by every typed client.
///
/// See the module docs for why each of the three moving parts — cache,
/// single-flight, liveness — is load-bearing.
#[derive(Default)]
pub struct Pool {
    inner: sync::Shared<PoolState>,
}

#[derive(Default)]
struct PoolState {
    roots: HashMap<Endpoint, RootLane>,
    inflight: HashMap<Endpoint, SharedDial>,
}

#[cfg(target_arch = "wasm32")]
type DialFuture = futures::future::LocalBoxFuture<'static, Result<RootLane, ConnectError>>;
#[cfg(not(target_arch = "wasm32"))]
type DialFuture = futures::future::BoxFuture<'static, Result<RootLane, ConnectError>>;
type SharedDial = futures::future::Shared<DialFuture>;

impl Pool {
    /// An empty pool.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The caller for `endpoint`, dialling over WebSocket if needed.
    ///
    /// # Errors
    ///
    /// [`ConnectError`] if the dial or the vox handshake fails.
    pub async fn caller(&self, endpoint: &Endpoint) -> Result<vox_core::Caller, ConnectError> {
        self.caller_with(endpoint, ws::dial).await
    }

    /// A typed client over the shared connection for `endpoint`.
    ///
    /// Cheap: the connection is established once, and this is a view over
    /// its caller.
    ///
    /// # Errors
    ///
    /// [`ConnectError`] if the dial or the vox handshake fails.
    pub async fn client<C>(&self, endpoint: &Endpoint) -> Result<C, ConnectError>
    where
        C: vox_core::FromVoxLane,
    {
        let caller = self.caller(endpoint).await?;
        Ok(C::from_vox_lane(caller, None))
    }

    /// [`caller`](Self::caller) over a caller-supplied dial.
    ///
    /// The seam for transports that aren't WebSocket — task dials iroh
    /// peers this way, keyed as `iroh://<endpoint-id>`, and gets the same
    /// cache, single-flight and liveness for free.
    ///
    /// # Errors
    ///
    /// Whatever `dial` returns, or [`ConnectError`] from the handshake.
    pub async fn caller_with<D, Fut>(
        &self,
        endpoint: &Endpoint,
        dial: D,
    ) -> Result<vox_core::Caller, ConnectError>
    where
        D: FnOnce(Endpoint) -> Fut + MaybeSend + 'static,
        Fut: Future<Output = Result<RootLane, ConnectError>> + MaybeSend + 'static,
    {
        if let Some(caller) = self.live(endpoint) {
            return Ok(caller);
        }

        // SINGLE-FLIGHT. The cache can only be populated after a dial
        // completes, so without this every caller arriving during the
        // first dial starts its own. That window is exactly a page load.
        let shared = {
            let mut state = self.inner.lock();
            // `if let`, not `map_or_else`: the else arm has a side effect
            // (inserting the new dial), which reads worse as a closure.
            // `as DialFuture` is an unsizing coercion to a boxed future,
            // not a numeric cast.
            #[allow(clippy::option_if_let_else, clippy::as_conversions)]
            if let Some(existing) = state.inflight.get(endpoint) {
                existing.clone()
            } else {
                let fut = Self::dial_once(self.inner.clone(), endpoint.clone(), dial);
                let shared: SharedDial = futures::FutureExt::shared(Box::pin(fut) as DialFuture);
                state.inflight.insert(endpoint.clone(), shared.clone());
                shared
            }
        };

        // Awaited with no lock held — a dial is a network round trip.
        shared.await.map(|root| root.caller)
    }

    /// The dial body: establish, cache, then clear the in-flight slot.
    async fn dial_once<D, Fut>(
        inner: sync::Shared<PoolState>,
        endpoint: Endpoint,
        dial: D,
    ) -> Result<RootLane, ConnectError>
    where
        D: FnOnce(Endpoint) -> Fut,
        Fut: Future<Output = Result<RootLane, ConnectError>>,
    {
        let result = dial(endpoint.clone()).await;
        let mut state = inner.lock();
        // Clear the slot either way, so a later attempt starts fresh.
        // Callers already awaiting this `Shared` still get its result.
        state.inflight.remove(&endpoint);
        if let Ok(root) = &result {
            state.roots.insert(endpoint, root.clone());
        }
        result
    }

    /// A cached caller, if the connection is still alive.
    ///
    /// Validated per access rather than by a generation counter: a pool
    /// sits *below* the app's `Connection`, and multi-endpoint fan-out
    /// reaches it for targets that connection isn't pointed at. The
    /// root's own liveness is the only invariant that always applies.
    fn live(&self, endpoint: &Endpoint) -> Option<vox_core::Caller> {
        let mut state = self.inner.lock();
        match state.roots.get(endpoint) {
            Some(root) if root.caller.is_connected() => Some(root.caller.clone()),
            Some(_) => {
                tracing::warn!(%endpoint, "connect: cached root is dead; re-establishing");
                state.roots.remove(endpoint);
                None
            }
            None => None,
        }
    }

    /// Drop the cached connection for `endpoint`.
    pub fn evict(&self, endpoint: &Endpoint) {
        let mut state = self.inner.lock();
        state.roots.remove(endpoint);
        state.inflight.remove(endpoint);
    }

    /// Drop every cached connection.
    ///
    /// Call on sign-in and sign-out: a connection established under one
    /// identity can never take on another, because the server read the
    /// credential at the handshake.
    pub fn evict_all(&self) {
        let mut state = self.inner.lock();
        state.roots.clear();
        state.inflight.clear();
    }

    /// How many live connections the pool is holding.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().roots.len()
    }

    /// Whether the pool holds no connections.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The process-wide (on wasm, page-wide) default pool.
///
/// Most apps want exactly one: the whole point is that a second pool is a
/// second set of sockets to the same server.
#[must_use]
pub fn pool() -> &'static Pool {
    sync::default_pool()
}

// ── Target-split interior mutability ────────────────────────────────────
//
// wasm is single-threaded and its callers are `!Send`; native is neither.
// One `Shared<T>` seam keeps the pool itself target-agnostic.
#[cfg(target_arch = "wasm32")]
mod sync {
    use super::{Pool, PoolState};
    use std::cell::{RefCell, RefMut};
    use std::rc::Rc;

    pub(super) struct Shared<T>(Rc<RefCell<T>>);

    impl<T: Default> Default for Shared<T> {
        fn default() -> Self {
            Self(Rc::new(RefCell::new(T::default())))
        }
    }
    impl<T> Clone for Shared<T> {
        fn clone(&self) -> Self {
            Self(Rc::clone(&self.0))
        }
    }
    impl<T> Shared<T> {
        pub(super) fn lock(&self) -> RefMut<'_, T> {
            self.0.borrow_mut()
        }
    }

    pub(super) fn default_pool() -> &'static Pool {
        thread_local! {
            static POOL: &'static Pool = Box::leak(Box::new(Pool::new()));
        }
        POOL.with(|p| *p)
    }

    // Kept so the native branch's bound is visible on both targets.
    #[allow(dead_code)]
    fn _assert(_: &PoolState) {}
}

#[cfg(not(target_arch = "wasm32"))]
mod sync {
    use super::{Pool, PoolState};
    use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

    pub(super) struct Shared<T>(Arc<Mutex<T>>);

    impl<T: Default> Default for Shared<T> {
        fn default() -> Self {
            Self(Arc::new(Mutex::new(T::default())))
        }
    }
    impl<T> Clone for Shared<T> {
        fn clone(&self) -> Self {
            Self(Arc::clone(&self.0))
        }
    }
    impl<T> Shared<T> {
        pub(super) fn lock(&self) -> MutexGuard<'_, T> {
            crate::lock::lock(&self.0)
        }
    }

    pub(super) fn default_pool() -> &'static Pool {
        static POOL: OnceLock<Pool> = OnceLock::new();
        POOL.get_or_init(Pool::new)
    }

    #[allow(dead_code)]
    const fn _assert(_: &PoolState) {}
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{ConnectError, Endpoint, Pool, RootLane};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A dial that never touches the network: it counts calls and hands
    /// back a caller from an in-process [`LocalServer`](crate::local::LocalServer).
    ///
    /// `local` is a dev-dependency-shaped feature here; the pool itself
    /// doesn't need it, but a test needs a transport that actually
    /// completes a handshake.
    #[cfg(feature = "local")]
    fn counting_dial(
        dials: &Arc<AtomicUsize>,
        server: &crate::local::LocalServer,
    ) -> impl FnOnce(
        Endpoint,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<RootLane, ConnectError>> + Send>,
    > + use<> {
        let dials = Arc::clone(dials);
        let server = server.clone();
        move |_endpoint| {
            Box::pin(async move {
                dials.fetch_add(1, Ordering::SeqCst);
                server
                    .establish::<RootLane>()
                    .await
                    .map_err(|e| ConnectError::Handshake {
                        endpoint: "local".to_owned(),
                        reason: format!("{e:?}"),
                    })
            })
        }
    }

    /// An empty in-process server — enough to complete a handshake.
    #[cfg(feature = "local")]
    fn local_server() -> (
        crate::local::LocalServer,
        std::sync::Arc<crate::resource::Scope>,
    ) {
        let scope = crate::resource::Scope::new();
        let server =
            crate::local::LocalServer::serve(crate::layer::LayerRouter::new(), scope.clone());
        (server, scope)
    }

    #[test]
    fn identity_is_part_of_the_key() {
        // A root established anonymously can never become authenticated —
        // the server read the credential once, at the handshake. Two
        // identities must therefore never share one entry.
        let anon = Endpoint::url("wss://example/vox");
        let authed = Endpoint::url("wss://example/vox").with_bearer("token".to_owned());
        assert_ne!(anon, authed);

        // …and an empty token is not an identity.
        let empty = Endpoint::url("wss://example/vox").with_bearer(String::new());
        assert_eq!(empty, anon);
    }

    #[test]
    fn display_never_renders_the_credential() {
        let endpoint = Endpoint::url("wss://example/vox").with_bearer("s3cr3t".to_owned());
        let rendered = endpoint.to_string();
        assert!(!rendered.contains("s3cr3t"), "{rendered}");
        assert!(rendered.contains("authenticated"), "{rendered}");
    }

    #[test]
    fn a_pool_starts_empty() {
        let pool = Pool::new();
        assert!(pool.is_empty());
        assert_eq!(pool.len(), 0);
    }

    #[cfg(feature = "local")]
    #[tokio::test]
    async fn concurrent_callers_share_one_dial() {
        // The 72-sockets bug: without single-flight every caller that
        // arrives during the first dial starts its own.
        let pool = Pool::new();
        let (server, scope) = local_server();
        let endpoint = Endpoint::url("memory://one");
        let dials = Arc::new(AtomicUsize::new(0));

        let waves: Vec<_> = (0..16)
            .map(|_| pool.caller_with(&endpoint, counting_dial(&dials, &server)))
            .collect();
        let results = futures::future::join_all(waves).await;

        assert!(results.iter().all(Result::is_ok), "every caller resolves");
        assert_eq!(
            dials.load(Ordering::SeqCst),
            1,
            "16 concurrent callers must share ONE dial"
        );
        assert_eq!(pool.len(), 1, "and leave one cached connection");
        scope.close().await;
    }

    #[cfg(feature = "local")]
    #[tokio::test]
    async fn a_second_call_reuses_the_cached_connection() {
        let pool = Pool::new();
        let (server, scope) = local_server();
        let endpoint = Endpoint::url("memory://two");
        let dials = Arc::new(AtomicUsize::new(0));

        drop(
            pool.caller_with(&endpoint, counting_dial(&dials, &server))
                .await
                .expect("first"),
        );
        drop(
            pool.caller_with(&endpoint, counting_dial(&dials, &server))
                .await
                .expect("second"),
        );

        assert_eq!(dials.load(Ordering::SeqCst), 1, "the second call is cached");
        scope.close().await;
    }

    #[cfg(feature = "local")]
    #[tokio::test]
    async fn different_identities_get_different_connections() {
        let pool = Pool::new();
        let (server, scope) = local_server();
        let dials = Arc::new(AtomicUsize::new(0));
        let anon = Endpoint::url("memory://three");
        let authed = Endpoint::url("memory://three").with_bearer("token".to_owned());

        drop(
            pool.caller_with(&anon, counting_dial(&dials, &server))
                .await
                .expect("anon"),
        );
        drop(
            pool.caller_with(&authed, counting_dial(&dials, &server))
                .await
                .expect("authed"),
        );

        assert_eq!(dials.load(Ordering::SeqCst), 2, "one socket per identity");
        assert_eq!(pool.len(), 2);
        scope.close().await;
    }

    #[cfg(feature = "local")]
    #[tokio::test]
    async fn evict_all_forces_a_fresh_dial() {
        // Sign-in / sign-out: the old identity's sockets must go.
        let pool = Pool::new();
        let (server, scope) = local_server();
        let endpoint = Endpoint::url("memory://four");
        let dials = Arc::new(AtomicUsize::new(0));

        drop(
            pool.caller_with(&endpoint, counting_dial(&dials, &server))
                .await
                .expect("first"),
        );
        pool.evict_all();
        assert!(pool.is_empty());
        drop(
            pool.caller_with(&endpoint, counting_dial(&dials, &server))
                .await
                .expect("second"),
        );

        assert_eq!(dials.load(Ordering::SeqCst), 2);
        scope.close().await;
    }

    #[tokio::test]
    async fn a_failed_dial_is_not_cached() {
        let pool = Pool::new();
        let endpoint = Endpoint::url("memory://five");
        let result = pool
            .caller_with(&endpoint, |_| async { Err(ConnectError::NoEndpoint) })
            .await;
        assert!(result.is_err());
        assert!(pool.is_empty(), "a failure must not poison the cache");
    }
}
