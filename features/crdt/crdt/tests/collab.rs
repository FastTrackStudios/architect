//! Two people in one document, over the in-process vox transport: they
//! type into one text at once and both end with the same text, each sees
//! the other's caret where the other has it, and a pointer in a view's
//! coordinate space reaches the other side as a place, not pixels.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::panic
)]
#![cfg(not(target_arch = "wasm32"))]

use std::time::Duration;

use architect::{LayerRouter, LocalServer, Scope};
use crdt::CrdtDoc;
use crdt::presence::{Here, PeerState, Place, Roster, Selection, clock_ms};
use crdt::registry::DocRegistry;
use crdt::sync::{
    DocPresenceClient, DocPresenceDispatcher, DocSyncClient, DocSyncDispatcher, PresencePeer,
    SyncedDoc, doc_presence_service_descriptor, doc_sync_service_descriptor,
};
use crdt::text::{EditorBinding, TextBinding, TextOp};
use uuid::Uuid;

/// An editor: its text, applying what the binding hands it.
#[derive(Default)]
struct Editor(Vec<char>);

impl Editor {
    fn text(&self) -> String {
        self.0.iter().collect()
    }
}

impl EditorBinding for Editor {
    fn apply_remote(&mut self, ops: &[TextOp]) {
        for op in ops {
            match op {
                TextOp::Insert { pos, text } => {
                    let at = *pos as usize;
                    self.0.splice(at..at, text.chars());
                }
                TextOp::Delete { pos, len } => {
                    let from = *pos as usize;
                    self.0.drain(from..from + *len as usize);
                }
            }
        }
    }
}

/// One person: a replica of the doc, its presence, an editor bound to the
/// doc's `body` text.
struct Person {
    doc: CrdtDoc,
    peer: PresencePeer,
    here: Here<PresencePeer>,
    binding: TextBinding,
    editor: Editor,
}

impl Person {
    async fn join(local: &LocalServer, doc_id: Uuid, me: &str, name: &str) -> Self {
        let client: DocSyncClient = local.establish().await.unwrap();
        let mut synced = SyncedDoc::new(doc_id, CrdtDoc::ephemeral());
        let doc = synced.doc().clone();
        tokio::spawn(async move {
            let _ = synced.run(&client).await;
        });
        let client: DocPresenceClient = local.establish().await.unwrap();
        let (peer, mut driver) = PresencePeer::new(doc_id, 30_000);
        tokio::spawn(async move {
            let _ = driver.run(&client).await;
        });
        let here = Here::new(peer.clone(), me, PeerState::new(me, name));
        let binding = TextBinding::new(doc.loro(), doc.loro().get_text("body"));
        let editor = Editor(binding.shown().chars().collect());
        Self {
            doc,
            peer,
            here,
            binding,
            editor,
        }
    }

    /// Type `text` at `pos`, as the editor would report it.
    fn type_at(&mut self, pos: u32, text: &str) {
        let op = TextOp::Insert {
            pos,
            text: text.to_owned(),
        };
        self.editor.apply_remote(std::slice::from_ref(&op));
        self.binding.on_local(&[op]).unwrap();
    }

    /// Put the caret at `head` with the selection from `anchor`, and say so.
    fn caret(&self, anchor: u32, head: u32) {
        let caret = self.binding.caret(anchor, head);
        self.here.edit_state(0.0, |s| s.caret = caret);
        self.here.flush(f64::MAX);
    }

    fn pull(&mut self) {
        self.binding.sync_into(&mut self.editor);
    }

    fn roster(&self, me: &str) -> Roster {
        let mut roster = Roster::default();
        roster.sync(me, &self.peer.states(), clock_ms());
        roster
    }
}

fn serve(scope: &std::sync::Arc<Scope>) -> LocalServer {
    let registry = DocRegistry::new(|_doc_id| Box::pin(async { Ok(CrdtDoc::ephemeral()) }));
    let router = LayerRouter::new()
        .with(
            doc_sync_service_descriptor(),
            DocSyncDispatcher::new(registry.clone()),
        )
        .with(
            doc_presence_service_descriptor(),
            DocPresenceDispatcher::new(registry),
        );
    LocalServer::serve(router, scope.clone())
}

