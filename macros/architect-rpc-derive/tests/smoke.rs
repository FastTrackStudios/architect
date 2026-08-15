//! End-to-end tests for `#[architect::rpc]` codegen.
//!
//! Tests run without the `vox` cargo feature, so the mirror trait is
//! a plain async trait (no `#[vox::service]` decoration). That lets us
//! exercise the bridge logic in isolation — wiring it onto a real vox
//! link is covered by the architect example crates.
//!
//! The public surface the macro exposes is three nouns + one verb:
//!
//! - the user's trait (e.g. `AllSync`)
//! - `<T>Client` (auto, vox-emitted) — for callers
//! - `serve` (auto, in the trait's module) — for mounting
//!
//! Each `#[rpc]` trait lives in its own module — the macro emits a
//! `Service` token at the trait's module scope, so two `#[rpc]`
//! traits in the same module would collide on `Service` /
//! `BindAny`/`Bind`/`Append`/`Descriptors` impls. Real consumers
//! (scheduling-proto, finance-proto, …) already structure
//! per-capability submodules; the tests mirror that.
//!
//! The internal bridge (`__<T>Bridge`) is `#[doc(hidden)]` and isn't
//! intended for direct use, but tests reach for it to exercise the
//! bridge body without going through vox::service.

use architect::rpc;

// ── All-sync trait ──────────────────────────────────────────────────

mod all_sync {
    use std::sync::Mutex;

    use architect::dispatch::CurrentThreadDispatcher;

    use super::rpc;

    #[rpc]
    pub trait AllSync {
        fn read(&self, key: u32) -> Option<String>;
        fn write(&self, key: u32, value: String) -> Result<(), String>;
        fn echo_str(&self, s: &str) -> String;
    }

    #[derive(Default)]
    struct AllSyncBackend {
        store: Mutex<Vec<(u32, String)>>,
    }

    impl AllSync for AllSyncBackend {
        fn read(&self, key: u32) -> Option<String> {
            self.store
                .lock()
                .unwrap()
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone())
        }

        fn write(&self, key: u32, value: String) -> Result<(), String> {
            self.store.lock().unwrap().push((key, value));
            Ok(())
        }

        fn echo_str(&self, s: &str) -> String {
            s.to_string()
        }
    }

    #[test]
    fn all_sync_bridge_marshals_through_dispatcher() {
        let bridge = __AllSyncBridge::new(AllSyncBackend::default(), CurrentThreadDispatcher);

        // The bridge implements the AllSyncRpc mirror; calling its async
        // methods runs the underlying sync ops through the dispatcher.
        futures_lite::future::block_on(async {
            AllSyncRpc::write(&bridge, 1, "hello".into()).await.unwrap();
            let v = AllSyncRpc::read(&bridge, 1).await;
            assert_eq!(v.as_deref(), Some("hello"));

            // Borrowed-arg path: `&str` was rewritten to `String` in the
            // mirror; the bridge passes `&owned` back into the sync trait.
            let echoed = AllSyncRpc::echo_str(&bridge, "ping".into()).await;
            assert_eq!(echoed, "ping");
        });
    }

    #[test]
    fn user_trait_remains_directly_callable_in_process() {
        // The whole point of #[architect::rpc] is that the user-written
        // trait still works as a plain sync API — no .await, no bridge.
        let backend = AllSyncBackend::default();
        backend.write(7, "direct".into()).unwrap();
        assert_eq!(backend.read(7).as_deref(), Some("direct"));
        assert_eq!(backend.echo_str("x"), "x");
    }
}

// ── All-async trait ─────────────────────────────────────────────────

mod all_async {
    use std::sync::{Arc, Mutex};

    use super::rpc;

    // `async_fn_in_trait` is the whole point of the AllAsync shape —
    // vox::service is applied directly and rewrites the futures itself.
    #[rpc]
    #[allow(async_fn_in_trait)]
    pub trait AllAsync {
        async fn read(&self, key: u32) -> Option<String>;
        async fn write(&self, key: u32, value: String) -> Result<(), String>;
    }

