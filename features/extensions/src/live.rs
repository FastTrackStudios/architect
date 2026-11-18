//! An extension's live documents (feature `doc-sync`).
//!
//! A replica of the node's copy of a vault document of a kind it
//! declares, kept in step both ways (`docs/content/guide/documents.md`,
//! "An extension edits the node's copy").
//!
//! A vault file of an extension's `[[doc_kinds]]` (a Keyflow `.session`)
//! is a document of the app's node: the node opens it with the file as its
//! storage and syncs it like a note. A file has one writer at a time, so
//! the extension does not open the file; it edits the node's copy:
//!
//! 1. it asks the host for the document by vault path
//!    (`HostRequest::LiveDocument`, `live_document`): the host resolves it
//!    ([`KindDocuments::resolve`]), refuses it unless its kind is one the
//!    extension's manifest declares and the extension may read documents
//!    (`reads = ["documents"]`), and answers a [`LiveDoc`] (its id, its
//!    kind, whether the extension may write it: `writes = ["documents"]`);
//! 2. it syncs a replica against the node with the ordinary `DocSync`
//!    session (`crdt::sync`): a process extension over the connection it
//!    already has to the host (a `DocSync` lane beside the host service),
//!    a linked one over an in-memory lane. The host relays the session to
//!    the node's registry as the extension (`Principal::Extension`, for
//!    whoever it acts for), so the registry's authorizer and the vault's
//!    write guard decide as they do for any editor. A session for a
//!    document the extension was not answered, of a kind it no longer
//!    declares, or after its capability was taken back is refused
//!    (`SyncError::UnknownDoc`); without `writes` its updates are dropped
//!    and it only follows.
//!
//! On the extension's side: `linked::Handle::document(path)` (linked in or
//! as a process) or `guest::document(path)` give a [`DocLane`];
//! [`DocLane::sync`] runs one session of a `SyncedDoc` the extension keeps
//! (its own `CrdtDoc`), [`DocLane::replica`] keeps an in-memory replica in
//! step on a task of its own.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The document an extension may sync (`live_document`'s answer).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveDoc {
    /// The document's id: the one in its file's header.
    pub doc_id: Uuid,
    /// Its kind's id (`fts.session`): one of the extension's `[[doc_kinds]]`.
    pub kind: String,
    /// The vault path it was named by.
    pub path: String,
    /// Whether its edits reach the node (`writes = ["documents"]`); when
    /// not, the replica only follows.
    pub writable: bool,
    /// This app's node's endpoint id, when it has one: what a leader
    /// writes as the document's presence host
    /// (`crdt::presence::set_presence_host`), so the org's other devices
    /// follow its presence through this node (session#192).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
}

#[cfg(all(feature = "host", not(target_family = "wasm")))]
pub use host_side::{
    ExtensionDocPresence, ExtensionDocSync, KindDoc, KindDocuments, PresenceLink,
    RegistryDocuments, serve_in_memory,
};

#[cfg(not(target_family = "wasm"))]
pub use client::{DocLane, Replica, SyncClient};

/// The host's side: where the documents are, and the relay.
#[cfg(all(feature = "host", not(target_family = "wasm")))]
mod host_side {
    use std::sync::Arc;

    use crdt::peer::SelfLane;
    use crdt::registry::DocRegistry;
    use crdt::sync::{DocSync, DocSyncClient, SyncDown, SyncError};
    use uuid::Uuid;

    use super::client::SyncClient;
    use crate::documents::DocError;
    use crate::hostapi::ExtensionApi;

