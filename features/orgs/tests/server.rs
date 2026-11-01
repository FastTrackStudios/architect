//! Every service confined to the caller's org, over a real vox link: the
//! gate's identity learns each session's orgs from the identity server,
//! a per-org lane confines each call to its org, and the engine refuses
//! non-members, other orgs' resources and what the role may not do.

#![cfg(feature = "server")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use architect::layer::{LayerRouter, handler_acceptor};
use architect::permissions_gate::{PermissionsGate, UnlistedPolicy};
use architect_orgs::server::{
    BoxOrgsFuture, ConfinedEngine, Directory, Members, MembersResolver, confine, current_org,
    in_org,
};
use architect_orgs::{Org, OrgId, Role, Scope};
use architect_permissions::{
    Action, BoxIdentityFuture, IdentityResolver, MethodPermit, PermissionEngine, Principal,
    Resource, Rule, ServicePermits,
};

mod probe {
    #[vox::service]
    pub trait OrgProbe {
        async fn read_note(&self) -> String;
        async fn write_note(&self) -> String;
        /// The org the call ran in, or `"none"`.
        async fn where_am_i(&self) -> String;
    }

    #[derive(Clone)]
    pub struct OrgProbeService;

    impl OrgProbe for OrgProbeService {
        async fn read_note(&self) -> String {
            "read".into()
        }
        async fn write_note(&self) -> String {
            "written".into()
        }
        async fn where_am_i(&self) -> String {
            architect_orgs::server::current_org().map_or_else(|| "none".into(), |o| o.0)
        }
    }
}

use probe::{OrgProbeClient, OrgProbeDispatcher, OrgProbeService, org_probe_service_descriptor};

const PERMITS: ServicePermits = ServicePermits {
    service: "org-probe",
    methods: &[
        MethodPermit::new("read_note", "read", "notes/**"),
        MethodPermit::new("write_note", "write", "notes/**"),
        MethodPermit::new("where_am_i", "read", "notes/**"),
    ],
};

/// The identity server: alice is an admin of `fts` and a viewer
/// of `hp`; bob a member of `hp`. Counts its calls.
#[derive(Default)]
struct FakeIdentityServer {
    asked: Arc<AtomicUsize>,
}

impl Directory for FakeIdentityServer {
    fn orgs<'a>(&'a self, token: &'a str) -> BoxOrgsFuture<'a> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            match token {
                "alice" => Ok(vec![
                    Org::new("fts", "FastTrackStudio", Role::Admin),
                    Org::new("hp", "Harbor & Pine", Role::Viewer),
                ]),
                "bob" => Ok(vec![Org::new("hp", "Harbor & Pine", Role::Member)]),
                _ => Err("unknown session".into()),
            }
        })
    }
}

/// Sessions: the token is the user id.
struct Sessions;

impl IdentityResolver for Sessions {
    fn resolve<'a>(&'a self, bearer_token: Option<&'a str>) -> BoxIdentityFuture<'a> {
        let who = bearer_token.map_or(Principal::Anonymous, |t| Principal::User {
            user_id: t.to_owned(),
        });
        Box::pin(async move { who })
    }
}

fn engine(members: &Members) -> ConfinedEngine {
    ConfinedEngine::new(members.clone())
        .role(&Role::Viewer, vec![Rule::new("**", &["read"])])
        .role(&Role::Member, vec![Rule::new("**", &["read", "write"])])
        .role(
            &Role::Admin,
            vec![Rule::new("**", &["read", "write", "admin"])],
        )
}

struct Lanes {
    gate: Arc<PermissionsGate>,
    router: LayerRouter,
    asked: Arc<AtomicUsize>,
}

fn lanes() -> Lanes {
    let members = Members::new();
    let directory = FakeIdentityServer::default();
    let asked = Arc::clone(&directory.asked);
    let identity = MembersResolver::new(Sessions, directory, members.clone());
    let gate = PermissionsGate::new(Arc::new(engine(&members)), Arc::new(identity))
        .unlisted(UnlistedPolicy::Deny)
        .permit(org_probe_service_descriptor(), &PERMITS);
    let router = LayerRouter::new().with(
        org_probe_service_descriptor(),
        OrgProbeDispatcher::new(OrgProbeService),
    );
    Lanes {
        gate: Arc::new(gate),
        router,
        asked,
    }
}

