//! Spaces end to end over a real vox link: an owner makes a space in her
//! org and adds a person from another org as a commenter; that person
//! reads and comments on the space's documents and files and nothing
//! else, through the gate, the documents' permissions and the files
//! feature's generated grants; removing them binds on their next call.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

mod common;

use std::sync::Arc;

use architect::layer::handler_acceptor;
use architect::local::RawLaneCaller;
use architect::permissions_gate::PermissionsGate;
use architect_files::{Capability, FilesClient, Save};
use architect_orgs::server::{confine, in_org};
use architect_spaces::server::{FileStore, MemoryStore, Spaces};
use architect_spaces::{
    COMMENTER, EDITOR, NewSpace, SpaceChange, SpaceGrant, SpacePatch, SpacePolicy, SpaceRole,
    SpacesFault, SpacesService, SpacesServiceClient, Subject, VIEWER,
};
use common::{WorkspaceClient, World, hp, may, sign_in, user, world};
use crdt::registry::{DocAccess, DocPermissions};
use uuid::Uuid;

/// What one person holds over one connection to `hp`'s lane.
struct Session {
    spaces: SpacesServiceClient,
    files: FilesClient,
    docs: WorkspaceClient,
}

/// `who`, connected to `hp`'s lane (`/org/hp/vox`) over a vox memory link.
async fn connect(world: &World, who: &str) -> Session {
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
    let caller = lane.caller.clone();
    std::mem::forget(lane.connection);
    Session {
        spaces: SpacesServiceClient::new(caller.clone()),
        files: FilesClient::new(caller.clone()),
        docs: WorkspaceClient::new(caller),
    }
}

fn carol() -> Subject {
    Subject::User("carol".into())
}

