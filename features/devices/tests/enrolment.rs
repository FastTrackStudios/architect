//! Per-org enrolment over in-process vox: a member enrols their device in
//! the lane's org with a role no higher than their own; the org's engine
//! then gives the device that role's rules there (as a `Principal::Host`
//! member); revoking it takes them away.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use architect::layer::{LayerRouter, handler_acceptor};
use architect::permissions_gate::PermissionsGate;
use architect_devices::server::{EnrolmentHost, project_members};
use architect_devices::{
    Admission, DeviceEnrolmentClient, DeviceEnrolmentDispatcher, ENROL_PERMITS, EnrolError,
    Enrolment, Grant, device_enrolment_service_descriptor,
};
use architect_orgs::server::{ConfinedEngine, Members, confine};
use architect_orgs::{Org, OrgId, Role};
use architect_permissions::{
    Action, BoxIdentityFuture, IdentityResolver, PermissionEngine, Principal, Resource, Rule,
};

const DEVICE: &str = "a1b2c3d4";

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

fn user(id: &str) -> Principal {
    Principal::User { user_id: id.into() }
}

fn device() -> Principal {
    Principal::Host {
        endpoint: DEVICE.into(),
    }
}

struct Org1 {
    gate: Arc<PermissionsGate>,
    router: LayerRouter,
    engine: Arc<ConfinedEngine>,
    admission: Admission,
}

/// The org `fts`: alice and eve are members, root an admin; bob is in no
/// org.
fn org() -> Org1 {
    let admission = Admission::new();
    let members = Members::new();
    project_members(&admission, &members);
    let fts = |role| vec![Org::new("fts", "FastTrackStudio", role)];
    members.set(&user("alice"), fts(Role::Member));
    members.set(&user("eve"), fts(Role::Member));
    members.set(&user("root"), fts(Role::Admin));
    let engine = Arc::new(
        ConfinedEngine::new(members.clone())
            .role(&Role::Viewer, vec![Rule::new("**", &["read"])])
            .role(&Role::Member, vec![Rule::new("**", &["read", "write"])])
            .role(
                &Role::Admin,
                vec![Rule::new("**", &["read", "write", "admin"])],
            ),
    );
    let gate = PermissionsGate::new(engine.clone(), Arc::new(Sessions))
        .permit(device_enrolment_service_descriptor(), &ENROL_PERMITS);
    let router = LayerRouter::new().with(
        device_enrolment_service_descriptor(),
        DeviceEnrolmentDispatcher::new(EnrolmentHost::new(admission.clone(), members)),
    );
    Org1 {
        gate: Arc::new(gate),
        router,
        engine,
        admission,
    }
}

/// A connection to `org`'s lane signed in as `token`.
async fn connect(lanes: &Org1, org: &str, token: &str) -> DeviceEnrolmentClient {
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
        .establish::<DeviceEnrolmentClient>()
        .await
        .unwrap()
}

fn enrolment(role: Role) -> Enrolment {
    Enrolment {
        endpoint: DEVICE.to_uppercase(),
        name: "Alice's laptop".into(),
        platform: "macos".into(),
        role,
    }
}

fn refused<T: std::fmt::Debug>(res: Result<T, vox::VoxError<EnrolError>>) -> EnrolError {
    match res {
        Err(vox::VoxError::User(e)) => *e,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn a_member_enrols_their_device_with_a_role_the_org_then_enforces() {
    let org = org();
    let alice = connect(&org, "fts", "alice").await;
    let note = Resource::new("org/fts/notes/1");

    // Not yet enrolled: the device is nobody in the org.
    assert!(
        !org.engine
            .check(&device(), &note, &Action::read())
            .allowed()
    );

    let peer = alice.enrol(enrolment(Role::Viewer)).await.unwrap();
    assert_eq!(peer.endpoint, DEVICE);
    assert_eq!(
        peer.grants,
        vec![Grant::Org {
            org: OrgId::new("fts"),
            role: Role::Viewer,
            by: "user:alice".into(),
        }]
    );
    assert!(
        org.engine
            .check(&device(), &note, &Action::read())
            .allowed()
    );
    assert!(
        !org.engine
            .check(&device(), &note, &Action::write())
            .allowed(),
        "a viewer's device may not write"
    );
    let elsewhere = Resource::new("org/hp/notes/1");
    assert!(
        !org.engine
            .check(&device(), &elsewhere, &Action::read())
            .allowed()
    );

    // Above her own role: refused.
    assert!(matches!(
        refused(alice.enrol(enrolment(Role::Admin)).await),
        EnrolError::Denied(_)
    ));
    // Again, as a member: the role changes.
    alice.enrol(enrolment(Role::Member)).await.unwrap();
    assert!(
        org.engine
            .check(&device(), &note, &Action::write())
            .allowed()
    );

    // Revoked: the device is nobody in the org again.
    alice.revoke(DEVICE.into()).await.unwrap();
    assert!(
        !org.engine
            .check(&device(), &note, &Action::read())
            .allowed()
    );
    assert!(!org.admission.admits(DEVICE));
    assert_eq!(
        refused(alice.revoke(DEVICE.into()).await),
        EnrolError::NotEnrolled
    );
}

#[tokio::test]
async fn enrolments_are_listed_and_revoked_by_their_member_or_an_admin() {
    let org = org();
    let alice = connect(&org, "fts", "alice").await;
    let eve = connect(&org, "fts", "eve").await;
    let root = connect(&org, "fts", "root").await;
    alice.enrol(enrolment(Role::Member)).await.unwrap();

    assert_eq!(alice.enrolments().await.unwrap().len(), 1);
    assert_eq!(
        root.enrolments().await.unwrap().len(),
        1,
        "an admin sees all"
    );
    assert!(eve.enrolments().await.unwrap().is_empty(), "not hers");

    // Another member neither revokes it nor takes it over.
    assert!(matches!(
        refused(eve.revoke(DEVICE.into()).await),
        EnrolError::Denied(_)
    ));
    assert!(matches!(
        refused(eve.enrol(enrolment(Role::Viewer)).await),
        EnrolError::Denied(_)
    ));
    // An admin does.
    root.revoke(DEVICE.into()).await.unwrap();
    assert!(root.enrolments().await.unwrap().is_empty());
}

#[tokio::test]
async fn only_a_member_of_the_lanes_org_enrols() {
    let org = org();
    // Bob is in no org: the gate refuses him before the service runs.
    let bob = connect(&org, "fts", "bob").await;
    assert!(bob.enrol(enrolment(Role::Viewer)).await.is_err());
    // Alice on another org's lane: not a member there.
    let alice_hp = connect(&org, "hp", "alice").await;
    assert!(alice_hp.enrol(enrolment(Role::Viewer)).await.is_err());
    assert!(!org.admission.admits(DEVICE));
}
