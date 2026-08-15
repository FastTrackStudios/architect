//! Construction layer — lazy, composable builders for the backends a
//! `Layer` binds.
//!
//! `Layer<B>` answers "given a backend, mount its
//! services." This module answers the step *before* that: **build** the
//! backend — `config → pool → repo → service` — with dependencies,
//! build-once sharing, and ordered teardown. It's the Effect-`Layer`
//! idea (dependency wiring + scoped resources) expressed as plain
//! value-level combinators — no `Effect<A,E,R>` monad, no typed
//! requirement tracking.
//!
//! ```ignore
//! let scope = Scope::new();
//! let router = Resource::from_fn(|_| async { load_config() })
//!     .and_then(|cfg| Resource::acquire_release(
//!         Resource::from_fn(move |_| async move { Pool::connect(&cfg.url).await }),
//!         |pool| async move { pool.close().await },   // LIFO finalizer
//!     ))
//!     .map(ExampleRepoStorage::new)                    // pool -> backend
//!     .into_router(&scope).await?;                     // build + Services::into_router
//! // serve router …
//! scope.close().await;                                 // reverse-order teardown
//! ```
//!
//! Backends often share a resource (one pool, many repos) — wrap the
//! shared step in [`Resource::memoize`] so it builds once and every
//! dependent gets the same instance.

use std::future::Future;
use std::sync::{Arc, Mutex};

// ── Send / wasm cfg-split ───────────────────────────────────────────────
//
// Mirrors `layer::DynHandler`: native futures are `+ Send` (tokio
// multi-thread executors), wasm futures are not. `MaybeSend` lets the
// signatures below be written once.

#[cfg(not(target_arch = "wasm32"))]
mod bounds {
    pub trait MaybeSend: Send {}
    impl<T: Send> MaybeSend for T {}
    pub trait MaybeSync: Sync {}
    impl<T: Sync> MaybeSync for T {}
    pub type BoxFuture<'a, T> =
        std::pin::Pin<Box<dyn core::future::Future<Output = T> + Send + 'a>>;
}
#[cfg(target_arch = "wasm32")]
mod bounds {
    pub trait MaybeSend {}
    impl<T> MaybeSend for T {}
    pub trait MaybeSync {}
    impl<T> MaybeSync for T {}
    pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn core::future::Future<Output = T> + 'a>>;
}

use bounds::{BoxFuture, MaybeSend, MaybeSync};

// Boxed builder / finalizer. `MaybeSend` is a custom (non-auto) trait, so
// it can't appear inside a `dyn` object alongside `FnOnce` — spell the
// `Send` bound concretely here, cfg-split for wasm.
#[cfg(not(target_arch = "wasm32"))]
type Finalizer = Box<dyn FnOnce() -> BoxFuture<'static, ()> + Send>;
#[cfg(target_arch = "wasm32")]
type Finalizer = Box<dyn FnOnce() -> BoxFuture<'static, ()>>;

// ── Scope ───────────────────────────────────────────────────────────────

/// Collects async finalizers and runs them in **reverse registration
/// order** (LIFO) on [`close`](Scope::close) — the standard
/// acquire/release discipline, so a pool opened after a config is closed
/// before it.
///
/// Held behind an [`Arc`] so a [`Resource`] graph and the eventual owner
/// (a server's shutdown path) share the same scope.
#[derive(Default)]
pub struct Scope {
    finalizers: Mutex<Vec<Finalizer>>,
}

impl Scope {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Register a finalizer. Runs once, on [`close`](Scope::close), after
    /// every finalizer registered *later* (LIFO).
    pub fn defer<F, Fut>(&self, f: F)
    where
        F: FnOnce() -> Fut + MaybeSend + 'static,
        Fut: Future<Output = ()> + MaybeSend + 'static,
    {
        self.finalizers
            .lock()
            .expect("scope finalizers poisoned")
            .push(Box::new(move || Box::pin(f())));
    }

    /// Run all finalizers in reverse order, draining the scope. Safe to
    /// call once; a second call is a no-op (finalizers already drained).
    pub async fn close(&self) {
        let mut pending = {
            let mut guard = self.finalizers.lock().expect("scope finalizers poisoned");
            std::mem::take(&mut *guard)
        };
        while let Some(f) = pending.pop() {
            f().await;
        }
    }

    /// Number of finalizers currently registered (for tests / metrics).
    pub fn pending(&self) -> usize {
        self.finalizers
            .lock()
            .expect("scope finalizers poisoned")
            .len()
    }
}

// ── Resource ────────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
type BuildFn<T, E> = Box<dyn FnOnce(Arc<Scope>) -> BoxFuture<'static, Result<T, E>> + Send>;
#[cfg(target_arch = "wasm32")]
type BuildFn<T, E> = Box<dyn FnOnce(Arc<Scope>) -> BoxFuture<'static, Result<T, E>>>;

