//! Serving auth's services over vox: one connection answers every service
//! mounted on a [`VoxRouter`], over a WebSocket ([`axum_ws`]) or in this
//! process ([`local`]).

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use vox::{
    DriverReplySink, Handler, MethodId, RequestCall, SchemaRecvTracker, SelfRef, ServiceDescriptor,
};

/// The WebSocket subprotocol a vox client asks for.
pub const VOX_SUBPROTOCOL: &str = "vox.v1";

/// A vox handler behind a pointer, so services of different types share
/// one router.
trait DynHandler: Send + Sync + 'static {
    fn handle(
        &self,
        call: SelfRef<RequestCall<'static>>,
        reply: DriverReplySink,
        schemas: Arc<SchemaRecvTracker>,
    ) -> Pin<Box<dyn core::future::Future<Output = ()> + Send + '_>>;
    fn args_have_channels(&self, method_id: MethodId) -> bool;
    fn response_wire_shape(&self, method_id: MethodId) -> Option<&'static facet::Shape>;
}

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
}

/// Every mounted service's methods, each call sent to the service that
/// owns its method id (a call for none is answered `UnknownMethod`).
#[derive(Clone, Default)]
pub struct VoxRouter {
    methods: HashMap<MethodId, usize>,
    handlers: Vec<Arc<dyn DynHandler>>,
}

impl VoxRouter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mount a service: its descriptor and its dispatcher
    /// (`AuthServiceDispatcher::new(service)`).
    #[must_use]
    pub fn with<H>(mut self, descriptor: &'static ServiceDescriptor, handler: H) -> Self
    where
        H: Handler<DriverReplySink> + Send + Sync + 'static,
    {
        let idx = self.handlers.len();
        self.handlers.push(Arc::new(handler));
        for method in descriptor.methods {
            self.methods.insert(method.id, idx);
        }
        self
    }

    /// How many methods it answers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.methods.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.methods.is_empty()
    }

    fn handler(&self, method_id: MethodId) -> Option<&Arc<dyn DynHandler>> {
        self.methods
            .get(&method_id)
            .and_then(|&i| self.handlers.get(i))
    }

    /// Every incoming lane of a connection answered by this router.
    #[must_use]
    pub fn acceptor(&self) -> impl vox::LaneAcceptor {
        let router = self.clone();
        vox::lane_acceptor_fn(move |_req, connection| {
            connection.handle_with(router.clone());
            Ok(())
        })
    }
}

impl Handler<DriverReplySink> for VoxRouter {
    fn args_have_channels(&self, method_id: MethodId) -> bool {
        self.handler(method_id)
            .is_some_and(|h| h.args_have_channels(method_id))
    }

    fn response_wire_shape(&self, method_id: MethodId) -> Option<&'static facet::Shape> {
        self.handler(method_id)
            .and_then(|h| h.response_wire_shape(method_id))
    }

    async fn handle(
        &self,
        call: SelfRef<RequestCall<'static>>,
        reply: DriverReplySink,
        schemas: Arc<SchemaRecvTracker>,
    ) {
        let method_id = call.get().method_id;
        if let Some(handler) = self.handler(method_id) {
            handler.handle(call, reply, schemas).await;
        } else {
            use vox::ReplySink as _;
            reply
                .send_error(vox::VoxError::<core::convert::Infallible>::UnknownMethod)
                .await;
        }
    }
}

/// In this process: a client of a router over an in-memory link (tests,
/// and an app that embeds the engine).
pub mod local {
    use super::VoxRouter;

    /// The serving side of an in-process connection: dropping it shuts
    /// the session down, so the client sees it close.
    pub struct Serving(tokio::task::JoinHandle<()>);

    impl Drop for Serving {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    /// Dropping the session's handle alone only closes its local queues;
    /// this shuts the session itself down.
    struct ShutdownOnDrop(vox_core::ConnectionHandle);

    impl Drop for ShutdownOnDrop {
        fn drop(&mut self) {
            // `Err` only when the session is already gone.
            let _ = self.0.shutdown();
        }
    }

    /// A fresh in-memory connection to `router`, as the client `C`
    /// (`AuthServiceClient`, …), and its serving side (keep it while the
    /// client is used).
    pub async fn establish<C>(router: &VoxRouter) -> Result<(C, Serving), String>
    where
        C: vox_core::FromVoxLane,
    {
        let (client_link, server_link) = vox_core::memory_link_pair(16);
        let acceptor = router.acceptor();
        let task = tokio::spawn(async move {
            match vox_core::acceptor_on(server_link)
                .on_lane(acceptor)
                .establish_connection()
                .await
            {
                Ok(connection) => {
                    let _connection = ShutdownOnDrop(connection);
                    std::future::pending::<()>().await;
                }
                Err(e) => tracing::warn!(error = %e, "an in-memory vox connection failed"),
            }
        });
        let serving = Serving(task);
        let client = vox_core::initiator_on(client_link)
            .establish::<C>()
            .await
            .map_err(|e| format!("local establish: {e:?}"))?;
        Ok((client, serving))
    }
}

/// vox over an axum WebSocket: each upgraded socket is one connection
/// whose lanes the router answers.
pub mod axum_ws {
    use axum::extract::ws::{Message as AxumWsMessage, WebSocket, WebSocketUpgrade};
    use axum::response::{IntoResponse, Response};
    use tokio::sync::{mpsc, oneshot};
    use tracing::warn;

    use super::{VOX_SUBPROTOCOL, VoxRouter};

    /// Answer a WebSocket upgrade (subprotocol [`VOX_SUBPROTOCOL`]) with
    /// `router`.
    pub fn upgrade(ws: WebSocketUpgrade, router: VoxRouter) -> Response {
        ws.protocols([VOX_SUBPROTOCOL])
            .on_upgrade(move |socket| serve(socket, router))
            .into_response()
    }

    /// Serve one socket until it closes.
    pub async fn serve(socket: WebSocket, router: VoxRouter) {
        let (closed_tx, closed_rx) = oneshot::channel();
        let connection = match vox_core::acceptor_on(AxumWsLink::new(socket, closed_tx))
            .on_lane(router.acceptor())
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

    /// An axum WebSocket as a vox link.
    pub struct AxumWsLink {
        socket: WebSocket,
        closed: oneshot::Sender<()>,
    }

    impl AxumWsLink {
        pub const fn new(socket: WebSocket, closed: oneshot::Sender<()>) -> Self {
            Self { socket, closed }
        }
    }

    impl vox_types::Link for AxumWsLink {
        type Tx = AxumWsTx;
        type Rx = AxumWsRx;

        fn split(self) -> (Self::Tx, Self::Rx) {
            let (tx_out, rx_out) = mpsc::channel::<Vec<u8>>(1);
            let (tx_in, rx_in) = mpsc::channel::<Result<AxumWsMessage, AxumWsError>>(1);
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
                        if tx_in.send(Ok(msg)).await.is_err() {
                            return;
                        }
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
                    Some(Ok(AxumWsMessage::Ping(_) | AxumWsMessage::Pong(_))) => {}
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
}
