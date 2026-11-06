//! One trait, two ways to run it: the same `CommandPalette` linked into a
//! host and run as its own process (a development copy of its id, which
//! replaces the built-in live) draws both overlays of the palette slot,
//! binds ⌘K and ⌘O, opens them through `COMMANDS_SHOW` and ranks the
//! app's commands through the engine (`palette.search`). Turned off, its
//! keys and views go with it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_pass_by_value
)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use architect::action::{ActionList, ActionMeta, Effect};
use architect_command_palette::{
    CommandPalette, ID, OPEN_ACTION, OPEN_ANYTHING_ACTION, SEARCH_ACTION, SHOW_ACTION,
};
use architect_extensions::Origin;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_extensions::process::ProcessRunner;
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

/// `COMMANDS_SHOW`'s arguments (`architect_commands::ShowCommands`).
#[derive(facet::Facet)]
struct Show {
    overlay: String,
    #[facet(default)]
    query: Option<String>,
}

fn meta(id: &'static str) -> &'static ActionMeta {
    Box::leak(Box::new(ActionMeta {
        id,
        trait_name: "",
        method_name: "",
        display_name: id,
        description: "",
        category: "Commands",
        group: "Commands",
        shortcut: "",
        toggleable: false,
        undo: false,
    }))
}

/// `COMMANDS_LIST`'s answer (`architect_commands::CommandList`).
#[derive(facet::Facet, Clone)]
struct Listed {
    commands: Vec<Info>,
}

#[derive(facet::Facet, Clone)]
struct Info {
    id: String,
    label: String,
    group: String,
    effect: String,
    enabled: bool,
    disabled_reason: Option<String>,
}

fn info(id: &str, label: &str, group: &str, enabled: bool) -> Info {
    Info {
        id: id.to_owned(),
        label: label.to_owned(),
        group: group.to_owned(),
        effect: "mutating".to_owned(),
        enabled,
        disabled_reason: (!enabled).then(|| "not now".to_owned()),
    }
}

/// The app: a `COMMANDS_SHOW` that keeps what it was asked, and a
/// `COMMANDS_LIST` of three commands.
fn new_app() -> (ActionList, Arc<Mutex<Vec<Value>>>) {
    let list = ActionList::new();
    let shown = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&shown);
    list.register(meta("COMMANDS_SHOW"), Effect::Mutating, move |ask: Show| {
        kept.lock()
            .unwrap()
            .push(json!({"overlay": ask.overlay, "query": ask.query}));
        Ok(())
    })
    .unwrap();
    let commands = Listed {
        commands: vec![
            info("WORKSPACE_CLOSE_TAB", "Close tab", "Tabs", true),
            info("WORKSPACE_SPLIT_RIGHT", "Split right", "Panes", true),
            info("TOURS_START", "Start a tour", "Help", false),
        ],
    };
    list.register(meta("COMMANDS_LIST"), Effect::ReadOnly, move |(): ()| {
        Ok(commands.clone())
    })
    .unwrap();
    (list, shown)
}

fn run(list: &ActionList, id: &str, args: Value) -> Value {
    let out = block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .unwrap_or_else(|e| panic!("{id}: {e}"));
    serde_json::from_slice(&out).unwrap_or(Value::Null)
}

