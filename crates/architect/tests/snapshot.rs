//! `architect::snapshot`: the write gate parks vox calls while a snapshot
//! holds it and lets them through when it drops; holds are bounded by
//! `max_hold`; `quiesce` runs checkpoint then step under the gate; the
//! sqlite `wal_checkpoint` callback empties the WAL.

// Integration-test crate: `allow-*-in-tests` in clippy.toml only reaches
// `#[test]` fns, so the fixture impls below need the crate-wide allow.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects
)]
#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use architect::local::LocalServer;
use architect::resource::Scope;
use architect::snapshot::{Admission, GateTimeout, WriteGate, quiesce, wal_checkpoint};
use architect::{Services, layers};

mod counter_proto {
    #[architect::rpc]
    pub trait Counter {
        /// Increment and return the new value (the "write").
        fn bump(&self) -> u64;
    }
}

use counter_proto::CounterClient;
use counter_proto::prelude::CounterService;

#[derive(Clone, architect::HasDispatcher)]
#[dispatch(architect::dispatch::CurrentThreadDispatcher)]
struct CounterBackend {
    value: Arc<AtomicU64>,
}

impl counter_proto::Counter for CounterBackend {
    fn bump(&self) -> u64 {
        self.value.fetch_add(1, Ordering::SeqCst) + 1
    }
}

impl Services for CounterBackend {
    fn layers() -> impl architect::Layer<Self> {
        layers![CounterService]
    }
}

/// The acceptance test: a write over vox blocks while the gate is held
/// and completes once the hold drops.
#[tokio::test(flavor = "multi_thread")]
async fn writes_block_while_gated_and_resume() {
    let value = Arc::new(AtomicU64::new(0));
    let gate = WriteGate::with_max_hold(Duration::from_secs(30));
    let router = CounterBackend {
        value: Arc::clone(&value),
    }
    .into_router()
    .gated(&gate);
    let server = LocalServer::serve(router, Scope::new());
    let client: CounterClient = server.establish().await.expect("establish");

    // Open gate: the write goes straight through.
    assert_eq!(client.bump().await.expect("bump"), 1);

    let hold = gate.pause().await;
    assert!(!gate.is_open());
    let pending = tokio::spawn({
        let client = client.clone();
        async move { client.bump().await.expect("bump") }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!pending.is_finished(), "write ran while the gate was held");
    assert_eq!(value.load(Ordering::SeqCst), 1, "backend saw the write");

    drop(hold);
    let got = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .expect("write resumed after release")
        .expect("join");
    assert_eq!(got, 2);
    assert_eq!(value.load(Ordering::SeqCst), 2);
    assert!(gate.is_open());
}

/// A hold that outlives `max_hold` stops blocking: waiters are admitted
/// with `Admission::Expired`.
#[tokio::test(flavor = "multi_thread")]
async fn hold_is_bounded_by_max_hold() {
    let gate = WriteGate::with_max_hold(Duration::from_millis(200));
    let hold = gate.pause().await;
    let started = std::time::Instant::now();
    let admission = tokio::time::timeout(Duration::from_secs(5), gate.wait_open())
        .await
        .expect("waiter admitted at the deadline");
    assert_eq!(admission, Admission::Expired);
    assert!(started.elapsed() >= Duration::from_millis(150));
    assert!(hold.expired());
    assert!(gate.is_open());
    drop(hold);
    assert_eq!(gate.wait_open().await, Admission::Open);
}

#[tokio::test(flavor = "multi_thread")]
async fn second_pause_waits_for_the_first() {
    let gate = WriteGate::new();
    let first = gate.pause().await;
    let second = tokio::spawn({
        let gate = gate.clone();
        async move {
            let _hold = gate.pause().await;
        }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!second.is_finished());
    drop(first);
    tokio::time::timeout(Duration::from_secs(5), second)
        .await
        .expect("second pause got the gate")
        .expect("join");
    assert!(gate.is_open());
}

#[tokio::test(flavor = "multi_thread")]
async fn quiesce_runs_checkpoint_then_step_under_the_gate() {
    let gate = WriteGate::new();
    let log = Arc::new(Mutex::new(Vec::<&str>::new()));
    let out = quiesce(
        &gate,
        || {
            let (gate, log) = (gate.clone(), Arc::clone(&log));
            async move {
                assert!(!gate.is_open(), "checkpoint ran with the gate open");
                log.lock().unwrap().push("checkpoint");
            }
        },
        || {
            let (gate, log) = (gate.clone(), Arc::clone(&log));
            async move {
                assert!(!gate.is_open(), "step ran with the gate open");
                log.lock().unwrap().push("step");
                "commit-sha"
            }
        },
    )
    .await
    .expect("within max hold");
    assert_eq!(out, "commit-sha");
    assert_eq!(*log.lock().unwrap(), ["checkpoint", "step"]);
    assert!(gate.is_open());
}

#[tokio::test(flavor = "multi_thread")]
async fn quiesce_cancels_a_step_past_max_hold() {
    let gate = WriteGate::with_max_hold(Duration::from_millis(100));
    let err = quiesce(
        &gate,
        || async {},
        || tokio::time::sleep(Duration::from_secs(30)),
    )
    .await
    .expect_err("step outlived the hold");
    assert_eq!(
        err,
        GateTimeout {
            max_hold: Duration::from_millis(100)
        }
    );
    assert!(gate.is_open());
}

/// The stock sqlite callback folds the WAL into the main file and
/// truncates the `-wal` sidecar to zero bytes.
#[tokio::test(flavor = "multi_thread")]
async fn wal_checkpoint_truncates_the_wal() {
    use sea_orm::{ConnectionTrait, Database};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.db");
    let db = Database::connect(format!("sqlite://{}?mode=rwc", path.display()))
        .await
        .unwrap();
    db.execute_unprepared("PRAGMA journal_mode=WAL;")
        .await
        .unwrap();
    db.execute_unprepared("CREATE TABLE t (v INTEGER); INSERT INTO t VALUES (1), (2), (3);")
        .await
        .unwrap();
    let wal = dir.path().join("data.db-wal");
    assert!(
        std::fs::metadata(&wal).unwrap().len() > 0,
        "the writes are in the WAL"
    );

    let failures = wal_checkpoint([&db]).await;
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(std::fs::metadata(&wal).unwrap().len(), 0);
}
