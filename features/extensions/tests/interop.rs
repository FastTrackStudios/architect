//! Extensions with each other and with the app's windows
//! (`architect_extensions::interop`): the manifest's `[[windows]]`,
//! `[[points]]`, `[[contribute."<point>"]]`, `[provides]` / `[requires]`
//! and `reserved`, read and refused; a point's items checked against its
//! schema (a bad one a problem, not a failed load) and delivered to its
//! owner live as extensions load and unload; a service called across two
//! extensions (linked, Luau), with its provider missing or too old; a
//! window opened as a sheet and as an OS window.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Event, Host, LoadError, SharedSections};
use architect_extensions::hostcall::{HostError, HostRequest};
use architect_extensions::install::Installs;
use architect_extensions::interop::WindowKind;
use architect_extensions::linked::{Builder, Extension, Handle};
use architect_extensions::{Manifest, ManifestError, Origin, Package};
use architect_settings::Registry;
use architect_workspace::{Popout, WindowId, WindowSystem, Windows};
use dioxus::prelude::*;
use futures::executor::block_on;
use serde_json::{Value, json};

fn host(list: &ActionList) -> Host {
    Host::new(list.clone(), SharedSections::new(Registry::new()))
}

#[allow(clippy::needless_pass_by_value)] // json!(…) at every call.
fn run(list: &ActionList, id: &str, args: Value) -> Result<Value, String> {
    block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
}

fn until(what: &str, ok: impl Fn() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "waited for {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

// ---- the manifest -------------------------------------------------------

const NOTES: &str = r#"
id = "lab.notes"
name = "Notes"
version = "0.1.0"
kind = "process"
reserved = ["APP_NOTES"]

[entry]
path = "notes"

[[actions]]
id = "APP_NOTES"
name = "Notes…"

[[actions]]
id = "LAB_NOTES_CARDS"
name = "Cards delivered"

[[views]]
id = "board"
title = "Board"

[[windows]]
id = "notes"
view = "board"
title = "Notes"
kind = "window"
opens = "APP_NOTES"
width = 640

[[points]]
id = "lab.notes.cards"
deliver = "LAB_NOTES_CARDS"
schema = { type = "object", required = ["title"], properties = { title = { type = "string" }, n = { type = "integer" } } }

[[contribute."lab.notes.cards"]]
title = "Its own"

[provides]
"lab.notes" = "1.4.0"

[requires]
"lab.tempo" = "^1"
"#;

#[test]
fn a_manifest_reads_its_windows_points_and_services() {
    let m = Manifest::parse(NOTES).unwrap();
    assert_eq!(m.windows[0].kind, WindowKind::Window);
    assert_eq!(m.windows[0].width, Some(640.0));
    assert_eq!(m.window_opened_by("APP_NOTES").unwrap().id, "notes");
    assert!(m.host_runs(&m.actions[0]), "the host opens its window");
    assert_eq!(m.points[0].schema_json()["required"], json!(["title"]));
    assert_eq!(
        m.point_items()["lab.notes.cards"],
        vec![json!({"title": "Its own"})]
    );
    assert_eq!(m.provides["lab.notes"], "1.4.0");
    assert_eq!(m.requires["lab.tempo"], "^1");

    let refused = |from: &str, to: &str| -> String {
        let text = NOTES.replacen(from, to, 1);
        match Manifest::parse(&text) {
            Err(ManifestError::Interop { field, .. }) => field,
            other => panic!("{to}: not refused: {other:?}"),
        }
    };
    assert_eq!(
        refused(
            "view = \"board\"\ntitle = \"Notes\"",
            "view = \"nope\"\ntitle = \"Notes\""
        ),
        "windows[notes].view"
    );
    assert_eq!(
        refused("opens = \"APP_NOTES\"", "opens = \"LAB_NOTES_NOPE\""),
        "windows[notes].opens"
    );
    assert_eq!(
        refused("id = \"lab.notes.cards\"", "id = \"other.cards\""),
        "points[other.cards].id"
    );
    assert_eq!(
        refused("type = \"object\", required", "type = \"thing\", required"),
        "points[lab.notes.cards].schema"
    );
    assert_eq!(
        refused("\"lab.notes\" = \"1.4.0\"", "\"lab.notes\" = \"one\""),
        "provides.lab.notes"
    );
    assert_eq!(
        refused("\"lab.tempo\" = \"^1\"", "\"lab.tempo\" = \"most\""),
        "requires.lab.tempo"
    );
    assert_eq!(
        refused(
            "reserved = [\"APP_NOTES\"]",
            "reserved = [\"APP_NOTES\", \"LAB_X\"]"
        ),
        "reserved[LAB_X]"
    );
    // An app id it does not list as reserved is outside its namespace.
    assert!(matches!(
        Manifest::parse(&NOTES.replacen("reserved = [\"APP_NOTES\"]", "", 1)),
        Err(ManifestError::ActionNamespace { .. })
    ));
}

// ---- points -------------------------------------------------------------

/// What the owner was told, in order.
static TOLD: Mutex<Vec<Value>> = Mutex::new(Vec::new());

struct Owner;

const OWNER: &str = r#"
id = "lab.owner"
name = "Owner"
version = "0.1.0"
kind = "process"

[entry]
path = "owner"

[[actions]]
id = "LAB_OWNER_CARDS"
name = "Cards delivered"

[[points]]
id = "lab.owner.cards"
deliver = "LAB_OWNER_CARDS"
schema = { type = "object", required = ["title"], properties = { title = { type = "string" } } }
"#;

impl Extension for Owner {
    fn manifest(&self) -> &'static str {
        OWNER
    }

    fn build(&self, ext: &mut Builder) {
        ext.action("LAB_OWNER_CARDS", |delivery| async move {
            TOLD.lock().unwrap().push(delivery);
            Ok(Value::Null)
        });
    }
}

