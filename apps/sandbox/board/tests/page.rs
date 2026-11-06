//! The Board's tab as a page file (pages/board.tsx): the linked Board's
//! data and actions, drawn by its TSX page on `QuickJS`. A pin from the
//! page pins on the board; counting primes runs in the page's worker.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections, gate};
use architect_extensions::install::Installs;
use architect_extensions::remote::RemoteDocument;
use architect_extensions::wire::{EventData, Pointer, ViewEvent};
use architect_pages::files::{Origin, PageFile, write_builtin};
use architect_pages::host::AppHost;
use architect_pages::runner::{PageSession, Pages};
use architect_sandbox_board::{Board, ID, PAGES};
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::Value;

struct Pane {
    session: PageSession,
    view: RemoteDocument,
    _events: futures::channel::mpsc::UnboundedReceiver<ViewEvent>,
}

impl Pane {
    fn pump(&mut self) {
        while let Ok(frame) = self.session.frames.try_recv() {
            self.view.apply(frame).unwrap();
        }
    }

    fn text(&self, selector: &str) -> Option<String> {
        let doc = self.view.document();
        let node = doc.query_selector(selector).ok()??;
        Some(doc.get_node(node)?.text_content())
    }

    fn until(&mut self, what: &str, ok: impl Fn(&Self) -> bool) {
        let start = Instant::now();
        loop {
            self.pump();
            if ok(self) {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "waited for {what}: {:?}",
                self.session.status().log.lines()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn click(&self, selector: &str) {
        let doc = self.view.document();
        let node = doc.query_selector(selector).unwrap().unwrap();
        let element = doc
            .get_node(node)
            .unwrap()
            .attr(blitz_dom::LocalName::from("data-dioxus-id"))
            .unwrap()
            .parse()
            .unwrap();
        self.session.event(ViewEvent {
            element,
            name: "click".to_owned(),
            bubbles: true,
            data: EventData::Mouse(Pointer::default()),
        });
    }
}

#[test]
fn the_boards_tab_is_a_page_over_the_boards_own_actions() {
    let app = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(app.path().join("installs")))
        .with_linked(&Board)
        .unwrap();
    let _ = ext.sync();
    let manifest = ext.host().get(ID).unwrap().manifest;
    assert_eq!(
        manifest
            .views
            .iter()
            .find(|v| v.id == "board")
            .and_then(|v| v.page.as_deref()),
        Some("pages/board.tsx")
    );
    let _ = block_on(list.run_json_async("LAB_BOARD_CLEAR", b""));

    // The built-in's page, read-only in the app's folder.
    let dir = write_builtin(app.path(), ID, PAGES).unwrap();
    let page = PageFile::new(dir.join("board.tsx"), Origin::Builtin).unwrap();
    assert!(!page.editable());
    let pages = Pages::new(AppHost::new(list.clone()))
        .with_app_dir(app.path())
        .with_js();
    pages.declare(ID, "board", page.clone());
    assert_eq!(pages.resolve(ID, "board", None), Some(page.clone()));

    let session = pages
        .open(&page, ID, "board", Some(gate(&manifest)))
        .unwrap();
    let (to, from) = futures::channel::mpsc::unbounded();
    let mut tab = Pane {
        session,
        view: RemoteDocument::new(to),
        _events: from,
    };
    tab.until("the board", |p| {
        p.text("#board-title").as_deref() == Some("Board")
    });
    tab.until("its data", |p| {
        p.text("#board-page")
            .is_some_and(|t| t.contains("Nothing pinned yet."))
    });

    // The page's Pin runs the Board's action; the board says so.
    tab.click("#pin");
    tab.until("the pin", |p| {
        p.text(".pin").as_deref() == Some("📌 a page's pin")
    });
    let state: Value =
        serde_json::from_slice(&block_on(list.run_json_async("LAB_BOARD_STATE", b"")).unwrap())
            .unwrap();
    assert_eq!(state["pins"], serde_json::json!(["a page's pin"]));

    // Heavy work in the page's worker; the page answers meanwhile.
    tab.click("#primes");
    tab.until("the worker", |p| {
        p.text("#primes-state").as_deref() == Some("Counting in a worker…")
    });
    tab.click("#clear");
    tab.until("a clear while it counts", |p| p.text(".pin").is_none());
    tab.until("the count", |p| {
        p.text("#primes-state").as_deref() == Some("78498 primes below 1,000,000.")
    });
}

/// The Board's Preact view (pages/preact.tsx): Preact, vendored in its
/// folder, rendering to the page's own DOM (`architect/dom`); a click
/// reaches its listener and its state draws again.
#[test]
fn the_boards_preact_page_renders_and_answers_clicks() {
    let app = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(app.path().join("installs")))
        .with_linked(&Board)
        .unwrap();
    let _ = ext.sync();
    let manifest = ext.host().get(ID).unwrap().manifest;
    assert_eq!(
        manifest
            .views
            .iter()
            .find(|v| v.id == "preact")
            .and_then(|v| v.page.as_deref()),
        Some("pages/preact.tsx")
    );
    let dir = write_builtin(app.path(), ID, PAGES).unwrap();
    let page = PageFile::new(dir.join("preact.tsx"), Origin::Builtin).unwrap();
    let pages = Pages::new(AppHost::new(list))
        .with_app_dir(app.path())
        .with_js();
    let session = pages
        .open(&page, ID, "preact", Some(gate(&manifest)))
        .unwrap();
    let (to, from) = futures::channel::mpsc::unbounded();
    let mut tab = Pane {
        session,
        view: RemoteDocument::new(to),
        _events: from,
    };
    tab.until("the page", |p| {
        p.text("#preact-title").as_deref() == Some("Preact")
    });
    assert_eq!(tab.text("#preact-count").as_deref(), Some("0"));
    tab.click("#preact-inc");
    tab.until("a click", |p| {
        p.text("#preact-count").as_deref() == Some("1")
    });
    tab.click("#preact-inc");
    tab.until("another", |p| {
        p.text("#preact-count").as_deref() == Some("2")
    });
    tab.click("#preact-add");
    tab.until("an item", |p| {
        p.text(".preact-item").as_deref() == Some("Item 1")
    });
    tab.click("#preact-reset");
    tab.until("the reset", |p| {
        p.text("#preact-count").as_deref() == Some("0")
    });
    assert_eq!(tab.session.status().error(), None);
}