    /// A vault file that is a document of a kind, as the node holds it.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct KindDoc {
        pub doc_id: Uuid,
        /// Its kind's id.
        pub kind: String,
    }

    /// The node's documents of a kind, for extensions: what an app gives
    /// its host (`Host::set_kind_documents`) so its extensions edit the
    /// node's copy of a vault document of their kind.
    pub trait KindDocuments: Send + Sync {
        /// The document the vault file `path` is (already checked as a
        /// path): its id and kind.
        ///
        /// # Errors
        ///
        /// [`DocError::NotFound`]: no such file, or not a document of a
        /// registered kind.
        fn resolve(&self, path: &str) -> Result<KindDoc, DocError>;

        /// The node's registry: a session is attached to it as the
        /// extension, so its authorizer decides.
        fn registry(&self) -> DocRegistry;
    }

    /// Resolves a path.
    type Resolve = dyn Fn(&str) -> Option<KindDoc> + Send + Sync;

    /// [`KindDocuments`] over a registry and a function that names a
    /// path's document: a node's `VaultCollab` (`document_of`, `registry`).
    #[derive(Clone)]
    pub struct RegistryDocuments {
        registry: DocRegistry,
        resolve: Arc<Resolve>,
    }

    impl RegistryDocuments {
        /// `registry`'s documents, a path's named by `resolve`.
        pub fn new(
            registry: DocRegistry,
            resolve: impl Fn(&str) -> Option<KindDoc> + Send + Sync + 'static,
        ) -> Self {
            Self {
                registry,
                resolve: Arc::new(resolve),
            }
        }
    }

    impl KindDocuments for RegistryDocuments {
        fn resolve(&self, path: &str) -> Result<KindDoc, DocError> {
            (self.resolve)(path)
                .ok_or_else(|| DocError::NotFound(format!("document of a kind at {path}")))
        }

        fn registry(&self) -> DocRegistry {
            self.registry.clone()
        }
    }

    /// The `DocSync` service an extension's sessions reach: the host's
    /// relay to the node's registry, as the extension.
    #[derive(Clone)]
    pub struct ExtensionDocSync {
        api: ExtensionApi,
    }

    impl ExtensionDocSync {
        /// The relay for `api`'s extension.
        #[must_use]
        pub const fn new(api: ExtensionApi) -> Self {
            Self { api }
        }
    }

    /// What one frame of a channel holds, owned.
    macro_rules! owned {
        ($incoming:expr) => {{
            let mut owned = None;
            if let Ok(Some(frame)) = $incoming {
                let _ = frame.map(|f| owned = Some(f));
            }
            owned
        }};
    }

    impl DocSync for ExtensionDocSync {
        async fn sync(
            &self,
            doc_id: Uuid,
            from: Vec<u8>,
            mut up: vox::Rx<Vec<u8>>,
            down: vox::Tx<SyncDown>,
        ) -> Result<(), SyncError> {
            let session = self.api.live_session(doc_id)?;
            let lane = SelfLane::open(session.registry, session.principal)
                .await
                .map_err(SyncError::Internal)?;
            let (node_up, node_up_rx) = vox::channel::<Vec<u8>>();
            let (node_down_tx, mut node_down) = vox::channel::<SyncDown>();
            let mut call =
                std::pin::pin!(lane.client().sync(doc_id, from, node_up_rx, node_down_tx));
            loop {
                tokio::select! {
                    // The node's call first: its request binds `node_up`.
                    biased;
                    res = &mut call => return res.map_err(sync_error),
                    incoming = up.recv() => match owned!(incoming) {
                        Some(bytes) => {
                            // A follower's edits stay its own.
                            if session.writable && node_up.send(bytes).await.is_err() {
                                return call.await.map_err(sync_error);
                            }
                        }
                        // The extension ended its session.
                        None => return Ok(()),
                    },
                    incoming = node_down.recv() => match owned!(incoming) {
                        Some(frame) => {
                            if down.send(frame).await.is_err() {
                                return Ok(());
                            }
                        }
                        None => return call.await.map_err(sync_error),
                    },
                }
            }
        }
    }

    /// How this app's node reaches the presence another node hosts.
    ///
    /// The node's own endpoint id, and a presence client of another node,
    /// dialled over the org's iroh lane by its endpoint id alone
    /// (session#192).
    pub trait PresenceLink: Send + Sync {
        /// This node's endpoint id.
        fn me(&self) -> String;
        /// A `DocPresence` client of node `host`.
        fn dial(
            &self,
            host: String,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<crdt::sync::DocPresenceClient, String>>
                    + Send,
            >,
        >;
    }

    /// The host's relay of an extension's presence sessions (session#192).
    ///
    /// To the node's own registry when the document's presence is hosted
    /// here (or nowhere said), else to the node that hosts it
    /// (`crdt::presence::presence_host`), over the org's iroh lane.
    ///
    /// The clock: `now` answers the clock of the host its last session
    /// reached, as this node estimates it (it pings the host's `now` while
    /// a session to it lasts), so an extension's estimate is of the host's
    /// clock. One remote host per connection at a time.
    #[derive(Clone)]
    pub struct ExtensionDocPresence {
        api: ExtensionApi,
        remote: Arc<std::sync::Mutex<Option<crdt::clock::SharedClock>>>,
    }

    impl ExtensionDocPresence {
        /// The relay for `api`'s extension.
        #[must_use]
        pub fn new(api: ExtensionApi) -> Self {
            Self {
                api,
                remote: Arc::default(),
            }
        }

        fn set_remote(&self, clock: Option<crdt::clock::SharedClock>) {
            *self
                .remote
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = clock;
        }
    }

    impl crdt::sync::DocPresence for ExtensionDocPresence {
        async fn presence(
            &self,
            doc_id: Uuid,
            mut up: vox::Rx<Vec<u8>>,
            down: vox::Tx<Vec<u8>>,
        ) -> Result<(), SyncError> {
            let session = self.api.live_session(doc_id)?;
            let doc = session
                .registry
                .doc(doc_id)
                .await
                .map_err(|_| SyncError::UnknownDoc)?;
            let host = crdt::presence::presence_host(doc.loro());
            let remote = match (host, &session.presence) {
                (Some(host), Some(link)) if host != link.me() => {
                    Some(link.dial(host).await.map_err(SyncError::Internal)?)
                }
                (Some(_), None) => {
                    return Err(SyncError::Internal(
                        "hosted on another node, and this app has no node to reach it".to_owned(),
                    ));
                }
                _ => None,
            };
            // The host's clock, estimated here while the session lasts.
            let pinger = remote.clone().map(|client| {
                let clock = crdt::clock::SharedClock::default();
                self.set_remote(Some(clock.clone()));
                tokio::spawn(async move {
                    loop {
                        let sent = crdt::presence::clock_ms();
                        if let Ok(there) = client.now().await {
                            clock.record(sent, there, crdt::presence::clock_ms());
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                })
            });
            let local;
            let client = if let Some(client) = &remote {
                client
            } else {
                local = crdt::peer::SelfPresenceLane::open(session.registry, session.principal)
                    .await
                    .map_err(SyncError::Internal)?;
                local.client()
            };
            let (node_up, node_up_rx) = vox::channel::<Vec<u8>>();
            let (node_down_tx, mut node_down) = vox::channel::<Vec<u8>>();
            let mut call = std::pin::pin!(client.presence(doc_id, node_up_rx, node_down_tx));
            let ended = loop {
                tokio::select! {
                    biased;
                    res = &mut call => break res.map_err(sync_error),
                    incoming = up.recv() => match owned!(incoming) {
                        Some(bytes) => {
                            // A reader's presence reaches nobody.
                            if session.writable && node_up.send(bytes).await.is_err() {
                                break call.await.map_err(sync_error);
                            }
                        }
                        None => break Ok(()),
                    },
                    incoming = node_down.recv() => match owned!(incoming) {
                        Some(frame) => {
                            if down.send(frame).await.is_err() {
                                break Ok(());
                            }
                        }
                        None => break call.await.map_err(sync_error),
                    },
                }
            };
            if let Some(pinger) = pinger {
                pinger.abort();
                self.set_remote(None);
            }
            ended
        }

        async fn now(&self) -> f64 {
            self.remote
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .and_then(crdt::clock::SharedClock::now_ms)
                .unwrap_or_else(crdt::presence::clock_ms)
        }
    }

    fn sync_error(e: vox::VoxError<SyncError>) -> SyncError {
        match e {
            vox::VoxError::User(e) => *e,
            other => SyncError::Internal(format!("{other:?}")),
        }
    }

    /// A `DocSync` lane for `api`'s extension in this process (a linked
    /// one): an in-memory connection to its [`ExtensionDocSync`], held as
    /// long as the client.
    ///
    /// # Errors
    ///
    /// The lane could not be made.
    pub async fn serve_in_memory(api: ExtensionApi) -> Result<SyncClient, String> {
        let router = architect::layer::LayerRouter::new().with(
            crdt::sync::doc_sync_service_descriptor(),
            crdt::sync::DocSyncDispatcher::new(ExtensionDocSync::new(api)),
        );
        let (client_link, server_link) = vox::memory_link_pair(16);
        let server = tokio::spawn(async move {
            if let Ok(connection) = vox::acceptor_on(server_link)
                .on_lane(architect::layer::handler_acceptor(router))
                .establish_connection()
                .await
            {
                let _hold = connection;
                std::future::pending::<()>().await;
            }
        });
        match vox::initiator_on(client_link)
            .establish::<DocSyncClient>()
            .await
        {
            Ok(client) => Ok(SyncClient::held(client, Arc::new(Abort(server)))),
            Err(e) => {
                server.abort();
                Err(format!("the documents lane: {e:?}"))
            }
        }
    }

    /// Stops the in-memory lane's server when the last client goes.
    struct Abort(tokio::task::JoinHandle<()>);

    impl Drop for Abort {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
}

/// The extension's side: a lane to the host and a replica over it.
#[cfg(not(target_family = "wasm"))]
mod client {
    use std::sync::Arc;

    use crdt::sync::{DocSyncClient, SyncedDoc};
    use crdt::{CrdtDoc, PersistError};
    use tokio::sync::watch;

    use super::LiveDoc;

    /// A `DocSync` client to the host, and whatever keeps its lane open.
    #[derive(Clone)]
    pub struct SyncClient {
        client: DocSyncClient,
        _hold: Option<Arc<dyn std::any::Any + Send + Sync>>,
    }

    impl SyncClient {
        /// `client`, on a lane that stays open on its own (a connection's).
        #[must_use]
        pub const fn new(client: DocSyncClient) -> Self {
            Self {
                client,
                _hold: None,
            }
        }

        /// `client`, its lane open as long as `hold` is.
        #[must_use]
        pub fn held(client: DocSyncClient, hold: Arc<dyn std::any::Any + Send + Sync>) -> Self {
            Self {
                client,
                _hold: Some(hold),
            }
        }

        /// The client.
        #[must_use]
        pub const fn client(&self) -> &DocSyncClient {
            &self.client
        }
    }

    /// One live document of the extension's kind, and its lane to the
    /// node's copy through the host.
    #[derive(Clone)]
    pub struct DocLane {
        doc: LiveDoc,
        client: SyncClient,
    }

    impl std::fmt::Debug for DocLane {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("DocLane")
                .field("doc", &self.doc)
                .finish_non_exhaustive()
        }
    }

    impl DocLane {
        /// The document `doc`, synced over `client`.
        #[must_use]
        pub const fn new(doc: LiveDoc, client: SyncClient) -> Self {
            Self { doc, client }
        }

        /// The document of `live_document`'s answer, over `client`.
        ///
        /// # Errors
        ///
        /// The answer is not a [`LiveDoc`].
        pub fn from_answer(
            answer: serde_json::Value,
            client: SyncClient,
        ) -> Result<Self, crate::hostcall::HostError> {
            let doc =
                serde_json::from_value(answer).map_err(|e| crate::hostcall::HostError::Failed {
                    message: format!("the host's live_document answer: {e}"),
                })?;
            Ok(Self::new(doc, client))
        }

        /// What the host answered: its id, kind, path, whether writable.
        #[must_use]
        pub const fn doc(&self) -> &LiveDoc {
            &self.doc
        }

        /// The app's node's endpoint id, when it has one: what a leader
        /// names as the document's presence host
        /// (`crdt::presence::set_presence_host` on its replica), so the org's
        /// other devices follow its presence through this node (session#192).
        #[must_use]
        pub fn node(&self) -> Option<&str> {
            self.doc.node.as_deref()
        }

        /// The `DocSync` client to the host, for a session of one's own.
        #[must_use]
        pub const fn client(&self) -> &DocSyncClient {
            self.client.client()
        }

        /// Runs one session of `synced` (a `SyncedDoc` of
        /// [`Self::doc`]'s id over the extension's own `CrdtDoc`) against
        /// the node's copy: until the host or the node ends it, or the
        /// future is dropped. Again after it ends catches up by version.
        ///
        /// # Errors
        ///
        /// The host refused the session, or the replica could not take
        /// what came.
        pub async fn sync(&self, synced: &mut SyncedDoc) -> Result<(), PersistError> {
            synced.run(self.client()).await
        }

        /// An in-memory replica kept in step with the node's copy, on a
        /// task of its own (a tokio runtime's) until it is dropped.
        #[must_use]
        pub fn replica(self) -> Replica {
            let doc = CrdtDoc::ephemeral();
            let (tell, attached) = watch::channel(false);
            let synced =
                SyncedDoc::new(self.doc.doc_id, doc.clone()).on_attached(Arc::new(move || {
                    let _ = tell.send(true);
                }));
            let (ended_tx, ended) = watch::channel(None);
            let lane = self.clone();
            let task = tokio::spawn(async move {
                let mut synced = synced;
                let end = lane.sync(&mut synced).await.map_err(|e| e.to_string());
                let _ = ended_tx.send(Some(end));
            });
            Replica {
                lane: self,
                doc,
                attached,
                ended,
                task,
            }
        }
    }

    /// A replica of a live document ([`DocLane::replica`]): edit
    /// [`Self::doc`] and commit, and the node gets it; what others commit
    /// arrives in it.
    pub struct Replica {
        lane: DocLane,
        doc: CrdtDoc,
        attached: watch::Receiver<bool>,
        ended: watch::Receiver<Option<Result<(), String>>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl std::fmt::Debug for Replica {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Replica")
                .field("doc", self.lane.doc())
                .finish_non_exhaustive()
        }
    }

    impl Replica {
        /// The replica's document.
        #[must_use]
        pub const fn doc(&self) -> &CrdtDoc {
            &self.doc
        }

        /// What the host answered for it.
        #[must_use]
        pub const fn info(&self) -> &LiveDoc {
            self.lane.doc()
        }

        /// Waits until its session has caught up with the node's copy.
        ///
        /// # Errors
        ///
        /// The session ended before it did (refused, or the host gone):
        /// why.
        pub async fn attached(&self) -> Result<(), String> {
            let mut attached = self.attached.clone();
            let mut ended = self.ended.clone();
            loop {
                if *attached.borrow_and_update() {
                    return Ok(());
                }
                let end = ended.borrow_and_update().clone();
                if let Some(end) = end {
                    return Err(end.err().unwrap_or_else(|| "the session ended".to_owned()));
                }
                tokio::select! {
                    changed = attached.changed() => if changed.is_err() {
                        return Err("the session ended".to_owned());
                    },
                    changed = ended.changed() => if changed.is_err() {
                        return Err("the session ended".to_owned());
                    },
                }
            }
        }

        /// How its session ended, once it has (`None` while it runs).
        #[must_use]
        pub fn ended(&self) -> Option<Result<(), String>> {
            self.ended.borrow().clone()
        }
    }

    impl Drop for Replica {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
}
