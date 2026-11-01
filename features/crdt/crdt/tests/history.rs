//! Named undo over transactions, and kinds' schema upgrades.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crdt::kinds::{kind_of, stamp_of};
use crdt::{DocKind, DocKinds, History, Migration};
use loro::LoroDoc;

fn text(doc: &LoroDoc) -> String {
    doc.get_text("body").to_string()
}

#[test]
fn a_transaction_is_one_named_step_undone_and_redone_as_one() {
    let doc = LoroDoc::new();
    let mut h = History::new(&doc, 100);
    h.transact(&doc, "Type hello", |d| {
        let t = d.get_text("body");
        t.insert(0, "hel").unwrap();
        d.commit();
        t.insert(3, "lo").unwrap();
    });
    h.transact(&doc, "Type world", |d| {
        d.get_text("body").insert(5, " world").unwrap()
    });
    assert_eq!(text(&doc), "hello world");
    assert_eq!(h.next_undo(), Some("Type world"));
    assert_eq!(h.undo().as_deref(), Some("Type world"));
    assert_eq!(text(&doc), "hello");
    // Two commits in one transaction undo as one.
    assert_eq!(h.undo().as_deref(), Some("Type hello"));
    assert_eq!(text(&doc), "");
    assert_eq!(h.undo(), None);
    assert_eq!(h.redo().as_deref(), Some("Type hello"));
    assert_eq!(text(&doc), "hello");
    assert_eq!(h.next_redo(), Some("Type world"));
}

#[test]
fn a_new_step_drops_what_could_be_redone_and_nothing_is_no_step() {
    let doc = LoroDoc::new();
    let mut h = History::new(&doc, 100);
    h.transact(&doc, "A", |d| d.get_text("body").insert(0, "a").unwrap());
    h.transact(&doc, "Nothing", |_| ());
    assert_eq!(h.steps(), ["A"]);
    h.undo();
    h.transact(&doc, "B", |d| d.get_text("body").insert(0, "b").unwrap());
    assert_eq!(h.next_redo(), None);
    assert_eq!(h.steps(), ["B"]);
}

#[test]
fn undo_is_local_a_collaborators_change_stays() {
    let mine = LoroDoc::new();
    mine.set_peer_id(1).unwrap();
    let theirs = LoroDoc::new();
    theirs.set_peer_id(2).unwrap();
    let mut h = History::new(&mine, 100);
    h.transact(&mine, "Mine", |d| {
        d.get_text("body").insert(0, "mine ").unwrap()
    });
    theirs
        .import(&mine.export(loro::ExportMode::all_updates()).unwrap())
        .unwrap();
    theirs.get_text("body").insert(5, "theirs").unwrap();
    theirs.commit();
    mine.import(&theirs.export(loro::ExportMode::all_updates()).unwrap())
        .unwrap();
    assert_eq!(text(&mine), "mine theirs");
    h.undo();
    assert_eq!(text(&mine), "theirs");
}

#[test]
fn a_kind_stamps_and_upgrades_a_document_a_migration_at_a_time() {
    fn add_title(doc: &LoroDoc) -> Result<(), String> {
        doc.get_map("song")
            .insert("title", "Untitled")
            .map_err(|e| e.to_string())
    }
    fn rename_bpm(doc: &LoroDoc) -> Result<(), String> {
        let m = doc.get_map("song");
        m.insert("tempo", 120).map_err(|e| e.to_string())
    }
    let v1 = DocKind::new("studio.song", "Song").extension("session");
    let doc = LoroDoc::new();
    v1.stamp(&doc);
    assert_eq!(stamp_of(&doc), Some(1));
    assert_eq!(kind_of(&doc).as_deref(), Some("studio.song"));
    let v3 = DocKind::new("studio.song", "Song")
        .extension("session")
        .schema(
            3,
            vec![
                Migration {
                    from: 1,
                    name: "add a title",
                    run: add_title,
                },
                Migration {
                    from: 2,
                    name: "tempo",
                    run: rename_bpm,
                },
            ],
        );
    assert_eq!(v3.upgrade(&doc), Ok(3));
    assert_eq!(stamp_of(&doc), Some(3));
    // A newer document refuses an older build.
    assert!(v1.upgrade(&doc).is_err());
    let mut kinds = DocKinds::new();
    kinds.register(v3);
    assert_eq!(kinds.for_extension(".SESSION").unwrap().id, "studio.song");
}
