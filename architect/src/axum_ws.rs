//! axum WebSocket ↔ vox transport adapter.
//!
//! Plain boilerplate that every architect-using server reuses. Pulled
//! into the crate so a server's `main.rs` only writes the *interesting*
//! part — the per-service dispatcher match. Everything else (the Link
//! implementation, the io loop, the closed-channel notification, the
//! `acceptor_on(...).establish_connection()` dance) lives here.
//!
//! ## Usage
//!
//! ```ignore
//! use architect::axum_ws;
//!
//! async fn vox_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
//!     ws.on_upgrade(move |socket| async move {
//!         let db = state.db.clone();
//!         let factory = vox::lane_acceptor_fn(move |req, connection| {
//!             match req.service() {
//!                 "ExampleRepo" => {
//!                     connection.handle_with(ExampleRepoDispatcher::new(
//!                         ExampleRepoStorage::new(db.clone())));
//!                     Ok(())
//!                 }
//!                 _ => Err(vox::LaneRejection::new(vox::LaneRejectReason::UnknownService)),
//!             }
//!         });
//!         axum_ws::serve(socket, factory).await;
//!     }).into_response()
//! }
//! ```
//!
//! Gated behind `--features server-axum`; the wasm side of an architect
//! project never sees it.

use axum::extract::ws::{Message as AxumWsMessage, WebSocket};
use tokio::sync::{mpsc, oneshot};

use tracing::warn;

// Re-export so the user's match arms can write `architect::axum_ws::lane_acceptor_fn`
// if they prefer, instead of importing vox at the top level.
pub use vox_core::{acceptor_on, lane_acceptor_fn};

/// Drive a single vox connection on an upgraded axum WebSocket.
///
/// Spawns the IO loop, completes the vox handshake, then blocks until
/// the peer disconnects (the closed channel fires from `Drop` on the
/// io task's notifier).
pub async fn serve<A>(socket: WebSocket, acceptor: A)
where
    A: vox_core::LaneAcceptor,
{
    let (closed_tx, closed_rx) = oneshot::channel();
    let connection = match acceptor_on(AxumWsLink::new(socket, closed_tx))
        .on_lane(acceptor)
        .establish_connection()
        .await
    {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "vox WebSocket session failed");
            return;
        }
    };
    let _connection = connection;
    let _ = closed_rx.await;
}

/// Serve a whole router over an upgraded axum WebSocket — the common case.
/// Accepts any vox [`vox::Handler`]: a [`crate::LayerRouter`], or
/// a wrapper around one (e.g. a snapshot-gating router). Collapses the
/// per-binary `lane_acceptor_fn(|_, conn| conn.handle_with(router.clone()))` +
/// [`serve`] boilerplate to one call, so a vox route is just:
/// `ws.on_upgrade(move |sock| serve_router(sock, state.router.clone()))`.
pub async fn serve_router<H>(socket: WebSocket, router: H)
where
    H: vox::Handler<vox::DriverReplySink> + Clone + Send + Sync + 'static,
{
    serve(socket, crate::layer::handler_acceptor(router)).await;
}

// ── Link adapter ──────────────────────────────────────────────────────

pub struct AxumWsLink {
    socket: WebSocket,
    closed: oneshot::Sender<()>,
}

impl AxumWsLink {
    pub fn new(socket: WebSocket, closed: oneshot::Sender<()>) -> Self {
        Self { socket, closed }
    }
}

impl vox_types::Link for AxumWsLink {
    type Tx = AxumWsTx;
    type Rx = AxumWsRx;

    fn split(self) -> (Self::Tx, Self::Rx) {
        let (tx_out, rx_out) = mpsc::channel::<Vec<u8>>(1);
        let (tx_in, rx_in) =
            mpsc::channel::<Result<AxumWsMessage, AxumWsError>>(1);
        let io_task = tokio::task::spawn(io_loop(self.socket, rx_out, tx_in, self.closed));
        (
            AxumWsTx {
                tx: tx_out,
                io_task,
            },
            AxumWsRx { rx: rx_in },
        )
    }
}

async fn io_loop(
    socket: WebSocket,
    mut rx_out: mpsc::Receiver<Vec<u8>>,
    tx_in: mpsc::Sender<Result<AxumWsMessage, AxumWsError>>,
    closed: oneshot::Sender<()>,
) {
    use futures::{SinkExt, StreamExt};
    let (mut ws_tx, mut ws_rx) = socket.split();
    let _notify = NotifyOnDrop(Some(closed));
    loop {
        tokio::select! {
            outbound = rx_out.recv() => match outbound {
                Some(bytes) => {
                    if let Err(e) = ws_tx.send(AxumWsMessage::Binary(bytes.into())).await {
                        let _ = tx_in.send(Err(AxumWsError(e.to_string()))).await;
                        return;
                    }
                }
                None => return,
            },
            inbound = ws_rx.next() => match inbound {
                Some(Ok(msg)) => {
                    if tx_in.send(Ok(msg)).await.is_err() { return; }
                }
                Some(Err(e)) => {
                    let _ = tx_in.send(Err(AxumWsError(e.to_string()))).await;
                    return;
                }
                None => return,
            },
        }
    }
}

struct NotifyOnDrop(Option<oneshot::Sender<()>>);
impl Drop for NotifyOnDrop {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}

pub struct AxumWsTx {
    tx: mpsc::Sender<Vec<u8>>,
    io_task: tokio::task::JoinHandle<()>,
}

impl vox_types::LinkTx for AxumWsTx {
    async fn send(&self, bytes: Vec<u8>) -> std::io::Result<()> {
        self.tx
            .send(bytes)
            .await
            .map_err(|_| std::io::Error::other("axum websocket writer stopped"))
    }
    async fn close(self) -> std::io::Result<()> {
        drop(self.tx);
        self.io_task.await.map_err(std::io::Error::other)
    }
}

pub struct AxumWsRx {
    rx: mpsc::Receiver<Result<AxumWsMessage, AxumWsError>>,
}

#[derive(Debug)]
pub struct AxumWsError(String);
impl std::fmt::Display for AxumWsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "axum websocket: {}", self.0)
    }
}
impl std::error::Error for AxumWsError {}

impl vox_types::LinkRx for AxumWsRx {
    type Error = AxumWsError;
    async fn recv(&mut self) -> Result<Option<vox_types::Backing>, Self::Error> {
        loop {
            match self.rx.recv().await {
                Some(Ok(AxumWsMessage::Binary(data))) => {
                    return Ok(Some(vox_types::Backing::Boxed(
                        Vec::from(data).into_boxed_slice(),
                    )));
                }
                Some(Ok(AxumWsMessage::Close(_))) | None => return Ok(None),
                Some(Ok(AxumWsMessage::Ping(_) | AxumWsMessage::Pong(_))) => continue,
                Some(Ok(AxumWsMessage::Text(_))) => {
                    return Err(AxumWsError(
                        "text frames are not valid vox websocket payloads".into(),
                    ));
                }
                Some(Err(e)) => return Err(e),
            }
        }
    }
}
