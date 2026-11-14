//! The extension showcase's interface examples (#710, `examples/showcase`,
//! `docs/content/guide/showcase.md`), each loaded from its own folder as a
//! development copy, as `just showcase` loads it:
//!
//! - the Toolbar sampler (C2, C3): its toolbars where they say, a toggle
//!   whose tick and status item follow its state, a phone bar button,
//!   context rows on a note and a tab, each run told where it came from,
//!   and a menu at a point;
//! - Settings and theme (C5, C13, H4): its section on the settings page,
//!   the values it reads, a theme and tokens in the theme store, and a
//!   count in its data that a new host (a restart) still has;
//! - Iconize-lite (C9): icons and colours through the navigator's own
//!   actions, a badge on a file, and both taken off again.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use architect::action::{
    ActionCaller, ActionError, ActionList, ActionMeta, Effect, Invocation, InvocationSource,
    RawJson,
};
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::hostapi::{MemoryValues, SettingValues};
use architect_extensions::hostcall::Target;
use architect_extensions::hostui::FakeUi;
use architect_extensions::toolbar::ToolbarPlace;
use architect_extensions::{Origin, Package};
use architect_settings::Registry;
use architect_theme::ThemeStore;
use facet::Facet;
use futures::executor::block_on;
use serde_json::{Value, json};

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/showcase")
        .join(name)
}

fn run(list: &ActionList, id: &str, args: &Value) -> Result<Value, ActionError> {
    run_as(list, &ActionCaller::app(), id, args)
}

fn run_as(
    list: &ActionList,
    caller: &ActionCaller,
    id: &str,
    args: &Value,
) -> Result<Value, ActionError> {
    block_on(list.run_json_async_with(caller, id, args.to_string().as_bytes()))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
}

fn load(host: &Host, name: &str) {
    let package = Package::read(&example(name)).unwrap();
    host.load(&package, Origin::Dev).unwrap();
}

// ---- the Toolbar sampler ------------------------------------------------

#[test]
fn the_toolbar_samplers_toggle_toolbars_status_and_context_rows() {
    let list = ActionList::new();
    let ui = FakeUi::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new())).with_ui(ui.clone());
    load(&host, "toolbar-sampler");
    let reg = host.contributions().clone();
    let now = architect_extensions::host::now_ms;

    // C2: a toolbar docked on the right, one on every note's header.
    let bars = reg.toolbars();
    let docked = bars.iter().find(|b| b.item.id == "showcase").unwrap();
    assert_eq!(
        docked.item.place(),
        Some(ToolbarPlace::Dock("right".to_owned()))
    );
    assert_eq!(docked.item.items.len(), 2);
    let on_notes: Vec<String> = reg
        .header_toolbars("note")
        .into_iter()
        .map(|b| b.item.id)
        .collect();
    assert_eq!(on_notes, ["note"]);

    // The toggle: off, then on from its button (the tick and the status
    // item follow), then off again.
    let tick = |l: &ActionList| l.info("SHOWCASE_TOOLBAR_FOCUS").unwrap().checked;
    assert_eq!(tick(&list), Some(false));
    let status = |r: &architect_extensions::contrib::Contributions| {
        r.status(now())
            .into_iter()
            .find(|s| s.item.id == "focus")
            .unwrap()
            .item
            .text
    };
    assert_eq!(status(&reg), "Focus off");
    let button = ActionCaller::app().with_invocation(
        Invocation::new(InvocationSource::Toolbar).on("dock:right", "showcase.toolbar/showcase"),
    );
    let out = run_as(&list, &button, "SHOWCASE_TOOLBAR_FOCUS", &Value::Null).unwrap();
    assert_eq!(out["on"], true);
    assert_eq!(tick(&list), Some(true));
    assert_eq!(status(&reg), "Focus on");
    run_as(&list, &button, "SHOWCASE_TOOLBAR_FOCUS", &Value::Null).unwrap();
    assert_eq!(tick(&list), Some(false));
    assert_eq!(status(&reg), "Focus off");

    // The phone bar's button.
    assert!(
        reg.bar()
            .iter()
            .any(|b| b.item.action == "SHOWCASE_TOOLBAR_FOCUS"),
        "a bar button"
    );

    // C3: a row in a Markdown file's menu and a tab's, none in another
    // file's; each run is told where it came from.
    let file = reg.context("file", "Samples/Tables and lists.md");
    assert_eq!(file.len(), 1);
    assert_eq!(file[0].item.title, "Showcase: where is this note");
    assert!(reg.context("file", "Attachments/a.png").is_empty());
    assert_eq!(reg.context("tab", "vault-1").len(), 1);
    let menu = ActionCaller::app()
        .with_invocation(Invocation::new(InvocationSource::ContextMenu).on("tab", "vault-1"));
    let told = run_as(&list, &menu, "SHOWCASE_TOOLBAR_WHERE", &Value::Null).unwrap();
    assert_eq!(told["source"], "context_menu");
    assert_eq!(told["id"], "vault-1");
    assert!(
        ui.notices()
            .iter()
            .any(|n| n.text == "Run from context_menu on vault-1")
    );

    // And a menu at a point.
    let chosen = run(&list, "SHOWCASE_TOOLBAR_MENU", &json!({ "x": 10, "y": 10 })).unwrap();
    assert_eq!(chosen, "bold");
}