const GIVER: &str = r#"
id = "lab.giver"
name = "Giver"
version = "0.1.0"
kind = "data"

[[contribute."lab.owner.cards"]]
title = "Hello"

[[contribute."lab.owner.cards"]]
title = 7

[[contribute."lab.owner.cards"]]
title = "World"
"#;

/// The titles the owner was last told, once they are `want`.
fn told(want: &[&str]) -> Vec<String> {
    let titles = || -> Vec<String> {
        let newest = TOLD
            .lock()
            .unwrap()
            .iter()
            .max_by_key(|t| t["revision"].as_u64())
            .cloned();
        newest
            .map(|n| {
                n["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|i| i["value"]["title"].as_str().unwrap().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    };
    let start = Instant::now();
    while titles() != want && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    titles()
}

#[test]
fn a_points_items_are_checked_and_delivered_live() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let ext = Extensions::new(host(&list), Installs::new(tmp.path().join("i")))
        .with_linked(&Owner)
        .unwrap();
    ext.install_contribution_actions().unwrap();
    let _ = ext.sync();
    // Items given before their point's owner loads wait for it.
    let _ = ext.disable("lab.owner");
    let giver = tmp.path().join("giver");
    std::fs::create_dir_all(&giver).unwrap();
    std::fs::write(giver.join("extension.toml"), GIVER).unwrap();
    let _ = ext.dev(&giver);
    assert_eq!(
        ext.host().points().waiting()["lab.owner.cards"],
        [("lab.giver".to_owned(), 3)]
    );
    let _ = ext.enable("lab.owner");
    assert_eq!(told(&["Hello", "World"]), ["Hello", "World"]);
    // The one that does not meet the schema is left out, a problem; the
    // rest of the giver loaded.
    let problems = ext.problems();
    assert!(
        problems.iter().any(|p| p
            .message
            .contains("lab.giver: its item 1 for lab.owner.cards was left out")
            && p.message.contains("title: is integer, not string")),
        "{problems:?}"
    );
    assert!(ext.host().get("lab.giver").is_some());
    let points = run(&list, "EXT_POINTS", Value::Null).unwrap();
    // The app's own point (icons and badges, #306) sorts first; the
    // features' context-menu targets (`context.*`, #313) are points too.
    assert_eq!(points["points"][0]["id"], "app.decorations", "{points}");
    let owner = points["points"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "lab.owner.cards")
        .unwrap_or_else(|| panic!("{points}"));
    assert_eq!(owner["owner"], "lab.owner", "{points}");
    assert_eq!(points["rejected"][0]["index"], 1, "{points}");

    // The app's own come first.
    ext.host()
        .contribute_app("lab.owner.cards", vec![json!({"title": "App"})]);
    assert_eq!(told(&["App", "Hello", "World"]), ["App", "Hello", "World"]);
    // The giver goes, and its items with it.
    let _ = ext.end_dev(&giver);
    assert_eq!(told(&["App"]), ["App"]);
    assert!(ext.problems().is_empty(), "{:?}", ext.problems());
    // Its owner reads them too (the host service's `point`); another may not.
    let items = block_on(ext.host().api("lab.owner").call(HostRequest::Point {
        point: "lab.owner.cards".to_owned(),
    }))
    .unwrap();
    assert_eq!(items, json!([{"from": "app", "value": {"title": "App"}}]));
    assert!(matches!(
        block_on(ext.host().api("lab.other").call(HostRequest::Point {
            point: "lab.owner.cards".to_owned(),
        })),
        Err(HostError::NotLoaded { .. })
    ));
}

// ---- services -----------------------------------------------------------

const TEMPO: &str = r#"
id = "lab.tempo"
name = "Tempo"
version = "1.2.0"
kind = "process"

[entry]
path = "tempo"

[provides]
"lab.tempo" = "1.2.0"

[[actions]]
id = "LAB_TEMPO_GET"
name = "Tempo: get"
effect = "read_only"
params = '{"type":"object","properties":{"from":{"type":"string"}}}'

[[actions]]
id = "LAB_TEMPO_RESET"
name = "Tempo: reset"
effect = "destructive"
"#;

struct Tempo;

impl Extension for Tempo {
    fn manifest(&self) -> &'static str {
        TEMPO
    }

    fn build(&self, ext: &mut Builder) {
        ext.action("LAB_TEMPO_GET", |args| async move {
            Ok(json!({"bpm": 120, "asked": args}))
        })
        .action("LAB_TEMPO_RESET", |_| async move { Ok(Value::Null) });
    }
}

const CLICK: &str = r#"
id = "lab.click"
name = "Click"
version = "0.1.0"
kind = "process"

[entry]
path = "click"

[requires]
"lab.tempo" = "^1.1"

[capabilities]
effect = "mutating"

[[actions]]
id = "LAB_CLICK_BPM"
name = "Click: the tempo"
"#;

static CLICK_HANDLE: Mutex<Option<Handle>> = Mutex::new(None);

struct Click;

impl Extension for Click {
    fn manifest(&self) -> &'static str {
        CLICK
    }

    fn build(&self, ext: &mut Builder) {
        *CLICK_HANDLE.lock().unwrap() = Some(ext.handle());
        ext.action("LAB_CLICK_BPM", |_| async move {
            let handle = CLICK_HANDLE.lock().unwrap().clone().unwrap();
            handle
                .service("lab.tempo", "get", json!({"from": "click"}))
                .await
                .map_err(|e| e.to_string())
        });
    }
}

const LUAU_CLICK: &str = r#"
id = "lab.luau-click"
name = "Luau click"
version = "0.1.0"
kind = "luau"

[requires]
"lab.tempo" = "1"

[[actions]]
id = "LAB_LUAU_CLICK_BPM"
name = "Luau click: the tempo"
script = "bpm.luau"
"#;

const NEWER: &str = r#"
id = "lab.newer"
name = "Newer"
version = "0.1.0"
kind = "data"

[requires]
"lab.tempo" = "^2"
"#;

#[test]
fn a_service_is_called_across_extensions_and_needs_its_provider() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    // The requirer is registered first: the load order follows [requires].
    let ext = Extensions::new(host(&list), Installs::new(tmp.path().join("i")))
        .with_linked(&Click)
        .unwrap()
        .with_linked(&Tempo)
        .unwrap();
    ext.install_contribution_actions().unwrap();
    let events = ext.sync();
    assert!(
        !events.iter().any(|e| matches!(e, Event::Failed { .. })),
        "{events:?}"
    );
    // Linked: through its handle.
    assert_eq!(
        run(&list, "LAB_CLICK_BPM", Value::Null).unwrap(),
        json!({"bpm": 120, "asked": {"from": "click"}})
    );
    // Luau: through `host.service`.
    let dir = tmp.path().join("luau-click");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.toml"), LUAU_CLICK).unwrap();
    std::fs::write(
        dir.join("bpm.luau"),
        "local t = host.service(\"lab.tempo\", \"get\", { from = \"luau\" })\nreturn t.bpm",
    )
    .unwrap();
    let _ = ext.dev(&dir);
    assert_eq!(
        run(&list, "LAB_LUAU_CLICK_BPM", Value::Null).unwrap(),
        json!(120)
    );
    // A page (or anything that only runs actions) calls as its extension.
    let as_click = architect::action::ActionCaller::new(architect::action::Principal::extension(
        "lab.click",
        None,
    ));
    let out = block_on(list.run_json_async_with(
        &as_click,
        "EXT_SERVICE_CALL",
        br#"{"service":"lab.tempo","method":"get"}"#,
    ))
    .unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&out).unwrap()["bpm"], 120);
    assert!(
        run(
            &list,
            "EXT_SERVICE_CALL",
            json!({"service": "lab.tempo", "method": "get"})
        )
        .unwrap_err()
        .contains("only an extension"),
        "the app is no extension"
    );

    // Refused: past its effect, a service it does not require, a method
    // there is not.
    let api = ext.host().api("lab.click");
    let call = |service: &str, method: &str| {
        block_on(api.call(HostRequest::Service {
            service: service.to_owned(),
            method: method.to_owned(),
            args: Value::Null,
        }))
    };
    assert!(matches!(
        call("lab.tempo", "reset"),
        Err(HostError::Refused { capability, .. }) if capability == "effect"
    ));
    assert!(matches!(
        call("lab.metronome", "get"),
        Err(HostError::Refused { capability, .. }) if capability == "requires"
    ));
    assert!(matches!(
        call("lab.tempo", "nope"),
        Err(HostError::NotFound { .. })
    ));

    // Too new a requirement: not loaded, saying why.
    let newer = Package::embedded(NEWER, &[]).unwrap();
    match ext.host().load(&newer, Origin::Installed) {
        Err(LoadError::Requires { service, why, .. }) => {
            assert_eq!(service, "lab.tempo");
            assert_eq!(why, "the loaded provider is lab.tempo 1.2.0");
        }
        other => panic!("{other:?}"),
    }

    // The provider goes: what requires it goes too, a problem until it is
    // back.
    let _ = ext.disable("lab.tempo");
    assert!(ext.host().get("lab.click").is_none());
    let problems = ext.problems();
    assert!(
        problems.iter().any(|p| p.message.contains("lab.click")
            && p.message.contains("it requires the service lab.tempo ^1.1")),
        "{problems:?}"
    );
    let _ = ext.enable("lab.tempo");
    until("the requirer back", || {
        ext.host().get("lab.click").is_some()
    });
    assert_eq!(
        run(&list, "LAB_CLICK_BPM", Value::Null).unwrap()["bpm"],
        120
    );
}

