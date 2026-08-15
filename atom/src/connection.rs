//! [`Connection`] — generic connection state for a typed client bundle.
//!
//! Every feature used to hand-roll its own `enum ConnState { Connecting,
//! Ready(Clients), Failed(String) }` + `use_conn()` pair. `Connection<C>`
//! is that pattern once, generic over the client type: the shell
//! establishes the clients (with whatever transport + retry policy) and
//! resolves the connection; feature hooks read it from context.
//!
//! ```ignore
//! // app root — one line per client bundle:
//! use_connect(|| async { transport::connect(&transport).await });
//!
//! // a feature data hook:
//! let conn = use_connection::<ExampleRepoClient>();
//! let Some(client) = conn.ready() else { return /* still connecting */ };
//! ```

use dioxus::prelude::*;

/// The lifecycle of one typed client bundle.
#[derive(Clone, PartialEq, Debug)]
pub enum ConnectionState<C> {
    /// The connect (+ retry policy) hasn't resolved yet.
    Connecting,
    /// The clients are established and ready to call.
    Ready(C),
    /// Connecting failed after the configured retries.
    Failed(String),
}

/// A `Copy` handle to a [`ConnectionState`] provided at the app root.
pub struct Connection<C: 'static> {
    state: Signal<ConnectionState<C>>,
    /// Bumped on every transition into `Ready` — see [`Connection::generation`].
    generation: Signal<u64>,
}

impl<C: 'static> Clone for Connection<C> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<C: 'static> Copy for Connection<C> {}

impl<C: Clone + 'static> Connection<C> {
    /// The current state (clones the bundle — client handles are cheap).
    pub fn state(&self) -> ConnectionState<C> {
        self.state.read().clone()
    }

    /// The clients if the connection is up, `None` while connecting or
    /// after a failure. The reactive read: a hook that calls this re-runs
    /// when the connection resolves.
    pub fn ready(&self) -> Option<C> {
        match &*self.state.read() {
            ConnectionState::Ready(c) => Some(c.clone()),
            _ => None,
        }
    }

    /// The connect failure, if there is one.
    pub fn error(&self) -> Option<String> {
        match &*self.state.read() {
            ConnectionState::Failed(e) => Some(e.clone()),
            _ => None,
        }
    }

    /// True until the connect future settles.
    pub fn is_connecting(&self) -> bool {
        matches!(&*self.state.read(), ConnectionState::Connecting)
    }

    /// How many times this connection has (re-)entered `Ready`. Reactive
    /// read — a hook that calls this re-runs on every re-establish.
    ///
    /// Semantics: starts at `0` (never connected), bumps by one on **every**
    /// successful establish — including reconnects after a death *and*
    /// re-establishes triggered by a reactive connect (org switch, URL
    /// edit). Monotonic for the lifetime of the providing component, so
    /// caches keyed on a caller can compare generations to decide whether a
    /// cached value still belongs to the live connection.
    pub fn generation(&self) -> u64 {
        (self.generation)()
    }

    /// [`generation`](Connection::generation) without subscribing — for
    /// async drive loops. Inside a `use_resource` future a reactive read
    /// would re-subscribe the resource and **restart it** on every
    /// re-establish (dioxus polls resource futures in a reactive
    /// context); inside a `use_future` the subscription is silently
    /// useless. Loops should peek.
    pub fn generation_now(&self) -> u64 {
        *self.generation.peek()
    }

    // ── Supervisor-facing setters ───────────────────────────────────────
    //
    // The connect hooks below (and `architect`'s supervised variant) drive
    // the state machine through these so the transitions stay consistent:
    // `Ready` always bumps the generation, and the non-`Ready` setters are
    // change-aware (they return whether they wrote) so supervisors can log
    // on state *change*, not on every retry attempt.

    /// Transition into `Ready(c)`, bumping the generation. Always writes —
    /// a fresh bundle replaces a stale one even mid-`Ready`.
    pub fn set_ready(&self, c: C) {
        let mut state = self.state;
        let mut generation = self.generation;
        state.set(ConnectionState::Ready(c));
        generation += 1;
    }

    /// Transition into `Connecting`. Returns `true` if the state actually
    /// changed (it was not already `Connecting`).
    pub fn set_connecting(&self) -> bool {
        if matches!(&*self.state.peek(), ConnectionState::Connecting) {
            return false;
        }
        let mut state = self.state;
        state.set(ConnectionState::Connecting);
        true
    }

    /// Transition into `Failed(err)`. Returns `true` if the state actually
    /// changed (different variant, or a different error message).
    pub fn set_failed(&self, err: String) -> bool {
        if matches!(&*self.state.peek(), ConnectionState::Failed(e) if *e == err) {
            return false;
        }
        let mut state = self.state;
        state.set(ConnectionState::Failed(err));
        true
    }
}