#[test]
fn the_toolbar_samplers_focus_mode_comes_back_after_a_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let host_on = |list: &ActionList| {
        let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
            .with_ui(FakeUi::new())
            .with_data_root(tmp.path().join("data"));
        load(&host, "toolbar-sampler");
        host
    };
    let list = ActionList::new();
    let host = host_on(&list);
    run(&list, "SHOWCASE_TOOLBAR_FOCUS", &Value::Null).unwrap();
    drop(host);

    // A new host on the same data (a restart): its startup hook puts the
    // tick and the status item back.
    let list = ActionList::new();
    let host = host_on(&list);
    // Hooks run off the loader's thread: give it a moment.
    let started = std::time::Instant::now();
    while list.info("SHOWCASE_TOOLBAR_FOCUS").unwrap().checked != Some(true) {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(3),
            "the tick did not come back"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let status = host
        .contributions()
        .status(architect_extensions::host::now_ms())
        .into_iter()
        .find(|s| s.item.id == "focus")
        .unwrap();
    assert_eq!(status.item.text, "Focus on");
}

// ---- Settings and theme -------------------------------------------------

/// The settings page's values: a greeting, said loudly.
fn values() -> MemoryValues {
    let values = MemoryValues::default();
    values
        .set("showcase.dusk.greeting", json!("Hi there"))
        .unwrap();
    values.set("showcase.dusk.loud", json!(true)).unwrap();
    values
}