#[tokio::test]
async fn a_person_from_another_org_comments_in_a_space_and_nothing_else() {
    let world = world(MemoryStore::new()).await;
    let alice = connect(&world, "alice").await;
    let carol_on_hp = connect(&world, "carol").await;

    // Alice's org: a project's documents and a root of files.
    let (alpha_doc, beta_doc) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let root = alice
        .files
        .ensure_root("projects", "Projects")
        .await
        .unwrap();
    alice
        .files
        .put(
            root.id,
            "Alpha/brief.txt",
            b"the brief",
            Save::create_only(),
        )
        .await
        .unwrap();
    alice
        .files
        .put(root.id, "Beta/secret.txt", b"no", Save::create_only())
        .await
        .unwrap();

    // Before any space: carol, from another org, holds nothing here.
    assert!(
        carol_on_hp
            .docs
            .read_doc(alpha_doc.to_string())
            .await
            .is_err()
    );
    assert!(carol_on_hp.files.service.roots().await.is_err());
    assert!(carol_on_hp.spaces.spaces().await.is_err());

    let space = alice
        .spaces
        .create_space(NewSpace {
            name: "Alpha".into(),
            kind: "project".into(),
            scope: vec![
                format!("doc/{alpha_doc}"),
                format!("files/{}/Alpha/", root.id),
            ],
            exclude: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(space.members.len(), 1, "the creator manages it");
    alice
        .spaces
        .set_member(space.id.clone(), carol(), Some(COMMENTER.into()), None)
        .await
        .unwrap();

    // Documents: read and comment inside, nothing else.
    assert_eq!(
        carol_on_hp
            .docs
            .read_doc(alpha_doc.to_string())
            .await
            .unwrap(),
        format!("contents of {alpha_doc}")
    );
    assert_eq!(
        carol_on_hp
            .docs
            .comment(alpha_doc.to_string(), "lgtm".into())
            .await
            .unwrap(),
        format!("user:carol on {alpha_doc}: lgtm")
    );
    let refused = carol_on_hp
        .docs
        .write_doc(alpha_doc.to_string(), "x".into())
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("permission denied"), "{refused}");
    assert!(
        carol_on_hp
            .docs
            .read_doc(beta_doc.to_string())
            .await
            .is_err()
    );
    assert!(
        carol_on_hp
            .docs
            .comment(beta_doc.to_string(), "hi".into())
            .await
            .is_err()
    );

    // The documents' own permissions (what `DocRegistry` and `DocsHost`
    // ask) agree: read the space's document, not write it, nor another.
    let docs = DocPermissions::new(Arc::clone(&world.engine));
    let carol_principal = user("carol");
    in_org(hp(), async {
        assert!(
            docs.check(Some(&carol_principal), alpha_doc, DocAccess::Read)
                .is_ok()
        );
        assert!(
            docs.check(Some(&carol_principal), alpha_doc, DocAccess::Write)
                .is_err()
        );
        assert!(
            docs.check(Some(&carol_principal), beta_doc, DocAccess::Read)
                .is_err()
        );
    })
    .await;

    // Files: the space's folder, generated as a grant; nothing beside it.
    let roots = carol_on_hp.files.service.roots().await.unwrap();
    assert_eq!(roots.len(), 1);
    let names: Vec<String> = carol_on_hp
        .files
        .browse(root.id, "")
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.path.as_str().to_owned())
        .collect();
    assert_eq!(names, vec!["Alpha".to_owned()], "Beta is absent");
    assert_eq!(
        carol_on_hp
            .files
            .get(root.id, "Alpha/brief.txt")
            .await
            .unwrap(),
        b"the brief"
    );
    assert!(
        carol_on_hp
            .files
            .get(root.id, "Beta/secret.txt")
            .await
            .is_err()
    );
    assert!(
        carol_on_hp
            .files
            .put(root.id, "Alpha/notes.txt", b"mine", Save::create_only())
            .await
            .is_err(),
        "a commenter does not write files"
    );
    let at = carol_on_hp
        .files
        .service
        .effective(root.id, architect_files::RootPath::parse("Alpha").unwrap())
        .await
        .unwrap();
    assert_eq!(at.capabilities, vec![Capability::Read, Capability::History]);

    // The space itself: she sees it and what she holds, manages nothing.
    let listed = carol_on_hp.spaces.spaces().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].role.as_deref(), Some(COMMENTER));
    carol_on_hp.spaces.space(space.id.clone()).await.unwrap();
    let held = carol_on_hp
        .spaces
        .effective(space.id.clone())
        .await
        .unwrap();
    assert!(
        held.iter()
            .any(|g| g.resource == format!("doc/{alpha_doc}") && g.actions == ["read", "comment"])
    );
    assert!(
        carol_on_hp
            .spaces
            .set_member(space.id.clone(), carol(), Some(EDITOR.into()), None)
            .await
            .is_err(),
        "a commenter cannot promote herself"
    );
    assert!(carol_on_hp.spaces.audit(space.id.clone()).await.is_err());
    assert!(
        carol_on_hp
            .spaces
            .create_space(NewSpace {
                name: "Mine".into(),
                kind: "project".into(),
                scope: vec!["doc/**".into()],
                exclude: Vec::new(),
            })
            .await
            .is_err(),
        "no space-making in an org one is not in"
    );

    // Removed: the next call is refused, on the same connection.
    alice
        .spaces
        .remove_member(space.id.clone(), carol())
        .await
        .unwrap();
    assert!(
        carol_on_hp
            .docs
            .read_doc(alpha_doc.to_string())
            .await
            .is_err()
    );
    assert!(
        carol_on_hp
            .files
            .get(root.id, "Alpha/brief.txt")
            .await
            .is_err()
    );
    assert!(carol_on_hp.spaces.spaces().await.is_err());

    // Every change is in the trail, in order.
    let trail = alice.spaces.audit(space.id.clone()).await.unwrap();
    let kinds: Vec<&str> = trail
        .iter()
        .map(|e| match &e.change {
            SpaceChange::Created { .. } => "created",
            SpaceChange::MemberSet { .. } => "member-set",
            SpaceChange::MemberRemoved { .. } => "member-removed",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["created", "member-set", "member-removed"]);
    assert!(trail.iter().all(|e| e.by == "user:alice"));
}