/// A connection to `org`'s lane (`/org/{slug}/vox`) signed in as `token`.
async fn connect(lanes: &Lanes, org: &str, token: &str) -> OrgProbeClient {
    let handler = confine(
        OrgId::new(org),
        PermissionsGate::wrap_shared_with_bearer(
            Arc::clone(&lanes.gate),
            lanes.router.clone(),
            Some(token.to_owned()),
        ),
    );
    let (client_link, server_link) = vox::memory_link_pair(16);
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
    vox::initiator_on(client_link)
        .establish::<OrgProbeClient>()
        .await
        .expect("establish")
}

#[tokio::test]
async fn a_call_runs_in_its_lanes_org_and_the_role_there_decides() {
    let lanes = lanes();

    let alice_fts = connect(&lanes, "fts", "alice").await;
    assert_eq!(alice_fts.where_am_i().await.unwrap(), "fts");
    assert_eq!(alice_fts.write_note().await.unwrap(), "written");

    // A viewer in Harbor & Pine: reads, may not write.
    let alice_hp = connect(&lanes, "hp", "alice").await;
    assert_eq!(alice_hp.read_note().await.unwrap(), "read");
    assert!(alice_hp.write_note().await.is_err());

    // Bob is not in FastTrackStudio: nothing there, not even a read.
    let bob_fts = connect(&lanes, "fts", "bob").await;
    assert!(bob_fts.read_note().await.is_err());
    let bob_hp = connect(&lanes, "hp", "bob").await;
    assert_eq!(bob_hp.write_note().await.unwrap(), "written");

    // An unknown session is in no org; anonymous neither.
    let mallory = connect(&lanes, "fts", "mallory").await;
    assert!(mallory.read_note().await.is_err());

    // Each session was asked about once, not once per call.
    assert_eq!(lanes.asked.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn a_resource_names_its_org_and_another_orgs_is_refused() {
    let members = Members::new();
    let alice = Principal::User {
        user_id: "alice".into(),
    };
    members.set(
        &alice,
        vec![
            Org::new("fts", "FastTrackStudio", Role::Admin),
            Org::new("hp", "Harbor & Pine", Role::Viewer),
        ],
    );
    let engine = engine(&members);
    let read = Action::read();
    let write = Action::write();

    // On the All-organizations lane (no org): the resource's org decides.
    assert!(current_org().is_none());
    let hp_note = Resource::new(OrgId::new("hp").path("notes/a"));
    assert!(engine.check(&alice, &hp_note, &read).allowed());
    assert!(!engine.check(&alice, &hp_note, &write).allowed());
    let other = Resource::new(OrgId::new("nope").path("notes/a"));
    assert!(!engine.check(&alice, &other, &read).allowed());
    // In no org at all: the inner engine's (allow).
    assert!(
        engine
            .check(&alice, &Resource::new("public/x"), &write)
            .allowed()
    );

    // On FastTrackStudio's lane, Harbor & Pine's notes are out of reach,
    // whatever her role there.
    in_org(OrgId::new("fts"), async {
        assert!(
            engine
                .check(&alice, &Resource::new("notes/a"), &write)
                .allowed()
        );
        assert!(!engine.check(&alice, &hp_note, &read).allowed());
    })
    .await;

    // An in-process service is in every org.
    let service = Principal::Service {
        name: "indexer".into(),
    };
    in_org(OrgId::new("hp"), async {
        assert!(
            engine
                .check(&service, &Resource::new("notes/a"), &write)
                .allowed()
        );
    })
    .await;

    // The All read on the server covers the caller's orgs.
    assert_eq!(
        members.scope_of(&alice),
        Scope::All(vec![OrgId::new("fts"), OrgId::new("hp")])
    );
    members.forget(&alice);
    assert!(members.orgs_of(&alice).is_empty());
}
