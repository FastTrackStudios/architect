//! Loopback test for the iroh transport: two endpoints on 127.0.0.1 with
//! relays and address lookup disabled (`presets::Minimal`), dialing direct
//! by full `EndpointAddr` — fully offline.
// Integration-test crate: `clippy.toml`'s `allow-*-in-tests` only reaches
// `#[test]` fns and `#[cfg(test)]` modules, so the fixture/mock `impl`
// blocks below still trip the panic lints. See the same block in the
// other `tests/` crates.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::large_stack_arrays,
    clippy::future_not_send,
    clippy::panic
)]
#![cfg(feature = "iroh")]

use architect::iroh_link::{self, IrohLink, VOX_ALPN};
use vox_types::{Link, LinkRx, LinkTx};

async fn bind_local() -> iroh::Endpoint {
    iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .alpns(vec![VOX_ALPN.to_vec()])
        .bind_addr("127.0.0.1:0")
        .expect("bind_addr")
        .bind()
        .await
        .expect("bind endpoint")
}

fn direct_addr(endpoint: &iroh::Endpoint) -> iroh::EndpointAddr {
    iroh::EndpointAddr::from_parts(
        endpoint.id(),
        endpoint
            .bound_sockets()
            .into_iter()
            .map(iroh::TransportAddr::Ip),
    )
}

#[tokio::test]
async fn frames_roundtrip_and_close() {
    let server = bind_local().await;
    let client = bind_local().await;
    let server_addr = direct_addr(&server);

    // Server: accept one connection + one bi-stream, echo frames back
    // until the client closes, then close in turn.
    let accept_task = tokio::spawn({
        let server = server.clone();
        async move {
            let incoming = server.accept().await.expect("endpoint closed");
            let connection = incoming.await.expect("incoming connection");
            let (send, recv) = connection.accept_bi().await.expect("accept_bi");
            let link = IrohLink::new(connection, send, recv);
            let (tx, mut rx) = link.split();
            while let Some(frame) = rx.recv().await.expect("server recv") {
                tx.send(frame.as_bytes().to_vec())
                    .await
                    .expect("server send");
            }
            tx.close().await.expect("server close");
        }
    });

    let link = iroh_link::connect(&client, server_addr)
        .await
        .expect("connect");
    let (tx, mut rx) = link.split();

    // The client writes first, which is also what un-lazies the QUIC
    // stream so the server's accept_bi fires.
    let frames: [&[u8]; 3] = [b"hello", &[0u8; 100_000], b""];
    for frame in frames {
        tx.send(frame.to_vec()).await.expect("client send");
        let echoed = rx.recv().await.expect("client recv").expect("echo frame");
        assert_eq!(echoed.as_bytes(), frame);
    }

    // Closing the client's send half finishes the stream; the server
    // sees recv() -> None, closes, and the client sees None too.
    tx.close().await.expect("client close");
    assert!(rx.recv().await.expect("client recv after close").is_none());
    accept_task.await.expect("server task");
}

#[tokio::test]
async fn vox_connection_over_iroh() {
    let server = bind_local().await;
    let client = bind_local().await;
    let server_addr = direct_addr(&server);

    // Serve an acceptor that rejects every lane — establishing the vox
    // connection itself (handshake over the framed link) is the point.
    let serve_task = tokio::spawn({
        let server = server.clone();
        async move {
            let acceptor = iroh_link::lane_acceptor_fn(|_req, _connection| {
                Err(vox_core::LaneRejection::new(
                    vox_core::LaneRejectReason::UnknownService,
                ))
            });
            iroh_link::serve_endpoint(&server, acceptor).await;
        }
    });

    let link = iroh_link::connect(&client, server_addr)
        .await
        .expect("connect");
    let connection = vox_core::initiator_on(link)
        .establish_connection()
        .await
        .expect("vox handshake over iroh");
    drop(connection);

    server.close().await;
    serve_task.await.expect("serve task");
}

#[test]
fn secret_key_hex_roundtrip() {
    let key = iroh::SecretKey::generate();
    let hex = iroh_link::secret_key_to_hex(&key);
    let parsed = iroh_link::secret_key_from_hex(&hex).expect("parse hex key");
    assert_eq!(parsed.to_bytes(), key.to_bytes());
}

