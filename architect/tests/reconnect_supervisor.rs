//! Integration test for the supervised connection lifecycle: a
//! `use_connect_supervised`-provided `Connection<vox::Caller>` against an
//! in-process [`LocalServer`], whose server side is **killed and
//! recreated** mid-session. The supervisor must detect the death (via
//! `Caller::closed()`), flip the state off `Ready`, retry with backoff
//! while the server is down, and re-establish — bumping the generation —
//! once a fresh server appears. No remount, no "refresh".
//!
//! This is the headless half of the acceptance scenario ("kill -9 the
//! server, restart it, every browser is live again within ~5s with one
//! reconnect log line"). What it can't cover headlessly: the real
//! browser WebSocket close semantics (`vox_websocket::WsLink` over a
//! killed TCP server) and the visible roster badge — those need the
//! browser e2e (see the PRD's manual acceptance run).
//!
//! Run with: `cargo test -p architect --features atom,platform,local
//! --test reconnect_supervisor`

#![cfg(all(feature = "atom", feature = "platform", feature = "local"))]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use architect::{ConnectionState, LayerRouter, LocalServer, Scope};
use dioxus::prelude::*;

/// A minimal `FromVoxLane` client that carries only the connection's
/// `Caller` — the vox 0.10 replacement for the removed `NoopClient`. The
/// supervised connection anchors on this lane (no service calls) and
/// watches `caller.closed()` for the death-watch.
#[derive(Clone)]
struct AnchorClient {
    caller: vox_core::Caller,
    #[allow(dead_code)]
    connection: Option<vox_core::ConnectionHandle>,
}

impl vox_core::FromVoxLane for AnchorClient {
    const SERVICE_NAME: &'static str = "Noop";

    fn from_vox_lane(
        caller: vox_core::Caller,
        connection: Option<vox_core::ConnectionHandle>,
    ) -> Self {
        Self { caller, connection }
    }
}

/// The "current server" — `None` while the server is down. The connect
/// closure reads it on every attempt, exactly like a URL that points at
/// a restarted process.
#[derive(Clone, Default)]
struct ServerSlot(Arc<Mutex<Option<LocalServer>>>);

/// What the mounted component mirrors out for assertions: the connection
/// state's variant (+ error) and the generation counter.
#[derive(Clone, Default)]
struct Probe(Arc<Mutex<(String, u64)>>);

impl Probe {
    fn snapshot(&self) -> (String, u64) {
        self.0.lock().unwrap().clone()
    }
}

fn app() -> Element {
    let slot = use_context::<ServerSlot>();
    let probe = use_context::<Probe>();

    let conn = architect::use_connect_supervised(
        move || {
            let slot = slot.clone();
            async move {
                let server = slot.0.lock().unwrap().clone();
                let Some(server) = server else {
                    return Err("server down".to_string());
                };
                let root: AnchorClient = server
                    .establish()
                    .await
                    .map_err(|e| format!("establish: {e:?}"))?;
                Ok(root.caller.clone())
            }
        },
        // The death-watch: vox's session-liveness primitive.
        |caller: vox_core::Caller| async move { caller.closed().await },
    );

    // Reading `state()`/`generation()` in the render body subscribes this
    // component — every supervisor transition re-renders and re-mirrors.
    let state = match conn.state() {
        ConnectionState::Connecting => "connecting".to_string(),
        ConnectionState::Ready(_) => "ready".to_string(),
        ConnectionState::Failed(e) => format!("failed: {e}"),
    };
    *probe.0.lock().unwrap() = (state, conn.generation());
    rsx! {}
}

/// Pump the VirtualDom until the probe satisfies `pred` (or panic after
/// `secs`). The 25ms tick keeps re-checking while the supervisor's async
/// work (connects, death-watch, backoff sleeps) settles.
async fn pump_until(
    dom: &mut VirtualDom,
    probe: &Probe,
    secs: u64,
    what: &str,
    pred: impl Fn(&(String, u64)) -> bool,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let snap = probe.snapshot();
        if pred(&snap) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}; last probe: {snap:?}"
        );
        tokio::select! {
            _ = dom.wait_for_work() => dom.render_immediate(&mut dioxus::core::NoOpMutations),
            _ = tokio::time::sleep(Duration::from_millis(25)) => {}
        }
    }
}

fn fresh_server() -> (LocalServer, Arc<Scope>) {
    let scope = Scope::new();
    // An empty router is enough: the supervised connection anchors on the
    // root `AnchorClient` handshake, no service calls involved.
    let server = LocalServer::serve(LayerRouter::new(), scope.clone());
    (server, scope)
}

#[tokio::test(flavor = "multi_thread")]
async fn supervisor_reestablishes_after_server_death() {
    let (server1, scope1) = fresh_server();
    let slot = ServerSlot(Arc::new(Mutex::new(Some(server1))));
    let probe = Probe::default();

    let mut dom = VirtualDom::new(app)
        .with_root_context(slot.clone())
        .with_root_context(probe.clone());
    dom.rebuild_in_place();

    // 1. Initial establish: Ready, generation 1.
    pump_until(&mut dom, &probe, 10, "initial Ready(gen 1)", |(s, g)| {
        s == "ready" && *g == 1
    })
    .await;

    // 2. Kill the server (abort its acceptor tasks — the in-process
    //    equivalent of kill -9: the link drops, no goodbye). The
    //    supervisor must notice via `Caller::closed()` and leave `Ready`
    //    without any external nudge; attempts against the empty slot
    //    fail, parking the state at Failed while it retries.
    *slot.0.lock().unwrap() = None;
    scope1.close().await;
    pump_until(
        &mut dom,
        &probe,
        10,
        "death detected (left Ready)",
        |(s, g)| s != "ready" && *g == 1,
    )
    .await;

    // 3. "Restart" the server. The supervisor's backoff loop must pick
    //    it up on its own and re-establish: Ready again, generation 2.
    let (server2, scope2) = fresh_server();
    *slot.0.lock().unwrap() = Some(server2);
    pump_until(
        &mut dom,
        &probe,
        10,
        "re-established Ready(gen 2)",
        |(s, g)| s == "ready" && *g == 2,
    )
    .await;

    // Generation semantics: monotonic, one bump per successful
    // (re-)establish — exactly 2 after one death + one recovery.
    assert_eq!(probe.snapshot().1, 2);

    scope2.close().await;
}

/// A connect that fails outright (no server yet) must park the state at
/// `Failed` and keep generation 0 — then succeed (gen 1) once a server
/// appears, all without remounting. This is the "server boots after the
/// app" half of the lifecycle.
#[tokio::test(flavor = "multi_thread")]
async fn supervisor_recovers_from_failed_initial_connect() {
    let slot = ServerSlot::default(); // no server
    let probe = Probe::default();

    let mut dom = VirtualDom::new(app)
        .with_root_context(slot.clone())
        .with_root_context(probe.clone());
    dom.rebuild_in_place();

    pump_until(&mut dom, &probe, 10, "initial Failed(gen 0)", |(s, g)| {
        s.starts_with("failed") && *g == 0
    })
    .await;

    let (server, scope) = fresh_server();
    *slot.0.lock().unwrap() = Some(server);
    pump_until(&mut dom, &probe, 10, "first establish (gen 1)", |(s, g)| {
        s == "ready" && *g == 1
    })
    .await;

    scope.close().await;
}