/// A lazy, composable recipe that builds a `T`, optionally registering
/// cleanup on a [`Scope`]. The construction analog of
/// `Layer`: a `Layer` *binds* services; a
/// `Resource` *builds* the backend they bind to.
///
/// `E` defaults to [`eyre::Report`]; name a concrete error for typed
/// failures.
pub struct Resource<T, E = eyre::Report> {
    build: BuildFn<T, E>,
}

impl<T, E> Resource<T, E>
where
    T: MaybeSend + 'static,
    E: MaybeSend + 'static,
{
    /// A resource that's already a value — no work, no failure.
    pub fn succeed(value: T) -> Self {
        Self::from_fn(move |_| async move { Ok(value) })
    }

    /// Build from an async closure. The closure receives the [`Scope`]
    /// so it can register finalizers (or build sub-resources against it).
    pub fn from_fn<F, Fut>(f: F) -> Self
    where
        F: FnOnce(Arc<Scope>) -> Fut + MaybeSend + 'static,
        Fut: Future<Output = Result<T, E>> + MaybeSend + 'static,
    {
        Self {
            build: Box::new(move |scope| Box::pin(f(scope))),
        }
    }

    /// Acquire a resource and register its release on the scope. The
    /// release runs LIFO when the scope closes. Needs `T: Clone` (one
    /// clone for the caller, one for the finalizer).
    pub fn acquire_release<Rel, RelFut>(acquire: Resource<T, E>, release: Rel) -> Self
    where
        T: Clone,
        Rel: FnOnce(T) -> RelFut + MaybeSend + 'static,
        RelFut: Future<Output = ()> + MaybeSend + 'static,
    {
        Self::from_fn(move |scope| async move {
            let value = acquire.build(&scope).await?;
            let held = value.clone();
            scope.defer(move || release(held));
            Ok(value)
        })
    }

    /// Transform the built value.
    pub fn map<U, F>(self, f: F) -> Resource<U, E>
    where
        U: MaybeSend + 'static,
        F: FnOnce(T) -> U + MaybeSend + 'static,
    {
        Resource::from_fn(move |scope| async move {
            let value = self.build(&scope).await?;
            Ok(f(value))
        })
    }

    /// Transform the error.
    pub fn map_err<E2, F>(self, f: F) -> Resource<T, E2>
    where
        E2: MaybeSend + 'static,
        F: FnOnce(E) -> E2 + MaybeSend + 'static,
    {
        Resource::from_fn(move |scope| async move { self.build(&scope).await.map_err(f) })
    }

    /// **Dependent composition.** Build this resource, then build the
    /// next one from its value — the `config → pool → repo` chain. The
    /// same scope threads through, so finalizers from both stack LIFO.
    pub fn and_then<U, F>(self, f: F) -> Resource<U, E>
    where
        U: MaybeSend + 'static,
        F: FnOnce(T) -> Resource<U, E> + MaybeSend + 'static,
    {
        Resource::from_fn(move |scope| async move {
            let value = self.build(&scope).await?;
            f(value).build(&scope).await
        })
    }

    /// **Independent pairing.** Build both (this one first), return the
    /// pair. For resources with no dependency between them.
    pub fn zip<U>(self, other: Resource<U, E>) -> Resource<(T, U), E>
    where
        U: MaybeSend + 'static,
    {
        Resource::from_fn(move |scope| async move {
            let a = self.build(&scope).await?;
            let b = other.build(&scope).await?;
            Ok((a, b))
        })
    }

    /// Run the recipe, building the value and registering any finalizers
    /// on `scope`.
    pub async fn build(self, scope: &Arc<Scope>) -> Result<T, E> {
        (self.build)(Arc::clone(scope)).await
    }
}

impl<T, E> Resource<T, E>
where
    T: Clone + MaybeSend + MaybeSync + 'static,
    E: MaybeSend + 'static,
{
    /// **Build once, share.** Returns a cloneable handle whose first
    /// build runs the recipe and caches the value; every later build (on
    /// any clone) returns that same instance. Use for a pool/config
    /// several backends depend on. Errors are *not* cached — a failed
    /// build can be retried.
    pub fn memoize(self) -> SharedResource<T, E> {
        SharedResource {
            cell: Arc::new(tokio::sync::OnceCell::new()),
            builder: Arc::new(Mutex::new(Some(self.build))),
        }
    }
}

// ── SharedResource (memoized) ───────────────────────────────────────────

/// A memoized [`Resource`] — cloneable, builds at most once. Created by
/// [`Resource::memoize`].
pub struct SharedResource<T, E = eyre::Report> {
    cell: Arc<tokio::sync::OnceCell<T>>,
    builder: Arc<Mutex<Option<BuildFn<T, E>>>>,
}