    // Backend is `Clone` because the vox-enabled `Service::bind_into`
    // path clones the backend to seed the layer's dispatcher. Arc
    // around the inner state keeps the writes visible across clones.
    #[derive(Default, Clone)]
    struct AllAsyncBackend {
        store: Arc<Mutex<Vec<(u32, String)>>>,
    }

    impl AllAsync for AllAsyncBackend {
        async fn read(&self, key: u32) -> Option<String> {
            self.store
                .lock()
                .unwrap()
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone())
        }

        async fn write(&self, key: u32, value: String) -> Result<(), String> {
            self.store.lock().unwrap().push((key, value));
            Ok(())
        }
    }

    #[test]
    fn all_async_bridge_passes_through() {
        let bridge = __AllAsyncBridge::new(AllAsyncBackend::default());

        futures_lite::future::block_on(async {
            AllAsync::write(&bridge, 1, "async".into()).await.unwrap();
            let v = AllAsync::read(&bridge, 1).await;
            assert_eq!(v.as_deref(), Some("async"));
        });
    }
}

// ── Mixed trait ─────────────────────────────────────────────────────

mod mixed {
    use std::sync::Mutex;

    use architect::dispatch::CurrentThreadDispatcher;

    use super::rpc;

    #[rpc]
    pub trait Mixed {
        fn read(&self, key: u32) -> Option<String>;
        async fn write(&self, key: u32, value: String) -> Result<(), String>;
    }

    #[derive(Default)]
    struct MixedBackend {
        store: Mutex<Vec<(u32, String)>>,
    }

    impl Mixed for MixedBackend {
        fn read(&self, key: u32) -> Option<String> {
            self.store
                .lock()
                .unwrap()
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone())
        }

        async fn write(&self, key: u32, value: String) -> Result<(), String> {
            self.store.lock().unwrap().push((key, value));
            Ok(())
        }
    }

    #[test]
    fn mixed_bridge_marshals_sync_and_passes_async() {
        let bridge = __MixedBridge::new(MixedBackend::default(), CurrentThreadDispatcher);

        futures_lite::future::block_on(async {
            MixedRpc::write(&bridge, 1, "x".into()).await.unwrap();
            let v = MixedRpc::read(&bridge, 1).await;
            assert_eq!(v.as_deref(), Some("x"));
        });
    }
}

// ── #[derive(HasDispatcher)] ────────────────────────────────────────

mod derived_dispatcher {
    use std::sync::Mutex;

    // One `use` pulls both namespaces: the trait (type ns) and the
    // derive (macro ns) share the `architect::HasDispatcher` path.
    use architect::HasDispatcher;
    use architect::dispatch::{self, CurrentThreadDispatcher};

    use super::rpc;

    #[rpc]
    pub trait Counter {
        fn add(&self, n: u32) -> u32;
    }

    #[derive(Default, HasDispatcher)]
    #[dispatch(CurrentThreadDispatcher)]
    struct ExplicitBackend {
        total: Mutex<u32>,
    }

    impl Counter for ExplicitBackend {
        fn add(&self, n: u32) -> u32 {
            let mut total = self.total.lock().unwrap();
            *total += n;
            *total
        }
    }

    // No #[dispatch] attr → DefaultDispatcher. Which concrete type that
    // is depends on architect's `dispatch-tokio` feature; the test only
    // asserts the impl exists and constructs.
    #[derive(Default, HasDispatcher)]
    struct DefaultBackend;

    impl Counter for DefaultBackend {
        fn add(&self, n: u32) -> u32 {
            n
        }
    }

    #[test]
    fn derive_with_explicit_dispatch_attr_round_trips() {
        let backend = ExplicitBackend::default();
        let dispatcher = backend.dispatcher();
        let bridge = __CounterBridge::new(backend, dispatcher);

        futures_lite::future::block_on(async {
            assert_eq!(CounterRpc::add(&bridge, 2).await, 2);
            assert_eq!(CounterRpc::add(&bridge, 3).await, 5);
        });
    }