#[test]
fn settings_and_theme_adds_a_section_a_theme_and_tokens_and_keeps_its_data() {
    let tmp = tempfile::tempdir().unwrap();
    let store = ThemeStore::default();
    let list = ActionList::new();
    architect_theme::actions::register(&list, &store).unwrap();
    let ui = FakeUi::new();
    let sections = SharedSections::new(Registry::new());
    let host = Host::new(list.clone(), sections.clone())
        .with_themes(store.clone())
        .with_setting_values(values())
        .with_data_root(tmp.path().join("data"))
        .with_ui(ui.clone());
    load(&host, "settings-theme");

    let registry = sections.registry();
    // C5: its section (under its id), with its three settings.
    let section = registry.section("showcase.dusk").expect("its section");
    assert_eq!(section.settings().count(), 3);

    // C13: its theme is in the picker's list, as the extension's.
    let out = block_on(list.run_json_async("THEME_LIST", b"")).unwrap();
    let listed: Value = serde_json::from_slice(&out).unwrap();
    let dusk = listed["themes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "showcase-dusk")
        .expect("Showcase Dusk");
    assert_eq!(dusk["source"], "extension:showcase.dusk");
    assert_eq!(dusk["appearance"], "dark");
    // Its tokens resolve in whatever theme shows, and a theme may set
    // them: Showcase Dusk's own `dusk.card` wins over the extension's
    // default while it shows.
    assert!(store.current().get("dusk.radius").is_some());
    let default_card = store.current().get("dusk.card").map(ToString::to_string);
    assert!(default_card.is_some());
    block_on(list.run_json_async("THEME_SET", br#"{"one":"showcase-dusk","layer":"pick"}"#))
        .unwrap();
    let dusk = store.current();
    assert_eq!(
        dusk.get("background").map(ToString::to_string).as_deref(),
        Some("#14111f")
    );
    let dusk_card = dusk.get("dusk.card").map(ToString::to_string);
    assert!(dusk_card.is_some());
    assert_ne!(
        dusk_card, default_card,
        "the theme's override, not the default"
    );

    // It reads the settings page's values.
    let out = run(&list, "SHOWCASE_DUSK_GREET", &Value::Null).unwrap();
    assert_eq!(out, json!({ "text": "HI THERE", "times": 1 }));
    assert_eq!(ui.notices()[0].text, "HI THERE");
    run(&list, "SHOWCASE_DUSK_GREET", &Value::Null).unwrap();

    // H4: a new host on the same folders (a restart) keeps its count.
    drop(host);
    let list = ActionList::new();
    let again = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_setting_values(values())
        .with_data_root(tmp.path().join("data"))
        .with_ui(FakeUi::new());
    load(&again, "settings-theme");
    let out = run(&list, "SHOWCASE_DUSK_GREET", &Value::Null).unwrap();
    assert_eq!(out["times"], 3, "{out}");
}

// ---- Iconize-lite -------------------------------------------------------

#[derive(Facet)]
struct MetaArgs {
    #[facet(default)]
    target: Option<String>,
    #[facet(default)]
    folder: Option<String>,
    #[facet(default)]
    icon: Option<String>,
    #[facet(default)]
    colour: Option<String>,
}

const fn meta(id: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "",
        method_name: id,
        display_name: id,
        description: "",
        category: "Navigator",
        group: "Navigator",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

static ICON: ActionMeta = meta("NAVIGATOR_ICON");
static COLOUR: ActionMeta = meta("NAVIGATOR_COLOUR");

/// What the navigator's actions were asked: (action, target, value).
type Asked = Arc<Mutex<Vec<(String, String, String)>>>;

fn navigator(list: &ActionList) -> Asked {
    let asked: Asked = Arc::default();
    for (meta, id) in [(&ICON, "NAVIGATOR_ICON"), (&COLOUR, "NAVIGATOR_COLOUR")] {
        let seen = Arc::clone(&asked);
        list.register_with(
            meta,
            Effect::Mutating,
            move |_: &ActionCaller, a: MetaArgs| {
                let target = a
                    .target
                    .or_else(|| a.folder.map(|f| format!("folder:{f}")))
                    .unwrap_or_default();
                let value = a.icon.or(a.colour).unwrap_or_default();
                seen.lock().unwrap().push((id.to_owned(), target, value));
                Ok::<_, String>(RawJson::from("{}".to_owned()))
            },
        )
        .unwrap();
    }
    asked
}

#[test]
fn iconize_lite_colours_and_badges_through_the_navigators_actions() {
    let list = ActionList::new();
    let asked = navigator(&list);
    let host = Host::new(list.clone(), SharedSections::new(Registry::new())).with_ui(FakeUi::new());
    load(&host, "iconize-lite");

    let out = run(&list, "SHOWCASE_ICONIZE_DECORATE", &Value::Null).unwrap();
    assert_eq!(out.as_array().unwrap().len(), 6);
    let calls = asked.lock().unwrap().clone();
    assert!(calls.contains(&(
        "NAVIGATOR_ICON".to_owned(),
        "folder:@me/C Contributions".to_owned(),
        "puzzle".to_owned()
    )));
    assert!(calls.contains(&(
        "NAVIGATOR_COLOUR".to_owned(),
        "folder:@me/H Host services".to_owned(),
        "#22c55e".to_owned()
    )));
    assert_eq!(calls.len(), 12);
    let badge = host.decorations_of(&Target::file("Showcase.md"));
    assert_eq!(badge[0].from, "showcase.iconize");
    assert_eq!(
        badge[0].decoration.badge.as_deref(),
        Some("6"),
        "the folders it decorated"
    );

    // Taken off again: empty icons and colours, no badge.
    run(
        &list,
        "SHOWCASE_ICONIZE_DECORATE",
        &json!({ "clear": true }),
    )
    .unwrap();
    let calls = asked.lock().unwrap().clone();
    assert!(
        calls[12..].iter().all(|(_, _, v)| v.is_empty()),
        "{calls:?}"
    );
    assert!(host.decorations_of(&Target::file("Showcase.md")).is_empty());
}

#[test]
fn each_example_says_what_it_stands_in_for_and_asks_for_little() {
    for name in ["toolbar-sampler", "settings-theme", "iconize-lite"] {
        let m = Package::read(&example(name)).unwrap().manifest;
        assert!(m.description.contains("Stands in for"), "{name}");
        assert!(m.capabilities.hosts.is_empty(), "{name}: no network");
        assert!(m.capabilities.writes.is_empty(), "{name}: no documents");
    }
}