/// Runs the palette through `ext`: its slot and keys, ⌘K and ⌘O, a
/// search. What the search found.
fn exercise(
    ext: &Extensions,
    list: &ActionList,
    shown: &Mutex<Vec<Value>>,
    origin: Origin,
) -> Value {
    assert_eq!(ext.host().get(ID).unwrap().origin, origin);
    let reg = ext.host().contributions();
    let views: Vec<(String, String, String)> = reg
        .palette_views()
        .into_iter()
        .map(|by| (by.owner, by.item.overlay, by.item.view))
        .collect();
    assert_eq!(
        views,
        [
            (ID.to_owned(), "palette".to_owned(), "palette".to_owned()),
            (
                ID.to_owned(),
                "quick-open".to_owned(),
                "quick-open".to_owned()
            ),
        ]
    );
    let keys: Vec<(String, String)> = reg
        .key_bindings()
        .into_iter()
        .map(|by| (by.item.chord, by.item.action))
        .collect();
    for (chord, action) in [
        ("⌘K", OPEN_ACTION),
        ("⌃K", OPEN_ACTION),
        ("⌘O", OPEN_ANYTHING_ACTION),
    ] {
        assert!(
            keys.contains(&(chord.to_owned(), action.to_owned())),
            "{chord} → {action}: {keys:?}"
        );
    }
    shown.lock().unwrap().clear();
    let _ = run(list, OPEN_ACTION, Value::Null);
    let _ = run(list, OPEN_ANYTHING_ACTION, Value::Null);
    let _ = run(
        list,
        SHOW_ACTION,
        json!({"overlay": "quick-open", "query": "theme:"}),
    );
    assert_eq!(
        *shown.lock().unwrap(),
        [
            json!({"overlay": "palette", "query": null}),
            json!({"overlay": "quick-open", "query": null}),
            json!({"overlay": "quick-open", "query": "theme:"}),
        ]
    );
    let found = run(list, SEARCH_ACTION, json!({"query": "spl"}));
    assert_eq!(found["items"][0]["id"], "WORKSPACE_SPLIT_RIGHT", "{found}");
    let all = run(list, SEARCH_ACTION, json!({"query": "", "limit": 2}));
    assert_eq!(all["items"].as_array().unwrap().len(), 2, "{all}");
    // A disabled one comes last.
    let t = run(list, SEARCH_ACTION, json!({"query": "t"}));
    assert_eq!(
        t["items"].as_array().unwrap().last().unwrap()["id"],
        "TOURS_START"
    );
    found
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
fn one_palette_two_ways() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, shown) = new_app();
    let list = ActionList::new();

    // Linked in.
    let ext = Extensions::new(host(&list, &app, None), Installs::new(tmp.path().join("a")))
        .with_linked(&CommandPalette)
        .unwrap();
    let _ = ext.sync();
    let linked = exercise(&ext, &list, &shown, Origin::Builtin);
    let _ = ext.disable(ID);
    let reg = ext.host().contributions();
    assert!(reg.of(ID).is_none(), "its slot and keys went with it");
    assert!(reg.palette_views().is_empty() && reg.key_bindings().is_empty());
    assert!(list.info(OPEN_ACTION).is_none(), "and ⌘K's action");

    // As a process: the development copy of its id replaces it.
    let dev = tmp.path().join("palette");
    std::fs::create_dir_all(&dev).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("extension.toml"),
        dev.join("extension.toml"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        env!("CARGO_BIN_EXE_architect-command-palette"),
        dev.join("architect-command-palette"),
    )
    .unwrap();
    let (app, shown) = new_app();
    let list = ActionList::new();
    let runner = ProcessRunner::new().unwrap().with_socket_dir(tmp.path());
    let ext = Extensions::new(
        host(&list, &app, Some(runner)),
        Installs::new(tmp.path().join("b")),
    )
    .with_linked(&CommandPalette)
    .unwrap();
    let _ = ext.sync();
    let _ = ext.dev(&dev);
    let process = exercise(&ext, &list, &shown, Origin::Dev);
    assert_eq!(linked, process, "the same ranking both ways");
    let _ = ext.end_dev(&dev);
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Builtin);
}

#[test]
fn the_manifest_is_checked_and_says_what_it_owns() {
    let m = architect_extensions::Manifest::parse(architect_command_palette::MANIFEST).unwrap();
    assert_eq!(m.id, ID);
    assert_eq!(m.contributes.palette_views.len(), 2);
    assert_eq!(m.contributes.key_bindings.len(), 6);
    assert!(m.provides.contains_key("architect.palette"));
}
