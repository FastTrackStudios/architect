//! The deck strip over a workspace, laid out by Blitz: a tab per deck,
//! switching shows each deck's own layout, and a restart brings back the
//! strip and each deck as it was left.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::rc::Rc;

use architect_workspace::{
    DeckContext, DeckHandle, DeckStores, DeckStrip, DecksConfig, FormClass, MemoryContextStore,
    MemoryStore, Pane, Screenset, Screensets, Side, Target, View, Views, Workspace, WorkspaceStore,
    WorkspaceView, use_decks,
};
use blitz_dom::Document as _;
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native::{DioxusDocument, DocumentConfig};

fn mixing() -> Workspace {
    let mut w = Workspace::new();
    let _ = w.open(Pane::new("mixer", "song.mixer", "Mixer"), Target::Tab);
    w
}

fn charts() -> Workspace {
    let mut w = Workspace::new();
    let _ = w.open(Pane::new("chart", "song.chart", "Chart"), Target::Tab);
    let _ = w.open(
        Pane::new("lyrics", "song.lyrics", "Lyrics"),
        Target::Split(Side::Right),
    );
    w
}

#[derive(Clone)]
struct Kept {
    session: WorkspaceStore,
    contexts: Rc<MemoryContextStore>,
}

thread_local! {
    static KEPT: RefCell<Option<Kept>> = const { RefCell::new(None) };
    static HANDLE: RefCell<Option<DeckHandle>> = const { RefCell::new(None) };
    static FRESH: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

#[component]
fn App() -> Element {
    let workspace = use_signal(Workspace::new);
    let screensets = use_signal(|| {
        Screensets::default().with_builtins([
            Screenset::new("Mixing").layout(FormClass::Desk, mixing()),
            Screenset::new("Charts").layout(FormClass::Desk, charts()),
        ])
    });
    let kept = KEPT.with(|k| k.borrow().clone().unwrap());
    let handle = use_decks(DecksConfig {
        workspace,
        screensets,
        form: FormClass::Desk,
        stores: DeckStores {
            session: Some(kept.session.clone()),
            screensets: None,
            contexts: Some(kept.contexts),
        },
        templates: vec![architect_workspace::DeckTemplate::new("song", "mixing")],
        start: vec![
            DeckContext::new("song", "amazing-grace", "Amazing Grace"),
            DeckContext::new("song", "slow-river", "Slow River"),
        ],
        views: None,
        bridge: architect_workspace::Bridge::default(),
        on_new: None,
        actions: None,
        resolve: None,
        on_fresh: Some(Callback::new(|c: DeckContext| {
            FRESH.with(|f| f.borrow_mut().push(c.key()));
        })),
    });
    use_hook(|| HANDLE.with(|h| *h.borrow_mut() = Some(handle)));
    let views = Views::new()
        .view(View::new(
            "song.mixer",
            "Mixer",
            |_| rsx! { div { "mixer" } },
        ))
        .view(View::new(
            "song.chart",
            "Chart",
            |_| rsx! { div { "chart" } },
        ))
        .view(View::new(
            "song.lyrics",
            "Lyrics",
            |_| rsx! { div { "lyrics" } },
        ));
    rsx! {
        div { style: "position:absolute; inset:0; display:flex; flex-direction:column;",
            DeckStrip {}
            div { style: "flex:1 1 0; min-height:0; display:flex; position:relative;",
                WorkspaceView { workspace, views, screensets, under_strip: true }
            }
        }
    }
}

fn doc() -> DioxusDocument {
    let mut doc = DioxusDocument::new(
        VirtualDom::new(App),
        DocumentConfig {
            viewport: Some(Viewport::new(1100, 700, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();
    settle(&mut doc);
    doc
}

fn settle(doc: &mut DioxusDocument) {
    for _ in 0..4 {
        doc.poll(None);
        doc.inner.borrow_mut().resolve(0.0);
    }
}

fn count(doc: &DioxusDocument, selector: &str) -> usize {
    let d = doc.inner.borrow();
    d.query_selector_all(selector).map_or(0, |v| v.len())
}

fn attr(doc: &DioxusDocument, selector: &str, name: &str) -> Option<String> {
    let d = doc.inner.borrow();
    let id = d.query_selector(selector).ok()??;
    d.get_node(id)?
        .element_data()?
        .attr(blitz_dom::local_name!("title"))
        .filter(|_| name == "title")
        .map(str::to_owned)
}

fn handle() -> DeckHandle {
    HANDLE.with(|h| h.borrow().unwrap())
}

#[test]
fn decks_on_a_strip_switch_layouts_and_come_back_after_a_restart() {
    architect_shell::motion::set_reduced_motion(Some(true));
    KEPT.with(|k| {
        *k.borrow_mut() = Some(Kept {
            session: WorkspaceStore::new(MemoryStore::new()),
            contexts: Rc::new(MemoryContextStore::new()),
        });
    });
    let mut d = doc();
    assert_eq!(count(&d, "[data-deck]"), 2, "a tab per deck");
    assert_eq!(count(&d, "[data-deck-strip] [aria-selected=\"true\"]"), 1);
    assert_eq!(
        count(&d, "[data-tab=\"mixer\"]"),
        1,
        "Amazing Grace in Mixing"
    );

    // Slow River switched to Charts, then rearranged by hand.
    let h = handle();
    let slow = d.vdom.in_runtime(|| h.decks.peek().decks[1].id.clone());
    d.vdom.in_runtime(|| {
        h.activate(&slow);
        h.set_screenset(&slow, "charts");
    });
    settle(&mut d);
    assert_eq!(
        count(&d, "[data-tab=\"lyrics\"]"),
        1,
        "Slow River in Charts"
    );
    let ws = d.vdom.in_runtime(|| {
        let mut st = h.act(|st| st.live.clone());
        let _ = st.open(Pane::new("mixer", "song.mixer", "Mixer"), Target::Tab);
        st
    });
    d.vdom.in_runtime(|| h.act(|st| st.live = ws.clone()));
    settle(&mut d);
    assert_eq!(
        attr(&d, &format!("[data-deck=\"{}\"]", slow.0), "title").as_deref(),
        Some("Slow River — changed from its screenset")
    );
    // Back to Amazing Grace: Mixing as saved; and back again: as left.
    let grace = d.vdom.in_runtime(|| h.decks.peek().decks[0].id.clone());
    d.vdom.in_runtime(|| h.activate(&grace));
    settle(&mut d);
    assert_eq!(count(&d, "[data-tab=\"lyrics\"]"), 0);
    assert_eq!(count(&d, "[data-tab=\"mixer\"]"), 1);
    d.vdom.in_runtime(|| h.activate(&slow));
    settle(&mut d);
    assert_eq!(count(&d, "[data-tab=\"lyrics\"]"), 1);
    assert_eq!(count(&d, "[data-tab=\"mixer\"]"), 1, "its own change kept");
    d.vdom.in_runtime(|| h.keep_all());
    drop(d);

    // A restart: the strip, the deck on show, each deck as it was left.
    let mut again = doc();
    settle(&mut again);
    assert_eq!(count(&again, "[data-deck]"), 2);
    let h = handle();
    let active = again
        .vdom
        .in_runtime(|| h.decks.peek().active_deck().unwrap().context.id.clone());
    assert_eq!(active, "slow-river");
    assert_eq!(count(&again, "[data-tab=\"lyrics\"]"), 1);
    assert_eq!(count(&again, "[data-tab=\"mixer\"]"), 1);
    assert_eq!(
        count(&again, "[data-deck][data-screenset=\"charts\"]"),
        1,
        "its screenset kept with it"
    );
}

#[test]
fn a_context_loaded_with_nothing_kept_is_told_once() {
    architect_shell::motion::set_reduced_motion(Some(true));
    KEPT.with(|k| {
        *k.borrow_mut() = Some(Kept {
            session: WorkspaceStore::new(MemoryStore::new()),
            contexts: Rc::new(MemoryContextStore::new()),
        });
    });
    let mut d = doc();
    FRESH.with(|f| f.borrow_mut().clear());
    let h = handle();
    let album = DeckContext::new("song", "blue-moon", "Blue Moon Walk");
    d.vdom
        .in_runtime(|| h.load(album.clone(), architect_workspace::Load::NewDeck));
    settle(&mut d);
    assert_eq!(FRESH.with(|f| f.borrow().clone()), vec!["song:blue-moon"]);
    // Open already: it comes forward, and is not new.
    d.vdom
        .in_runtime(|| h.load(album, architect_workspace::Load::Here));
    settle(&mut d);
    assert_eq!(FRESH.with(|f| f.borrow().len()), 1);
}
