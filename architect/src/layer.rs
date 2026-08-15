//! Effect-style layer composition — one combinator, [`Layer::merge`].
//!
//! Mirrors Effect-ts: a [`Layer`] is the single composable unit, and
//! [`Layer::merge`] is the only combinator users need. Service tokens
//! emitted by `#[architect::rpc]` are themselves one-element layers,
//! so tokens and pre-built bundles compose the same way.
//!
//! ```ignore
//! use architect::{Layer, layers};
//! use daw_proto::{transport, project, marker};
//!
//! // Build a bundle:
//! let bundle = layers![transport::Service, project::Service, marker::Service];
//!
//! // Bind and route:
//! let router = bundle.provide(Reaper);
//!
//! // Compose sub-bundles via .merge() — same call site shape:
//! let timeline = layers![transport::Service, marker::Service];
//! let routing  = layers![project::Service];
//! let router   = timeline.merge(routing).provide(Reaper);
//!
//! // Override / bolt-on (last-add wins on method_id):
//! let router = layers![transport::Service, project::Service]
//!     .merge(fx_chains::mock())          // override
//!     .merge(dock_host::layer(dh))       // bolt-on, different backend
//!     .provide(Reaper);
//! ```
//!
//! Bundle definitions need **no where clause** — service tokens defer
//! backend binding to `.provide(B)` time. Forgetting an impl surfaces
//! at the `.provide(...)` call site, naming the missing trait.
//!
//! # The pieces
//!
//! - [`BindAny`] — "I know my descriptor." Backend-free.
//! - [`Bind<B>`] — `BindAny` + "given backend B, produce a [`Mounted`]."
//!   Macro-emitted per service.
//! - [`Mounted`] — a service that's been bound. One-element layer.
//! - [`Empty`] / [`Cons<S, R>`] — type-level list of services.
//!   Hidden behind `impl Layer` at function return sites.
//! - [`Layer`] — exposes `merge` / `provide` / `descriptors`.
//!   The `Bind<B>` chain impl is recursive: `Cons<S, R>: Bind<B>`
//!   requires `S: Bind<B>` and `R: Bind<B>`, so a missing per-service
//!   impl surfaces at `.provide(B)` naming the trait.
//! - [`Append<R>`] — type-level concat backing `Layer::merge`.
//! - [`LayerRouter`] — the terminal sink, implements
//!   [`vox::Handler<DriverReplySink>`].
//!
//! # Deployment shapes
//!
//! A trait declared with `#[architect::rpc]` has four deployment
//! shapes, all from the same source. The choice is made at the call
//! site, not at the trait definition.
//!
//! 1. **Direct sync (zero overhead).** Call trait methods on the
//!    backend. No router, no dispatcher, no future. One virtual call
//!    per invocation (monomorphized away in release). Right for
//!    same-thread, can-block hot loops.
//!
//!    ```ignore
//!    let id = Markers::add(&reaper, "intro", 0.0)?;
//!    ```
//!
//! 2. **In-process async (dispatcher-marshaled).** Build a
//!    [`LayerRouter`] via [`Services::into_router`] and call through
//!    the vox-generated `<T>Client`. Calls marshal through the
//!    backend's dispatcher; useful when the caller can't block the
//!    backend's thread (e.g. UI thread → DAW main thread).
//!
//!    ```ignore
//!    let router = Reaper.into_router();
//!    // Pair with a vox::Driver + in-memory transport; clients use
//!    // the same MarkersClient type used over the network.
//!    ```
//!
//! 3. **Cross-process via vox.** The same [`LayerRouter`] is a
//!    `vox::Handler<DriverReplySink>` — plug it into any vox
//!    transport (Unix socket, named pipe, websocket) and external
//!    processes share the client types. Wire encoding is facet, no
//!    serde glue.
//!
//! 4. **HTTP / WebSocket via axum.** Enable `architect`'s
//!    `server-axum` feature and wrap the same router with
//!    `axum_ws::serve` (not linked: the module is feature-gated, so the
//!    intra-doc link wouldn't resolve under every feature combo). Browser
//!    clients use the same `<T>Client` types compiled for wasm.
//!
//! See `examples/layered-services/` for a runnable composition
//! walkthrough and `examples/custom-server/` for the axum mount
//! variant.

