//! A `CodeEditor` wired to its language's server (`ui::use_code_document`),
//! played headlessly on Blitz against the stub server (this binary, run
//! with `--stub-lsp`): the server's diagnostic is drawn under its range,
//! a format the server answers lands in the editor's text, and the
//! server stops when its last editor goes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::redundant_closure_for_method_calls
)]

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_editor::languages::{Language, Registered};
use architect_editor::lsp::{LanguageServers, stub};
use architect_editor::state::EditorState;
use architect_editor::ui::{CodeEditor, EDITOR_CSS, language_named, use_code_document};
use architect_guide::run::{App, Desktop, Film, Pointer};
use dioxus::prelude::*;

const TEXT: &str = "fn main() {}\n";

struct Setup {
    servers: LanguageServers,
    language: Registered,
    root: PathBuf,
    path: PathBuf,
}

static SETUP: OnceLock<Setup> = OnceLock::new();

const NAME: &str = "a_code_editor_shows_its_servers_diagnostics_and_takes_its_format";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(at) = args.iter().position(|a| a == "--stub-lsp") {
        stub::serve(&args[at + 1..]);
        return;
    }
    if args.iter().any(|a| a == "--list") {
        if !args.iter().any(|a| a == "--ignored") {
            println!("{NAME}: test");
        }
        return;
    }
    if args
        .iter()
        .skip(1)
        .any(|a| !a.starts_with('-') && a != NAME)
    {
        return;
    }
    a_code_editor_shows_its_servers_diagnostics_and_takes_its_format();
    println!("test {NAME} ... ok");
}

#[component]
fn Root() -> Element {
    let state = use_signal(|| EditorState::new(TEXT));
    let language = use_signal(|| language_named("rs"));
    let code = use_code_document(
        || {
            let s = SETUP.get()?;
            s.servers.open(&s.language, &s.root, &s.path, TEXT).ok()
        },
        state,
        None,
    );
    // Blocks the scenario can wait for (an inline span has no box to
    // find): the diagnostic arrived, the format landed.
    let heard = !code.diagnostics.read().is_empty();
    let formatted = state.read().doc.to_string().starts_with(stub::FORMATTED);
    rsx! {
        document::Style { {EDITOR_CSS} }
        if heard {
            div { id: "heard", style: "height: 4px", "heard" }
        }
        if formatted {
            div { id: "formatted", style: "height: 4px", "formatted" }
        }
        div { "data-test": "code",
            CodeEditor {
                state,
                language,
                diagnostics: code.diagnostics,
                on_transaction: code.on_transaction,
            }
        }
    }
}

#[derive(Default)]
struct Html(String);

impl Film for Html {
    fn frame(&mut self, desktop: &Desktop, _: Pointer) {
        let Some(main) = desktop.windows().first() else {
            return;
        };
        let doc = main.doc.inner.borrow();
        if let Ok(found) = doc.query_selector_all("[data-test=\"code\"]") {
            self.0 = found
                .iter()
                .filter_map(|id| doc.get_node(*id))
                .map(|n| n.outer_html_pretty())
                .collect::<Vec<_>>()
                .join("\n");
        }
    }
}

const SCENARIO: &str = r##"id lsp-view
title "CodeEditor and its server"
view {width 800, height 400}
capture {fps 10, width 400}
steps (
    {id drawn, target @css".editor-code", done {timeout 10}}
    {id heard, target @css"#heard", done {timeout 15}}
    {id formatted, target @css"#formatted", done {timeout 15}}
    {id end, hold 0.2}
)
"##;

fn a_code_editor_shows_its_servers_diagnostics_and_takes_its_format() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("main.rs");
    let exe = std::env::current_exe().unwrap();
    let servers = LanguageServers::new(runtime.handle().clone());
    let setup = SETUP.get_or_init(|| Setup {
        servers: servers.clone(),
        language: Registered {
            owner: "lab.stub".to_owned(),
            language: Language::linked("rust", "Rust")
                .extensions(&["rs"])
                .server(exe.to_str().unwrap(), &["--stub-lsp"]),
        },
        root: root.path().to_owned(),
        path: path.clone(),
    });
    // Once the editor's document is open and has its diagnostic, format
    // it, as `EDITOR_FORMAT` does.
    let formatting = setup.servers.clone();
    runtime.spawn(async move {
        let start = Instant::now();
        loop {
            if let Some(doc) = formatting.document(&path)
                && !doc.diagnostics().is_empty()
            {
                doc.format().await.unwrap();
                return;
            }
            assert!(start.elapsed() < Duration::from_secs(15), "never opened");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
    let scenario = architect_guide::load(SCENARIO).unwrap();
    let mut film = Html::default();
    architect_guide::run::run(
        &App::new(Root, Arc::new(ActionList::new())),
        &scenario,
        Some(&mut film),
    )
    .unwrap_or_else(|e| panic!("{e}\n{}", film.0));
    let html = film.0;
    // The server's warning, drawn as the editor's underline and its
    // line's number marked.
    assert!(html.contains("cm-lsp-warning"), "the underline: {html}");
    assert!(
        html.contains("code-ln-warning"),
        "the line's number: {html}"
    );
    assert!(
        html.contains("formatted"),
        "the server's format is in the text: {html}"
    );
    // The editor gone with the app, its document closed: the server
    // stopped with its last editor.
    assert!(
        setup.servers.status().is_empty(),
        "{:?}",
        setup.servers.status()
    );
}