impl<T, E> Clone for SharedResource<T, E> {
    fn clone(&self) -> Self {
        Self {
            cell: Arc::clone(&self.cell),
            builder: Arc::clone(&self.builder),
        }
    }
}

impl<T, E> SharedResource<T, E>
where
    T: Clone + MaybeSend + MaybeSync + 'static,
    E: MaybeSend + 'static,
{
    /// Build (or return the cached instance). The recipe runs at most
    /// once across all clones; concurrent first-builds are deduplicated.
    pub async fn build(&self, scope: &Arc<Scope>) -> Result<T, E> {
        if let Some(value) = self.cell.get() {
            return Ok(value.clone());
        }
        let scope = Arc::clone(scope);
        let builder = &self.builder;
        let value = self
            .cell
            .get_or_try_init(|| async {
                let build = builder
                    .lock()
                    .expect("memoized builder poisoned")
                    .take()
                    .expect("memoized builder already consumed");
                build(scope).await
            })
            .await?;
        Ok(value.clone())
    }

    /// View this memoized resource as a fresh [`Resource`] so it can feed
    /// further `.map` / `.and_then` chains. Each built copy still shares
    /// the one cached instance.
    pub fn resource(&self) -> Resource<T, E> {
        let shared = self.clone();
        Resource::from_fn(move |scope| async move { shared.build(&scope).await })
    }
}

// ── Bridge into the service layer ───────────────────────────────────────

#[cfg(feature = "vox")]
impl<B, E> Resource<B, E>
where
    B: crate::layer::Services + Clone + Send + Sync + 'static,
    E: MaybeSend + 'static,
{
    /// Build the backend, then mount its [`Services`](crate::layer::Services)
    /// bundle — `Resource<B>` → [`LayerRouter`](crate::layer::LayerRouter).
    /// The terminal step of a construction chain on the server.
    pub async fn into_router(self, scope: &Arc<Scope>) -> Result<crate::layer::LayerRouter, E> {
        let backend = self.build(scope).await?;
        Ok(backend.into_router())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_lite::future::block_on;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn scope_closes_finalizers_lifo() {
        let scope = Scope::new();
        let order = Arc::new(Mutex::new(Vec::<u8>::new()));
        for i in 1..=3u8 {
            let order = Arc::clone(&order);
            scope.defer(move || async move {
                order.lock().unwrap().push(i);
            });
        }
        assert_eq!(scope.pending(), 3);
        block_on(scope.close());
        assert_eq!(*order.lock().unwrap(), vec![3, 2, 1]);
        // second close is a no-op
        block_on(scope.close());
        assert_eq!(*order.lock().unwrap(), vec![3, 2, 1]);
    }

    #[test]
    fn and_then_threads_the_value() {
        let scope = Scope::new();
        let r: Resource<i32> = Resource::succeed(2).and_then(|n: i32| Resource::succeed(n * 10));
        assert_eq!(block_on(r.build(&scope)).unwrap(), 20);
    }

    #[test]
    fn zip_pairs_independent_resources() {
        let scope = Scope::new();
        let r = Resource::<i32>::succeed(1).zip(Resource::<&str>::succeed("a"));
        assert_eq!(block_on(r.build(&scope)).unwrap(), (1, "a"));
    }

    #[test]
    fn acquire_release_registers_finalizer() {
        let scope = Scope::new();
        let released = Arc::new(AtomicUsize::new(0));
        let r = {
            let released = Arc::clone(&released);
            Resource::<i32>::acquire_release(Resource::succeed(7), move |v| {
                let released = Arc::clone(&released);
                async move {
                    assert_eq!(v, 7);
                    released.fetch_add(1, Ordering::SeqCst);
                }
            })
        };
        assert_eq!(block_on(r.build(&scope)).unwrap(), 7);
        assert_eq!(
            released.load(Ordering::SeqCst),
            0,
            "not released until close"
        );
        block_on(scope.close());
        assert_eq!(released.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn memoize_builds_once_across_clones() {
        let scope = Scope::new();
        let builds = Arc::new(AtomicUsize::new(0));
        let shared = {
            let builds = Arc::clone(&builds);
            Resource::<i32>::from_fn(move |_| {
                let builds = Arc::clone(&builds);
                async move {
                    builds.fetch_add(1, Ordering::SeqCst);
                    Ok(42)
                }
            })
            .memoize()
        };
        let a = shared.clone();
        let b = shared.clone();
        assert_eq!(block_on(a.build(&scope)).unwrap(), 42);
        assert_eq!(block_on(b.build(&scope)).unwrap(), 42);
        assert_eq!(block_on(shared.build(&scope)).unwrap(), 42);
        assert_eq!(builds.load(Ordering::SeqCst), 1, "built exactly once");
    }
}
