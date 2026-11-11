//! Language servers run for the editor (`lsp::LanguageServers`), against
//! the stub server (`lsp::stub`): this test binary is the server too, run
//! with `--stub-lsp`, so CI needs no real one.
//!
//! - a document gets the server's diagnostic, a hover, a completion, a
//!   format and a rename (handed to its editor) and a definition;
//! - one server per language and root, gone with its last document;
//! - a server that dies is started again and the document reopened;
//! - its owner stopped: no diagnostics, requests refused;
//! - a server that cannot start fails after its attempts;
//! - a language without a server has none.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::arithmetic_side_effects,
    clippy::single_match_else,
    clippy::redundant_closure_for_method_calls,
    clippy::indexing_slicing,
    clippy::unreachable
)]

use std::path::Path;
use std::time::{Duration, Instant};

use architect_editor::languages::{Language, Registered};
use architect_editor::lsp::stub;
use architect_editor::lsp::{LanguageServers, LspError, Restart, State};
use architect_editor::state::{Changes, Doc};
use architect_editor::{Diagnostic, Severity};
use futures::StreamExt;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(at) = args.iter().position(|a| a == "--stub-lsp") {
        stub::serve(&args[at + 1..]);
        return;
    }
    let tests: [(&str, fn()); 6] = [
        ("a_document_hears_its_server", a_document_hears_its_server),
        (
            "one_server_per_language_and_root",
            one_server_per_language_and_root,
        ),
        (
            "a_server_that_dies_is_started_again",
            a_server_that_dies_is_started_again,
        ),
        (
            "its_owner_gone_the_server_stops",
            its_owner_gone_the_server_stops,
        ),
        (
            "a_server_that_cannot_start_fails",
            a_server_that_cannot_start_fails,
        ),
        (
            "a_language_without_a_server_has_none",
            a_language_without_a_server_has_none,
        ),
    ];
    // The libtest protocol nextest speaks: `--list --format terse`
    // (`--ignored`: none are), then one run per test with `--exact`.
    if args.iter().any(|a| a == "--list") {
        if !args.iter().any(|a| a == "--ignored") {
            for (name, _) in &tests {
                println!("{name}: test");
            }
        }
        return;
    }
    let exact = args.iter().any(|a| a == "--exact");
    let filter = args
        .iter()
        .skip(1)
        .find(|a| !a.starts_with('-') && !a.chars().all(|c| c.is_ascii_digit()));
    let mut failed = 0;
    for (name, test) in tests {
        if filter.is_some_and(|f| {
            if exact {
                name != f
            } else {
                !name.contains(f.as_str())
            }
        }) {
            continue;
        }
        match std::panic::catch_unwind(test) {
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        std::process::exit(1);
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

const FAST: Restart = Restart {
    first: Duration::from_millis(20),
    most: Duration::from_millis(100),
    attempts: 3,
    healthy: Duration::from_secs(30),
};

/// A language whose server is this binary as the stub.
fn stub_language(owner: &str, extra: &[&str]) -> Registered {
    let exe = std::env::current_exe().unwrap();
    let mut args = vec!["--stub-lsp"];
    args.extend_from_slice(extra);
    Registered {
        owner: owner.to_owned(),
        language: Language::linked("stubby", "Stub")
            .extensions(&["stub"])
            .server(exe.to_str().unwrap(), &args),
    }
}

async fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "timed out: {what}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn next_diagnostics(
    rx: &mut futures::channel::mpsc::UnboundedReceiver<Vec<Diagnostic>>,
    want_some: bool,
) -> Vec<Diagnostic> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let got = tokio::time::timeout(left, rx.next())
            .await
            .expect("diagnostics in time")
            .expect("the document is open");
        if got.is_empty() != want_some {
            return got;
        }
    }
}

fn path_in(root: &Path, name: &str) -> std::path::PathBuf {
    root.join(name)
}

fn a_document_hears_its_server() {
    let rt = runtime();
    let servers = LanguageServers::with_restart(rt.handle().clone(), FAST);
    let root = tempfile::tempdir().unwrap();
    let path = path_in(root.path(), "a.stub");
    let text = "hi there\n";
    let doc = servers
        .open(&stub_language("lab.stub", &[]), root.path(), &path, text)
        .unwrap();
    rt.block_on(async {
        // Its diagnostic, as the editor draws it.
        let mut diagnostics = doc.watch_diagnostics();
        let got = next_diagnostics(&mut diagnostics, true).await;
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!((got[0].from, got[0].to), (0, 2));
        assert_eq!(got[0].severity, Severity::Warning);
        assert_eq!(got[0].message, stub::DIAGNOSTIC);
        assert_eq!(servers.status()[0].state, State::Running);
        assert_eq!(servers.status()[0].documents, 1);

        // Hover and completion.
        assert_eq!(doc.hover(3).await.unwrap().as_deref(), Some(stub::HOVER));
        let completions = doc.completion(3).await.unwrap();
        assert_eq!(completions[0].label, stub::COMPLETION);
        assert_eq!(completions[0].insert, stub::COMPLETION);

        // An edit goes as didChange; the diagnostic comes back for it.
        let changes = Changes::insert(0, "> ");
        let after = changes.apply(&Doc::new(text));
        doc.change(changes, after);
        let again = next_diagnostics(&mut diagnostics, true).await;
        assert_eq!(again[0].message, stub::DIAGNOSTIC);

        // Format and rename hand their edits to the editor.
        let mut edits = doc.watch_edits();
        let formatted = doc.format().await.unwrap();
        assert_eq!(formatted.len(), 1);
        assert_eq!((formatted[0].from, formatted[0].to), (0, 0));
        assert_eq!(formatted[0].text, stub::FORMATTED);
        let batch = edits.next().await.unwrap();
        assert_eq!(batch.against.to_string(), format!("> {text}"));
        assert_eq!(
            batch.changes().apply(&batch.against).to_string(),
            format!("{}> {text}", stub::FORMATTED)
        );
        let renamed = doc.rename(0, "yo").await.unwrap();
        assert_eq!(renamed.here.len(), 1);
        assert_eq!(
            (
                renamed.here[0].from,
                renamed.here[0].to,
                renamed.here[0].text.as_str()
            ),
            (0, 2, "yo")
        );
        assert!(renamed.elsewhere.is_empty());
        assert!(edits.next().await.is_some(), "the rename's edits too");

        // Go to definition: the start of this document.
        let targets = doc.definition(4).await.unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].at, Some(0));

        // Found by path, and focused for the actions.
        let found = servers.document(&path).unwrap();
        servers.focus(&found);
        assert_eq!(servers.focused().unwrap().path(), path);
    });
    // Closed: its server goes with its last document.
    drop(doc);
    assert!(servers.status().is_empty());
}