/// Pull the connection for client bundle `C` that the shell provided with
/// [`use_connect`].
pub fn use_connection<C: 'static>() -> Connection<C> {
    use_context::<Connection<C>>()
}

/// Create an unresolved `Connection<C>` (state `Connecting`, generation 0)
/// and provide it as context. The building block the connect hooks share —
/// and the entry point for *external* supervisors (e.g. `architect`'s
/// `use_connect_supervised`) that drive the state machine through
/// [`Connection::set_ready`] / [`set_connecting`](Connection::set_connecting)
/// / [`set_failed`](Connection::set_failed).
pub fn use_connection_root<C: 'static>() -> Connection<C> {
    let state = use_signal(|| ConnectionState::<C>::Connecting);
    let generation = use_signal(|| 0u64);
    use_context_provider(|| Connection { state, generation })
}

/// Reactive sibling of [`use_connect`]: re-runs `connect` whenever a
/// signal it reads **synchronously** (before its first await) changes —
/// an org/workspace switcher, an editable server URL — resetting the
/// connection to `Connecting` and re-establishing.
///
/// ```ignore
/// // app root: reconnects when the active org changes
/// use_connect_reactive(move || {
///     let slug = active_org_slug();          // signal read → dependency
///     async move { connect_org(&slug).await }
/// });
/// ```
///
/// Dependency tracking is dioxus-resource semantics: signals read by the
/// closure re-trigger the connect. Note that dioxus polls the resource's
/// future in a reactive context, so signal reads **inside the future**
/// are tracked as dependencies too — prefer `peek` in the async body for
/// reads that must not re-trigger.
///
/// This variant settles **once** per (re-)trigger — it has no death
/// detection. For a connection that must survive a server restart, use
/// `architect::use_connect_supervised`, which layers death-watch +
/// backoff-with-jitter reconnection on top of the same state machine.
pub fn use_connect_reactive<C, F, Fut>(connect: F) -> Connection<C>
where
    C: Clone + 'static,
    F: Fn() -> Fut + 'static,
    Fut: std::future::Future<Output = Result<C, String>> + 'static,
{
    let conn = use_connection_root::<C>();
    let _resource = use_resource(move || {
        // Synchronous part: signal reads here register as dependencies.
        let fut = connect();
        async move {
            conn.set_connecting();
            match fut.await {
                Ok(c) => conn.set_ready(c),
                Err(e) => {
                    conn.set_failed(e);
                }
            }
        }
    });
    conn
}

/// Establish a client bundle once at the app root and provide it as
/// context: starts `Connecting`, runs `connect` on mount, resolves to
/// `Ready`/`Failed`. Call once per bundle type `C`; features pull it with
/// [`use_connection::<C>()`](use_connection).
pub fn use_connect<C, F, Fut>(connect: F) -> Connection<C>
where
    C: Clone + 'static,
    F: FnOnce() -> Fut + 'static,
    Fut: std::future::Future<Output = Result<C, String>> + 'static,
{
    let conn = use_connection_root::<C>();
    // One-shot connect on mount. `use_future` (not use_resource): the
    // connect closure is FnOnce and shouldn't restart reactively — the
    // retry policy lives inside it.
    let mut connect = use_hook(|| CopyValue::new(Some(connect)));
    use_future(move || async move {
        let Some(connect) = connect.write().take() else {
            return;
        };
        match connect().await {
            Ok(c) => conn.set_ready(c),
            Err(e) => {
                conn.set_failed(e);
            }
        }
    });
    conn
}
