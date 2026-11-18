//! A pane whose view is not in the app yet (a window restoring its layout
//! before the extension that draws it has loaded) says so, and draws its
//! view as soon as the app has it: the pane keeps its state until the
//! view comes back, and then it is back (architect#877).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use architect_workspace::{Pane, Target, View, Views, Workspace, WorkspaceView};
use blitz_dom::Document as _;
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native::{DioxusDocument, DocumentConfig};

thread_local! {
    /// Whether the app has the chart view at the start.
    static START: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Whether the app has the chart view yet (its extension loaded).
    static LOADED: std::cell::RefCell<Option<Signal<bool>>> = const { std::cell::RefCell::new(None) };
}

fn views(loaded: bool) -> Views {
    let views = Views::new().view(View::new("note", "Note", |_| rsx! { div { "a note" } }).many());
    if loaded {
        views.view(
            View::new(
                "ext/chart",
                "Chart",
                |_| rsx! { div { "data-chart": "", "the chart" } },
            )
            .many(),
        )
    } else {
        views
    }
}

fn app() -> Element {
    let _ = architect_commands::use_commands_provider();
    let loaded = use_signal(|| START.with(std::cell::Cell::get));
    LOADED.with(|s| *s.borrow_mut() = Some(loaded));
    // The layout as restored: the chart's pane, open before its view.
    let workspace = use_signal(|| {
        let mut w = Workspace::new();
        w.open(Pane::new("c1", "ext/chart", "Chart"), Target::Tab)
            .unwrap();
        w
    });
    // The page renders again when an extension loads, its views made
    // anew (as the app's Vault page does).
    rsx! {
        WorkspaceView { workspace, views: views(loaded()), commands: Vec::new() }
    }
}

fn settle(doc: &mut DioxusDocument) {
    for _ in 0..4 {
        doc.poll(None);
        doc.inner.borrow_mut().resolve(0.0);
    }
}

fn count(doc: &DioxusDocument, selector: &str) -> usize {
    doc.inner
        .borrow()
        .query_selector_all(selector)
        .unwrap()
        .len()
}

fn document() -> DioxusDocument {
    let mut doc = DioxusDocument::new(
        VirtualDom::new(app),
        DocumentConfig {
            viewport: Some(Viewport::new(1200, 800, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();
    settle(&mut doc);
    doc
}

#[test]
fn a_pane_draws_its_view() {
    START.with(|s| s.set(true));
    let doc = document();
    assert_eq!(count(&doc, "[data-pane=\"c1\"] [data-chart]"), 1);
}

#[test]
fn a_restored_pane_draws_its_view_once_the_view_is_loaded() {
    let mut doc = document();
    assert_eq!(count(&doc, "[data-chart]"), 0, "no chart view yet");

    let loaded = LOADED.with(|s| s.borrow().unwrap());
    doc.vdom.in_runtime(|| {
        let mut loaded = loaded;
        loaded.set(true);
    });
    settle(&mut doc);
    assert_eq!(
        count(&doc, "[data-pane=\"c1\"] [data-chart]"),
        1,
        "the restored pane draws its view once the app has it"
    );
}
