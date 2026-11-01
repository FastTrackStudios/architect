//! The files feature end to end, over a real vox link: the permissions
//! gate resolves each session and learns its orgs from the identity
//! server, each connection is confined to its org's lane, and the files
//! feature decides per path by the caller's role and grants.

#![cfg(feature = "server")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::needless_pass_by_value
)]

use std::sync::{Arc, Mutex};

use architect::blob::LocalFs;
use architect::layer::{LayerRouter, handler_acceptor};
use architect::local::RawLaneCaller;
use architect::permissions_gate::{PermissionsGate, UnlistedPolicy};
use architect_files::server::FilesServer;
use architect_files::{
    Capability, ClientError, EntryKind, FilesClient, FilesEvent, FilesFault, FilesService,
    OnConflict, PERMITS, Relocation, Resolved, RootPath, STREAM_PERMITS, Save, UploadFrame,
    UploadSpec, content_of, files_service_rpc_service_descriptor,
    files_service_stream_service_descriptor, serve, stream_layer,
};
use architect_orgs::server::{
    BoxOrgsFuture, ConfinedEngine, Directory, Members, MembersResolver, confine,
};
use architect_orgs::{Org, OrgId, Role};
use architect_permissions::{
    AllowAll, BoxIdentityFuture, IdentityResolver, PermissionEngine, Principal,
};

/// The identity server: alice owns `fts` and views `hp`; bob views
/// `fts`; carol is a member of it; erin is in `hp` only.
struct Identity;