use core::any::Any;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use vox::{
    DriverReplySink, Handler, MethodDescriptor, MethodId, RequestCall, SchemaRecvTracker, SelfRef,
    ServiceDescriptor,
};

#[cfg(feature = "telemetry")]
use tracing::Instrument as _;

// ── Erased handler ────────────────────────────────────────────────────────
//
// Send / Sync requirements gated on target_arch — vox's Handler
// future is `+ MaybeSend` (non-Send on wasm32). Native keeps the
// thread bounds for tokio multi-thread executors.

#[cfg(not(target_arch = "wasm32"))]
pub trait DynHandler: Send + Sync + 'static {
    fn handle(
        &self,
        call: SelfRef<RequestCall<'static>>,
        reply: DriverReplySink,
        schemas: Arc<SchemaRecvTracker>,
    ) -> Pin<Box<dyn core::future::Future<Output = ()> + Send + '_>>;

    fn args_have_channels(&self, method_id: MethodId) -> bool;
    fn response_wire_shape(&self, method_id: MethodId) -> Option<&'static facet::Shape>;
    fn as_any(&self) -> &dyn Any;
}

#[cfg(not(target_arch = "wasm32"))]
impl<H> DynHandler for H
where
    H: Handler<DriverReplySink> + Send + Sync + 'static,
{
    fn handle(
        &self,
        call: SelfRef<RequestCall<'static>>,
        reply: DriverReplySink,
        schemas: Arc<SchemaRecvTracker>,
    ) -> Pin<Box<dyn core::future::Future<Output = ()> + Send + '_>> {
        Box::pin(Handler::handle(self, call, reply, schemas))
    }
    fn args_have_channels(&self, method_id: MethodId) -> bool {
        Handler::args_have_channels(self, method_id)
    }
    fn response_wire_shape(&self, method_id: MethodId) -> Option<&'static facet::Shape> {
        Handler::response_wire_shape(self, method_id)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(target_arch = "wasm32")]
pub trait DynHandler: 'static {
    fn handle(
        &self,
        call: SelfRef<RequestCall<'static>>,
        reply: DriverReplySink,
        schemas: Arc<SchemaRecvTracker>,
    ) -> Pin<Box<dyn core::future::Future<Output = ()> + '_>>;

    fn args_have_channels(&self, method_id: MethodId) -> bool;
    fn response_wire_shape(&self, method_id: MethodId) -> Option<&'static facet::Shape>;
    fn as_any(&self) -> &dyn Any;
}

#[cfg(target_arch = "wasm32")]
impl<H> DynHandler for H
where
    H: Handler<DriverReplySink> + 'static,
{
    fn handle(
        &self,
        call: SelfRef<RequestCall<'static>>,
        reply: DriverReplySink,
        schemas: Arc<SchemaRecvTracker>,
    ) -> Pin<Box<dyn core::future::Future<Output = ()> + '_>> {
        Box::pin(Handler::handle(self, call, reply, schemas))
    }
    fn args_have_channels(&self, method_id: MethodId) -> bool {
        Handler::args_have_channels(self, method_id)
    }
    fn response_wire_shape(&self, method_id: MethodId) -> Option<&'static facet::Shape> {
        Handler::response_wire_shape(self, method_id)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ── Mounted ───────────────────────────────────────────────────────────────

/// A service bound to a backend — descriptor + erased handler.
#[derive(Clone)]
pub struct Mounted {
    descriptor: &'static ServiceDescriptor,
    handler: Arc<dyn DynHandler>,
}

