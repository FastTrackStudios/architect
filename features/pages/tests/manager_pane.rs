//! The extensions manager in a workspace pane (`ManagerView`): it fills
//! the pane, every item is in its list (which scrolls on its own, not the
//! first twelve), its pages among them, and a narrow pane shows the list
//! without an item's page beside it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::browse::Browse;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_extensions::manager::{Item, ItemKind, Manager};
use architect_extensions::page::{ManagerView, use_manager_page};
use architect_pages::files::{Origin, PageFile, write_builtin};
use architect_pages::host::AppHost;
use architect_pages::manage::PagesKind;
use architect_pages::runner::Pages;
use architect_settings::Registry;
use blitz_dom::{BaseDocument, Document as _, DocumentConfig};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;
use serde_json::Value;

/// Thirty things to manage.
struct Many;

impl ItemKind for Many {
    fn id(&self) -> &'static str {
        "many"
    }

    fn title(&self) -> &'static str {
        "Many"
    }

    fn list(&self) -> Vec<Item> {
        (0..30)
            .map(|n| Item::new("many", format!("m{n:02}"), format!("Thing {n:02}")))
            .collect()
    }

    fn run(&self, _: &str, _: &str, _: &Value) -> Result<Value, String> {
        Ok(Value::Null)
    }
}

#[derive(Props, Clone)]
struct AppProps {
    manager: Manager,
    browse: Browse,
    width: u32,
}

impl PartialEq for AppProps {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

#[allow(non_snake_case, clippy::needless_pass_by_value)]
fn App(props: AppProps) -> Element {
    use_manager_page(props.manager, props.browse);
    rsx! {
        div { style: "width: {props.width}px; height: 300px; display: flex; flex-direction: column;",
            ManagerView {}
        }
    }
}

fn pump(doc: &mut DioxusDocument, what: &str, until: impl Fn(&BaseDocument) -> bool) {
    let started = Instant::now();
    let waker = futures::task::noop_waker();
    loop {
        for _ in 0..16 {
            if !doc.poll(Some(std::task::Context::from_waker(&waker))) {
                break;
            }
        }
        doc.inner.borrow_mut().resolve(0.0);
        if until(&doc.inner.borrow()) {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}: {}",
            text(&doc.inner.borrow())
                .chars()
                .take(600)
                .collect::<String>()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn text(doc: &BaseDocument) -> String {
    let Some(node) = doc.query_selector(".manager-list").ok().flatten() else {
        return String::new();
    };
    let list = doc
        .get_node(node)
        .map(|n| n.text_content())
        .unwrap_or_default();
    let page = doc.query_selector(".manager-item").ok().flatten();
    let page = page
        .and_then(|n| doc.get_node(n))
        .map(|n| n.text_content())
        .unwrap_or_default();
    format!("{list}\n{page}")
}

fn height(doc: &BaseDocument, selector: &str) -> Option<f32> {
    let node = doc.query_selector(selector).ok()??;
    Some(doc.get_node(node)?.final_layout().size.height)
}

fn manager_in(width: u32) -> DioxusDocument {
    let tmp = tempfile::tempdir().unwrap();
    let pages = Pages::new(AppHost::new(ActionList::new()))
        .with_app_dir(tmp.path())
        .with_js();
    let dir = write_builtin(
        tmp.path(),
        "lab.board",
        &[(
            "board.tsx",
            "export default function B() { return null; }\n",
        )],
    )
    .unwrap();
    pages.declare(
        "lab.board",
        "board",
        PageFile::new(dir.join("board.tsx"), Origin::Builtin).unwrap(),
    );
    let manager = Manager::new().with(Many).with(PagesKind::new(pages));
    let host = Host::new(ActionList::new(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")));
    let vdom = VirtualDom::new_with_props(
        App,
        AppProps {
            manager,
            browse: Browse::new(ext),
            width,
        },
    );
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(1000, 600, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();
    // Kept on disk until the test ends (its pages' files).
    std::mem::forget(tmp);
    doc
}

#[test]
fn the_manager_fills_its_pane_and_lists_every_item() {
    let mut doc = manager_in(900);
    pump(&mut doc, "the list", |d| text(d).contains("Thing 29"));
    let all = text(&doc.inner.borrow());
    assert!(all.contains("lab.board/board"), "the page is listed");
    assert!(all.contains("Thing 00"), "{all}");
    // Wide: the first item's page beside the list.
    assert!(
        doc.inner
            .borrow()
            .query_selector(".manager-item")
            .unwrap()
            .is_some()
    );
    // The list scrolls inside the pane: it is no taller than the pane.
    let list = height(&doc.inner.borrow(), ".manager-list").unwrap();
    assert!(list > 0.0 && list < 300.0, "the list is {list}px tall");
}

#[test]
fn a_narrow_pane_shows_the_list_alone() {
    let mut doc = manager_in(360);
    pump(&mut doc, "the list, alone", |d| {
        text(d).contains("Thing 29") && d.query_selector(".manager-item").unwrap().is_none()
    });
    let list = height(&doc.inner.borrow(), ".manager-list").unwrap();
    assert!(list > 0.0 && list < 300.0, "the list is {list}px tall");
}