    #[test]
    fn derive_without_attr_uses_default_dispatcher() {
        let backend = DefaultBackend;
        // Type-level assertion: the derived impl names DefaultDispatcher.
        let _dispatcher: dispatch::DefaultDispatcher = backend.dispatcher();
    }

    #[test]
    fn derive_works_on_generic_backends() {
        #[derive(HasDispatcher)]
        #[dispatch(CurrentThreadDispatcher)]
        struct Generic<T: Send + Sync + 'static> {
            _inner: T,
        }

        let backend = Generic { _inner: 7u8 };
        let _dispatcher: CurrentThreadDispatcher = backend.dispatcher();
    }
}

// ── prelude module ──────────────────────────────────────────────────

mod prelude_reexports {
    use super::rpc;

    pub mod ping {
        #[super::rpc]
        pub trait Ping {
            fn ping(&self) -> u32;
        }
    }

    // Without the `vox` feature the prelude re-exports just the trait —
    // the client/dispatcher/Service items don't exist. The glob is the
    // whole point: proto crates write one `pub use m::prelude::*` per
    // service instead of hand-renaming five items.
    use ping::prelude::*;

    struct Backend;

    impl Ping for Backend {
        fn ping(&self) -> u32 {
            42
        }
    }

    #[test]
    fn prelude_glob_exposes_the_trait() {
        assert_eq!(Backend.ping(), 42);
    }
}

// ── #[subscribe] declarations ───────────────────────────────────────

mod subscriptions {
    // Facet because the `--all-features` build enables this crate's
    // `vox` feature, which materializes the stream sibling (PubSub
    // bounds the event on `Clone + Facet`). In the default no-vox
    // build only the (stripped) declaration references the type.
    #[allow(dead_code)]
    #[derive(Clone, Debug, PartialEq, facet::Facet)]
    pub struct TickEvent(pub u64);

    pub mod ticker {
        #[super::super::rpc]
        pub trait Ticker {
            fn count(&self) -> u64;

            /// Stream declaration — stripped from the user trait; the
            /// async subscribe RPC lives on the `TickerStream` sibling
            /// (vox-gated, so absent in this no-vox test build).
            #[subscribe]
            fn ticks(&self) -> super::TickEvent;
        }
    }

    use ticker::prelude::*;

    struct Backend;

    // Implementing only `count` proves `ticks` was removed from the
    // user trait — a `#[subscribe]` marker is not a callable method.
    impl Ticker for Backend {
        fn count(&self) -> u64 {
            7
        }
    }

    #[test]
    fn subscribe_declarations_are_stripped_from_the_user_trait() {
        assert_eq!(Backend.count(), 7);
    }
}

// ── #[rpc(sync_client)] opt-in ──────────────────────────────────────

mod sync_client_arg {
    use super::rpc;

    // The facade itself is vox-gated (absent in this build); this
    // proves the argument parses and the trait still works.
    #[rpc(sync_client)]
    pub trait Adder {
        fn add(&self, a: u32, b: u32) -> u32;
    }

    struct Backend;

    impl Adder for Backend {
        fn add(&self, a: u32, b: u32) -> u32 {
            a + b
        }
    }

    #[test]
    fn sync_client_arg_parses_and_trait_remains_callable() {
        assert_eq!(Backend.add(2, 3), 5);
    }
}

// ── context = Type ──────────────────────────────────────────────────

mod ambient_context {
    use std::sync::Mutex;

    use architect::dispatch::CurrentThreadDispatcher;

    // Facet: the context crosses the wire as the first argument of
    // every call, so the vox-enabled build needs it encodable.
    #[derive(Clone, Debug, PartialEq, facet::Facet)]
    pub struct Project(pub u32);

    pub mod store {
        #[super::super::rpc(context = super::Project)]
        pub trait Store {
            fn put(&self, key: u32, value: String) -> bool;
            fn get(&self, key: u32) -> Option<String>;
        }
    }

    use store::prelude::*;
    // Bridge + mirror are module-scoped (the prelude carries the
    // public surface; tests reach for the internals).
    use store::{__StoreBridge, StoreRpc};

    #[derive(Default)]
    struct Backend {
        rows: Mutex<Vec<(u32, u32, String)>>,
    }

