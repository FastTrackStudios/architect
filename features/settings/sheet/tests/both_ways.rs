//! One trait, two ways to run it: the same `SettingsSheet` linked into a
//! host and run as its own process (a development copy of its id, which
//! overrides the built-in live) owns the window `APP_SETTINGS` opens and
//! is told the same sections: the app's own and another extension's
//! `[[sections]]` (sugar for its point, `settings.sections`), live as that
//! extension loads and unloads.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, LoadError, SharedSections};
use architect_extensions::install::Installs;
use architect_extensions::process::ProcessRunner;
use architect_extensions::{Origin, Package};
use architect_settings::Registry;
use architect_settings_sheet::{ID, SettingsSheet, WINDOW};
use futures::executor::block_on;
use serde_json::{Value, json};

const METRONOME: &str = r#"
id = "fts.metronome"
name = "Metronome"
version = "0.1.0"
kind = "data"

[[sections]]
id = "metronome"
title = "Metronome"
icon = "timer"

[[sections.settings]]
key = "fts.metronome.accent"
title = "Accent the downbeat"
control = "switch"
default = true
"#;

#[allow(clippy::needless_pass_by_value)] // json!(…) at every call.
fn run(list: &ActionList, id: &str, args: Value) -> Value {
    let out = block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .unwrap_or_else(|e| panic!("{id}: {e}"));
    serde_json::from_slice(&out).unwrap_or(Value::Null)
}

/// The section ids `SETTINGS_LIST` says, once they are `want`.
fn listed(list: &ActionList, want: &[&str]) -> Vec<String> {
    let start = Instant::now();
    loop {
        let out = run(list, "SETTINGS_LIST", Value::Null);
        let ids: Vec<String> = out["sections"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap().to_owned())
            .collect();
        if ids == want || start.elapsed() > Duration::from_secs(10) {
            return ids;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn host(list: &ActionList, runner: Option<ProcessRunner>) -> Host {
    let mut host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    if let Some(r) = runner {
        host = host.with_runner(r);
    }
    host.allow_reserved(ID, &["APP_SETTINGS"]);
    // The app's own section, as the app gives it.
    host.contribute_app(
        "settings.sections",
        vec![json!({"id": "workspace", "title": "Workspace", "group": ""})],
    );
    host
}

/// What the sheet is told and owns, whichever way it runs.
fn exercise(ext: &Extensions, list: &ActionList, origin: Origin) {
    assert_eq!(ext.host().get(ID).unwrap().origin, origin);
    assert_eq!(listed(list, &["workspace"]), ["workspace"]);
    // Another extension's [[sections]] arrive live, and go with it.
    let metronome = Package::embedded(METRONOME, &[]).unwrap();
    let _ = ext.host().load(&metronome, Origin::Installed).unwrap();
    assert_eq!(
        listed(list, &["workspace", "metronome"]),
        ["workspace", "metronome"]
    );
    ext.host().unload("fts.metronome");
    assert_eq!(listed(list, &["workspace"]), ["workspace"]);
    // ⌘, opens its window, the host's handler; its state is the window's.
    let opened = run(list, "APP_SETTINGS", json!({"section": "workspace"}));
    assert_eq!(opened, json!({"open": "settings/settings"}));
    let open = ext.host().open_windows();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].window.id, WINDOW);
    assert_eq!(open[0].state, json!({"section": "workspace"}));
    assert!(ext.host().close_window(ID, WINDOW));
}

#[test]
fn one_sheet_two_ways() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let ext = Extensions::new(host(&list, None), Installs::new(tmp.path().join("a")))
        .with_linked(&SettingsSheet)
        .unwrap();
    let _ = ext.sync();
    exercise(&ext, &list, Origin::Builtin);

    // As a process: the development copy of its id overrides it.
    let dev = tmp.path().join("sheet");
    std::fs::create_dir_all(&dev).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("extension.toml"),
        dev.join("extension.toml"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        env!("CARGO_BIN_EXE_architect-settings-sheet"),
        dev.join("architect-settings-sheet"),
    )
    .unwrap();
    let list = ActionList::new();
    let runner = ProcessRunner::new().unwrap().with_socket_dir(tmp.path());
    let ext = Extensions::new(
        host(&list, Some(runner)),
        Installs::new(tmp.path().join("b")),
    )
    .with_linked(&SettingsSheet)
    .unwrap();
    let _ = ext.sync();
    let _ = ext.dev(&dev);
    exercise(&ext, &list, Origin::Dev);
    let _ = ext.end_dev(&dev);
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Builtin);
}

/// The app's id is the app's: without its leave, or as an installed copy,
/// the sheet does not load.
#[test]
fn app_settings_is_the_apps_to_give() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list, SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.path().join("a")))
        .with_linked(&SettingsSheet)
        .unwrap();
    let _ = ext.sync();
    assert!(ext.host().get(ID).is_none());
    let package = Package::embedded(architect_settings_sheet::MANIFEST, &[]).unwrap();
    assert_eq!(
        ext.host().load(&package, Origin::Builtin).unwrap_err(),
        LoadError::Reserved("APP_SETTINGS".to_owned())
    );
    ext.host().allow_reserved(ID, &["APP_SETTINGS"]);
    assert_eq!(
        ext.host().load(&package, Origin::Installed).unwrap_err(),
        LoadError::Reserved("APP_SETTINGS".to_owned()),
        "an installed copy never owns it"
    );
}
