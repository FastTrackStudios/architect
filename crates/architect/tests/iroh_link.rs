//! Loopback test for the iroh transport: endpoints on 127.0.0.1 with
//! relays off (`presets::Minimal`), dialing direct by full `EndpointAddr`
//! or by bare id through an in-memory address lookup — fully offline.
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
    /// The process dies: nothing is sent, so the dialler's old
    /// connection lingers until its idle timeout. On loopback the old
    /// port then answers ICMP port-unreachable.
    Crash,
    /// A crash whose old address then swallows packets with no ICMP
    /// reply at all, like a redeployed pod on a new IP or a firewall
    /// drop. A silent socket holds the old port.
    CrashBlackhole,
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

/// Echo every frame on every bi-stream until the endpoint closes.
async fn serve_echo(endpoint: iroh::Endpoint) {
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
}

/// Peer B, on a runtime of its own so a crash can stop it dead:
/// shutting that runtime down drops every task without a packet sent.
struct Peer {
    runtime: tokio::runtime::Runtime,
    endpoint: iroh::Endpoint,
}

impl Peer {
    async fn start(secret_key: iroh::SecretKey) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("peer runtime");
        let endpoint = runtime
            .spawn(bind_keyed(secret_key))
            .await
            .expect("bind peer");
        runtime.spawn(serve_echo(endpoint.clone()));
        Self { runtime, endpoint }
    }

    async fn stop(self, restart: Restart) {
        let Self { runtime, endpoint } = self;
        let old_sockets = endpoint.bound_sockets();
        if matches!(restart, Restart::Graceful) {
            let closing = endpoint.clone();
            runtime
                .spawn(async move { closing.close().await })
                .await
                .expect("close peer");
        }
        runtime.shutdown_background();
        drop(endpoint);
        if matches!(restart, Restart::CrashBlackhole) {
            for addr in old_sockets {
                // Leaked on purpose: the old port stays silent for the
                // rest of the test process.
                std::mem::forget(hold_port(addr).await);
            }
        }
    }
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

    let mut peer = Peer::start(key.clone()).await;
    lookup.set_endpoint_info(direct_addr(&peer.endpoint));
    // Keep the pre-restart link open on A, the way a long-lived client
    // holds its session while its server is redeployed.
    let mut held = echo_once(dial(&dialler, peer_id).await.expect("first dial")).await;

    let mut failures = Vec::new();
    for round in 0..ROUNDS {
        let old_sockets = peer.endpoint.bound_sockets();
        peer.stop(restart).await;
        peer = Peer::start(key.clone()).await;
        assert_ne!(
            peer.endpoint.bound_sockets(),
            old_sockets,
            "rebound on new ports"
        );
        lookup.set_endpoint_info(direct_addr(&peer.endpoint));

        let started = std::time::Instant::now();
        match dial(&dialler, peer_id).await {
            Ok(link) => held = echo_once(link).await,
            Err(e) => failures.push(format!("round {round}: {e} after {:?}", started.elapsed())),
        }
    }
    drop(held);
    peer.stop(Restart::Graceful).await;
    dialler.close().await;
    assert!(
        failures.is_empty(),
        "{restart:?}: {} of {ROUNDS} redials failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Bind a socket that never answers on `addr`, once a crashed endpoint
/// has let go of it (its runtime shuts down asynchronously).
async fn hold_port(addr: std::net::SocketAddr) -> std::net::UdpSocket {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match std::net::UdpSocket::bind(addr) {
            Ok(socket) => return socket,
            Err(e) if std::time::Instant::now() < deadline => {
                assert_eq!(e.kind(), std::io::ErrorKind::AddrInUse, "{e}");
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Err(e) => panic!("old port {addr} still held: {e}"),
        }
    }
}

/// The dial budget the restart tests hold every redial to. A crash costs
/// the first attempt (half of it) before the retry gets through.
const DIAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// One dial that must finish inside `DIAL_TIMEOUT`.
async fn dial(dialler: &iroh::Endpoint, peer: iroh::EndpointId) -> std::io::Result<IrohLink> {
    let options = iroh_link::DialOptions::default().timeout(DIAL_TIMEOUT);
    let started = std::time::Instant::now();
    let link = iroh_link::connect_with(dialler, peer, options).await?;
    let took = started.elapsed();
    if took > DIAL_TIMEOUT {
        return Err(std::io::Error::other(format!(
            "connected after {took:?}, past the {DIAL_TIMEOUT:?} dial timeout"
        )));
    }
    Ok(link)
}

#[tokio::test]
async fn an_unknown_peer_is_unreachable() {
    let dialler = bind_with_lookup(iroh::address_lookup::MemoryLookup::new()).await;
    let nobody = iroh::SecretKey::generate().public();
    let options = iroh_link::DialOptions::default().timeout(std::time::Duration::from_secs(1));

    let Err(error) = iroh_link::connect_with(&dialler, nobody, options).await else {
        panic!("dialled a peer nobody published");
    };
    assert_eq!(error.kind(), std::io::ErrorKind::HostUnreachable);
    let dial_error = iroh_link::DialError::of(&error).expect("a DialError inside");
    assert_eq!(dial_error.kind(), iroh_link::DialErrorKind::PeerUnreachable);
    assert_eq!(dial_error.remote(), nobody);
    dialler.close().await;
}

#[tokio::test]
async fn a_silent_peer_times_out_without_retry_and_is_unreachable_with_it() {
    let lookup = iroh::address_lookup::MemoryLookup::new();
    let dialler = bind_with_lookup(lookup.clone()).await;
    let silent = hold_port("127.0.0.1:0".parse().expect("addr")).await;
    let nobody = iroh::SecretKey::generate().public();
    lookup.set_endpoint_info(iroh::EndpointAddr::from_parts(
        nobody,
        [iroh::TransportAddr::Ip(
            silent.local_addr().expect("silent addr"),
        )],
    ));
    let options = iroh_link::DialOptions::default().timeout(std::time::Duration::from_secs(1));

    let error = iroh_link::connect_with(&dialler, nobody, options.retry(false))
        .await
        .err()
        .expect("nothing answers");
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert_eq!(
        iroh_link::DialError::of(&error).map(iroh_link::DialError::kind),
        Some(iroh_link::DialErrorKind::Timeout)
    );

    let error = iroh_link::connect_with(&dialler, nobody, options)
        .await
        .err()
        .expect("nothing answers");
    assert_eq!(
        iroh_link::DialError::of(&error).map(iroh_link::DialError::kind),
        Some(iroh_link::DialErrorKind::PeerUnreachable)
    );
    dialler.close().await;
}

#[tokio::test]
async fn redial_by_id_after_graceful_restart() {
    redial_after_restarts(Restart::Graceful).await;
}

#[tokio::test]
async fn redial_by_id_after_crash_restart() {
    redial_after_restarts(Restart::Crash).await;
}

#[tokio::test]
async fn redial_by_id_after_crash_restart_behind_a_blackhole() {
    redial_after_restarts(Restart::CrashBlackhole).await;
}