impl Directory for Identity {
    fn orgs<'a>(&'a self, token: &'a str) -> BoxOrgsFuture<'a> {
        Box::pin(async move {
            Ok(match token {
                "alice" => vec![
                    Org::new("fts", "FastTrackStudio", Role::Owner),
                    Org::new("hp", "Harbor & Pine", Role::Viewer),
                ],
                "bob" => vec![Org::new("fts", "FastTrackStudio", Role::Viewer)],
                "carol" => vec![Org::new("fts", "FastTrackStudio", Role::Member)],
                "erin" => vec![Org::new("hp", "Harbor & Pine", Role::Member)],
                _ => Vec::new(),
            })
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

struct World {
    dir: tempfile::TempDir,
    files: FilesServer,
    members: Members,
    gate: Arc<PermissionsGate>,
    router: LayerRouter,
}

/// A server whose gate checks with `engine` (`None`: membership, by
/// `ConfinedEngine`).
fn world(engine: Option<Arc<dyn PermissionEngine>>) -> World {
    let dir = tempfile::tempdir().unwrap();
    let members = Members::new();
    let files = FilesServer::new(
        dir.path(),
        LocalFs::new(dir.path().join("blobs")),
        members.clone(),
    );
    let engine = engine.unwrap_or_else(|| Arc::new(ConfinedEngine::new(members.clone())));
    let identity = MembersResolver::new(Sessions, Identity, members.clone());
    let gate = PermissionsGate::new(engine, Arc::new(identity))
        .unlisted(UnlistedPolicy::Deny)
        .permit(files_service_rpc_service_descriptor(), &PERMITS)
        .permit(files_service_stream_service_descriptor(), &STREAM_PERMITS);
    let router = LayerRouter::new()
        .with(files_service_rpc_service_descriptor(), serve(files.clone()))
        .merge(stream_layer(files.clone()));
    World {
        dir,
        files,
        members,
        gate: Arc::new(gate),
        router,
    }
}

/// `who`, connected to `org`'s lane (`/org/{slug}/vox`).
async fn connect(world: &World, org: &str, who: &str) -> FilesClient {
    let handler = confine(
        OrgId::new(org),
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
    // The connection lives as long as the test does.
    std::mem::forget(lane.connection);
    FilesClient::new(lane.caller)
}

fn p(s: &str) -> RootPath {
    RootPath::parse(s).unwrap()
}

fn fault(e: ClientError) -> FilesFault {
    e.fault()
        .cloned()
        .unwrap_or_else(|| panic!("not a fault: {e}"))
}

#[tokio::test]
async fn a_member_saves_reads_versions_and_follows_a_pin() {
    let world = world(None);
    let alice = connect(&world, "fts", "alice").await;
    let root = alice
        .ensure_root("signal/impulse-responses", "Impulse responses")
        .await
        .unwrap();
    // Asking again finds the same root.
    assert_eq!(
        alice
            .ensure_root("signal/impulse-responses", "x")
            .await
            .unwrap(),
        root
    );

    // More than a frame, so the save is chunked.
    let v1: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
    let saved = alice
        .put(root.id, "Cab/4x12.wav", &v1, Save::create_only())
        .await
        .unwrap();
    let pin = saved.content.clone().unwrap();
    assert_eq!(pin, content_of(&v1));
    assert_eq!(alice.get(root.id, "Cab/4x12.wav").await.unwrap(), v1);
    assert_eq!(
        alice
            .read_range(root.id, "Cab/4x12.wav", 1_000, 1_010)
            .await
            .unwrap(),
        v1[1_000..1_010].to_vec()
    );
    let folder = alice.browse(root.id, "").await.unwrap();
    assert_eq!(folder.len(), 1);
    assert_eq!(folder[0].kind, EntryKind::Directory);

    // A second save against the current etag lands; against the old one
    // it is refused as stale.
    let v2: Vec<u8> = v1.iter().map(|b| b.wrapping_add(1)).collect();
    alice
        .put(root.id, "Cab/4x12.wav", &v2, Save::replacing(pin.clone()))
        .await
        .unwrap();
    let stale = alice
        .put(
            root.id,
            "Cab/4x12.wav",
            b"late",
            Save::replacing(pin.clone()),
        )
        .await
        .unwrap_err();
    assert!(stale.is_stale(), "{stale}");
    let taken = alice
        .put(root.id, "Cab/4x12.wav", b"new", Save::create_only())
        .await
        .unwrap_err();
    assert!(stale.is_stale() && taken.is_stale(), "{taken}");

    // The pinned reference resolves as moved, and still reads v1.
    match alice
        .resolve(root.id, "Cab/4x12.wav", Some(pin.clone()))
        .await
        .unwrap()
    {
        Resolved::Moved { pinned, .. } => assert_eq!(pinned, pin),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        alice
            .get_ref(root.id, "Cab/4x12.wav", Some(pin.clone()))
            .await
            .unwrap(),
        v1
    );

    // History, a label, a restore, a delete and its undoing.
    let history = alice
        .service
        .history(root.id, p("Cab/4x12.wav"))
        .await
        .unwrap();
    assert_eq!(history.len(), 2);
    let label = alice
        .service
        .label(root.id, p("Cab/4x12.wav"), history[0].id, "Take one".into())
        .await
        .unwrap();
    assert_eq!(
        alice.service.labels(root.id, None).await.unwrap(),
        vec![label.clone()]
    );
    let again = alice
        .service
        .label(root.id, p("Cab/4x12.wav"), history[1].id, "Take one".into())
        .await
        .unwrap_err();
    assert!(matches!(fault(again.into()), FilesFault::Exists { .. }));
    let restored = alice
        .service
        .restore(root.id, p("Cab/4x12.wav"), history[0].id)
        .await
        .unwrap();
    assert_eq!(restored.content, Some(pin.clone()));
    alice
        .service
        .delete_paths(root.id, vec![p("Cab")])
        .await
        .unwrap();
    let gone = alice.get(root.id, "Cab/4x12.wav").await.unwrap_err();
    assert_eq!(fault(gone), FilesFault::PathNotFound(p("Cab/4x12.wav")));
    let back = alice
        .service
        .restore(root.id, p("Cab/4x12.wav"), history[1].id)
        .await
        .unwrap();
    assert_eq!(back.content, Some(content_of(&v2)));
    assert_eq!(
        alice
            .service
            .history(root.id, p("Cab/4x12.wav"))
            .await
            .unwrap()
            .len(),
        5,
        "two saves, a restore, a delete, a restore"
    );
    alice.service.unlabel(label.id).await.unwrap();
    assert!(
        alice
            .service
            .labels(root.id, None)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn moves_and_copies_keep_history_and_ask_on_collision() {
    let world = world(None);
    let carol = connect(&world, "fts", "carol").await;
    let root = carol.ensure_root("session/song", "Song").await.unwrap();
    carol
        .put(root.id, "a/take.wav", b"one", Save::create_only())
        .await
        .unwrap();
    carol
        .put(root.id, "a/take.wav", b"two", Save::overwrite())
        .await
        .unwrap();
    carol
        .put(root.id, "b/take.wav", b"other", Save::create_only())
        .await
        .unwrap();

    let copied = carol
        .service
        .copy_paths(
            root.id,
            vec![Relocation {
                from: p("a/take.wav"),
                to: p("b/take.wav"),
            }],
            OnConflict::KeepBoth,
        )
        .await
        .unwrap();
    assert_eq!(copied.outcomes[0].landed_at, Some(p("b/take (2).wav")));
    assert_eq!(carol.get(root.id, "b/take (2).wav").await.unwrap(), b"two");

    let refused = carol
        .service
        .move_paths(
            root.id,
            vec![Relocation {
                from: p("a"),
                to: p("b"),
            }],
            OnConflict::Fail,
        )
        .await
        .unwrap_err();
    assert_eq!(fault(refused.into()), FilesFault::Exists { path: p("b") });

    carol
        .service
        .rename(root.id, p("a"), "renamed".into())
        .await
        .unwrap();
    assert_eq!(
        carol
            .service
            .history(root.id, p("renamed/take.wav"))
            .await
            .unwrap()
            .len(),
        2,
        "a moved file keeps its history"
    );
    // All or none: one bad move in a batch moves nothing.
    let batch = carol
        .service
        .move_paths(
            root.id,
            vec![
                Relocation {
                    from: p("renamed"),
                    to: p("c"),
                },
                Relocation {
                    from: p("nothing"),
                    to: p("d"),
                },
            ],
            OnConflict::Fail,
        )
        .await
        .unwrap_err();
    assert_eq!(fault(batch.into()), FilesFault::PathNotFound(p("nothing")));
    assert!(carol.entry(root.id, "renamed").await.is_ok());
    let names: Vec<String> = carol
        .browse(root.id, "")
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.path.to_string())
        .collect();
    assert_eq!(names, vec!["b", "renamed"]);
}

#[tokio::test]
async fn a_role_decides_everywhere_and_a_grant_in_its_folder() {
    let world = world(None);
    let alice = connect(&world, "fts", "alice").await;
    let bob = connect(&world, "fts", "bob").await;
    let root = alice.ensure_root("mixes", "Mixes").await.unwrap();
    alice
        .put(root.id, "Stems/kick.wav", b"kick", Save::create_only())
        .await
        .unwrap();
    alice
        .service
        .create_dirs(root.id, vec![p("Deliverables")])
        .await
        .unwrap();

    // A viewer reads everywhere and writes nowhere.
    assert_eq!(bob.get(root.id, "Stems/kick.wav").await.unwrap(), b"kick");
    let refused = bob
        .put(root.id, "Stems/snare.wav", b"snare", Save::create_only())
        .await
        .unwrap_err();
    assert!(matches!(fault(refused), FilesFault::Denied { .. }));
    assert!(bob.ensure_root("bobs", "Bob's").await.is_err());
    assert!(bob.service.rename_root(root.id, "x".into()).await.is_err());

    // A grant: write, in Deliverables only.
    let grant = alice
        .service
        .grant(
            root.id,
            p("Deliverables"),
            "bob".into(),
            vec![Capability::Write],
        )
        .await
        .unwrap();
    bob.put(root.id, "Deliverables/mix.wav", b"mix", Save::create_only())
        .await
        .unwrap();
    assert!(
        bob.put(root.id, "Stems/snare.wav", b"snare", Save::create_only())
            .await
            .is_err()
    );
    assert_eq!(
        bob.service
            .effective(root.id, p("Deliverables/mix.wav"))
            .await
            .unwrap()
            .capabilities,
        vec![Capability::Read, Capability::Write, Capability::History]
    );
    // Bob may not share what he holds without `Share`.
    assert!(
        bob.service
            .grant(
                root.id,
                p("Deliverables"),
                "carol".into(),
                vec![Capability::Read]
            )
            .await
            .is_err()
    );
    assert_eq!(
        alice
            .service
            .grants(root.id, Some(p("Deliverables/mix.wav")))
            .await
            .unwrap(),
        vec![grant.clone()]
    );

    // Revoked: binds on the next call.
    alice.service.revoke(grant.id).await.unwrap();
    assert!(
        bob.put(
            root.id,
            "Deliverables/late.wav",
            b"late",
            Save::create_only()
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn outside_what_a_person_holds_a_path_is_absent() {
    // In process, as the server's own jobs call it: erin is in no role
    // in FastTrackStudio and holds one folder.
    let world = world(None);
    let fts = OrgId::new("fts");
    let owner = world.files.acting(
        fts.clone(),
        Principal::Service {
            name: "seed".into(),
        },
    );
    let root = owner
        .ensure_root("client".into(), "Client".into())
        .await
        .unwrap();
    owner
        .create_dirs(root.id, vec![p("Client/Deliverables"), p("Stems")])
        .await
        .unwrap();
    owner
        .grant(
            root.id,
            p("Client/Deliverables"),
            "erin".into(),
            vec![Capability::Read],
        )
        .await
        .unwrap();

    let erin = world.files.acting(
        fts,
        Principal::User {
            user_id: "erin".into(),
        },
    );
    assert_eq!(erin.roots().await.unwrap(), vec![root.clone()]);
    let top: Vec<_> = erin.browse(root.id, RootPath::root()).await.unwrap();
    assert_eq!(
        top.iter().map(|e| e.path.clone()).collect::<Vec<_>>(),
        vec![p("Client")]
    );
    let inside = erin.browse(root.id, p("Client")).await.unwrap();
    assert_eq!(inside.len(), 1);
    assert_eq!(
        erin.entry(root.id, p("Stems")).await.unwrap_err(),
        FilesFault::PathNotFound(p("Stems"))
    );
    assert!(matches!(
        erin.create_dirs(root.id, vec![p("Client/Deliverables/x")])
            .await,
        Err(FilesFault::Denied { .. })
    ));
    assert!(matches!(erin.usage().await, Err(FilesFault::Denied { .. })));
    // No identity is no access, never the server's.
    let anonymous = world.files.acting(OrgId::new("fts"), Principal::Anonymous);
    assert_eq!(
        anonymous.roots().await.unwrap_err(),
        FilesFault::Unauthenticated
    );
}

#[tokio::test]
async fn an_org_sees_only_its_own_files_and_the_gate_keeps_non_members_out() {
    let world = world(None);
    let alice_fts = connect(&world, "fts", "alice").await;
    let fts_root = alice_fts.ensure_root("notes", "Notes").await.unwrap();
    let erin_hp = connect(&world, "hp", "erin").await;
    let hp_root = erin_hp.ensure_root("notes", "Notes").await.unwrap();
    assert_ne!(fts_root.id, hp_root.id, "the same dir, two orgs, two roots");

    let alice_hp = connect(&world, "hp", "alice").await;
    assert_eq!(
        alice_hp.service.roots().await.unwrap(),
        vec![hp_root.clone()]
    );
    let other = alice_hp.service.root(fts_root.id).await.unwrap_err();
    assert_eq!(fault(other.into()), FilesFault::RootNotFound(fts_root.id));

    // Erin is not in FastTrackStudio: its lane refuses her at the gate.
    let erin_fts = connect(&world, "fts", "erin").await;
    let refused = erin_fts.service.roots().await.unwrap_err();
    assert!(matches!(
        ClientError::from(refused),
        ClientError::Transport(_)
    ));

    // Each org's files live under its own `org/<id>/files`.
    assert!(world.dir.path().join("org/fts/files/state.json").exists());
    assert!(world.dir.path().join("org/hp/files/state.json").exists());
}

#[tokio::test]
async fn a_quota_bounds_an_orgs_live_bytes() {
    let world = world(None);
    world
        .files
        .set_quota(&OrgId::new("fts"), Some(1_000))
        .await
        .unwrap();
    let carol = connect(&world, "fts", "carol").await;
    let root = carol.ensure_root("audio", "Audio").await.unwrap();
    carol
        .put(root.id, "a.wav", &[1; 600], Save::create_only())
        .await
        .unwrap();
    let over = carol
        .put(root.id, "b.wav", &[2; 600], Save::create_only())
        .await
        .unwrap_err();
    assert_eq!(
        fault(over),
        FilesFault::QuotaExceeded {
            used: 600,
            quota: 1_000,
            adding: 600
        }
    );
    // Replacing counts the difference.
    carol
        .put(root.id, "a.wav", &[3; 900], Save::overwrite())
        .await
        .unwrap();
    let copy = carol
        .service
        .copy_paths(
            root.id,
            vec![Relocation {
                from: p("a.wav"),
                to: p("c.wav"),
            }],
            OnConflict::Fail,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        fault(copy.into()),
        FilesFault::QuotaExceeded { .. }
    ));
    carol
        .service
        .delete_paths(root.id, vec![p("a.wav")])
        .await
        .unwrap();
    let usage = carol.service.usage().await.unwrap();
    assert_eq!((usage.used, usage.quota), (0, Some(1_000)));
    carol
        .put(root.id, "b.wav", &[2; 600], Save::create_only())
        .await
        .unwrap();
}

#[tokio::test]
async fn an_upload_resumes_and_bytes_the_org_holds_are_not_sent() {
    let world = world(None);
    let carol = connect(&world, "fts", "carol").await;
    let root = carol.ensure_root("takes", "Takes").await.unwrap();
    let bytes: Vec<u8> = (0..10_000u32).map(|i| (i % 7) as u8).collect();
    let plan = carol
        .service
        .begin_upload(UploadSpec {
            root_id: root.id,
            path: p("take.wav"),
            size: bytes.len() as u64,
            content: Some(content_of(&bytes)),
            expect: None,
        })
        .await
        .unwrap();
    assert_eq!(plan.needed.len(), 1);

    // The first half arrives, then the connection drops.
    let (tx, rx) = vox::channel::<UploadFrame>();
    let half = bytes[..4_000].to_vec();
    let sending = async move {
        tx.send(UploadFrame::Chunk {
            offset: 0,
            bytes: half,
        })
        .await
        .unwrap();
    };
    let (received, ()) = tokio::join!(carol.service.send_bytes(plan.upload_id, rx), sending);
    let received = received.unwrap();
    assert_eq!(received.written, 4_000);
    let early = carol
        .service
        .complete_upload(plan.upload_id, OnConflict::Fail)
        .await
        .unwrap_err();
    assert_eq!(fault(early.into()), FilesFault::Incomplete(plan.upload_id));

    // Resumed, from the server's account of what it has.
    let progress = carol.service.upload_progress(plan.upload_id).await.unwrap();
    assert_eq!(progress.received, 4_000);
    let rest = progress.needed[0];
    let (tx, rx) = vox::channel::<UploadFrame>();
    let tail = bytes[rest.start as usize..rest.end as usize].to_vec();
    let sending = async move {
        tx.send(UploadFrame::Chunk {
            offset: rest.start,
            bytes: tail,
        })
        .await
        .unwrap();
        tx.send(UploadFrame::Finished).await.unwrap();
    };
    let (received, ()) = tokio::join!(carol.service.send_bytes(plan.upload_id, rx), sending);
    assert!(received.unwrap().needed.is_empty());
    carol
        .service
        .complete_upload(plan.upload_id, OnConflict::Fail)
        .await
        .unwrap();
    assert_eq!(carol.get(root.id, "take.wav").await.unwrap(), bytes);

    // The same bytes again: nothing to send.
    let again = carol
        .service
        .begin_upload(UploadSpec {
            root_id: root.id,
            path: p("copy of take.wav"),
            size: bytes.len() as u64,
            content: Some(content_of(&bytes)),
            expect: None,
        })
        .await
        .unwrap();
    assert!(again.needed.is_empty());
    carol
        .service
        .complete_upload(again.upload_id, OnConflict::Fail)
        .await
        .unwrap();
    assert_eq!(carol.get(root.id, "copy of take.wav").await.unwrap(), bytes);

    // Bytes that are not what the client named do not land.
    let lie = carol
        .service
        .begin_upload(UploadSpec {
            root_id: root.id,
            path: p("lie.wav"),
            size: 3,
            content: Some(content_of(b"abc")),
            expect: None,
        })
        .await
        .unwrap();
    let (tx, rx) = vox::channel::<UploadFrame>();
    let sending = async move {
        tx.send(UploadFrame::Chunk {
            offset: 0,
            bytes: b"xyz".to_vec(),
        })
        .await
        .unwrap();
    };
    let (sent, ()) = tokio::join!(carol.service.send_bytes(lie.upload_id, rx), sending);
    sent.unwrap();
    let refused = carol
        .service
        .complete_upload(lie.upload_id, OnConflict::Fail)
        .await
        .unwrap_err();
    assert!(matches!(fault(refused.into()), FilesFault::Invalid(_)));
    assert!(carol.entry(root.id, "lie.wav").await.is_err());
}

#[tokio::test]
async fn events_reach_only_who_may_see_them() {
    // The gate checks identity only: the files feature alone decides who
    // hears what.
    let world = world(Some(Arc::new(AllowAll)));
    let fts = OrgId::new("fts");
    let owner = world.files.acting(
        fts.clone(),
        Principal::Service {
            name: "seed".into(),
        },
    );
    let root = owner
        .ensure_root("client".into(), "Client".into())
        .await
        .unwrap();
    owner
        .grant(
            root.id,
            p("Deliverables"),
            "erin".into(),
            vec![Capability::Read],
        )
        .await
        .unwrap();

    let bob = connect(&world, "fts", "bob").await;
    let erin = connect(&world, "fts", "erin").await;
    let heard_bob = Arc::new(Mutex::new(Vec::new()));
    let heard_erin = Arc::new(Mutex::new(Vec::new()));
    for (client, heard) in [(&bob, &heard_bob), (&erin, &heard_erin)] {
        let client = client.clone();
        let heard = Arc::clone(heard);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let mut ready = Some(ready_tx);
        tokio::spawn(async move {
            let _ = client
                .events(Some(root.id), |event| {
                    if event == FilesEvent::Attached {
                        if let Some(ready) = ready.take() {
                            let _ = ready.send(());
                        }
                    } else {
                        heard.lock().unwrap().push(event);
                    }
                    true
                })
                .await;
        });
        ready_rx.await.unwrap();
    }

    owner
        .create_dirs(root.id, vec![p("Stems"), p("Deliverables/mix")])
        .await
        .unwrap();
    // A last event both may see, so both have heard everything before it.
    owner
        .create_dirs(root.id, vec![p("Deliverables/done")])
        .await
        .unwrap();
    for heard in [&heard_bob, &heard_erin] {
        for _ in 0..200 {
            let done = heard.lock().unwrap().iter().any(|e| {
                matches!(e, FilesEvent::Changed { entries, .. }
                    if entries.iter().any(|x| x.path == p("Deliverables/done")))
            });
            if done {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
    let paths = |heard: &Arc<Mutex<Vec<FilesEvent>>>| -> Vec<RootPath> {
        heard
            .lock()
            .unwrap()
            .iter()
            .flat_map(|e| match e {
                FilesEvent::Changed { entries, .. } => {
                    entries.iter().map(|x| x.path.clone()).collect()
                }
                _ => Vec::new(),
            })
            .collect()
    };
    assert_eq!(
        paths(&heard_bob),
        vec![
            p("Deliverables"),
            p("Deliverables/mix"),
            p("Stems"),
            p("Deliverables/done")
        ]
    );
    assert_eq!(
        paths(&heard_erin),
        vec![
            p("Deliverables"),
            p("Deliverables/mix"),
            p("Deliverables/done")
        ]
    );
}

#[tokio::test]
async fn an_orgs_files_survive_a_restart() {
    let world = world(None);
    let carol = connect(&world, "fts", "carol").await;
    let root = carol.ensure_root("keep", "Keep").await.unwrap();
    carol
        .put(root.id, "a.txt", b"kept", Save::create_only())
        .await
        .unwrap();

    let again = FilesServer::new(
        world.dir.path(),
        LocalFs::new(world.dir.path().join("blobs")),
        world.members.clone(),
    );
    let carol = again.acting(
        OrgId::new("fts"),
        Principal::User {
            user_id: "carol".into(),
        },
    );
    assert_eq!(carol.roots().await.unwrap(), vec![root.clone()]);
    let entry = carol.entry(root.id, p("a.txt")).await.unwrap();
    assert_eq!(entry.content, Some(content_of(b"kept")));
}