#[tokio::test]
async fn the_orgs_policy_caps_and_turns_off_outsiders_on_the_next_call() {
    let world = world(MemoryStore::new()).await;
    let alice = connect(&world, "alice").await;
    let bob = connect(&world, "bob").await;
    let carol_on_hp = connect(&world, "carol").await;
    let doc = Uuid::from_u128(7);

    let space = alice
        .spaces
        .create_space(NewSpace {
            name: "Room".into(),
            kind: "session".into(),
            scope: vec![format!("doc/{doc}")],
            exclude: Vec::new(),
        })
        .await
        .unwrap();
    // A manager from another org: capped below `admin` by default.
    alice
        .spaces
        .set_member(space.id.clone(), carol(), Some("manager".into()), None)
        .await
        .unwrap();
    carol_on_hp
        .docs
        .write_doc(doc.to_string(), "draft".into())
        .await
        .unwrap();
    let refused = carol_on_hp
        .spaces
        .set_member(
            space.id.clone(),
            Subject::User("eve".into()),
            Some(VIEWER.into()),
            None,
        )
        .await;
    assert!(
        matches!(&refused, Err(vox::VoxError::User(f)) if matches!(**f, SpacesFault::Denied(_))),
        "an outsider does not manage a space unless the policy says so: {refused:?}"
    );

    // A viewer of the org may not set the policy; the owner may.
    assert!(bob.spaces.set_policy(SpacePolicy::default()).await.is_err());
    let mut policy = alice.spaces.policy().await.unwrap();
    policy.external_cap = vec!["read".into()];
    alice.spaces.set_policy(policy.clone()).await.unwrap();
    assert!(
        carol_on_hp
            .docs
            .write_doc(doc.to_string(), "again".into())
            .await
            .is_err(),
        "capped to read on the next call"
    );
    carol_on_hp.docs.read_doc(doc.to_string()).await.unwrap();

    policy.external_members = false;
    alice.spaces.set_policy(policy).await.unwrap();
    assert!(carol_on_hp.docs.read_doc(doc.to_string()).await.is_err());
    assert!(carol_on_hp.spaces.space(space.id.clone()).await.is_err());
}

#[tokio::test]
async fn grants_roles_carve_outs_and_a_scope_change() {
    let world = world(MemoryStore::new()).await;
    let alice = connect(&world, "alice").await;
    let _carol = connect(&world, "carol").await;
    sign_in(&world, "carol").await;
    sign_in(&world, "bob").await;
    let (a, b) = (Uuid::from_u128(10), Uuid::from_u128(11));

    let space = alice
        .spaces
        .create_space(NewSpace {
            name: "Wiki".into(),
            kind: "wiki".into(),
            scope: vec!["vault/Wiki/".into(), format!("doc/{a}"), format!("doc/{b}")],
            exclude: vec!["vault/Wiki/Private/".into()],
        })
        .await
        .unwrap();

    // One resource, by grant, with no role.
    alice
        .spaces
        .grant(
            space.id.clone(),
            carol(),
            SpaceGrant::new(format!("doc/{a}"), &["read", "comment"]),
        )
        .await
        .unwrap();
    assert!(may(&world, "carol", &format!("doc/{a}"), "comment").await);
    assert!(!may(&world, "carol", &format!("doc/{b}"), "read").await);
    // A grant must stay inside the space.
    assert!(
        alice
            .spaces
            .grant(
                space.id.clone(),
                carol(),
                SpaceGrant::new("vault/**", &["read"]),
            )
            .await
            .is_err()
    );

    // A role of the space's own.
    alice
        .spaces
        .set_role(space.id.clone(), SpaceRole::new("reader", &["read"]))
        .await
        .unwrap();
    alice
        .spaces
        .set_member(space.id.clone(), carol(), Some("reader".into()), None)
        .await
        .unwrap();
    assert!(may(&world, "carol", "vault/Wiki/Home.md", "read").await);
    assert!(
        !may(&world, "carol", "vault/Wiki/Private/pay.md", "read").await,
        "carved out"
    );
    assert!(!may(&world, "carol", "vault/Other/x.md", "read").await);
    assert!(
        alice
            .spaces
            .remove_role(space.id.clone(), "reader".into())
            .await
            .is_err(),
        "a role a member holds stays"
    );
    assert!(
        alice
            .spaces
            .set_member(space.id.clone(), carol(), Some("nope".into()), None)
            .await
            .is_err()
    );

    // An org viewer made an editor writes in the space, not beside it.
    alice
        .spaces
        .set_member(
            space.id.clone(),
            Subject::User("bob".into()),
            Some(EDITOR.into()),
            None,
        )
        .await
        .unwrap();
    assert!(may(&world, "bob", "vault/Wiki/Home.md", "write").await);
    assert!(!may(&world, "bob", "vault/Other/x.md", "write").await);
    assert!(
        may(&world, "bob", "vault/Other/x.md", "read").await,
        "his org role"
    );

    // Narrow the scope: the grant on `a` is dropped, and what the space
    // no longer covers nobody holds through it.
    alice
        .spaces
        .update_space(
            space.id.clone(),
            SpacePatch {
                scope: Some(vec!["vault/Wiki/".into(), format!("doc/{b}")]),
                ..SpacePatch::default()
            },
        )
        .await
        .unwrap();
    assert!(!may(&world, "carol", &format!("doc/{a}"), "read").await);
    let room = alice.spaces.space(space.id.clone()).await.unwrap();
    let carol_now = room.member(&carol()).unwrap();
    assert!(carol_now.grants.is_empty());
    assert_eq!(carol_now.role.as_deref(), Some("reader"));

    // Expired membership conveys nothing.
    alice
        .spaces
        .set_member(space.id.clone(), carol(), Some("reader".into()), Some(1))
        .await
        .unwrap();
    assert!(!may(&world, "carol", "vault/Wiki/Home.md", "read").await);

    // Deleting the space ends everything in it.
    alice.spaces.delete_space(space.id.clone()).await.unwrap();
    assert!(!may(&world, "bob", "vault/Wiki/Home.md", "write").await);
    assert!(alice.spaces.spaces().await.unwrap().is_empty());
}