const SOFT: &str = r#"
id = "lab.soft"
name = "Soft click"
version = "0.1.0"
kind = "process"

[entry]
path = "soft"

[uses]
"lab.tempo" = "^1"

[capabilities]
effect = "read_only"

[[actions]]
id = "LAB_SOFT_BPM"
name = "Soft click: the tempo, when there is one"
"#;

static SOFT_HANDLE: Mutex<Option<Handle>> = Mutex::new(None);

struct Soft;

impl Extension for Soft {
    fn manifest(&self) -> &'static str {
        SOFT
    }

    fn build(&self, ext: &mut Builder) {
        *SOFT_HANDLE.lock().unwrap() = Some(ext.handle());
        ext.action("LAB_SOFT_BPM", |_| async move {
            let handle = SOFT_HANDLE.lock().unwrap().clone().unwrap();
            match handle
                .service("lab.tempo", "get", json!({"from": "soft"}))
                .await
            {
                Ok(v) => Ok(v),
                Err(HostError::NotFound { what }) => Ok(json!({ "none": what })),
                Err(e) => Err(e.to_string()),
            }
        });
    }
}

#[test]
fn a_service_it_uses_is_called_when_there_and_missed_when_not() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    // No provider: it loads all the same ([uses] is no [requires]).
    let ext = Extensions::new(host(&list), Installs::new(tmp.path().join("i")))
        .with_linked(&Soft)
        .unwrap();
    let events = ext.sync();
    assert!(
        !events.iter().any(|e| matches!(e, Event::Failed { .. })),
        "{events:?}"
    );
    assert_eq!(
        run(&list, "LAB_SOFT_BPM", Value::Null).unwrap(),
        json!({ "none": "provider of lab.tempo ^1" })
    );
    // The provider comes: its calls are answered.
    let ext = ext.with_linked(&Tempo).unwrap();
    let _ = ext.sync();
    assert_eq!(run(&list, "LAB_SOFT_BPM", Value::Null).unwrap()["bpm"], 120);
    // It goes: the user stays, and misses it again.
    let _ = ext.disable("lab.tempo");
    assert!(ext.host().get("lab.soft").is_some(), "not unloaded with it");
    assert!(
        run(&list, "LAB_SOFT_BPM", Value::Null)
            .unwrap()
            .get("none")
            .is_some()
    );
    // Its effect is still checked: reset is past what it may run.
    let _ = ext.enable("lab.tempo");
    let refused = block_on(ext.host().api("lab.soft").call(HostRequest::Service {
        service: "lab.tempo".to_owned(),
        method: "reset".to_owned(),
        args: Value::Null,
    }));
    assert!(
        matches!(refused, Err(HostError::Refused { capability, .. }) if capability == "effect")
    );
    // One table or the other, not both.
    let both = SOFT.replace("[uses]", "[requires]\n\"lab.tempo\" = \"^1\"\n\n[uses]");
    assert!(
        Manifest::parse(&both)
            .unwrap_err()
            .to_string()
            .contains("uses.lab.tempo")
    );
}