impl Mounted {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new<H>(descriptor: &'static ServiceDescriptor, handler: H) -> Self
    where
        H: Handler<DriverReplySink> + Send + Sync + 'static,
    {
        Self {
            descriptor,
            handler: Arc::new(handler),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn new<H>(descriptor: &'static ServiceDescriptor, handler: H) -> Self
    where
        H: Handler<DriverReplySink> + 'static,
    {
        Self {
            descriptor,
            handler: Arc::new(handler),
        }
    }

    pub fn from_arc(descriptor: &'static ServiceDescriptor, handler: Arc<dyn DynHandler>) -> Self {
        Self {
            descriptor,
            handler,
        }
    }

    pub fn descriptor(&self) -> &'static ServiceDescriptor {
        self.descriptor
    }

    pub fn handler(&self) -> &Arc<dyn DynHandler> {
        &self.handler
    }

    pub fn into_parts(self) -> (&'static ServiceDescriptor, Arc<dyn DynHandler>) {
        (self.descriptor, self.handler)
    }
}

impl core::fmt::Debug for Mounted {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Mounted")
            .field("descriptor", &self.descriptor.service_name)
            .finish_non_exhaustive()
    }
}

// ── BindAny / Bind ────────────────────────────────────────────────────────

/// Backend-free trait — "I know my descriptor." Implemented by every
/// service token and by [`Mounted`].
pub trait BindAny {
    fn descriptor(&self) -> &'static ServiceDescriptor;
}

/// Backend-aware bind — "given backend `B`, register this thing
/// (service token, pre-mounted service, or whole chain) into a
/// [`LayerRouter`]."
///
/// One trait for every form of binding. The `#[architect::rpc]`
/// derive emits an impl per service token; [`Empty`] / [`Cons`] /
/// [`Mounted`] get blanket impls in this crate. The chain impl
/// walks recursively, requiring each service to impl `Bind<B>` —
/// the bound check at [`Layer::provide`] cascades through and
/// surfaces missing impls at the call site.
#[diagnostic::on_unimplemented(
    message = "backend `{B}` cannot serve this service",
    label = "no `Bind<{B}>` impl — `{Self}` likely does not implement \
             the underlying RPC trait for `{B}` (or `{B}` is missing a \
             required bound such as `HasDispatcher` / `Send` / `Sync` / \
             `'static`).",
    note = "every service token in a `Layer` must impl `Bind<B>` for \
            the backend you pass to `.provide(B)`. Check the trait \
            impls on `{B}` for this service's underlying trait."
)]
pub trait Bind<B>: BindAny {
    /// Register self into the router. Per-service tokens build a
    /// `Mounted` from `backend.clone()` and call `router.add_mounted`;
    /// chains (`Cons`) walk their elements; `Mounted` registers
    /// itself directly.
    fn bind_into(self, backend: &B, router: &mut LayerRouter);
}

impl BindAny for Mounted {
    fn descriptor(&self) -> &'static ServiceDescriptor {
        self.descriptor
    }
}

impl<B> Bind<B> for Mounted {
    fn bind_into(self, _: &B, router: &mut LayerRouter) {
        router.add_mounted(self);
    }
}

impl BindAny for Empty {
    fn descriptor(&self) -> &'static ServiceDescriptor {
        &ServiceDescriptor::EMPTY
    }
}

impl<B> Bind<B> for Empty {
    fn bind_into(self, _: &B, _: &mut LayerRouter) {}
}

impl<S, R> BindAny for Cons<S, R>
where
    S: BindAny,
{
    fn descriptor(&self) -> &'static ServiceDescriptor {
        self.svc.descriptor()
    }
}

impl<B, S, R> Bind<B> for Cons<S, R>
where
    S: Bind<B>,
    R: Bind<B>,
{
    fn bind_into(self, backend: &B, router: &mut LayerRouter) {
        self.svc.bind_into(backend, router);
        self.rest.bind_into(backend, router);
    }
}

// ── Empty / Cons ──────────────────────────────────────────────────────────

/// Empty layer — base case of the cons chain.
#[derive(Debug, Default, Clone, Copy)]
pub struct Empty;

/// One service cell prepended to a tail layer. Built by
/// [`Layer::merge`] when a service token is merged into a layer.
pub struct Cons<S, R> {
    svc: S,
    rest: R,
}

impl<S, R> Cons<S, R> {
    pub fn new(svc: S, rest: R) -> Self {
        Self { svc, rest }
    }
}

// ── Layer<B> trait ────────────────────────────────────────────────────────

