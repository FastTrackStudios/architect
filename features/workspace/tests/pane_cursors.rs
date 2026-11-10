//! Cursors come with the pane (#509): a view with no presence code, in a
//! workspace under a `PaneRoom`, shows another person's pointer where
//! they put it and their dot on the pane's tab; a pane showing something
//! else shows neither. Laid out by Blitz.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use architect_workspace::pane_space::{PresenceBoard, Size, ViewSpace, pane_space};
use architect_workspace::{
    PaneRoom, Pane, Target, View, Views, Workspace, WorkspaceView, use_pane_room,
};
use blitz_dom::Document as _;
use blitz_traits::shell::{ColorScheme, Viewport};
use crdt::presence::{Here, PeerState, PresenceSink, Roster};
use dioxus::prelude::*;
use dioxus_native::{DioxusDocument, DocumentConfig};
use loro::LoroValue;

/// The presence channel Ann and Bob share.
#[derive(Clone, Default)]
struct Channel(Rc<RefCell<HashMap<String, LoroValue>>>);

impl PresenceSink for Channel {
    fn set(&self, key: &str, value: LoroValue) {
        self.0.borrow_mut().insert(key.to_owned(), value);
    }
    fn delete(&self, key: &str) {
        self.0.borrow_mut().remove(key);
    }
}

thread_local! {
    static CHANNEL: Channel = Channel::default();
}

fn clock() -> f64 {
    thread_local! { static BASE: Instant = Instant::now(); }
    BASE.with(|b| b.elapsed().as_secs_f64() * 1000.0)
}

/// Bob's app: two notes in one tab group, neither view knowing about presence.
#[component]
fn Bobs() -> Element {
    let workspace = use_signal(|| {
        let mut w = Workspace::new();
        // The one opened last is in front.
        let _ = w.open(Pane::new("p2", "note", "Silent Night"), Target::Tab);
        let _ = w.open(Pane::new("p1", "note", "Night Drive"), Target::Tab);
        w
    });
    let channel = CHANNEL.with(Clone::clone);
    let roster = use_signal(|| {
        let mut roster = Roster::default();
        roster.sync("bob", &channel.0.borrow(), clock());
        roster
    });
    use_pane_room(|| {
        PaneRoom::new(
            Here::new(channel, "bob", PeerState::new("bob", "Bob")),
            roster,
            PresenceBoard::default(),
            clock,
        )
    });
    let views = Views::new().view(View::new("note", "Note", |_| rsx! { div { "a note" } }).many());
    rsx! {
        div { style: "position: absolute; inset: 0; display: flex;",
            WorkspaceView { workspace, views }
        }
    }
}

fn rect(doc: &DioxusDocument, selector: &str) -> Option<(f64, f64, f64, f64)> {
    let d = doc.inner.borrow();
    let id = d.query_selector(selector).ok()??;
    let r = d.get_client_bounding_rect(id)?;
    Some((r.x, r.y, r.width, r.height))
}

#[test]
fn another_persons_pointer_is_drawn_in_a_pane_whose_view_has_no_presence_code() {
    architect_shell::motion::set_reduced_motion(Some(true));
    // Ann, elsewhere, in "Night Drive": her pane publishes for her, a
    // quarter across and half way down a pane of another size.
    let space = ViewSpace::fractions(pane_space("note", "Night Drive"));
    let ann = Here::new(
        CHANNEL.with(Clone::clone),
        "ann",
        PeerState::new("ann", "Ann"),
    );
    ann.edit_state(clock(), |s| {
        s.looking_at = Some(space.name().to_owned());
    });
    ann.flush(clock() + 10_000.0);
    ann.pointer(
        space
            .to_place(100.0, 150.0, Size { w: 400.0, h: 300.0 })
            .unwrap(),
        clock(),
    );

    let mut doc = DioxusDocument::new(
        VirtualDom::new(Bobs),
        DocumentConfig {
            viewport: Some(Viewport::new(900, 600, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();
    // The pane measures itself after its first frame; give it a few.
    let mut pointer = None;
    for _ in 0..80 {
        doc.poll(None);
        doc.inner.borrow_mut().resolve(0.0);
        pointer = rect(&doc, "[data-presence-pointer=\"ann\"]");
        if pointer.is_some() {
            // One more frame, so the layer itself is laid out.
            doc.poll(None);
            doc.inner.borrow_mut().resolve(0.0);
            pointer = rect(&doc, "[data-presence-pointer=\"ann\"]");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let (px, py, _, _) = pointer.expect("Ann's pointer is drawn");
    // "Night Drive" is the tab in front; "Silent Night" is behind it.
    let (x, y, w, h) = rect(&doc, "[data-pane=\"p1\"]").expect("p1 is drawn");
    assert!(
        (px - w.mul_add(0.25, x)).abs() < 2.0 && (py - h.mul_add(0.5, y)).abs() < 2.0,
        "the pointer ({px}, {py}) a quarter across and half down p1 ({x}, {y}, {w}, {h})"
    );
    assert!(
        rect(&doc, "[data-tab-people=\"p1\"]").is_some(),
        "her dot is on p1's tab"
    );
    assert!(
        rect(&doc, "[data-tab-people=\"p2\"]").is_none(),
        "and not on p2's"
    );
}
