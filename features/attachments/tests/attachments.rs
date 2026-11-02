//! Attachments end to end over a real vox link and a real socket: a
//! document attaches an image and a PDF offline on one device, syncs,
//! pushes them when online, and a second device fetches them by hash;
//! access follows the document's (and its space's) permissions; uploads
//! resume, are sent once and count against the quota; a record keeps its
//! list across a restart; and a browser previews an attachment by range.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::as_conversions,
    clippy::cast_possible_truncation
)]

mod common;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use architect::blob::{Fetcher, LocalFs, Sha256};
use architect_attachments::server::AttachmentsServer;
use architect_attachments::{
    AttachFault, AttachmentsRemote, AttachmentsService as _, ClientError, Parent, attach_local,
    attachment_of, doc, hash_of, web,
};
use architect_files::RenditionKind;
use architect_files::server::FilesServer;
use architect_spaces::{NewSpace, SpacesService as _, Subject, VIEWER};
use common::{Fake, attachments, blobs_dir, connect, hp, user, world};
use crdt::{DocKind, DocKinds};
use loro::LoroDoc;
use uuid::Uuid;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n a small picture";

fn pdf() -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    bytes.extend((0..700_000u32).map(|i| (i % 251) as u8));
    bytes
}

/// A note: a document kind whose blobs are its attachments.
fn note_kind() -> DocKind {
    DocKind::new("test.note", "Note").blobs(doc::refs)
}

fn note() -> LoroDoc {
    let doc = LoroDoc::new();
    note_kind().stamp(&doc);
    doc
}

fn fault(e: &ClientError) -> &AttachFault {
    e.fault().unwrap_or_else(|| panic!("not a fault: {e}"))
}

#[tokio::test]
async fn a_document_attaches_offline_and_a_second_device_reads_them() {
    let world = world().await;
    let id = Uuid::from_u128(1);
    let device_a = tempfile::tempdir().unwrap();
    let device_b = tempfile::tempdir().unwrap();
    let store_a = LocalFs::new(device_a.path());
    let store_b = LocalFs::new(device_b.path());

    // Offline on device A: the bytes into its store, the entries into
    // the note.
    let doc_a = note();
    let pdf = pdf();
    let image = attach_local(&store_a, &doc_a, "shot.png", "image/png", PNG, 1, None)
        .await
        .unwrap();
    let brief = attach_local(
        &store_a,
        &doc_a,
        "brief.pdf",
        "application/pdf",
        &pdf,
        2,
        None,
    )
    .await
    .unwrap();
    assert_eq!(doc::list(&doc_a), vec![image.clone(), brief.clone()]);
    assert!(
        store_a
            .is_pinned(Sha256::from_hex(&brief.hash).unwrap())
            .await
            .unwrap()
    );

    // Online: the note syncs, and its attachments are pushed.
    world.sync_up(id, &doc_a);
    let alice = connect(&world, "alice").await;
    let pushed = alice.push_doc(&store_a, id, &doc_a).await.unwrap();
    assert_eq!(pushed.len(), 2);
    assert_eq!(
        alice.service.list(Parent::doc(id)).await.unwrap(),
        vec![image.clone(), brief.clone()]
    );
    // Pushing again sends nothing.
    let again = alice
        .service
        .begin(Parent::doc(id), brief.hash.clone(), brief.size)
        .await;
    assert!(again.unwrap().is_done());

    // Device B (dave's): the note arrives by sync, its attachments by
    // hash, through the parent's permissions.
    let doc_b = LoroDoc::new();
    world.sync_down(id, &doc_b);
    assert_eq!(doc::list(&doc_b), vec![image.clone(), brief.clone()]);
    let dave = connect(&world, "dave").await;
    let fetcher = Fetcher::new(store_b.clone())
        .with_remote(Arc::new(AttachmentsRemote::new(&dave, Parent::doc(id))));
    let kept = crdt::blobs::keep_offline(&note_kind(), &doc_b, &fetcher)
        .await
        .unwrap();
    assert_eq!(kept.len(), 2);
    let read = |hash: &str| tokio::fs::read(store_b.path_for(&Sha256::from_hex(hash).unwrap()));
    assert_eq!(read(&image.hash).await.unwrap(), PNG);
    assert_eq!(read(&brief.hash).await.unwrap(), pdf);
    assert_eq!(dave.read(&Parent::doc(id), &image.hash).await.unwrap(), PNG);

    // The note's attachments are its live blobs: a collection keeps them;
    // detaching one lets it go.
    let mut kinds = DocKinds::new();
    kinds.register(note_kind());
    let live = crdt::blobs::live(&kinds, [&doc_b]);
    assert_eq!(live.len(), 2);
    assert!(doc::detach(&doc_b, &image.hash).unwrap());
    let live = crdt::blobs::live(&kinds, [&doc_b]);
    store_b
        .unpin(Sha256::from_hex(&image.hash).unwrap())
        .await
        .unwrap();
    store_b
        .unpin(Sha256::from_hex(&brief.hash).unwrap())
        .await
        .unwrap();
    store_b.collect(&live, Duration::ZERO).await.unwrap();
    assert!(read(&image.hash).await.is_err());
    assert_eq!(read(&brief.hash).await.unwrap(), pdf);
}