/// The composable, bindable layer for backend `B`.
///
/// Auto-implemented for any type that's [`Bind<B>`], [`Descriptors`],
/// and `Sized` — service tokens (via the `#[architect::rpc]` derive),
/// [`Empty`], [`Cons`], and [`Mounted`]. User code interacts only
/// through the trait's methods.
///
/// `B` is the backend the layer binds to. A single layer expression
/// can satisfy `Layer<B>` for multiple backends (e.g. `Reaper` and
/// `MockReaper`) — Rust picks the right one at `.provide(...)` time.
///
/// ```ignore
/// fn layers() -> impl Layer<Reaper> {
///     layers![transport::Service, project::Service, /* … */]
/// }
/// ```
pub trait Layer<B>: Bind<B> + Descriptors + Sized {
    /// Merge a bound service into this layer. Mirrors Effect-ts's
    /// `Layer.merge` — pass anything convertible into a [`Mounted`]
    /// (a service's `layer(backend)` result, a `mock()` builder,
    /// etc.).
    ///
    /// On duplicate method IDs the **last merged** handler wins —
    /// that's how overrides and mocks compose.
    ///
    /// To compose two cons-chained sub-bundles, use the
    /// [`crate::layers!`] macro instead — it concatenates via
    /// [`Append`] internally.
    fn merge<M>(self, m: M) -> Cons<Mounted, Self>
    where
        M: Into<Mounted>,
    {
        Cons::new(m.into(), self)
    }

    /// Bind a backend and produce a [`LayerRouter`]. The
    /// [`Bind<B>`] supertrait guarantees every service in the chain
    /// can bind — if any can't, the compile error surfaces at this
    /// call site naming the missing trait.
    ///
    /// Per-service `Bind<B>` impls usually require `B: Clone` (each
    /// service clones the backend to build its own `Mounted`). For
    /// non-`Clone` backends, wrap in `Arc<Backend>` and impl the
    /// per-service traits on `Arc<Backend>` (or use `&'static`).
    /// `Copy` backends like REAPER's stateless `Reaper` token pay
    /// nothing.
    fn provide(self, backend: B) -> LayerRouter {
        let mut router = LayerRouter::new();
        self.bind_into(&backend, &mut router);
        router
    }

    /// Collect descriptors of every service in this layer — useful
    /// for capability lists / introspection before binding. The
    /// [`Descriptors`] supertrait guarantees the walk.
    fn descriptors(&self) -> Vec<&'static ServiceDescriptor> {
        let mut v = Vec::new();
        Descriptors::collect(self, &mut v);
        v
    }
}

impl<B, T> Layer<B> for T where T: Bind<B> + Descriptors + Sized {}

// ── Append<R> ─────────────────────────────────────────────────────────────

/// Type-level concat. `<Cons<A, Cons<B, Empty>> as Append<R>>::Output
/// = Cons<A, Cons<B, R>>`. Structural — no [`Layer`] bound, so the
/// `layers!` macro can build cons chains without committing to a
/// backend at the macro site.
pub trait Append<R>: Sized {
    type Output;
    fn append(self, rhs: R) -> Self::Output;
}

impl<R> Append<R> for Empty {
    type Output = R;
    fn append(self, rhs: R) -> R {
        rhs
    }
}

impl<S, T, R> Append<R> for Cons<S, T>
where
    T: Append<R>,
{
    type Output = Cons<S, <T as Append<R>>::Output>;
    fn append(self, rhs: R) -> Self::Output {
        Cons {
            svc: self.svc,
            rest: self.rest.append(rhs),
        }
    }
}

impl<R> Append<R> for Mounted {
    type Output = Cons<Mounted, R>;
    fn append(self, rhs: R) -> Self::Output {
        Cons {
            svc: self,
            rest: rhs,
        }
    }
}

// ── Descriptors ───────────────────────────────────────────────────────────

