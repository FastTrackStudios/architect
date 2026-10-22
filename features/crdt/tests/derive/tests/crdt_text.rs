//! `#[architect(crdt(text))]`: concurrent edits to the same field on two
//! replicas both survive the merge — the derive writes the field as a
//! `LoroText` diff, not a last-writer-wins string.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crdt::CrdtDoc;
use crdt::loro::{ExportMode, LoroDoc};
use crdt_derive_tests::{Note, NoteCreate, NoteRepo, NoteRepoLoro, NoteUpdate};

fn replica(peer: u64) -> CrdtDoc {
    let doc = LoroDoc::new();
    doc.set_peer_id(peer).unwrap();
    CrdtDoc::from_loro(doc)
}

/// Two-way sync of everything each side is missing.
fn sync(a: &CrdtDoc, b: &CrdtDoc) {
    let (la, lb) = (a.loro(), b.loro());
    let a_to_b = la.export(ExportMode::updates(&lb.oplog_vv())).unwrap();
    let b_to_a = lb.export(ExportMode::updates(&la.oplog_vv())).unwrap();
    lb.import(&a_to_b).unwrap();
    la.import(&b_to_a).unwrap();
}

fn body_update(body: &str) -> NoteUpdate {
    NoteUpdate {
        body: Some(body.to_owned()),
        ..Default::default()
    }
}

#[tokio::test]
async fn concurrent_text_edits_keep_both() {
    let a = replica(1);
    let b = replica(2);
    let repo_a = NoteRepoLoro::new(&a);
    let repo_b = NoteRepoLoro::new(&b);

    let note: Note = repo_a
        .create(NoteCreate {
            title: "plan".into(),
            body: "hello world".into(),
        })
        .await
        .unwrap();
    sync(&a, &b);
    assert_eq!(repo_b.get(note.id).await.unwrap().body, "hello world");

    // Offline, both peers edit the body: A appends, B prepends.
    repo_a
        .update(note.id, body_update("hello world!"))
        .await
        .unwrap();
    repo_b
        .update(note.id, body_update("> hello world"))
        .await
        .unwrap();
    sync(&a, &b);

    let on_a = repo_a.get(note.id).await.unwrap();
    let on_b = repo_b.get(note.id).await.unwrap();
    assert_eq!(on_a, on_b, "replicas diverged");
    assert_eq!(on_a.body, "> hello world!", "an edit was lost");
}

#[tokio::test]
async fn plain_string_fields_stay_last_writer_wins() {
    // Control: the same race on the non-text `title` keeps exactly one
    // side — which is what `crdt(text)` exists to avoid.
    let a = replica(1);
    let b = replica(2);
    let repo_a = NoteRepoLoro::new(&a);
    let repo_b = NoteRepoLoro::new(&b);
    let note = repo_a
        .create(NoteCreate {
            title: "plan".into(),
            body: String::new(),
        })
        .await
        .unwrap();
    sync(&a, &b);
    let t = |s: &str| NoteUpdate {
        title: Some(s.to_owned()),
        ..Default::default()
    };
    repo_a.update(note.id, t("plan!")).await.unwrap();
    repo_b.update(note.id, t("> plan")).await.unwrap();
    sync(&a, &b);
    let title = repo_a.get(note.id).await.unwrap().title;
    assert!(title == "plan!" || title == "> plan", "got {title}");
    assert_eq!(repo_b.get(note.id).await.unwrap().title, title);
}

#[tokio::test]
async fn legacy_string_body_reads_and_upgrades_on_write() {
    // A row written before `body` became `crdt(text)`: plain string.
    let a = replica(1);
    let repo = NoteRepoLoro::new(&a);
    let note = repo
        .create(NoteCreate {
            title: "old".into(),
            body: String::new(),
        })
        .await
        .unwrap();
    let rows = a.loro().get_map("notes");
    let row = match rows.get(&note.id.to_string()) {
        Some(crdt::loro::ValueOrContainer::Container(crdt::loro::Container::Map(m))) => m,
        other => panic!("row missing: {other:?}"),
    };
    row.delete("body").unwrap();
    row.insert("body", "legacy text").unwrap();
    a.loro().commit();

    assert_eq!(repo.get(note.id).await.unwrap().body, "legacy text");
    repo.update(note.id, body_update("legacy text, edited"))
        .await
        .unwrap();
    assert_eq!(repo.get(note.id).await.unwrap().body, "legacy text, edited");
    assert!(matches!(
        row.get("body"),
        Some(crdt::loro::ValueOrContainer::Container(
            crdt::loro::Container::Text(_)
        ))
    ));
}