#[tokio::test]
async fn access_follows_the_document_and_its_space() {
    let world = world().await;
    let (alpha, beta) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let alice = connect(&world, "alice").await;
    let doc = note();
    let shot = attachment_of(PNG, "shot.png", "image/png", 1, None);
    doc::attach(&doc, &shot).unwrap();
    world.sync_up(alpha, &doc);
    world.sync_up(beta, &note());
    alice.push(&Parent::doc(alpha), PNG).await.unwrap();

    // A viewer reads; a viewer does not attach.
    let bob = connect(&world, "bob").await;
    assert_eq!(
        bob.read(&Parent::doc(alpha), &shot.hash).await.unwrap(),
        PNG
    );
    let refused = bob.push(&Parent::doc(alpha), b"more").await.unwrap_err();
    assert!(
        matches!(fault(&refused), AttachFault::Denied { .. }),
        "{refused}"
    );

    // Knowing a hash reads nothing: beta does not point at it.
    let absent = alice
        .read(&Parent::doc(beta), &shot.hash)
        .await
        .unwrap_err();
    assert!(
        matches!(fault(&absent), AttachFault::NotFound { .. }),
        "{absent}"
    );

    // Carol, from another org: nothing — until a space shares alpha.
    let carol = connect(&world, "carol").await;
    assert!(carol.read(&Parent::doc(alpha), &shot.hash).await.is_err());
    let spaces = world.spaces.acting(hp(), user("alice"));
    let space = spaces
        .create_space(NewSpace {
            name: "Alpha".into(),
            kind: "project".into(),
            scope: vec![format!("doc/{alpha}")],
            exclude: Vec::new(),
        })
        .await
        .unwrap();
    spaces
        .set_member(
            space.id.clone(),
            Subject::User("carol".into()),
            Some(VIEWER.into()),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        carol.read(&Parent::doc(alpha), &shot.hash).await.unwrap(),
        PNG
    );
    assert_eq!(
        carol.service.list(Parent::doc(alpha)).await.unwrap(),
        vec![shot.clone()]
    );
    // A viewer of the space: reads, does not attach; nothing beyond it.
    assert!(carol.push(&Parent::doc(alpha), b"x").await.is_err());
    assert!(carol.service.list(Parent::doc(beta)).await.is_err());
    // Out of the space: refused on the next call (by the gate now: she
    // holds nothing in the org).
    spaces
        .set_member(space.id, Subject::User("carol".into()), None, None)
        .await
        .unwrap();
    assert!(carol.read(&Parent::doc(alpha), &shot.hash).await.is_err());
}

