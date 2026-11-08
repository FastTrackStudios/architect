//! Scope tabs in the tab bar, laid out by Blitz: a scope tab per deck at
//! the leading edge of the main window's first bar, then a divider, then
//! the scope's own tabs; switching shows each deck's own layout (the
//! general tabs' with none active), a restart brings each back as it was
//! left, and a custom tab kind draws its panes' tabs its way.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::rc::Rc;

use architect_workspace::{
    DeckContext, DeckHandle, DeckStores, DecksConfig, FormClass, MemoryContextStore, MemoryStore,
    Pane, Screenset, Screensets, Side, TabKind, TabKinds, TabKindsUi, TabWidth, Target, View,
    Views, Workspace, WorkspaceStore, WorkspaceView, use_decks, use_tab_kinds,
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
    static START: RefCell<Vec<DeckContext>> = const { RefCell::new(Vec::new()) };
}

fn two_songs() -> Vec<DeckContext> {
    vec![
        DeckContext::new("song", "amazing-grace", "Amazing Grace")
            .property("key", "G")
            .property("tempo", "72"),
        DeckContext::new("song", "slow-river", "Slow River").property("tempo", "96"),
    ]
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
        templates: vec![
            architect_workspace::DeckTemplate::new("song", "mixing"),
            architect_workspace::DeckTemplate::new("vault", "charts"),
        ],
        start: START.with(|s| s.borrow().clone()),
        general: Some(DeckContext::new("vault", "vault", "Vault")),
        views: None,
        bridge: architect_workspace::Bridge::default(),
        actions: None,
        resolve: None,
        on_fresh: Some(Callback::new(|c: DeckContext| {
            FRESH.with(|f| f.borrow_mut().push(c.key()));
        })),
    });
    use_hook(|| HANDLE.with(|h| *h.borrow_mut() = Some(handle)));
    use_tab_kinds(
        TabKindsUi::new(
            TabKinds::new()
                .with(
                    TabKind::scope("song", "Song")
                        .width(TabWidth::Wide)
                        .icon("music")
                        .subtitle("{key} · {tempo} bpm"),
                )
                .with(
                    TabKind::new("mixer", "Mixer")
                        .view("song.mixer")
                        .icon("sliders-horizontal")
                        .colour("#3f8fd1"),
                ),
        ),
        None,
    );
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
            div { style: "flex:1 1 0; min-height:0; display:flex; position:relative;",
                WorkspaceView { workspace, views, screensets }
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

fn title(doc: &DioxusDocument, selector: &str) -> Option<String> {
    let d = doc.inner.borrow();
    let id = d.query_selector(selector).ok()??;
    d.get_node(id)?
        .element_data()?
        .attr(blitz_dom::local_name!("title"))
        .map(str::to_owned)
}

fn fresh_kept() {
    KEPT.with(|k| {
        *k.borrow_mut() = Some(Kept {
            session: WorkspaceStore::new(MemoryStore::new()),
            contexts: Rc::new(MemoryContextStore::new()),
        });
    });
}

fn handle() -> DeckHandle {
    HANDLE.with(|h| h.borrow().unwrap())
}

#[test]
fn scope_tabs_lead_the_tab_bar_switch_layouts_and_come_back_after_a_restart() {
    architect_shell::motion::set_reduced_motion(Some(true));
    fresh_kept();
    START.with(|s| *s.borrow_mut() = two_songs());
    let mut d = doc();
    assert_eq!(count(&d, "[data-scope-tab]"), 2, "a scope tab per deck");
    assert_eq!(count(&d, "[data-scope-divider]"), 1);
    // In the main window's first tab bar, before its own tabs.
    assert_eq!(count(&d, "[data-group] [data-scope-tabs]"), 1);
    // A first run starts on the general tabs: none selected, their
    // screenset (Charts, the vault's template) on show.
    assert_eq!(count(&d, "[data-scope-tab][aria-selected=\"true\"]"), 0);
    assert_eq!(count(&d, "[data-tab=\"lyrics\"]"), 1, "the general tabs");
    // Drawn their kind's way: wide, key and tempo under the title; a
    // missing key drops out.
    assert_eq!(count(&d, "[data-scope-tab][data-tab-width=\"wide\"]"), 2);
    assert_eq!(count(&d, "[data-tab-subtitle=\"G · 72 bpm\"]"), 1);
    assert_eq!(count(&d, "[data-tab-subtitle=\"96 bpm\"]"), 1);

    let h = handle();
    let grace = d.vdom.in_runtime(|| h.decks.peek().decks[0].id.clone());
    let slow = d.vdom.in_runtime(|| h.decks.peek().decks[1].id.clone());
    d.vdom.in_runtime(|| h.activate(&grace));
    settle(&mut d);
    assert_eq!(count(&d, "[data-scope-tab][aria-selected=\"true\"]"), 1);
    assert_eq!(
        count(&d, "[data-tab=\"mixer\"]"),
        1,
        "Amazing Grace in Mixing"
    );
    assert_eq!(count(&d, "[data-tab=\"lyrics\"]"), 0);
    // The mixer's tab is a custom kind's: drawn its way.
    assert_eq!(
        count(&d, "[data-tab=\"mixer\"] [data-tab-kind=\"mixer\"]"),
        1
    );

    // Slow River switched to Charts, then rearranged by hand.
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
    assert!(
        title(&d, &format!("[data-scope-tab=\"{}\"]", slow.0))
            .is_some_and(|t| t.starts_with("Slow River — click again")),
        "the one on show says a click goes back to the general tabs"
    );
    // Back to Amazing Grace: Mixing as saved; and back again: as left.
    d.vdom.in_runtime(|| h.activate(&grace));
    settle(&mut d);
    assert_eq!(
        title(&d, &format!("[data-scope-tab=\"{}\"]", slow.0)).as_deref(),
        Some("Slow River — changed from its screenset")
    );
    assert_eq!(count(&d, "[data-tab=\"lyrics\"]"), 0);
    d.vdom.in_runtime(|| h.activate(&slow));
    settle(&mut d);
    assert_eq!(count(&d, "[data-tab=\"lyrics\"]"), 1);
    assert_eq!(count(&d, "[data-tab=\"mixer\"]"), 1, "its own change kept");
    // The general tabs, then back.
    d.vdom.in_runtime(|| h.show_general());
    settle(&mut d);
    assert_eq!(count(&d, "[data-scope-tab][aria-selected=\"true\"]"), 0);
    assert_eq!(
        count(&d, "[data-tab=\"mixer\"]"),
        0,
        "the general tabs as they were"
    );
    d.vdom.in_runtime(|| h.activate(&slow));
    settle(&mut d);
    d.vdom.in_runtime(|| h.keep_all());
    drop(d);

    // A restart: the scope tabs, the one on show, each as it was left.
    let mut again = doc();
    settle(&mut again);
    assert_eq!(count(&again, "[data-scope-tab]"), 2);
    let h = handle();
    let active = again
        .vdom
        .in_runtime(|| h.decks.peek().active_deck().unwrap().context.id.clone());
    assert_eq!(active, "slow-river");
    assert_eq!(count(&again, "[data-tab=\"lyrics\"]"), 1);
    assert_eq!(count(&again, "[data-tab=\"mixer\"]"), 1);
    assert_eq!(
        count(&again, "[data-scope-tab][data-screenset=\"charts\"]"),
        1,
        "its screenset kept with it"
    );
    // Closing both: the general tabs, and the bar as without scope tabs.
    let ids = again.vdom.in_runtime(|| {
        h.decks
            .peek()
            .decks
            .iter()
            .map(|d| d.id.clone())
            .collect::<Vec<_>>()
    });
    again.vdom.in_runtime(|| {
        for id in &ids {
            h.close(id);
        }
    });
    settle(&mut again);
    assert_eq!(count(&again, "[data-scope-tabs]"), 0);
    assert_eq!(count(&again, "[data-scope-divider]"), 0);
}

#[test]
fn with_no_scope_tab_open_the_bar_is_as_without_them() {
    architect_shell::motion::set_reduced_motion(Some(true));
    fresh_kept();
    START.with(|s| s.borrow_mut().clear());
    let d = doc();
    assert_eq!(count(&d, "[data-scope-tabs]"), 0);
    assert_eq!(count(&d, "[data-scope-divider]"), 0);
    assert_eq!(count(&d, "[data-tab=\"lyrics\"]"), 1, "the general tabs");
}

#[test]
fn a_context_loaded_with_nothing_kept_is_told_once() {
    architect_shell::motion::set_reduced_motion(Some(true));
    fresh_kept();
    START.with(|s| *s.borrow_mut() = two_songs());
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
