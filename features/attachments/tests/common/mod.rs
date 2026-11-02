//! One server for the attachments tests: the gate (the org's
//! `ConfinedEngine` beside `SpacesEngine`), spaces, the files feature's
//! renditions (a fake transcoder), the server's copies of the documents,
//! attachments behind the gate on `hp`'s lane, and the byte face on a
//! real socket.
//!
//! People: alice owns Harbor & Pine (`hp`), bob views it, dave is a
//! member; carol is in the `fts` org only — from another org.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::significant_drop_tightening
)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use architect::Layer as _;
use architect::blob::{LocalFs, Sha256};
use architect::layer::{LayerRouter, handler_acceptor};
use architect::local::RawLaneCaller;
use architect::permissions_gate::{PermissionsGate, UnlistedPolicy};
use architect_attachments::server::{AttachmentsServer, WebAttachments};
use architect_attachments::{
    AttachmentsClient, PERMITS, attachments_service_rpc_service_descriptor,
};
use architect_files::server::{FilesServer, TranscodeError, Transcoder};
use architect_files::{MediaClass, RenditionKind};
use architect_orgs::server::{
    BoxOrgsFuture, ConfinedEngine, Directory, Members, MembersResolver, confine,
};
use architect_orgs::{Org, OrgId, Role};
use architect_permissions::{
    BoxIdentityFuture, CompositeEngine, IdentityResolver, PermissionEngine, Principal, Rule,
};
use architect_spaces::server::{MemoryStore, Spaces};
use futures_util::future::BoxFuture;
use loro::LoroDoc;
use uuid::Uuid;

/// The identity server.
pub struct Identity;

impl Directory for Identity {
    fn orgs<'a>(&'a self, token: &'a str) -> BoxOrgsFuture<'a> {
        Box::pin(async move {
            Ok(match token {
                "alice" => vec![Org::new("hp", "Harbor & Pine", Role::Owner)],
                "bob" => vec![Org::new("hp", "Harbor & Pine", Role::Viewer)],
                "dave" => vec![Org::new("hp", "Harbor & Pine", Role::Member)],
                "carol" => vec![Org::new("fts", "FastTrackStudio", Role::Member)],
                _ => Vec::new(),
            })
        })
    }
}

/// Sessions: the token is the user id.
pub struct Sessions;

impl IdentityResolver for Sessions {
    fn resolve<'a>(&'a self, bearer_token: Option<&'a str>) -> BoxIdentityFuture<'a> {
        let who = bearer_token.map_or(Principal::Anonymous, |t| Principal::User {
            user_id: t.to_owned(),
        });
        Box::pin(async move { who })
    }
}

/// Classifies by a prefix and renders the kind's name and the source's
/// hash.
pub struct Fake;

impl Transcoder for Fake {
    fn probe<'a>(&'a self, source: &'a Path) -> BoxFuture<'a, Result<MediaClass, TranscodeError>> {
        Box::pin(async move {
            let bytes = tokio::fs::read(source).await.unwrap();
            Ok(if bytes.starts_with(b"VIDEO") {
                MediaClass::Video
            } else {
                MediaClass::Other
            })
        })
    }

    fn render<'a>(
        &'a self,
        kind: RenditionKind,
        source: &'a Path,
        out: &'a Path,
    ) -> BoxFuture<'a, Result<(), TranscodeError>> {
        Box::pin(async move {
            let bytes = tokio::fs::read(source).await.unwrap();
            let made = format!("{kind}:{}", Sha256::of(&bytes).to_hex());
            tokio::fs::write(out, made).await.unwrap();
            Ok(())
        })
    }
}

/// The server's copies of `hp`'s documents (what a `DocRegistry` holds).
pub type Docs = Arc<Mutex<HashMap<Uuid, LoroDoc>>>;

pub struct World {
    pub dir: tempfile::TempDir,
    pub members: Members,
    pub spaces: Spaces,
    pub engine: Arc<dyn PermissionEngine>,
    pub docs: Docs,
    pub attachments: AttachmentsServer,
    pub gate: Arc<PermissionsGate>,
    pub router: LayerRouter,
    pub base: String,
}

pub fn hp() -> OrgId {
    OrgId::new("hp")
}

pub fn user(id: &str) -> Principal {
    Principal::User {
        user_id: id.to_owned(),
    }
}

