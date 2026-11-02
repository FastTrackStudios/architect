//! One server for the spaces tests: an identity server, the gate (the
//! org's `ConfinedEngine` beside `SpacesEngine`), spaces, the files
//! feature with grants generated from them, and a probe service that
//! checks each document it is asked about against the gate's engine —
//! the handler's half of the gate's contract.
//!
//! People: alice owns Harbor & Pine (`hp`), bob views it, dave is a
//! member; carol is in the `fts` org only — from another org.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc,
    clippy::must_use_candidate
)]

use std::sync::Arc;

use architect::blob::LocalFs;
use architect::layer::LayerRouter;
use architect::permissions_gate::{PermissionsGate, UnlistedPolicy};
use architect_files::server::FilesServer;
use architect_files::{
    PERMITS as FILES_PERMITS, STREAM_PERMITS, files_service_rpc_service_descriptor,
    files_service_stream_service_descriptor, stream_layer,
};
use architect_orgs::server::{BoxOrgsFuture, ConfinedEngine, Directory, Members, MembersResolver};
use architect_orgs::{Org, OrgId, Role};
use architect_permissions::{
    Action, BoxIdentityFuture, CompositeEngine, IdentityResolver, MethodPermit, PermissionEngine,
    Principal, Resource, Rule, ServicePermits,
};
use architect_spaces::server::{SpaceStore, Spaces};
use architect_spaces::{PERMITS as SPACES_PERMITS, spaces_service_rpc_service_descriptor};

pub mod probe {
    use std::sync::Arc;

    use architect_permissions::{Action, PermissionEngine, Principal, Resource};

    /// An org's documents, as a service whose handlers check the exact
    /// document against the gate's engine.
    #[vox::service]
    pub trait Workspace {
        async fn read_doc(&self, doc: String) -> Result<String, String>;
        async fn comment(&self, doc: String, text: String) -> Result<String, String>;
        async fn write_doc(&self, doc: String, body: String) -> Result<String, String>;
    }

    #[derive(Clone)]
    pub struct WorkspaceService {
        pub engine: Arc<dyn PermissionEngine>,
    }

    impl WorkspaceService {
        /// The handler's check: the gate let the caller in on `doc/**`;
        /// this document is the question.
        fn need(&self, doc: &str, action: &str) -> Result<Principal, String> {
            let who = architect::permissions_gate::caller().ok_or("no caller")?;
            let decision = self.engine.check(
                &who,
                &Resource::new(format!("doc/{doc}")),
                &Action::new(action),
            );
            match decision {
                architect_permissions::Decision::Allow => Ok(who),
                architect_permissions::Decision::Deny { reason } => Err(reason),
            }
        }
    }

    impl Workspace for WorkspaceService {
        async fn read_doc(&self, doc: String) -> Result<String, String> {
            self.need(&doc, Action::READ)
                .map(|_| format!("contents of {doc}"))
        }
        async fn comment(&self, doc: String, text: String) -> Result<String, String> {
            self.need(&doc, Action::COMMENT)
                .map(|who| format!("{} on {doc}: {text}", who.describe()))
        }
        async fn write_doc(&self, doc: String, body: String) -> Result<String, String> {
            self.need(&doc, Action::WRITE)
                .map(|_| format!("{doc} = {body}"))
        }
    }
}

pub use probe::{
    WorkspaceClient, WorkspaceDispatcher, WorkspaceService, workspace_service_descriptor,
};

pub const WORKSPACE_PERMITS: ServicePermits = ServicePermits {
    service: "workspace",
    methods: &[
        MethodPermit::new("read_doc", "read", "doc/{doc}"),
        MethodPermit::new("comment", "comment", "doc/{doc}"),
        MethodPermit::new("write_doc", "write", "doc/{doc}"),
    ],
};

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

pub fn user(id: &str) -> Principal {
    Principal::User {
        user_id: id.to_owned(),
    }
}

pub fn hp() -> OrgId {
    OrgId::new("hp")
}

pub struct World {
    pub dir: tempfile::TempDir,
    pub members: Members,
    pub spaces: Spaces,
    pub files: FilesServer,
    pub engine: Arc<dyn PermissionEngine>,
    pub gate: Arc<PermissionsGate>,
    pub router: LayerRouter,
}

/// The server, its spaces kept in `store`.
pub async fn world(store: impl SpaceStore + 'static) -> World {
    let dir = tempfile::tempdir().unwrap();
    let members = Members::new();
    let spaces = Spaces::open(store, members.clone()).await.unwrap();
    let org_engine = ConfinedEngine::new(members.clone())
        .role(&Role::Viewer, vec![Rule::new("**", &["read", "comment"])])
        .role(
            &Role::Member,
            vec![Rule::new("**", &["read", "comment", "write", "download"])],
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
        LocalFs::new(dir.path().join("blobs")),
        members.clone(),
    )
    .with_grants(spaces.clone());
    let identity = MembersResolver::new(Sessions, Identity, members.clone());
    let gate = PermissionsGate::new(Arc::clone(&engine), Arc::new(identity))
        .unlisted(UnlistedPolicy::Deny)
        .permit(workspace_service_descriptor(), &WORKSPACE_PERMITS)
        .permit(spaces_service_rpc_service_descriptor(), &SPACES_PERMITS)
        .permit(files_service_rpc_service_descriptor(), &FILES_PERMITS)
        .permit(files_service_stream_service_descriptor(), &STREAM_PERMITS);
    let router = LayerRouter::new()
        .with(
            workspace_service_descriptor(),
            WorkspaceDispatcher::new(WorkspaceService {
                engine: Arc::clone(&engine),
            }),
        )
        .with(
            spaces_service_rpc_service_descriptor(),
            architect_spaces::serve(spaces.clone()),
        )
        .with(
            files_service_rpc_service_descriptor(),
            architect_files::serve(files.clone()),
        )
        .merge(stream_layer(files.clone()));
    World {
        dir,
        members,
        spaces,
        files,
        engine,
        gate: Arc::new(gate),
        router,
    }
}

/// Let the server learn `who`'s orgs, as the gate would on their first
/// call.
pub async fn sign_in(world: &World, who: &str) {
    let resolver = MembersResolver::new(Sessions, Identity, world.members.clone());
    resolver.resolve(Some(who)).await;
}

/// Ask the composite engine directly, in `hp`.
pub async fn may(world: &World, who: &str, resource: &str, action: &str) -> bool {
    let engine = Arc::clone(&world.engine);
    let (who, resource, action) = (user(who), Resource::new(resource), Action::new(action));
    architect_orgs::server::in_org(hp(), async move {
        engine.check(&who, &resource, &action).allowed()
    })
    .await
}