fn one_server_per_language_and_root() {
    let rt = runtime();
    let servers = LanguageServers::with_restart(rt.handle().clone(), FAST);
    let one = tempfile::tempdir().unwrap();
    let two = tempfile::tempdir().unwrap();
    let language = stub_language("lab.stub", &[]);
    let a = servers
        .open(&language, one.path(), &one.path().join("a.stub"), "a")
        .unwrap();
    let b = servers
        .open(&language, one.path(), &one.path().join("b.stub"), "b")
        .unwrap();
    let c = servers
        .open(&language, two.path(), &two.path().join("c.stub"), "c")
        .unwrap();
    let status = servers.status();
    assert_eq!(status.len(), 2, "{status:?}");
    assert_eq!(status.iter().map(|s| s.documents).sum::<usize>(), 3);
    rt.block_on(async {
        until("both running", || {
            servers.status().iter().all(|s| s.state == State::Running)
        })
        .await;
    });
    drop(a);
    assert_eq!(servers.status().len(), 2, "b keeps the first root's server");
    drop(b);
    assert_eq!(servers.status().len(), 1);
    assert_eq!(servers.status()[0].root, two.path());
    drop(c);
    assert!(servers.status().is_empty());
}

fn a_server_that_dies_is_started_again() {
    let rt = runtime();
    let servers = LanguageServers::with_restart(rt.handle().clone(), FAST);
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("died");
    let language = stub_language("lab.stub", &["--die-once", marker.to_str().unwrap()]);
    let doc = servers
        .open(&language, root.path(), &root.path().join("a.stub"), "hello")
        .unwrap();
    rt.block_on(async {
        until("it died once", || marker.exists()).await;
        until("started again", || {
            let s = &servers.status()[0];
            s.restarts >= 1 && s.state == State::Running
        })
        .await;
        // Reopened with its text: the diagnostic and the answers again.
        let mut diagnostics = doc.watch_diagnostics();
        assert_eq!(next_diagnostics(&mut diagnostics, true).await.len(), 1);
        assert_eq!(doc.hover(1).await.unwrap().as_deref(), Some(stub::HOVER));
    });
}

fn its_owner_gone_the_server_stops() {
    let rt = runtime();
    let servers = LanguageServers::with_restart(rt.handle().clone(), FAST);
    let root = tempfile::tempdir().unwrap();
    let doc = servers
        .open(
            &stub_language("lab.stub", &[]),
            root.path(),
            &root.path().join("a.stub"),
            "x",
        )
        .unwrap();
    rt.block_on(async {
        let mut diagnostics = doc.watch_diagnostics();
        next_diagnostics(&mut diagnostics, true).await;
        assert_eq!(servers.stop_owner("someone.else"), 0);
        assert_eq!(servers.stop_owner("lab.stub"), 1);
        assert!(servers.status().is_empty());
        assert!(
            next_diagnostics(&mut diagnostics, false).await.is_empty(),
            "cleared"
        );
        until("stopped", || doc.server().state == State::Stopped).await;
        assert_eq!(doc.hover(0).await, Err(LspError::Stopped));
    });
    drop(doc);
}

fn a_server_that_cannot_start_fails() {
    let rt = runtime();
    let servers = LanguageServers::with_restart(rt.handle().clone(), FAST);
    let root = tempfile::tempdir().unwrap();
    let nowhere = Registered {
        owner: "lab.stub".to_owned(),
        language: Language::linked("ghost", "Ghost").server("/no/such/language-server", &[]),
    };
    let doc = servers
        .open(&nowhere, root.path(), &root.path().join("a.g"), "")
        .unwrap();
    rt.block_on(async {
        until("failed", || {
            matches!(servers.status()[0].state, State::Failed { .. })
        })
        .await;
        let status = servers.status();
        assert_eq!(status[0].restarts, FAST.attempts);
        let State::Failed { error } = &status[0].state else {
            unreachable!()
        };
        assert!(error.contains("could not start"), "{error}");
        assert!(matches!(doc.hover(0).await, Err(LspError::NotRunning(_))));
    });
}

fn a_language_without_a_server_has_none() {
    let rt = runtime();
    let servers = LanguageServers::new(rt.handle().clone());
    let plain = Registered {
        owner: "app".to_owned(),
        language: Language::linked("rust", "Rust"),
    };
    let root = tempfile::tempdir().unwrap();
    assert!(matches!(
        servers.open(&plain, root.path(), &root.path().join("a.rs"), ""),
        Err(LspError::NoServer(id)) if id == "rust"
    ));
    assert_eq!(Restart::default().delay(1), Duration::from_millis(250));
    assert_eq!(Restart::default().delay(3), Duration::from_secs(1));
    assert_eq!(Restart::default().delay(30), Duration::from_secs(10));
}
