//! One trait, two ways to run it: the same `ThemePicker` linked into a
//! host and run as its own process (a development copy of its id, which
//! overrides the built-in live) lists the same themes, picks one through
//! the host service, flips light and dark, and opens ⌘O on `theme:`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use architect::action::{ActionList, ActionMeta, Effect};
use architect_extensions::Origin;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_extensions::process::ProcessRunner;
use architect_settings::Registry;
use architect_theme::ThemeStore;
use architect_theme_picker::{ID, ThemePicker};
use futures::executor::block_on;
use serde_json::{Value, json};

/// `COMMANDS_SHOW`'s arguments (`architect_commands::ShowCommands`).
#[derive(facet::Facet)]
struct Show {
    overlay: String,
    #[facet(default)]
    query: Option<String>,
}

static SHOW: ActionMeta = ActionMeta {
    id: "COMMANDS_SHOW",
    trait_name: "",
    method_name: "",
    display_name: "Show the palette",
    description: "",
    category: "Commands",
    group: "Commands",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// The app: a theme store's `THEME_*`, and a `COMMANDS_SHOW` that keeps
/// what it was asked.
fn new_app() -> (ActionList, ThemeStore, Arc<Mutex<Vec<String>>>) {
    let list = ActionList::new();
    let store = ThemeStore::default();
    architect_theme::actions::register(&list, &store).unwrap();
    let shown = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&shown);
    list.register(&SHOW, Effect::Mutating, move |ask: Show| {
        kept.lock()
            .unwrap()
            .push(json!({"overlay": ask.overlay, "query": ask.query}).to_string());
        Ok(())
    })
    .unwrap();
    (list, store, shown)
}

fn run(list: &ActionList, id: &str, args: Value) -> Value {
    let out = block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .unwrap_or_else(|e| panic!("{id}: {e}"));
    serde_json::from_slice(&out).unwrap_or(Value::Null)
}

/// Runs the picker through `ext`: what it finds, a pick, light or dark,
/// ⌘O. What it found for "light".
fn exercise(ext: &Extensions, list: &ActionList, store: &ThemeStore, origin: Origin) -> Value {
    assert_eq!(ext.host().get(ID).unwrap().origin, origin);
    let reg = ext.host().contributions();
    let opener = reg
        .quick_open()
        .into_iter()
        .find(|o| o.owner == ID)
        .unwrap();
    assert_eq!(
        (opener.item.kind.as_str(), opener.item.open.as_str()),
        ("theme", "THEME_PICKER_PICK")
    );
    let light = run(
        list,
        "THEME_PICKER_FIND",
        json!({"query": "catppuccin light"}),
    );
    let items = light["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{light}");
    let id = items[0]["id"].as_str().unwrap().to_owned();
    let _ = run(list, "THEME_PICKER_PICK", json!({"id": id}));
    assert_eq!(store.current().id(), id, "picked, and light shows");
    let _ = run(list, "THEME_PICKER_SCHEME", Value::Null);
    assert!(store.current().scheme().is_dark(), "dark now");
    let _ = run(list, "THEME_PICKER_SCHEME", Value::Null);
    assert_eq!(store.current().id(), id, "and back");
    light
}

fn host(list: &ActionList, app: &ActionList, runner: Option<ProcessRunner>) -> Host {
    let mut host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    if let Some(r) = runner {
        host = host.with_runner(r);
    }
    host.set_app_list(app.clone());
    host
}

#[test]
fn one_picker_two_ways() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, store, shown) = new_app();
    let list = ActionList::new();

    // Linked in.
    let ext = Extensions::new(host(&list, &app, None), Installs::new(tmp.path().join("a")))
        .with_linked(&ThemePicker)
        .unwrap();
    let _ = ext.sync();
    let linked = exercise(&ext, &list, &store, Origin::Builtin);
    let _ = run(&list, "THEME_PICKER_CHOOSE", Value::Null);
    let asked: Value = serde_json::from_str(&shown.lock().unwrap()[0]).unwrap();
    assert_eq!(asked, json!({"overlay": "quick-open", "query": "theme:"}));
    let _ = ext.disable(ID);
    assert!(
        ext.host().contributions().of(ID).is_none(),
        "its opener went with it"
    );

    // As a process: the development copy of its id overrides it.
    let dev = tmp.path().join("picker");
    std::fs::create_dir_all(&dev).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("extension.toml"),
        dev.join("extension.toml"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        env!("CARGO_BIN_EXE_architect-theme-picker"),
        dev.join("architect-theme-picker"),
    )
    .unwrap();
    // A fresh app, as the linked one's picks are in its store.
    let (app, store, shown) = new_app();
    let list = ActionList::new();
    let runner = ProcessRunner::new().unwrap().with_socket_dir(tmp.path());
    let ext = Extensions::new(
        host(&list, &app, Some(runner)),
        Installs::new(tmp.path().join("b")),
    )
    .with_linked(&ThemePicker)
    .unwrap();
    let _ = ext.sync();
    let _ = ext.dev(&dev);
    let process = exercise(&ext, &list, &store, Origin::Dev);
    assert_eq!(linked, process, "the same themes both ways");
    let _ = run(&list, "THEME_PICKER_CHOOSE", Value::Null);
    assert_eq!(
        shown.lock().unwrap().len(),
        1,
        "⌘O asked for from its process"
    );
    let _ = ext.end_dev(&dev);
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Builtin);
}