#[tokio::test]
async fn an_upload_resumes_is_sent_once_and_counts_against_the_quota() {
    let world = world().await;
    let alice = connect(&world, "alice").await;
    let task = Parent::record("task", "T-1");
    let bytes = pdf();
    let hash = hash_of(&bytes);
    let size = bytes.len() as u64;
    world
        .attachments
        .set_quota(&hp(), Some(size + 1_000))
        .await
        .unwrap();

    // Half, then the link drops; the next begin says where it stands.
    let upload = alice
        .service
        .begin(task.clone(), hash.clone(), size)
        .await
        .unwrap();
    assert_eq!(upload.received, 0);
    let half = bytes.len() / 2;
    alice
        .service
        .send(task.clone(), hash.clone(), 0, bytes[..half].to_vec())
        .await
        .unwrap();
    let resumed = alice
        .service
        .begin(task.clone(), hash.clone(), size)
        .await
        .unwrap();
    assert_eq!(resumed.received, half as u64);
    // Bytes from anywhere else are refused, saying where.
    let elsewhere = alice
        .service
        .send(task.clone(), hash.clone(), 7, b"x".to_vec())
        .await
        .unwrap_err();
    assert!(format!("{elsewhere:?}").contains("Offset"), "{elsewhere:?}");
    // The rest, through the client (it resumes from the server's word).
    assert!(alice.push(&task, &bytes).await.unwrap().is_done());
    let usage = alice.service.usage().await.unwrap();
    assert_eq!(usage.used, size);

    // The same bytes on another record: nothing sent, nothing more used.
    let other = Parent::record("task", "T-2");
    let upload = alice
        .service
        .begin(other, hash.clone(), size)
        .await
        .unwrap();
    assert!(upload.is_done());
    assert_eq!(alice.service.usage().await.unwrap().used, size);

    // Beyond the quota: refused before a byte is sent.
    let big = vec![1u8; 2_000];
    let over = alice.push(&task, &big).await.unwrap_err();
    assert!(
        matches!(fault(&over), AttachFault::QuotaExceeded { .. }),
        "{over}"
    );

    // Bytes that are not what they were named: dropped, start again.
    let named = hash_of(b"right");
    alice
        .service
        .begin(task.clone(), named.clone(), 5)
        .await
        .unwrap();
    let wrong = alice
        .service
        .send(task.clone(), named.clone(), 0, b"wrong".to_vec())
        .await
        .unwrap_err();
    assert!(format!("{wrong:?}").contains("Mismatch"), "{wrong:?}");
    let again = alice.service.begin(task, named, 5).await.unwrap();
    assert_eq!(again.received, 0);
}

#[tokio::test]
async fn a_record_keeps_its_list_across_a_restart_and_lets_go_of_what_nothing_holds() {
    let world = world().await;
    let alice = connect(&world, "alice").await;
    let task = Parent::record("task", "T-1");
    let shot = attachment_of(PNG, "shot.png", "image/png", 0, None);
    alice.push(&task, PNG).await.unwrap();
    let attached = alice
        .service
        .attach(task.clone(), shot.clone())
        .await
        .unwrap();
    assert_eq!(attached.added_by.as_deref(), Some("user:alice"));
    assert!(attached.added_us > 0);
    // A document lists its own.
    assert!(
        alice
            .service
            .attach(Parent::doc(Uuid::nil()), shot.clone())
            .await
            .is_err()
    );
    // Bytes not sent cannot be attached.
    let unsent = attachment_of(b"never sent", "x.txt", "text/plain", 0, None);
    assert!(alice.service.attach(task.clone(), unsent).await.is_err());

    // A restart: the same data, a new server.
    let files = FilesServer::new(
        world.dir.path(),
        LocalFs::new(blobs_dir(world.dir.path())),
        world.members.clone(),
    )
    .with_renditions(Fake);
    let restarted: AttachmentsServer =
        attachments(world.dir.path(), &world.engine, &world.docs, &files);
    let as_alice = restarted.acting(hp(), user("alice"));
    assert_eq!(
        as_alice.list(task.clone()).await.unwrap(),
        vec![attached.clone()]
    );
    let blob = Sha256::from_hex(&shot.hash).unwrap();
    assert!(restarted.held(&hp()).await.unwrap().contains(&blob));

    // On a record it is kept, whatever the documents say.
    let far = i64::MAX;
    assert!(
        restarted
            .release_unreferenced(&hp(), &HashSet::new(), far)
            .await
            .unwrap()
            .is_empty()
    );
    // Detached and on nothing: let go, and the store may collect it.
    assert!(
        as_alice
            .detach(task.clone(), shot.hash.clone())
            .await
            .unwrap()
    );
    assert!(as_alice.list(task).await.unwrap().is_empty());
    let released = restarted
        .release_unreferenced(&hp(), &HashSet::new(), far)
        .await
        .unwrap();
    assert_eq!(released, vec![blob]);
    assert!(!restarted.held(&hp()).await.unwrap().contains(&blob));
    assert_eq!(as_alice.usage().await.unwrap().used, 0);
}