/// Walks the chain producing each service's descriptor.
pub trait Descriptors {
    fn collect(&self, out: &mut Vec<&'static ServiceDescriptor>);
}

impl Descriptors for Empty {
    fn collect(&self, _: &mut Vec<&'static ServiceDescriptor>) {}
}

impl<S, R> Descriptors for Cons<S, R>
where
    S: BindAny,
    R: Descriptors,
{
    fn collect(&self, out: &mut Vec<&'static ServiceDescriptor>) {
        out.push(self.svc.descriptor());
        self.rest.collect(out);
    }
}

impl Descriptors for Mounted {
    fn collect(&self, out: &mut Vec<&'static ServiceDescriptor>) {
        out.push(self.descriptor);
    }
}

// ── layers! macro ─────────────────────────────────────────────────────────

/// Build a [`Layer`] from a variadic list of layers — service tokens,
/// pre-mounted services, or already-composed sub-bundles all compose
/// uniformly. Rust's analog of Effect-ts's `Layer.mergeAll(...)`.
///
/// ```ignore
/// // Tokens only:
/// let router = layers![
///     transport::Service,
///     project::Service,
///     marker::Service,
/// ].provide(Reaper);
///
/// // Mix tokens, pre-mounted bolt-ons, and sub-bundles:
/// let timeline = layers![transport::Service, marker::Service];
/// let router = layers![
///     timeline,
///     project::Service,
///     dock_host::layer(dock_host_backend),  // pre-mounted, different backend
/// ].provide(Reaper);
/// ```
#[macro_export]
macro_rules! layers {
    () => { $crate::Empty };
    ($($svc:expr),+ $(,)?) => {{
        // Always terminate the cons chain in `Empty` so the per-layer
        // walker trait (`Descriptors`) bottoms out on the `Empty`
        // base impl rather than on the last service token.
        let __l = $crate::Empty;
        $(let __l = $crate::Append::append($svc, __l);)+
        __l
    }};
}

/// Build a [`LayerRouter`] from a list of backends — the app-level
/// mount registry. Each entry is a backend implementing [`Services`];
/// its whole canonical bundle mounts in one line, so registering a new
/// feature backend is one added expression, not a `.with(descriptor,
/// serve)` pair per service:
///
/// ```ignore
/// let router = architect::router![
///     org.scheduling.clone(),   // VaultScheduler: 7 services
///     org.inbox.clone(),        // InboxBackend:   1 service
///     org.agent_codex.clone(),  // CodexBackend:   3 services
/// ];
/// ```
///
/// Later entries win on method-id collision (same rule as
/// [`Layer::merge`]). Bolt-ons that aren't a backend's canonical
/// bundle — a single service on a shared backend, or a
/// middleware-wrapped dispatcher — chain on afterwards:
///
/// ```ignore
/// let router = architect::router![org.scheduling.clone()]
///     .merge(attachments_layer(org.attachments.clone()))
///     .with(auth_descriptor(), auth_dispatcher_with_middleware);
/// ```
#[macro_export]
macro_rules! router {
    ($($backend:expr),* $(,)?) => {{
        let __r = $crate::LayerRouter::new();
        $(let __r = $crate::LayerRouter::merge_router(
            __r,
            $crate::Services::into_router($backend),
        );)*
        __r
    }};
}

// ── Services trait ────────────────────────────────────────────────────────

/// "This backend provides a canonical bundle of services."
///
/// Implement once per backend (REAPER, Pro Tools, mock, …) declaring
/// which services the backend ships as its default surface. Callers
/// then get the full router in one call:
///
/// ```ignore
/// use architect::Services;
///
/// let router = Reaper.into_router();
/// ```
///
/// # Overriding a service
///
/// `LayerRouter` resolves duplicate method-ids by **last-merge wins** —
/// merge the override after the default bundle and it takes effect.
/// The default handler stays in memory but becomes unreachable.
///
/// ```ignore
/// let router = Reaper::layers()
///     .merge(fx_chains::mock())     // overrides the default fx_chains
///     .merge(dock_host::layer(dh))  // bolt-on, different backend
///     .provide(Reaper);
/// ```
///
/// # Sub-bundles
///
/// Compose groups of services with [`Layer::merge`] or `layers![...]`:
///
/// ```ignore
/// let timeline = layers![transport::Service, marker::Service, region::Service];
/// let routing  = layers![project::Service, routing::Service, track::Service];
/// let bundle   = layers![timeline, routing, fx_chains::mock()];
/// let router   = bundle.provide(Reaper);
/// ```
pub trait Services: Sized {
    /// Build the deferred bundle for this backend. Returns an opaque
    /// [`Layer<Self>`] — composable via `.merge(...)`, bindable via
    /// `.provide(self)`, introspectable via `.descriptors()`.
    fn layers() -> impl Layer<Self>;

