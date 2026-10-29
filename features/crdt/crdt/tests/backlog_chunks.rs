//! A long history sent to a replica behind it comes in frames a link takes:
//! cut by peer and span to at most the limit (a single op over it is a
//! frame of its own), and imported in any order it is the same doc.
// An integration-test crate: the panic lints' test allowances do not reach
// its helpers, where an `expect()` is the assertion.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(not(target_arch = "wasm32"))]

use crdt::sync::backlog_chunks;

/// A doc two peers wrote to, `rounds` edits each, every one a few KB.
fn edited(rounds: usize) -> loro::LoroDoc {
    let a = loro::LoroDoc::new();
    a.set_peer_id(1).expect("peer");
    let b = loro::LoroDoc::new();
    b.set_peer_id(2).expect("peer");
    let filler = "x".repeat(4_000);
    for i in 0..rounds {
        a.get_map("items")
            .insert(&format!("a{i}"), filler.as_str())
            .expect("insert");
        a.commit();
        b.get_map("items")
            .insert(&format!("b{i}"), filler.as_str())
            .expect("insert");
        b.commit();
    }
    a.import(&b.export(loro::ExportMode::all_updates()).expect("export"))
        .expect("import");
    a
}

#[test]
fn a_backlog_over_the_limit_comes_in_frames_under_it() {
    let doc = edited(200);
    // A replica that has the first few edits of peer 1 only.
    let behind = loro::LoroDoc::new();
    let early = doc
        .export(loro::ExportMode::UpdatesInRange {
            spans: std::borrow::Cow::Owned(vec![loro::IdSpan::new(1, 0, 10)]),
        })
        .expect("export");
    behind.import(&early).expect("import");

    let max = 64 * 1024;
    let chunks = backlog_chunks(&doc, &behind.oplog_vv(), max).expect("chunks");
    assert!(chunks.len() > 1, "{} chunk(s)", chunks.len());
    for chunk in &chunks {
        assert!(chunk.len() <= max, "a chunk of {} bytes", chunk.len());
    }
    // Last first: out of order, they still make the same doc.
    for chunk in chunks.iter().rev() {
        behind.import(chunk).expect("import a chunk");
    }
    assert_eq!(behind.get_deep_value(), doc.get_deep_value());
    assert_eq!(behind.oplog_vv(), doc.oplog_vv());
}

#[test]
fn a_backlog_under_the_limit_is_one_frame() {
    let doc = edited(3);
    let chunks =
        backlog_chunks(&doc, &loro::VersionVector::new(), 16 * 1024 * 1024).expect("chunks");
    assert_eq!(chunks.len(), 1);
}