// ---- project types (#344) -----------------------------------------------

const KINDS: &str = r#"
id = "lab.kinds"
name = "Kinds"
version = "0.1.0"
kind = "data"

[[views]]
id = "page"
title = "Song"

[[project_types]]
id = "song"
name = "Song"
icon = "music"
properties = [{ key = "key", name = "Key", kind = "text", default = '"C"' }]
template = "templates/song.md"
folders = ["Charts", "Stems"]
views = ["page"]
"#;

#[test]
fn project_types_are_registered_while_loaded() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let ext = Extensions::new(host(&list), Installs::new(tmp.path().join("i")));
    ext.install_contribution_actions().unwrap();
    let dir = tmp.path().join("kinds");
    std::fs::create_dir_all(dir.join("templates")).unwrap();
    std::fs::write(dir.join("extension.toml"), KINDS).unwrap();
    std::fs::write(dir.join("templates/song.md"), "# {{title}}\n\n## Lyric\n").unwrap();
    let _ = ext.dev(&dir);
    let types = run(&list, "EXT_PROJECT_TYPES", Value::Null).unwrap();
    assert_eq!(types[0]["id"], "song");
    assert_eq!(types[0]["from"], "lab.kinds");
    assert_eq!(types[0]["folders"], json!(["Charts", "Stems"]));
    assert_eq!(
        types[0]["template_text"], "# {{title}}\n\n## Lyric\n",
        "read from its package when it loaded"
    );
    assert_eq!(
        run(&list, "EXT_PROJECT_TYPES", json!({ "id": "song" })).unwrap()["name"],
        "Song"
    );
    assert!(
        run(&list, "EXT_PROJECT_TYPES", json!({ "id": "nope" }))
            .unwrap()
            .is_null()
    );
    // Off: its types go with it.
    let _ = ext.disable("lab.kinds");
    assert_eq!(
        run(&list, "EXT_PROJECT_TYPES", Value::Null).unwrap(),
        json!([])
    );
    // Checked at parse: an id it can write, a template in its package, its
    // own views.
    for (bad, why) in [
        ("id = \"song\"", "id = \"Song Type\""),
        (
            "template = \"templates/song.md\"",
            "template = \"../song.md\"",
        ),
        ("views = [\"page\"]", "views = [\"nope\"]"),
    ] {
        let text = KINDS.replacen(bad, why, 2);
        assert!(Manifest::parse(&text).is_err(), "{why}");
    }
}