    /// Convenience: build the bundle, bind `self`, return the
    /// terminal router. One-call mount when no overrides are needed.
    fn into_router(self) -> LayerRouter
    where
        // `MaybeSendSync` is exactly `Send + Sync` on native (byte-for-byte
        // the old bound) and empty on wasm — so a `!Send/!Sync` backend
        // (single-threaded wasm vox, `Rc<RefCell<..>>` sinks) can mount its
        // own router in-process. The generated per-service `Bind<B>` impls
        // already carry `MaybeSendSync`, so this is the last native-only
        // gate on the wasm in-process path.
        Self: Clone + crate::MaybeSendSync + 'static,
    {
        Self::layers().provide(self)
    }
}

// ── LayerSink ─────────────────────────────────────────────────────────────

/// Anything that can absorb a [`Mounted`]. Implemented by
/// [`LayerRouter`]; downstream consumers can implement for custom
/// dispatchers.
pub trait LayerSink {
    fn add_mounted(&mut self, mounted: Mounted);
}

// ── LayerRouter ───────────────────────────────────────────────────────────

/// Method-id-keyed dispatch + canonical [`vox::Handler<DriverReplySink>`]
/// impl. The terminal sink for layers.
#[derive(Default, Clone)]
pub struct LayerRouter {
    method_map: HashMap<MethodId, usize>,
    /// Instance-scoped routes — `(scope, method)` → handler, consulted
    /// before [`method_map`](Self::method_map) when the call carries
    /// `svc-scope` metadata (see [`crate::scoped`]). Lets the same service
    /// trait mount once per instance (one per rig) on one merged router.
    scoped_map: HashMap<(String, MethodId), usize>,
    handlers: Vec<Arc<dyn DynHandler>>,
    /// `MethodId` → its static descriptor, so dispatch can name the call
    /// it is about to make. A `MethodId` is a hash — without this, a span
    /// or a log line can only say "method 0x8f3a…", which is useless when
    /// you are staring at a trace trying to find the slow call.
    ///
    /// Built from the same descriptors [`register`](Self::register)
    /// already walks, so it costs one extra pointer-sized insert per
    /// method at mount time and nothing at dispatch.
    method_names: HashMap<MethodId, &'static MethodDescriptor>,
}

impl LayerRouter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Lower-level entry — prefer [`Layer::provide`] for bundles.
    ///
    /// The handler bound tracks [`DynHandler`]: `Send + Sync` on native,
    /// dropped on wasm (single-threaded vox), so an in-process
    /// [`crate::local::LocalServer`] router can be hand-assembled in the
    /// browser from `!Send` service handlers.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with<H>(mut self, descriptor: &'static ServiceDescriptor, handler: H) -> Self
    where
        H: Handler<DriverReplySink> + Send + Sync + 'static,
    {
        self.register(descriptor, Arc::new(handler));
        self
    }