    // Backends receive the declared-once context as the first
    // parameter — `&Project` on sync methods.
    impl Store for Backend {
        fn put(&self, ctx: &Project, key: u32, value: String) -> bool {
            self.rows.lock().unwrap().push((ctx.0, key, value));
            true
        }

        fn get(&self, ctx: &Project, key: u32) -> Option<String> {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .find(|(p, k, _)| *p == ctx.0 && *k == key)
                .map(|(_, _, v)| v.clone())
        }
    }

    #[test]
    fn context_threads_through_the_bridge_per_call() {
        let bridge = __StoreBridge::new(Backend::default(), CurrentThreadDispatcher);
        futures_lite::future::block_on(async {
            // The mirror carries the context as the first owned wire arg.
            assert!(StoreRpc::put(&bridge, Project(1), 7, "a".into()).await);
            assert!(StoreRpc::put(&bridge, Project(2), 7, "b".into()).await);
            let one = StoreRpc::get(&bridge, Project(1), 7).await;
            let two = StoreRpc::get(&bridge, Project(2), 7).await;
            assert_eq!(one.as_deref(), Some("a"));
            assert_eq!(two.as_deref(), Some("b"));
        });
    }

    #[test]
    fn direct_sync_callers_borrow_the_context() {
        let backend = Backend::default();
        let ctx = Project(9);
        backend.put(&ctx, 1, "x".into());
        assert_eq!(backend.get(&ctx, 1).as_deref(), Some("x"));
    }
}

// ── Empty trait ─────────────────────────────────────────────────────

mod empty {
    use super::rpc;

    #[rpc]
    #[allow(dead_code)]
    pub trait Empty {}

    #[test]
    fn empty_trait_compiles() {
        // Empty traits don't get a `serve` function — there's nothing to
        // serve. This test just verifies the macro accepts the shape.
        #[allow(dead_code)]
        struct Backend;
        impl Empty for Backend {}
    }
}

// ── Reified ops: #[rpc(ops)] ────────────────────────────────────────

mod ops_plain {
    #![allow(
        unused_imports,
        reason = "the #[rpc(ops)] expansion re-imports the trait name at the trait's own span for the reified-op module; false positive"
    )]
    use super::rpc;

    #[rpc(ops)]
    pub trait Counter {
        /// Bump by `n`, returning the new value.
        fn add(&self, n: u32) -> u32;
        fn reset(&self);
        fn label(&self, prefix: &str) -> String;
    }

    #[derive(Default)]
    struct Backend {
        value: std::sync::Mutex<u32>,
    }

    impl Counter for Backend {
        fn add(&self, n: u32) -> u32 {
            let mut v = self.value.lock().unwrap();
            *v += n;
            *v
        }

        fn reset(&self) {
            *self.value.lock().unwrap() = 0;
        }

        fn label(&self, prefix: &str) -> String {
            format!("{prefix}:{}", self.value.lock().unwrap())
        }
    }

    #[test]
    fn ops_replay_against_backend() {
        let backend = Backend::default();
        let program = vec![
            CounterOp::Add { n: 2 },
            CounterOp::Add { n: 3 },
            CounterOp::Label { prefix: "v".into() },
        ];
        let outputs: Vec<CounterOpOutput> =
            program.into_iter().map(|op| op.apply(&backend)).collect();
        assert!(matches!(outputs[0], CounterOpOutput::Add(2)));
        assert!(matches!(outputs[1], CounterOpOutput::Add(5)));
        match &outputs[2] {
            CounterOpOutput::Label(s) => assert_eq!(s, "v:5"),
            other => panic!("unexpected output: {other:?}"),
        }

        // Unit-return method → unit output variant; unit variant op.
        let out = CounterOp::Reset.apply(&backend);
        assert!(matches!(out, CounterOpOutput::Reset));
        assert!(matches!(
            CounterOp::Label { prefix: "v".into() }.apply(&backend),
            CounterOpOutput::Label(_)
        ));
    }
}

