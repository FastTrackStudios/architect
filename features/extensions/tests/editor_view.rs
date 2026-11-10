//! An editor in a linked extension's own view (#512, step (c)): the
//! extension names `architect-editor` only, its view is the editor's
//! component over a document of its own, a fence in it is highlighted in
//! the classes the theme's `syntax.*` tokens colour, and the extension
//! hears its changes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Mutex;

use architect::action::ActionList;
use architect_editor::state::{Changes, EditorState, TransactionSpec, markdown};
use architect_editor::ui::{DecorationSource, Editor, TransactionEvent, dispatch_spec};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_extensions::linked::{Builder, Extension};
use architect_settings::Registry;
use dioxus::prelude::*;

const MANIFEST: &str = r#"
id = "lab.snippets"
name = "Snippets"
version = "0.1.0"
kind = "process"

[entry]
path = "snippets-bin"

[[views]]
id = "source"
title = "Source"
"#;

const SOURCE: &str = "```rs\nfn main() {}\n```\n";

/// What the extension heard from its editor: each change's text after.
static HEARD: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The extension's view: the editor, read-only, over its own document.
#[component]
fn Source() -> Element {
    let state = use_signal(|| EditorState::new(SOURCE));
    let decorations = use_hook(|| DecorationSource::new(markdown::live_preview));
    let on_transaction = use_callback(|event: TransactionEvent| {
        if event.is_edit() {
            HEARD.lock().unwrap().push(event.doc_after.to_string());
        }
    });
    // An edit made by the extension's own code, through the editor's one
    // door for them: it reaches the sink as the person's typing does.
    use_hook(|| {
        let at = SOURCE.find("main").unwrap();
        let spec = TransactionSpec::new().changes(Changes::replace(at..at + 4, "start"));
        dispatch_spec(state, spec, Some(on_transaction));
    });
    rsx! {
        Editor { state, decorations, on_transaction, editable: false }
    }
}

struct Snippets;

impl Extension for Snippets {
    fn manifest(&self) -> &'static str {
        MANIFEST
    }

    fn build(&self, ext: &mut Builder) {
        ext.view("source", Source);
    }
}

#[test]
fn a_linked_extensions_view_holds_an_editor_and_hears_its_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list, SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_linked(&Snippets)
        .unwrap();
    let _ = ext.sync();
    let root = ext
        .host()
        .linked_view("lab.snippets", "source")
        .expect("its view is its component");

    let mut dom = VirtualDom::new(root);
    dom.rebuild_in_place();
    let html = dioxus_ssr::render(&dom);
    assert!(
        html.contains("editor-root"),
        "the editor is mounted: {html}"
    );
    assert!(html.contains("start"), "it shows the document: {html}");
    // The fence's `fn` is a keyword token: the class the theme's
    // `syntax.keyword` colours (`architect_editor::theme::css`).
    assert!(
        html.contains("md-tok-k"),
        "the fence is highlighted: {html}"
    );
    assert!(architect_editor::theme::css().contains(".md-tok-k"));
    assert_eq!(
        *HEARD.lock().unwrap(),
        vec![SOURCE.replace("main", "start")],
        "the extension heard the change"
    );
}