/// The attachments server over `data`: what a restart builds again.
pub fn attachments(
    data: &Path,
    engine: &Arc<dyn PermissionEngine>,
    docs: &Docs,
    files: &FilesServer,
) -> AttachmentsServer {
    let docs = Arc::clone(docs);
    let source = move |org: &OrgId, id: Uuid| -> BoxFuture<'static, Option<LoroDoc>> {
        let doc = (org == &hp())
            .then(|| docs.lock().unwrap().get(&id).cloned())
            .flatten();
        Box::pin(async move { doc })
    };
    AttachmentsServer::new(data, LocalFs::new(blobs_dir(data)), Arc::clone(engine))
        .with_docs(source)
        .with_renditions(files.rendition_engine().cloned())
        .ticket_key([9; 32])
}

pub fn blobs_dir(data: &Path) -> PathBuf {
    data.join("blobs")
}

pub async fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let members = Members::new();
    let spaces = Spaces::open(MemoryStore::new(), members.clone())
        .await
        .unwrap();
    let org_engine = ConfinedEngine::new(members.clone())
        .role(&Role::Viewer, vec![Rule::new("**", &["read", "comment"])])
        .role(
            &Role::Member,
            vec![Rule::new("**", &["read", "comment", "write"])],
        )
        .role(&Role::Admin, vec![Rule::new("**", &["*"])])
        .role(&Role::Owner, vec![Rule::new("**", &["*"])]);
    let engine: Arc<dyn PermissionEngine> = Arc::new(
        CompositeEngine::new()
            .push(Arc::new(org_engine))
            .push(Arc::new(spaces.engine())),
    );
    let files = FilesServer::new(
        dir.path(),
        LocalFs::new(blobs_dir(dir.path())),
        members.clone(),
    )
    .with_renditions(Fake);
    let docs: Docs = Arc::default();
    let attachments = attachments(dir.path(), &engine, &docs, &files);
    let identity: Arc<dyn IdentityResolver> =
        Arc::new(MembersResolver::new(Sessions, Identity, members.clone()));
    let gate = PermissionsGate::new(Arc::clone(&engine), Arc::clone(&identity))
        .unlisted(UnlistedPolicy::Deny)
        .permit(attachments_service_rpc_service_descriptor(), &PERMITS);
    let router = LayerRouter::new().with(
        attachments_service_rpc_service_descriptor(),
        architect_attachments::serve(attachments.clone()),
    );

    // The byte face, resolving callers with the gate's own resolver.
    let face = WebAttachments::new(attachments.clone(), identity);
    let app = architect::layers![architect_attachments::web::Service].provide_http(&face);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        architect::http::axum::serve(listener, app).await.unwrap();
    });
    for who in ["alice", "bob", "dave", "carol"] {
        sign_in(&members, who).await;
    }
    World {
        dir,
        members,
        spaces,
        engine,
        docs,
        attachments,
        gate: Arc::new(gate),
        router,
        base: format!("http://{addr}"),
    }
}

/// Let the server learn `who`'s orgs, as the gate does on their first
/// call.
pub async fn sign_in(members: &Members, who: &str) {
    let resolver = MembersResolver::new(Sessions, Identity, members.clone());
    resolver.resolve(Some(who)).await;
}

/// `who` on `hp`'s vox lane.
pub async fn connect(world: &World, who: &str) -> AttachmentsClient {
    let handler = confine(
        hp(),
        PermissionsGate::wrap_shared_with_bearer(
            Arc::clone(&world.gate),
            world.router.clone(),
            Some(who.to_owned()),
        ),
    );
    let (client_link, server_link) = vox::memory_link_pair(64);
    tokio::spawn(async move {
        if let Ok(connection) = vox::acceptor_on(server_link)
            .on_lane(handler_acceptor(handler))
            .establish_connection()
            .await
        {
            let _hold = connection;
            std::future::pending::<()>().await;
        }
    });
    let lane = vox::initiator_on(client_link)
        .establish::<RawLaneCaller>()
        .await
        .unwrap();
    std::mem::forget(lane.connection);
    AttachmentsClient::new(lane.caller)
}

impl World {
    /// The server's copy of a document catches up with `doc` (what
    /// `crdt::sync` does over the lane).
    pub fn sync_up(&self, id: Uuid, doc: &LoroDoc) {
        let mut docs = self.docs.lock().unwrap();
        let copy = docs.entry(id).or_default();
        copy.import(&doc.export(loro::ExportMode::all_updates()).unwrap())
            .unwrap();
    }

    /// A device's replica catches up with the server's copy.
    pub fn sync_down(&self, id: Uuid, doc: &LoroDoc) {
        let docs = self.docs.lock().unwrap();
        let copy = docs.get(&id).unwrap();
        doc.import(&copy.export(loro::ExportMode::all_updates()).unwrap())
            .unwrap();
    }
}