#[tokio::test]
async fn spaces_and_their_trail_survive_a_restart() {
    let data = tempfile::tempdir().unwrap();
    let id = {
        let world = world(FileStore::new(data.path())).await;
        sign_in(&world, "alice").await;
        let alice = world.spaces.acting(hp(), user("alice"));
        let space = alice
            .create_space(NewSpace {
                name: "Kept".into(),
                kind: "folder".into(),
                scope: vec!["vault/Kept/".into()],
                exclude: Vec::new(),
            })
            .await
            .unwrap();
        alice
            .set_member(space.id.clone(), carol(), Some(COMMENTER.into()), None)
            .await
            .unwrap();
        space.id
    };

    let members = architect_orgs::server::Members::new();
    let reopened = Spaces::open(FileStore::new(data.path()), members)
        .await
        .unwrap();
    let state = reopened.snapshot(&hp());
    let space = state.space(&id).unwrap();
    assert_eq!(space.name, "Kept");
    assert_eq!(
        space.member(&carol()).unwrap().role.as_deref(),
        Some(COMMENTER)
    );
    let service = architect_spaces::server::Spaces::acting(
        &reopened,
        hp(),
        architect_permissions::Principal::Service {
            name: "test".into(),
        },
    );
    let trail = service.audit(id).await.unwrap();
    assert_eq!(trail.len(), 2);
}

#[tokio::test]
async fn only_the_orgs_members_make_spaces_and_viewers_do_not() {
    let world = world(MemoryStore::new()).await;
    let bob = connect(&world, "bob").await;
    let dave = connect(&world, "dave").await;
    let new = || NewSpace {
        name: "X".into(),
        kind: "project".into(),
        scope: vec!["vault/X/".into()],
        exclude: Vec::new(),
    };
    assert!(bob.spaces.create_space(new()).await.is_err(), "a viewer");
    let made = dave.spaces.create_space(new()).await.unwrap();
    assert_eq!(made.created_by, "user:dave");
    assert!(
        dave.spaces
            .create_space(NewSpace {
                scope: vec!["org/fts/vault/".into()],
                ..new()
            })
            .await
            .is_err(),
        "a scope names this org's resources only"
    );
}
