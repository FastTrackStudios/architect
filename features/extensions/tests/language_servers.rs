//! Language servers through the host (#512): an extension's
//! `[languages.server]` runs only with `capabilities.languages =
//! ["servers"]`; a code document opened through the host gets the
//! server's diagnostic, and the `EDITOR_*` actions reach it (hover,
//! completion, definition, format, rename); the extension turned off, its
//! server stops. The server is the editor's stub, run by this binary
//! with `--stub-lsp`, so CI needs no real one.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_editor::Languages;
use architect_editor::languages::builtin;
use architect_editor::lsp::{LanguageServers, stub};
use architect_extensions::Manifest;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

const NAME: &str = "an_extensions_language_server_runs_through_the_host";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(at) = args.iter().position(|a| a == "--stub-lsp") {
        stub::serve(args.get(at.saturating_add(1)..).unwrap_or_default());
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
    an_extensions_language_server_runs_through_the_host();
    println!("test {NAME} ... ok");
}

fn run(list: &ActionList, id: &str, args: &Value) -> Result<Value, String> {
    block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
}

fn until(what: &str, ok: impl Fn() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "waited for {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn manifest(capability: bool) -> String {
    let exe = std::env::current_exe().unwrap().display().to_string();
    let caps = if capability {
        "[capabilities]\nlanguages = [\"servers\"]\n"
    } else {
        ""
    };
    format!(
        r#"id = "lab.langs"
name = "Languages"
version = "0.1.0"
kind = "data"
{caps}
[[languages]]
id = "stubby"
name = "Stub"
extensions = ["stub"]
[languages.server]
command = "{exe}"
args = ["--stub-lsp"]
"#
    )
}

fn an_extensions_language_server_runs_through_the_host() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let servers = LanguageServers::new(runtime.handle().clone());
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_languages(Languages::with_app(builtin()))
        .with_language_servers(servers.clone());
    host.install_editor_actions().unwrap();
    let ext = Extensions::new(host, Installs::new(tmp.path().join("i")));
    ext.install_contribution_actions().unwrap();

    // A value it cannot ask for is refused when the manifest is read.
    let bogus = manifest(true).replace("[\"servers\"]", "[\"bogus\"]");
    let why = Manifest::parse(&bogus).unwrap_err().to_string();
    assert!(why.contains("languages"), "{why}");

    let dir = tmp.path().join("langs");
    std::fs::create_dir_all(&dir).unwrap();
    let file = tmp.path().join("work/a.stub");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let root = file.parent().unwrap();

    // Without `languages = ["servers"]`: its language is there, its server
    // does not run.
    std::fs::write(dir.join("extension.toml"), manifest(false)).unwrap();
    let _ = ext.dev(&dir);
    assert!(ext.host().languages().get("stubby").is_some());
    let refused = ext.host().open_code(&file, root, "hi").err().unwrap();
    assert!(refused.contains("capabilities.languages"), "{refused}");
    assert!(servers.status().is_empty());

    // With it: the server runs for the document.
    std::fs::write(dir.join("extension.toml"), manifest(true)).unwrap();
    let _ = ext.dev(&dir);
    let doc = ext.host().open_code(&file, root, "hi there\n").unwrap();
    doc.focus();
    let path = file.to_str().unwrap();
    until("its diagnostic", || !doc.diagnostics().is_empty());
    let servers_now = run(&list, "EDITOR_SERVERS", &json!({})).unwrap();
    assert_eq!(servers_now[0]["language"], "stubby");
    assert_eq!(servers_now[0]["owner"], "lab.langs");
    assert_eq!(servers_now[0]["state"], "running");
    assert_eq!(servers_now[0]["documents"], 1);
    let diagnostics = run(&list, "EDITOR_DIAGNOSTICS", &json!({ "path": path })).unwrap();
    assert_eq!(diagnostics["diagnostics"][0]["message"], stub::DIAGNOSTIC);
    assert_eq!(diagnostics["diagnostics"][0]["severity"], "warning");

    // The focused document at its caret, or a path and a point.
    doc.set_caret(3);
    let hover = run(&list, "EDITOR_HOVER", &json!({})).unwrap();
    assert_eq!(
        (hover["at"].as_u64(), hover["hover"].as_str()),
        (Some(3), Some(stub::HOVER))
    );
    let complete = run(&list, "EDITOR_COMPLETE", &json!({ "path": path, "at": 2 })).unwrap();
    assert_eq!(complete["items"][0]["label"], stub::COMPLETION);
    let definition = run(&list, "EDITOR_DEFINITION", &json!({ "at": 4 })).unwrap();
    assert_eq!(definition["targets"][0]["at"], 0);
    let format = run(&list, "EDITOR_FORMAT", &json!({})).unwrap();
    assert_eq!(format["edits"][0]["text"], stub::FORMATTED);
    let rename = run(&list, "EDITOR_RENAME", &json!({ "name": "yo", "at": 0 })).unwrap();
    assert_eq!(rename["renamed"]["here"][0]["text"], "yo");
    assert!(run(&list, "EDITOR_RENAME", &json!({ "name": " " })).is_err());
    assert!(
        run(&list, "EDITOR_HOVER", &json!({ "path": "/not/open.stub" }))
            .unwrap_err()
            .contains("not open")
    );

    // Turned off: its server stops; the document gets no answers.
    let _ = ext.disable("lab.langs");
    assert!(
        run(&list, "EDITOR_SERVERS", &json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    until("no diagnostics", || doc.diagnostics().is_empty());
    let after = run(&list, "EDITOR_HOVER", &json!({})).unwrap_err();
    assert!(after.contains("stopped"), "{after}");
    drop(doc);
}
