//! A page in the app's window: a linked extension's compiled view, drawn
//! by `ExtensionView`, is overridden live by `<app dir>/pages/<ext>/<view>.tsx`
//! — its page drawn in a Blitz sub-document, following each save — and
//! comes back when the file is deleted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_extensions::interface::{ExtensionView, use_extension_ui};
use architect_extensions::linked::{Builder, Extension};
use architect_extensions::remote::mounted_views;
use architect_pages::host::AppHost;
use architect_pages::runner::Pages;
use architect_pages::ui::use_view_pages;
use architect_settings::Registry;
use blitz_dom::{BaseDocument, Document as _, DocumentConfig};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;

const MANIFEST: &str = r#"
id = "lab.paged"
name = "Paged"
version = "0.1.0"
kind = "process"

[entry]
path = "paged-bin"

[[views]]
id = "panel"
title = "Panel"
"#;

#[component]
fn Panel() -> Element {
    rsx! { span { id: "compiled", "the compiled panel" } }
}

struct Paged;

impl Extension for Paged {
    fn manifest(&self) -> &'static str {
        MANIFEST
    }

    fn build(&self, ext: &mut Builder) {
        ext.view("panel", Panel);
    }
}

#[derive(Props, Clone)]
struct AppProps {
    host: Host,
    pages: Pages,
}

impl PartialEq for AppProps {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

#[allow(non_snake_case, clippy::needless_pass_by_value)]
fn App(props: AppProps) -> Element {
    let ui = use_extension_ui(props.host.clone(), Arc::new(ActionList::new()));
    let _ = use_view_pages(props.pages);
    rsx! {
        div { style: "width: 400px; height: 300px; display: flex; flex-direction: column;",
            ExtensionView { ui, extension: "lab.paged", view: "panel" }
        }
    }
}

fn in_app(doc: &BaseDocument, selector: &str) -> Option<String> {
    let node = doc.query_selector(selector).ok()??;
    Some(doc.get_node(node)?.text_content())
}

fn in_page(doc: &BaseDocument, selector: &str) -> Option<String> {
    let at = *mounted_views(doc).first()?;
    let view = doc.subdoc(at)?.inner();
    let node = view.query_selector(selector).ok()??;
    Some(view.get_node(node)?.text_content())
}

fn pump(doc: &mut DioxusDocument, what: &str, until: impl Fn(&BaseDocument) -> bool) -> Duration {
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
            return started.elapsed();
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn save(path: &PathBuf, text: &str) {
    std::thread::sleep(Duration::from_millis(15));
    std::fs::write(path, text).unwrap();
}

#[test]
fn a_compiled_view_is_overridden_by_a_file_and_restored_when_it_goes() {
    let app_dir = tempfile::tempdir().unwrap();
    let host = Host::new(ActionList::new(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(app_dir.path().join("installs")))
        .with_linked(&Paged)
        .unwrap();
    let _ = ext.sync();
    let pages = Pages::new(AppHost::new(ActionList::new()))
        .with_app_dir(app_dir.path())
        .with_js();
    let vdom = VirtualDom::new_with_props(
        App,
        AppProps {
            host: ext.host().clone(),
            pages,
        },
    );
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();
    pump(&mut doc, "the compiled view", |d| {
        in_app(d, "#compiled").as_deref() == Some("the compiled panel")
    });

    // An override appears: the page draws the view.
    let dir = app_dir.path().join("pages").join("lab.paged");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("panel.tsx");
    let page = |text: &str| {
        format!(
            "import {{ h }} from \"architect\";\nexport default function Panel() {{\n  return <h1 id=\"title\">{text}</h1>;\n}}\n"
        )
    };
    save(&file, &page("from a file"));
    let took = pump(&mut doc, "the override", |d| {
        in_page(d, "#title").as_deref() == Some("from a file")
    });
    eprintln!("override shown {took:?} after it was written");
    assert!(in_app(&doc.inner.borrow(), "#compiled").is_none());

    // Edited live: the pane follows.
    save(&file, &page("edited live"));
    let took = pump(&mut doc, "the edit", |d| {
        in_page(d, "#title").as_deref() == Some("edited live")
    });
    eprintln!("edit shown {took:?} after the save");
    assert!(took < Duration::from_secs(1), "{took:?}");

    // Deleted: the compiled view is back.
    std::fs::remove_file(&file).unwrap();
    pump(&mut doc, "the compiled view again", |d| {
        in_app(d, "#compiled").as_deref() == Some("the compiled panel")
            && mounted_views(d).is_empty()
    });
}