#[tokio::test]
async fn a_browser_previews_an_attachment_by_range_with_a_ticket() {
    let world = world().await;
    let id = Uuid::from_u128(1);
    let alice = connect(&world, "alice").await;
    let mut clip = b"VIDEO".to_vec();
    clip.extend((0..1u32 << 20).map(|i| (i % 251) as u8));
    let doc = note();
    let video = attachment_of(&clip, "cut.mp4", "video/mp4", 1, None);
    let page = attachment_of(b"<script>", "x.html", "text/html", 2, None);
    doc::attach(&doc, &video).unwrap();
    doc::attach(&doc, &page).unwrap();
    world.sync_up(id, &doc);
    let parent = Parent::doc(id);
    alice.push(&parent, &clip).await.unwrap();
    alice.push(&parent, b"<script>").await.unwrap();
    let http = reqwest::Client::new();
    let get = |url: String, bearer: Option<&'static str>, range: Option<&'static str>| {
        let mut request = http.get(url);
        if let Some(bearer) = bearer {
            request = request.bearer_auth(bearer);
        }
        if let Some(range) = range {
            request = request.header("range", range);
        }
        request.send()
    };

    // A ticket, for an element that cannot send a header: seeks by range.
    let ticket = alice.service.ticket(parent.clone()).await.unwrap();
    let url = web::content_url(
        &world.base,
        &hp(),
        &parent,
        &video.hash,
        Some(&ticket.token),
    );
    let whole = get(url.clone(), None, None).await.unwrap();
    assert_eq!(whole.status(), 200);
    assert_eq!(whole.headers()["content-type"], "video/mp4");
    assert_eq!(whole.bytes().await.unwrap().as_ref(), clip.as_slice());
    let window = get(url, None, Some("bytes=1000-1999")).await.unwrap();
    assert_eq!(window.status(), 206);
    assert_eq!(window.bytes().await.unwrap().as_ref(), &clip[1000..2000]);

    // The ticket reads its parent only; sessions read as themselves.
    let other = Parent::doc(Uuid::from_u128(2));
    let elsewhere = web::content_url(&world.base, &hp(), &other, &video.hash, Some(&ticket.token));
    assert_eq!(get(elsewhere, None, None).await.unwrap().status(), 401);
    let bare = web::content_url(&world.base, &hp(), &parent, &video.hash, None);
    assert_eq!(get(bare.clone(), None, None).await.unwrap().status(), 401);
    assert_eq!(
        get(bare.clone(), Some("bob"), None).await.unwrap().status(),
        200
    );
    assert_eq!(get(bare, Some("carol"), None).await.unwrap().status(), 403);

    // A page someone attached is bytes, never a page on this origin.
    let script = web::content_url(&world.base, &hp(), &parent, &page.hash, Some(&ticket.token));
    let response = get(script, None, None).await.unwrap();
    assert_eq!(
        response.headers()["content-type"],
        "application/octet-stream"
    );

    // Its preview: made off the request path, then served.
    let poster = web::rendition_url(
        &world.base,
        &hp(),
        &parent,
        &video.hash,
        RenditionKind::Poster,
        Some(&ticket.token),
    );
    let made = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let response = get(poster.clone(), None, None).await.unwrap();
            if response.status() == 200 {
                return response.text().await.unwrap();
            }
            assert_eq!(response.status(), 503);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(made, format!("poster:{}", video.hash));
    let set = alice
        .service
        .renditions(parent, video.hash.clone())
        .await
        .unwrap();
    assert!(set.settled());
}