// ---- capture targets (#352) ---------------------------------------------

const IDEAS: &str = r#"
id = "lab.ideas"
name = "Ideas"
version = "0.1.0"
kind = "data"

[[capture_targets]]
id = "song-idea"
name = "Song idea"
icon = "music"
action = "LAB_IDEAS_CAPTURE"
chips = ["key", "tags"]
tokens = [{ prefix = "key:", key = "key" }]
"#;

#[test]
fn capture_targets_are_registered_while_loaded() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let ext = Extensions::new(host(&list), Installs::new(tmp.path().join("i")));
    ext.install_contribution_actions().unwrap();
    let dir = tmp.path().join("ideas");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.toml"), IDEAS).unwrap();
    let _ = ext.dev(&dir);
    let targets = run(&list, "EXT_CAPTURE_TARGETS", Value::Null).unwrap();
    assert_eq!(targets[0]["id"], "song-idea");
    assert_eq!(targets[0]["from"], "lab.ideas");
    assert_eq!(targets[0]["action"], "LAB_IDEAS_CAPTURE");
    assert_eq!(
        targets[0]["tokens"],
        json!([{ "prefix": "key:", "key": "key" }])
    );
    // Off: its targets go with it.
    let _ = ext.disable("lab.ideas");
    assert_eq!(
        run(&list, "EXT_CAPTURE_TARGETS", Value::Null).unwrap(),
        json!([])
    );
    // Checked at parse: an id it can have, the app's own ids left alone,
    // its own action, whole tokens.
    for (bad, why) in [
        ("id = \"song-idea\"", "id = \"Song Idea\""),
        ("id = \"song-idea\"", "id = \"note\""),
        (
            "action = \"LAB_IDEAS_CAPTURE\"",
            "action = \"VAULT_DELETE\"",
        ),
        ("prefix = \"key:\"", "prefix = \"\""),
    ] {
        let text = IDEAS.replacen(bad, why, 1);
        assert!(Manifest::parse(&text).is_err(), "{why}");
    }
}

