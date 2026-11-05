//! The main area is always there (Obsidian's): with every tab closed it
//! is "No file is open", with what the app offers (New note), Go to file
//! and no Close (it is the last group); a dock never takes its room.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use architect_workspace::{
    Command, Pane, Side, Target, View, Views, Workspace, WorkspaceView,
};
use blitz_dom::Document as _;
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native::{DioxusDocument, DocumentConfig};
use lucide_dioxus::SquarePen;

thread_local! {
    static WORKSPACE: std::cell::RefCell<Option<Signal<Workspace>>> = const { std::cell::RefCell::new(None) };
}

fn views() -> Views {
    Views::new()
        .view(View::new("note", "Note", |_| rsx! { div { "a note" } }).many())
        .view(View::new("files", "Files", |_| rsx! { div { "the files" } }).home(Target::Dock(Side::Left)))
}

fn app() -> Element {
    let commands = architect_commands::use_commands_provider();
    let _ = commands;
    let workspace = use_signal(|| {
        let mut w = Workspace::new();
        w.open(Pane::new("n1", "note", "One"), Target::Tab).unwrap();
        w.open(Pane::new("files", "files", "Files"), Target::Dock(Side::Left))
            .unwrap();
        w
    });
    WORKSPACE.with(|s| *s.borrow_mut() = Some(workspace));
    rsx! {
        WorkspaceView {
            workspace,
            views: views(),
            commands: vec![Command::new("new-note", "New note", architect_shell::icon!(SquarePen))],
            empty: vec!["new-note".to_owned()],
        }
    }
}

fn settle(doc: &mut DioxusDocument) {
    for _ in 0..4 {
        doc.poll(None);
        doc.inner.borrow_mut().resolve(0.0);
    }
}

fn rects(doc: &DioxusDocument, selector: &str) -> Vec<(f64, f64, f64, f64)> {
    let d = doc.inner.borrow();
    d.query_selector_all(selector)
        .unwrap()
        .into_iter()
        .filter_map(|id| d.get_client_bounding_rect(id))
        .map(|r| (r.x, r.y, r.width, r.height))
        .collect()
}

#[test]
fn with_every_tab_closed_the_main_area_says_no_file_is_open() {
    let mut doc = DioxusDocument::new(
        VirtualDom::new(app),
        DocumentConfig {
            viewport: Some(Viewport::new(1200, 800, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();
    settle(&mut doc);
    assert!(rects(&doc, "[data-empty-group]").is_empty(), "a tab is open");
    let workspace = WORKSPACE.with(|s| s.borrow().unwrap());
    doc.vdom.in_runtime(|| {
        let mut w = workspace;
        w.write()
            .close(&architect_workspace::PaneId::new("n1"))
            .unwrap();
    });
    settle(&mut doc);
    let empty = rects(&doc, "[data-empty-group]");
    assert_eq!(empty.len(), 1, "the main area stays, empty");
    let (x, _, w, _) = empty[0];
    assert!(w > 600.0, "it keeps its room beside the dock: {empty:?}");
    let dock = rects(&doc, "[data-group]");
    assert!(
        dock.iter().all(|(dx, _, dw, _)| dx + dw <= x + 0.5),
        "the dock stays at its side: {dock:?} / {empty:?}"
    );
    assert_eq!(rects(&doc, "[data-empty-command=\"new-note\"]").len(), 1);
    assert_eq!(rects(&doc, "[data-empty-command=\"go-to-file\"]").len(), 1);
    assert!(
        rects(&doc, "[data-empty-command=\"close\"]").is_empty(),
        "the last group has no Close"
    );
}
