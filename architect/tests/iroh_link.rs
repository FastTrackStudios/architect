//! Loopback test for the iroh transport: two endpoints on 127.0.0.1 with
//! relays and address lookup disabled (`presets::Minimal`), dialing direct
//! by full `EndpointAddr` — fully offline.

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
                tx.send(frame.as_bytes().to_vec()).await.expect("server send");
            }
            tx.close().await.expect("server close");
        }
    });

    let link = iroh_link::connect(&client, server_addr).await.expect("connect");
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

    let link = iroh_link::connect(&client, server_addr).await.expect("connect");
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