// ---- windows ------------------------------------------------------------

thread_local! {
    static DRAWN: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    static OPENED: RefCell<Vec<Popout>> = const { RefCell::new(Vec::new()) };
    static CLOSED: RefCell<Vec<WindowId>> = const { RefCell::new(Vec::new()) };
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

struct Platform;

impl WindowSystem for Platform {
    fn open(&self, popout: Popout) {
        OPENED.with(|o| o.borrow_mut().push(popout));
    }
    fn close(&self, window: &WindowId) {
        CLOSED.with(|c| c.borrow_mut().push(window.clone()));
    }
    fn origin(&self, _: &WindowId) -> Option<(f32, f32)> {
        None
    }
    fn window_at(&self, _: f32, _: f32) -> Option<WindowId> {
        None
    }
    fn on_release_outside(&self, _: &WindowId, _: Rc<dyn Fn(f32, f32)>) {}
}

const PANELS: &str = r#"
id = "lab.panels"
name = "Panels"
version = "0.1.0"
kind = "process"

[entry]
path = "panels"

[[actions]]
id = "LAB_PANELS_PREFS"
name = "Preferences…"

[[actions]]
id = "LAB_PANELS_MIXER"
name = "Mixer"

[[views]]
id = "prefs"
title = "Preferences"

[[views]]
id = "mixer"
title = "Mixer"

[[windows]]
id = "prefs"
view = "prefs"
title = "Preferences"
kind = "sheet"
opens = "LAB_PANELS_PREFS"

[[windows]]
id = "mixer"
view = "mixer"
title = "Mixer"
kind = "window"
opens = "LAB_PANELS_MIXER"
"#;

#[component]
fn Prefs() -> Element {
    DRAWN.with(|d| d.borrow_mut().push("prefs"));
    rsx! { "the preferences" }
}

#[component]
fn Mixer() -> Element {
    DRAWN.with(|d| d.borrow_mut().push("mixer"));
    rsx! { "the mixer" }
}

struct Panels;

impl Extension for Panels {
    fn manifest(&self) -> &'static str {
        PANELS
    }

    fn build(&self, ext: &mut Builder) {
        ext.view("prefs", Prefs).view("mixer", Mixer);
    }
}