#[test]
fn secret_key_persists() {
    let dir = std::env::temp_dir().join(format!("architect-iroh-key-{}", std::process::id()));
    let path = dir.join("iroh.key");
    let first = iroh_link::load_or_create_secret_key(&path).expect("create");
    let second = iroh_link::load_or_create_secret_key(&path).expect("reload");
    assert_eq!(first.to_bytes(), second.to_bytes());
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

// ── #56: dialling a restarted peer by id ─────────────────────────────

/// How the peer goes away before it comes back on the same key.
#[derive(Debug, Clone, Copy)]
enum Restart {
    /// `Endpoint::close`: the dialler gets a CONNECTION_CLOSE.
    Graceful,
    /// The endpoint is dropped without closing (a killed process): the
    /// dialler's old connection lingers until its idle timeout.
    Crash,
}

async fn bind_keyed(secret_key: iroh::SecretKey) -> iroh::Endpoint {
    iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .alpns(vec![VOX_ALPN.to_vec()])
        .secret_key(secret_key)
        .bind_addr("127.0.0.1:0")
        .expect("bind_addr")
        .bind()
        .await
        .expect("bind endpoint")
}

async fn bind_with_lookup(lookup: iroh::address_lookup::MemoryLookup) -> iroh::Endpoint {
    iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .alpns(vec![VOX_ALPN.to_vec()])
        .address_lookup(lookup)
        .bind_addr("127.0.0.1:0")
        .expect("bind_addr")
        .bind()
        .await
        .expect("bind endpoint")
}

/// Echo every frame on every bi-stream. Aborting the task drops the
/// endpoint clone and every connection it accepted.
fn serve_echo(endpoint: iroh::Endpoint) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        while let Some(incoming) = endpoint.accept().await {
            connections.spawn(async move {
                let Ok(connection) = incoming.await else {
                    return;
                };
                while let Ok((send, recv)) = connection.accept_bi().await {
                    let (tx, mut rx) = IrohLink::new(connection.clone(), send, recv).split();
                    while let Ok(Some(frame)) = rx.recv().await {
                        if tx.send(frame.as_bytes().to_vec()).await.is_err() {
                            break;
                        }
                    }
                }
            });
        }
    })
}

async fn echo_once(link: IrohLink) -> (iroh_link::IrohLinkTx, iroh_link::IrohLinkRx) {
    let (tx, mut rx) = link.split();
    tx.send(b"ping".to_vec()).await.expect("send ping");
    let echoed = rx.recv().await.expect("recv echo").expect("echo frame");
    assert_eq!(echoed.as_bytes(), b"ping");
    (tx, rx)
}

/// One dialler A, one peer B that restarts on the same key with new
/// ports `ROUNDS` times; after each restart B's new address replaces the
/// old one in A's lookup and the same A dials B by bare id.
async fn redial_after_restarts(restart: Restart) {
    const ROUNDS: usize = 20;
    let lookup = iroh::address_lookup::MemoryLookup::new();
    let dialler = bind_with_lookup(lookup.clone()).await;
    let key = iroh::SecretKey::generate();
    let peer_id = key.public();

    let mut peer = bind_keyed(key.clone()).await;
    lookup.set_endpoint_info(direct_addr(&peer));
    let mut server = serve_echo(peer.clone());
    // Keep the pre-restart link open on A, the way a long-lived client
    // holds its session while its server is redeployed.
    let mut held = echo_once(dial(&dialler, peer_id).await.expect("first dial")).await;

    let mut failures = Vec::new();
    for round in 0..ROUNDS {
        let old_sockets = peer.bound_sockets();
        if matches!(restart, Restart::Graceful) {
            peer.close().await;
        }
        server.abort();
        let _ = server.await;
        drop(peer);
        peer = bind_keyed(key.clone()).await;
        assert_ne!(peer.bound_sockets(), old_sockets, "rebound on new ports");
        lookup.set_endpoint_info(direct_addr(&peer));
        server = serve_echo(peer.clone());

        let started = std::time::Instant::now();
        match dial(&dialler, peer_id).await {
            Ok(link) => held = echo_once(link).await,
            Err(e) => failures.push(format!("round {round}: {e} after {:?}", started.elapsed())),
        }
    }
    drop(held);
    server.abort();
    peer.close().await;
    dialler.close().await;
    assert!(
        failures.is_empty(),
        "{restart:?}: {} of {ROUNDS} redials failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

async fn dial(dialler: &iroh::Endpoint, peer: iroh::EndpointId) -> std::io::Result<IrohLink> {
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        iroh_link::connect(dialler, peer),
    )
    .await
    .map_err(|_| std::io::Error::from(std::io::ErrorKind::TimedOut))?
}

#[tokio::test]
async fn redial_by_id_after_graceful_restart() {
    redial_after_restarts(Restart::Graceful).await;
}

#[tokio::test]
async fn redial_by_id_after_crash_restart() {
    redial_after_restarts(Restart::Crash).await;
}