async fn eventually(what: &str, mut pred: impl FnMut() -> bool) {
    // Generous for a loaded CI host; a passing wait returns at once.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while !pred() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for: {what}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn two_people_edit_one_note_and_see_each_others_carets() {
    let scope = Scope::new();
    let local = serve(&scope);
    let doc_id = Uuid::new_v4();
    let mut ann = Person::join(&local, doc_id, "ann", "Ann").await;
    let mut bob = Person::join(&local, doc_id, "bob", "Bob").await;

    ann.type_at(0, "Hello world");
    eventually("Bob's editor gets Ann's text", || {
        bob.pull();
        bob.editor.text() == "Hello world"
    })
    .await;

    // Both type at once, neither having seen the other's keystrokes.
    ann.type_at(5, ", dear");
    bob.type_at(11, "!");
    bob.type_at(0, "¡");
    let both = "¡Hello, dear world!";
    eventually("both editors converge on one text", || {
        ann.pull();
        bob.pull();
        ann.editor.text() == both && bob.editor.text() == both
    })
    .await;
    assert_eq!(ann.doc.loro().get_text("body").to_string(), both);

    // Ann selects "dear"; Bob's caret sits after "world".
    let dear = both.find("dear").unwrap() as u32 - 1; // "¡" is one char, two bytes
    ann.caret(dear, dear + 4);
    let after_world = both.chars().count() as u32 - 1;
    bob.caret(after_world, after_world);

    eventually("Bob sees Ann's selection of \"dear\"", || {
        let carets = bob.binding.remote_carets(&bob.roster("bob"));
        carets.len() == 1
            && carets[0].name == "Ann"
            && (carets[0].anchor, carets[0].head) == (dear, dear + 4)
    })
    .await;
    eventually("Ann sees Bob's caret after \"world\"", || {
        let carets = ann.binding.remote_carets(&ann.roster("ann"));
        carets.len() == 1 && carets[0].name == "Bob" && carets[0].head == after_world
    })
    .await;

    // Bob types at the start: Ann's selection moves with the word on his
    // screen without Ann sending anything new.
    bob.type_at(0, "Oh ");
    let carets = bob.binding.remote_carets(&bob.roster("bob"));
    assert_eq!(
        (carets[0].anchor, carets[0].head),
        (dear + 3, dear + 7),
        "her stable cursors follow the characters"
    );
    eventually("Ann's editor gets \"Oh \"", || {
        ann.pull();
        ann.editor.text() == "Oh ¡Hello, dear world!"
    })
    .await;

    scope.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pointer_reaches_the_other_side_as_a_place_in_a_view() {
    let scope = Scope::new();
    let local = serve(&scope);
    let doc_id = Uuid::new_v4();
    let ann = Person::join(&local, doc_id, "ann", "Ann").await;
    let bob = Person::join(&local, doc_id, "bob", "Bob").await;

    // Ann points at bar 12.5 of the kick track and has it selected.
    ann.here.pointer(
        Place::new("session.arrange")
            .with("time", 12.5)
            .with("track", "Kick")
            .with("y", 0.5),
        clock_ms(),
    );
    ann.here.edit_state(0.0, |s| {
        s.looking_at = Some("session.arrange".into());
        s.selections.push(Selection {
            space: "session.arrange".into(),
            keys: vec!["Kick".into()],
            range: None,
        });
    });
    ann.here.flush(f64::MAX);

    // Bob's arrangement: 10px a second, each track's row 40px tall; Kick
    // is the second row. A chart view shows no arrangement places.
    let arrangement = |p: &Place| {
        if p.space != "session.arrange" {
            return None;
        }
        let row = match p.key("track")? {
            "Snare" => 0.0,
            "Kick" => 1.0,
            _ => return None,
        };
        Some((p.num("time")? * 10.0, (row + p.num("y")?) * 40.0))
    };
    let chart = |p: &Place| (p.space == "chart").then_some((0.0, 0.0));

    // The pointer and the state (selection, view) are separate keys and
    // arrive on their own: wait for both, not the pointer alone (#401).
    eventually(
        "Bob sees Ann's pointer on the kick track, her selection and her view",
        || {
            let roster = bob.roster("bob");
            let drawn = roster.pointers(clock_ms() + 1_000.0, arrangement);
            let looking: Vec<_> = roster
                .here()
                .map(|(_, s)| s.looking_at.as_deref())
                .collect();
            drawn.len() == 1
                && drawn[0].name == "Ann"
                && (drawn[0].x, drawn[0].y) == (125.0, 60.0)
                && roster.selections_in("session.arrange").count() == 1
                && looking == [Some("session.arrange")]
        },
    )
    .await;
    let roster = bob.roster("bob");
    assert!(roster.pointers(clock_ms() + 1_000.0, chart).is_empty());

    // Ann's pointer leaves the window: gone from Bob's view.
    ann.here.no_pointer();
    eventually("Ann's pointer is gone", || {
        bob.roster("bob")
            .pointers(clock_ms() + 1_000.0, arrangement)
            .is_empty()
    })
    .await;

    drop(ann);
    scope.close().await;
}