mod ops_subst {
    #![allow(
        unused_imports,
        reason = "the #[rpc(ops)] expansion re-imports the trait name at the trait's own span for the reified-op module; false positive"
    )]
    use architect::ops::{OpResolver, ResolveArg};

    use super::rpc;

    /// The literal parameter type the trait methods take.
    #[derive(Clone, Debug, PartialEq, facet::Facet)]
    pub struct TrackRef(pub u32);

    /// Deferred wire representation — literal or an earlier step's output.
    #[repr(u8)]
    #[derive(Clone, Debug, facet::Facet)]
    pub enum TrackArg {
        Literal(TrackRef),
        FromStep(u32),
    }

    #[rpc(ops(TrackRef as TrackArg))]
    pub trait Mixer {
        fn mute(&self, track: TrackRef, muted: bool) -> bool;
        fn first_track(&self) -> TrackRef;
    }

    #[derive(Default)]
    struct Backend;

    impl Mixer for Backend {
        fn mute(&self, track: TrackRef, muted: bool) -> bool {
            track.0 == 7 && muted
        }

        fn first_track(&self) -> TrackRef {
            TrackRef(7)
        }
    }

    /// Resolver holding prior step outputs.
    struct Steps(Vec<TrackRef>);

    impl OpResolver for Steps {
        type Error = String;
    }

    impl ResolveArg<TrackArg, TrackRef> for Steps {
        fn resolve_arg(&self, arg: TrackArg) -> Result<TrackRef, String> {
            match arg {
                TrackArg::Literal(t) => Ok(t),
                TrackArg::FromStep(n) => self
                    .0
                    .get(n as usize)
                    .cloned()
                    .ok_or_else(|| format!("no step {n}")),
            }
        }
    }

    #[test]
    fn substituted_ops_resolve_deferred_args() {
        let backend = Backend;
        let mut steps = Steps(Vec::new());

        // Step 0: produce a track; record its output for later steps.
        let out = MixerOp::FirstTrack.apply(&backend, &steps).unwrap();
        let MixerOpOutput::FirstTrack(track) = out else {
            panic!("wrong output variant");
        };
        steps.0.push(track);

        // Step 1: reference step 0's output instead of a literal.
        let op = MixerOp::Mute {
            track: TrackArg::FromStep(0),
            muted: true,
        };
        match op.apply(&backend, &steps).unwrap() {
            MixerOpOutput::Mute(hit) => assert!(hit),
            other => panic!("unexpected output: {other:?}"),
        }

        // Literal path + resolver failure path.
        let op = MixerOp::Mute {
            track: TrackArg::Literal(TrackRef(1)),
            muted: true,
        };
        assert!(matches!(
            op.apply(&backend, &steps).unwrap(),
            MixerOpOutput::Mute(false)
        ));
        let missing = MixerOp::Mute {
            track: TrackArg::FromStep(99),
            muted: true,
        };
        assert_eq!(
            missing.apply(&backend, &steps).unwrap_err(),
            "no step 99".to_string()
        );
    }
}

mod ops_skip {
    #![allow(
        unused_imports,
        reason = "the #[rpc(ops)] expansion re-imports the trait name at the trait's own span for the reified-op module; false positive"
    )]
    use super::rpc;

    #[rpc(ops)]
    pub trait Meter {
        fn level(&self) -> f64;
        /// Tuple returns can't cross phon — callable, but not reified.
        #[ops(skip)]
        fn signature(&self) -> (i32, i32);
    }

    struct Backend;
    impl Meter for Backend {
        fn level(&self) -> f64 {
            0.5
        }
        fn signature(&self) -> (i32, i32) {
            (4, 4)
        }
    }

    #[test]
    fn skipped_methods_stay_callable_but_are_not_reified() {
        let backend = Backend;
        assert_eq!(backend.signature(), (4, 4));
        assert!(matches!(
            MeterOp::Level.apply(&backend),
            MeterOpOutput::Level(v) if v == 0.5
        ));
        // MeterOp has exactly one variant — Signature was skipped.
        // (Compile-time proof: this match is exhaustive.)
        match MeterOp::Level {
            MeterOp::Level => {}
        }
    }
}