fn window_app() -> Element {
    use_context_provider(|| Windows::new(Platform));
    let host = HOST.with(|h| h.borrow().clone().unwrap());
    let ui = architect_extensions::interface::use_extension_ui(host, Arc::new(ActionList::new()));
    rsx! { architect_extensions::interface::ExtensionOverlays { ui } }
}

/// Lets the window's futures run, then draws.
fn settle(dom: &mut VirtualDom) {
    for _ in 0..5 {
        block_on(async {
            let wait = dom.wait_for_work();
            let timeout = futures_timer::Delay::new(Duration::from_millis(50));
            futures::pin_mut!(wait);
            let _ = futures::future::select(wait, timeout).await;
        });
        dom.render_immediate(&mut dioxus::core::NoOpMutations);
    }
}

#[test]
fn a_window_opens_as_a_sheet_and_as_an_os_window() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let ext = Extensions::new(host(&list), Installs::new(tmp.path().join("i")))
        .with_linked(&Panels)
        .unwrap();
    ext.install_contribution_actions().unwrap();
    let _ = ext.sync();
    HOST.with(|h| *h.borrow_mut() = Some(ext.host().clone()));
    let mut main = VirtualDom::new(window_app);
    main.rebuild_in_place();
    assert!(DRAWN.with(|d| d.borrow().is_empty()), "nothing open yet");

    // A sheet: drawn over the main window, in its document.
    run(&list, "LAB_PANELS_PREFS", json!({"tab": "audio"})).unwrap();
    settle(&mut main);
    assert_eq!(DRAWN.with(|d| d.borrow().clone()), ["prefs"]);
    let windows = run(&list, "EXT_WINDOWS", Value::Null).unwrap();
    let prefs = windows["windows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == "prefs")
        .unwrap();
    assert_eq!(prefs["open"], true, "{windows}");
    assert_eq!(prefs["kind"], "sheet");
    assert_eq!(prefs["state"], json!({"tab": "audio"}));
    assert!(ext.host().close_window("lab.panels", "prefs"));

    // An OS window: the platform is asked for one, whose own document
    // draws the view; the person closing it closes it in the host.
    DRAWN.with(|d| d.borrow_mut().clear());
    run(&list, "LAB_PANELS_MIXER", Value::Null).unwrap();
    settle(&mut main);
    assert!(
        DRAWN.with(|d| d.borrow().is_empty()),
        "not in the main window"
    );
    let popout = OPENED
        .with(|o| o.borrow().first().cloned())
        .expect("an OS window");
    assert_eq!(popout.title(), "Mixer");
    assert_eq!(popout.id().0, "ext-lab.panels-mixer");
    let mut window = popout.dom();
    window.rebuild_in_place();
    assert_eq!(DRAWN.with(|d| d.borrow().clone()), ["mixer"]);
    window.in_runtime(|| popout.closed());
    drop(window);
    assert!(ext.host().open_windows().is_empty());
    settle(&mut main);

    // Closed by the host: its OS window closes.
    run(&list, "LAB_PANELS_MIXER", Value::Null).unwrap();
    settle(&mut main);
    assert_eq!(OPENED.with(|o| o.borrow().len()), 2);
    let _ = run(
        &list,
        "EXT_WINDOW_CLOSE",
        json!({"extension": "lab.panels", "id": "mixer"}),
    );
    settle(&mut main);
    assert!(
        CLOSED.with(|c| c.borrow().iter().any(|w| w.0 == "ext-lab.panels-mixer")),
        "its OS window closed"
    );
    // Unloaded: its windows close.
    run(&list, "LAB_PANELS_PREFS", Value::Null).unwrap();
    assert!(ext.host().unload("lab.panels"));
    assert!(ext.host().open_windows().is_empty());
}