    #[cfg(target_arch = "wasm32")]
    pub fn with<H>(mut self, descriptor: &'static ServiceDescriptor, handler: H) -> Self
    where
        H: Handler<DriverReplySink> + 'static,
    {
        self.register(descriptor, Arc::new(handler));
        self
    }

    /// Runtime bolt-on: merge a [`Mounted`] (or anything `Into<Mounted>`,
    /// like a service's `layer(backend)` result) into this already-built
    /// router. Parallels [`Layer::merge`] for the
    /// already-provided-then-extended case, e.g. loading a plugin
    /// service after the main bundle is mounted. Last-merge wins on
    /// duplicate method IDs.
    pub fn merge<M: Into<Mounted>>(mut self, m: M) -> Self {
        let (descriptor, handler) = m.into().into_parts();
        self.register(descriptor, handler);
        self
    }

    /// Absorb every handler from another built router. This is the
    /// multi-backend composition verb: each backend mounts its own
    /// canonical bundle (`Services::into_router`), and the app stitches
    /// the routers together — one line per backend, see [`router!`].
    /// `other`'s method IDs win on collision, consistent with
    /// [`merge`](Self::merge)'s last-merge-wins.
    pub fn merge_router(mut self, other: LayerRouter) -> Self {
        let base = self.handlers.len();
        self.handlers.extend(other.handlers);
        for (id, idx) in other.method_map {
            self.method_map.insert(id, base + idx);
        }
        for ((scope, id), idx) in other.scoped_map {
            self.scoped_map.insert((scope, id), base + idx);
        }
        self.method_names.extend(other.method_names);
        self
    }

    /// Absorb every handler from `other` under an instance `scope`: its
    /// methods only match calls carrying `svc-scope = scope` metadata
    /// ([`crate::scoped::ScopeMiddleware`]), so the same service trait can
    /// mount once per instance on one merged router without method-id
    /// collisions. `other`'s already-scoped entries keep their own scopes.
    pub fn merge_router_scoped(mut self, scope: &str, other: LayerRouter) -> Self {
        let base = self.handlers.len();
        self.handlers.extend(other.handlers);
        for (id, idx) in other.method_map {
            self.scoped_map.insert((scope.to_string(), id), base + idx);
        }
        for ((inner, id), idx) in other.scoped_map {
            self.scoped_map.insert((inner, id), base + idx);
        }
        self.method_names.extend(other.method_names);
        self
    }

    /// The span covering one dispatched RPC call.
    ///
    /// This is the single choke point every vox call passes through, which
    /// makes it the one place worth instrumenting: without it a whole
    /// WebSocket session is one opaque HTTP span, and every RPC inside it
    /// — the calls that actually do the work and actually fail — is
    /// invisible. With it, each call is a timed, named span carrying the
    /// service and method, so "which call was slow" and "which call
    /// errored" are answerable from the trace alone.
    ///
    /// `otel.name` is the field `tracing-opentelemetry` reads to name the
    /// exported span, so traces group per method (`Projects/list`) rather
    /// than collapsing into one `rpc` bucket. It is inert without that
    /// layer installed.
    ///
    /// Note the span measures *dispatch*, not the reply: the handler owns
    /// the [`DriverReplySink`] and answers on its own schedule, so a
    /// handler error is not visible here. Errors surface as `tracing`
    /// events emitted inside the handler, which land as child log records
    /// of this span and so carry its trace id.
    #[cfg(feature = "telemetry")]
    fn call_span(&self, method_id: MethodId, scope: Option<&str>) -> tracing::Span {
        let (service, method) = self
            .method_names
            .get(&method_id)
            .map(|d| (d.service_name, d.method_name))
            .unwrap_or(("unknown", "unknown"));
        tracing::info_span!(
            "rpc",
            otel.name = format!("{service}/{method}"),
            rpc.system = "vox",
            rpc.service = service,
            rpc.method = method,
            // Which rig/instance answered, when the same trait is mounted
            // more than once (see `merge_router_scoped`).
            rpc.scope = scope.unwrap_or_default(),
        )
    }

    fn register(&mut self, descriptor: &ServiceDescriptor, handler: Arc<dyn DynHandler>) {
        let idx = self.handlers.len();
        self.handlers.push(handler);
        for method in descriptor.methods {
            self.method_map.insert(method.id, idx);
            self.method_names.insert(method.id, method);
        }
    }

    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }

    /// A lane acceptor that dispatches every incoming lane onto a clone of
    /// this router — the one call every transport consumer needs. Collapses
    /// the `lane_acceptor_fn(|_, conn| conn.handle_with(router.clone()))`
    /// boilerplate that engine binaries otherwise repeat per transport; see
    /// [`crate::axum_ws::serve_router`] / [`crate::iroh_link::serve_router`],
    /// which wrap this directly. Available on wasm too now that the
    /// in-process [`crate::local::LocalServer`] serves a router in the
    /// browser — [`handler_acceptor`] has a wasm arm that drops the thread
    /// bounds (single-threaded wasm vox).
    pub fn acceptor(&self) -> impl vox::LaneAcceptor {
        handler_acceptor(self.clone())
    }
}

/// A lane acceptor that dispatches every incoming lane onto a clone of
/// `handler` — any vox [`vox::Handler`], whether a [`LayerRouter`] or a wrapper
/// around one (e.g. a snapshot-gating router). The single home for the
/// `lane_acceptor_fn(|_, conn| conn.handle_with(handler.clone()))` closure
/// every transport consumer otherwise repeats; [`crate::axum_ws::serve_router`]
/// and [`crate::iroh_link::serve_router`] wrap it.
///
/// On native the acceptor is moved onto a serving task, so it needs the
/// `Send + Sync` bounds that only hold off wasm; the wasm arm below drops
/// them (single-threaded wasm vox, where `vox::lane_acceptor_fn`'s bounds
/// are already `MaybeSend + MaybeSync` = empty). Both arms are otherwise
/// identical — the in-process [`crate::local::LocalServer`] uses this on
/// both targets.
#[cfg(not(target_arch = "wasm32"))]
pub fn handler_acceptor<H>(handler: H) -> impl vox::LaneAcceptor
where
    H: Handler<DriverReplySink> + Clone + Send + Sync + 'static,
{
    vox::lane_acceptor_fn(move |_req, connection| {
        connection.handle_with(handler.clone());
        Ok(())
    })
}

#[cfg(target_arch = "wasm32")]
pub fn handler_acceptor<H>(handler: H) -> impl vox::LaneAcceptor
where
    H: Handler<DriverReplySink> + Clone + 'static,
{
    vox::lane_acceptor_fn(move |_req, connection| {
        connection.handle_with(handler.clone());
        Ok(())
    })
}

impl LayerSink for LayerRouter {
    fn add_mounted(&mut self, mounted: Mounted) {
        let (descriptor, handler) = mounted.into_parts();
        self.register(descriptor, handler);
    }
}

impl LayerRouter {
    /// Any handler index that can answer shape queries for `method_id` —
    /// the flat map first, else any scoped mount of the same method (scoped
    /// instances share the trait, hence the shapes).
    fn shape_handler(&self, method_id: MethodId) -> Option<usize> {
        if let Some(&idx) = self.method_map.get(&method_id) {
            return Some(idx);
        }
        self.scoped_map
            .iter()
            .find(|((_, id), _)| *id == method_id)
            .map(|(_, &idx)| idx)
    }
}

impl Handler<DriverReplySink> for LayerRouter {
    fn args_have_channels(&self, method_id: MethodId) -> bool {
        self.shape_handler(method_id)
            .map(|idx| self.handlers[idx].args_have_channels(method_id))
            .unwrap_or(false)
    }

    fn response_wire_shape(&self, method_id: MethodId) -> Option<&'static facet::Shape> {
        self.shape_handler(method_id)
            .and_then(|idx| self.handlers[idx].response_wire_shape(method_id))
    }

    async fn handle(
        &self,
        call: SelfRef<RequestCall<'static>>,
        reply: DriverReplySink,
        schemas: Arc<SchemaRecvTracker>,
    ) {
        let method_id = call.get().method_id;
        // Scoped dispatch first: a call stamped `svc-scope` prefers the
        // matching instance; the flat map serves everything else.
        let scope = crate::scoped::scope_of(&call.get().metadata);
        // The span outlives the borrow — `call` is moved into the handler
        // below — so telemetry builds keep an owned copy. Only telemetry
        // builds pay for it.
        #[cfg(feature = "telemetry")]
        let scope_owned = scope.map(str::to_owned);
        let scoped = scope
            .and_then(|scope| self.scoped_map.get(&(scope.to_string(), method_id)))
            .copied();
        if let Some(idx) = scoped.or_else(|| self.method_map.get(&method_id).copied()) {
            #[cfg(feature = "telemetry")]
            {
                self.handlers[idx]
                    .handle(call, reply, schemas)
                    .instrument(self.call_span(method_id, scope_owned.as_deref()))
                    .await;
            }
            #[cfg(not(feature = "telemetry"))]
            self.handlers[idx].handle(call, reply, schemas).await;
        } else {
            use vox::ReplySink as _;
            reply
                .send_error(vox::VoxError::<core::convert::Infallible>::UnknownMethod)
                .await;
        }
    }
}
