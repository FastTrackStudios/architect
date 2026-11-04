//! The host service (#178) and consent, on the lab's metronome: through
//! `host` in its Luau script it runs an app action and reads its own
//! setting, as `Principal::Extension` (which a gate sees, for whoever it
//! acts for); a call outside its capabilities is refused and in its log;
//! installed from a registry, it runs with what a person granted — asked
//! on install, an agent told to ask (never left waiting), revoked at once.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect, Principal};
use architect_extensions::consent::Grants;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::hostapi::{AppContext, MemoryValues, SettingValues};
use architect_extensions::hostcall::{HostError, HostRequest};
use architect_extensions::install::{FolderRegistry, Installs};
use architect_permissions::{Action, Decision, PermissionEngine, Resource};
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

const ID: &str = "fts.metronome";
const STATUS: &str = "FTS_METRONOME_STATUS";

fn lab_registry() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/lab/extensions")
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

/// A gate that lets everyone run and remembers who asked.
#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Vec<(Principal, String)>>>);

impl PermissionEngine for Seen {
    fn check(&self, _: &Principal, _: &Resource, _: &Action) -> Decision {
        Decision::Allow
    }

    fn authorize(&self, who: &Principal, id: &str) -> Decision {
        self.0.lock().unwrap().push((who.clone(), id.to_owned()));
        Decision::Allow
    }
}

/// What the app knows: a selection, no organization.
struct Lab;

impl AppContext for Lab {
    fn selection(&self) -> Option<Value> {
        Some(json!({ "pane": "note-1", "view": "note" }))
    }
}

static THEME_SET: ActionMeta = ActionMeta {
    id: "THEME_SET",
    trait_name: "",
    method_name: "THEME_SET",
    display_name: "Set the theme",
    description: "Sets the theme.",
    category: "Theme",
    group: "Theme",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// The app: `APP_STATUS`, a `THEME_SET` the metronome may not run, a
/// gate, a selection, settings in memory (its volume at 55).
fn app(tmp: &Path) -> (ActionList, Seen, Host) {
    let list = ActionList::new();
    architect::action::app_status::register(&list).unwrap();
    list.register_with(&THEME_SET, Effect::Mutating, |_: &ActionCaller, (): ()| {
        Ok::<_, String>(())
    })
    .unwrap();
    let seen = Seen::default();
    list.set_authorizer(seen.clone());
    let values = MemoryValues::default();
    values.set("fts.metronome.volume", json!(55)).unwrap();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_app_context(Lab)
        .with_setting_values(values)
        .with_data_root(tmp.join("data"));
    (list, seen, host)
}

fn run(
    list: &ActionList,
    caller: &ActionCaller,
    id: &str,
    args: &Value,
) -> Result<Value, ActionError> {
    block_on(list.run_json_async_with(caller, id, args.to_string().as_bytes()))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
}

fn bob() -> Principal {
    Principal::User {
        user_id: "bob".to_owned(),
    }
}

#[test]
fn the_metronome_runs_an_app_action_and_reads_its_setting_through_the_host() {
    let tmp = tempfile::tempdir().unwrap();
    let (list, seen, host) = app(tmp.path());
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")));
    ext.install_actions().unwrap();
    let dev = tmp.path().join("dev/metronome");
    copy(&lab_registry().join("metronome"), &dev);
    let _ = ext.dev(&dev);

    let out = run(&list, &ActionCaller::app(), STATUS, &Value::Null).unwrap();
    assert_eq!(out["volume"], 55, "its setting, as the app keeps it: {out}");
    assert_eq!(out["sound"], "wood", "its default: {out}");
    assert_eq!(out["selection"]["pane"], "note-1", "{out}");
    assert_eq!(out["problems"], 0, "APP_STATUS ran: {out}");

    // The gate saw the extension, acting for the app itself.
    let calls = seen.0.lock().unwrap().clone();
    assert!(
        calls.contains(&(Principal::extension(ID, None), "APP_STATUS".to_owned())),
        "{calls:?}"
    );
    // Run by someone, it acts for them: the gate can tell both apart.
    run(&list, &ActionCaller::new(bob()), STATUS, &Value::Null).unwrap();
    let calls = seen.0.lock().unwrap().clone();
    assert!(
        calls.contains(&(
            Principal::extension(ID, Some(bob())),
            "APP_STATUS".to_owned()
        )),
        "{calls:?}"
    );
    assert!(calls.contains(&(bob(), STATUS.to_owned())), "{calls:?}");

    // Its data is kept for it; its print is in its log.
    let data = std::fs::read_to_string(tmp.path().join("data/fts.metronome/data.json")).unwrap();
    assert!(data.contains("\"last_volume\": 55"), "{data}");
    let logs = run(
        &list,
        &ActionCaller::app(),
        "EXT_LOGS",
        &json!({ "id": ID }),
    )
    .unwrap();
    let lines = logs["lines"].as_array().unwrap();
    assert!(
        lines
            .iter()
            .any(|l| l["source"] == "log" && l["text"].as_str().unwrap().contains("volume 55")),
        "{logs}"
    );
}

#[test]
fn a_call_outside_its_capabilities_is_refused_and_logged() {
    let tmp = tempfile::tempdir().unwrap();
    let (list, _, host) = app(tmp.path());
    let ext = Extensions::new(host.clone(), Installs::new(tmp.path().join("installs")));
    ext.install_actions().unwrap();
    let dev = tmp.path().join("dev/metronome");
    copy(&lab_registry().join("metronome"), &dev);
    let _ = ext.dev(&dev);
    let api = host.api(ID);
    let refused = |r: HostRequest| match block_on(api.call(r)) {
        Err(HostError::Refused { capability, .. }) => capability,
        other => panic!("not refused: {other:?}"),
    };
    assert_eq!(
        refused(HostRequest::Run {
            action: "THEME_SET".to_owned(),
            args: Value::Null,
            ticket: None,
        }),
        "actions"
    );
    assert_eq!(refused(HostRequest::Org), "reads.org");
    assert_eq!(
        refused(HostRequest::ReadFile {
            path: "/etc/hostname".to_owned()
        }),
        "folders"
    );
    assert_eq!(
        refused(HostRequest::Setting {
            key: "appearance.theme".to_owned()
        }),
        "settings"
    );
    // Its own folder needs nothing.
    block_on(api.call(HostRequest::WriteFile {
        path: "notes/today.txt".to_owned(),
        text: "four on the floor".to_owned(),
    }))
    .unwrap();
    let read = block_on(api.call(HostRequest::ReadFile {
        path: "notes/today.txt".to_owned(),
    }))
    .unwrap();
    assert_eq!(read, "four on the floor");

    // The same through EXT_HOST_CALL, as an agent tries it: an answer,
    // not a failure.
    let out = run(
        &list,
        &ActionCaller::app(),
        "EXT_HOST_CALL",
        &json!({ "id": ID, "request": { "op": "run", "action": "THEME_SET" } }),
    )
    .unwrap();
    assert_eq!(out["error"]["kind"], "refused", "{out}");
    assert_eq!(out["error"]["capability"], "actions", "{out}");

    let logs = run(
        &list,
        &ActionCaller::app(),
        "EXT_LOGS",
        &json!({ "id": ID }),
    )
    .unwrap();
    let refusals: Vec<&str> = logs["lines"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|l| l["source"] == "host" && l["level"] == "warn")
        .map(|l| l["text"].as_str().unwrap())
        .collect();
    assert!(
        refusals
            .iter()
            .any(|t| t.contains("run: refused (actions)") && t.contains("THEME_SET")),
        "{refusals:?}"
    );
}

#[test]
fn installed_it_runs_with_what_a_person_granted() {
    let tmp = tempfile::tempdir().unwrap();
    let (list, _, host) = app(tmp.path());
    let grants = Grants::open(tmp.path().join("grants.json"));
    let registry = tmp.path().join("registry");
    copy(&lab_registry(), &registry);
    let ext = Extensions::new(
        host.with_grants(grants),
        Installs::new(tmp.path().join("installs")),
    )
    .with_source(FolderRegistry::new(&registry));
    ext.install_actions().unwrap();
    list.require_confirmation(Duration::from_secs(60));
    let app = ActionCaller::app();

    // What it would be, before anything is installed.
    let preview = run(&list, &app, "EXT_PREVIEW", &json!({ "id": ID })).unwrap();
    assert_eq!(preview["kind"], "luau", "{preview}");
    assert!(
        preview["runner"].as_str().unwrap().contains("Luau"),
        "{preview}"
    );
    let asks: Vec<&str> = preview["needs_consent"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["kind"].as_str().unwrap())
        .collect();
    assert_eq!(asks, vec!["actions", "effect", "reads"], "{preview}");
    assert!(
        preview["adds"].to_string().contains("Metronome: status"),
        "{preview}"
    );

    // Installing asks first.
    let asked = run(&list, &app, "EXT_INSTALL", &json!({ "id": ID })).unwrap();
    assert_eq!(asked["needs_consent"], true, "{asked}");
    assert!(
        ext.installs().get(ID).is_none(),
        "not installed without a yes"
    );

    // An agent granting it is told to ask the person, at once.
    let agent = ActionCaller::new(bob());
    match run(
        &list,
        &agent,
        "EXT_INSTALL",
        &json!({ "id": ID, "grant": "device" }),
    ) {
        Err(ActionError::ConfirmationRequired { ticket, .. }) => assert!(!ticket.is_empty()),
        other => panic!("not asked: {other:?}"),
    }
    match run(&list, &agent, "EXT_GRANT", &json!({ "id": ID })) {
        Err(ActionError::ConfirmationRequired { .. }) => {}
        other => panic!("not asked: {other:?}"),
    }

    // The person says yes.
    let done = run(
        &list,
        &app,
        "EXT_INSTALL",
        &json!({ "id": ID, "grant": "device" }),
    )
    .unwrap();
    assert_eq!(done["loaded"], true, "{done}");
    assert_eq!(done["granted"], "device", "{done}");
    let out = run(&list, &app, STATUS, &Value::Null).unwrap();
    assert_eq!(out["volume"], 55, "{out}");

    // Taken back: refused at once.
    let revoked = run(&list, &app, "EXT_REVOKE", &json!({ "id": ID })).unwrap();
    assert_eq!(revoked["revoked"], 1, "{revoked}");
    let err = run(&list, &app, STATUS, &Value::Null)
        .unwrap_err()
        .to_string();
    assert!(err.contains("refused (reads.selection)"), "{err}");
    let grants_now = run(&list, &app, "EXT_GRANTS", &json!({ "id": ID })).unwrap();
    assert_eq!(
        grants_now["extensions"][0]["not_granted"]
            .as_array()
            .unwrap()
            .len(),
        3,
        "{grants_now}"
    );

    // Granted again from its page (EXT_GRANT by the person).
    run(&list, &app, "EXT_GRANT", &json!({ "id": ID })).unwrap();
    run(&list, &app, STATUS, &Value::Null).unwrap();

    // Nothing newer: the update skips it. Pinned, it says so.
    let updated = run(&list, &app, "EXT_UPDATE", &json!({})).unwrap();
    assert_eq!(updated["unchanged"][0]["id"], ID, "{updated}");
    let pinned = run(&list, &app, "EXT_PIN", &json!({ "id": ID })).unwrap();
    assert_eq!(pinned["pin"], "=0.1.0", "{pinned}");
    let info = run(&list, &app, "EXT_INFO", &json!({ "id": ID })).unwrap();
    assert_eq!(info["installed"]["pin"], "=0.1.0", "{info}");
    assert_eq!(info["runs"], "0.1.0, installed", "{info}");
    assert_eq!(info["grants"][0]["scope"], "device", "{info}");
    list.validate().unwrap();
}

/// The extensions page's list: one row an extension (here, or offered by a
/// registry), found by name, kind and what it adds; a development copy
/// overriding the built-in one says so, and ending it is the way back.
#[test]
fn the_page_finds_extensions_by_name_kind_and_what_they_add() {
    use architect_extensions::browse::{Browse, Where};
    let tmp = tempfile::tempdir().unwrap();
    let (_, _, host) = app(tmp.path());
    let builtin = architect_extensions::Package::embedded(
        include_str!("../../../apps/lab/extensions/metronome/extension.toml"),
        &[
            (
                "count_in.luau",
                include_str!("../../../apps/lab/extensions/metronome/count_in.luau"),
            ),
            (
                "status.luau",
                include_str!("../../../apps/lab/extensions/metronome/status.luau"),
            ),
        ],
    )
    .unwrap();
    let registry = tmp.path().join("registry");
    copy(&lab_registry(), &registry);
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_builtin(builtin)
        .with_source(FolderRegistry::new(&registry));
    let _ = ext.sync();
    let browse = Browse::new(ext.clone());

    let all = browse.rows("");
    assert_eq!(
        all.len(),
        1,
        "one row, though a registry lists it too: {all:?}"
    );
    assert_eq!(all[0].at, Where::Builtin);
    assert_eq!(all[0].registry.as_deref(), Some("Architect Lab"));
    for query in ["metro", "kind:luau", "adds:status", "count in", "built in"] {
        assert_eq!(browse.rows(query).len(), 1, "{query}");
    }
    for query in ["kind:process", "adds:tab", "saturation"] {
        assert!(browse.rows(query).is_empty(), "{query}");
    }

    // A development copy of its id, newer: it overrides the built-in.
    let dev = tmp.path().join("dev/metronome");
    copy(&lab_registry().join("metronome"), &dev);
    let manifest = dev.join("extension.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replace("version = \"0.1.0\"", "version = \"1.4.0-dev\""),
    )
    .unwrap();
    let _ = ext.dev(&dev);
    let row = &browse.rows("metronome")[0];
    assert_eq!(row.at, Where::Dev);
    assert!(row.overrides_builtin);
    assert_eq!(
        row.runs.as_deref(),
        Some("built in (0.1.0), overridden by 1.4.0-dev (in development)")
    );
    let _ = ext.end_dev(&dev);
    assert_eq!(
        browse.rows("metronome")[0].at,
        Where::Builtin,
        "the way back"
    );
}

/// The extensions manager: extensions, themes and layouts in one list,
/// searched together; each kind's ops as `MANAGER_RUN`; the manager itself
/// a linked-in extension with its own actions.
#[test]
fn the_manager_lists_and_acts_on_every_kind() {
    use architect_extensions::browse::Browse;
    use architect_extensions::manager::extension::ManagerExtension;
    use architect_extensions::manager::{Manager, kinds};
    let tmp = tempfile::tempdir().unwrap();
    let (list, _, host) = app(tmp.path());
    let manager = Manager::new();
    let registry = tmp.path().join("registry");
    copy(&lab_registry(), &registry);
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_source(FolderRegistry::new(&registry))
        .with_linked(&ManagerExtension::new(manager.clone()))
        .unwrap();
    ext.install_actions().unwrap();
    let _ = ext.sync();
    let home_dir = tmp.path().join("home");
    let home = architect_workspace::home::Home::open(&home_dir, &[("main", "{}")]);
    std::fs::write(home_dir.join("layouts/broken.json"), "{ \"chrome\": ").unwrap();
    let _ = home.poll();
    manager.register(kinds::ExtensionKind::new(Browse::new(ext.clone())));
    manager.register(kinds::ScriptKind::new(ext.clone()));
    manager.register(kinds::ThemeKind::new(
        architect_theme::ThemeStore::default(),
        list.clone(),
    ));
    manager.register(kinds::LayoutKind::new(home.clone()));
    let app = ActionCaller::app();

    // The manager is an extension, built in.
    assert_eq!(
        ext.host().get("manager").unwrap().origin,
        architect_extensions::Origin::Builtin
    );
    let kinds_now = run(&list, &app, "MANAGER_KINDS", &json!({})).unwrap();
    let names: Vec<&str> = kinds_now["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["kind"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["extension", "script", "theme", "layout"]);

    // One search across kinds.
    let found = run(
        &list,
        &app,
        "MANAGER_LIST",
        &json!({ "query": "metronome" }),
    )
    .unwrap();
    assert_eq!(found["items"][0]["kind"], "extension", "{found}");
    let themes = run(&list, &app, "MANAGER_LIST", &json!({ "kind": "theme" })).unwrap();
    assert!(!themes["items"].as_array().unwrap().is_empty(), "{themes}");
    let layouts = run(&list, &app, "MANAGER_LIST", &json!({ "kind": "layout" })).unwrap();
    let broken = layouts["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == "layout:broken")
        .unwrap();
    assert!(
        !broken["problems"].as_array().unwrap().is_empty(),
        "{layouts}"
    );

    // Ops: a built-in layout cloned to edit; the broken file removed.
    run(
        &list,
        &app,
        "MANAGER_RUN",
        &json!({ "kind": "layout", "id": "layout:main", "op": "clone" }),
    )
    .unwrap();
    assert!(home_dir.join("layouts/main-copy.json").is_file());
    run(
        &list,
        &app,
        "MANAGER_RUN",
        &json!({ "kind": "layout", "id": "layout:broken", "op": "remove" }),
    )
    .unwrap();
    assert!(home.problems().is_empty());
    // An agent removing something is asked first.
    manager.confirm_on(&list);
    list.require_confirmation(Duration::from_secs(60));
    match run(
        &list,
        &ActionCaller::new(bob()),
        "MANAGER_RUN",
        &json!({ "kind": "layout", "id": "layout:main-copy", "op": "remove" }),
    ) {
        Err(ActionError::ConfirmationRequired { .. }) => {}
        other => panic!("not asked: {other:?}"),
    }
    // Installing through the manager still asks for consent where it is kept
    // (here it is not: it installs).
    let out = run(
        &list,
        &app,
        "MANAGER_RUN",
        &json!({ "kind": "extension", "id": ID, "op": "install" }),
    )
    .unwrap();
    assert_eq!(out["loaded"], true, "{out}");
    let got = run(
        &list,
        &app,
        "MANAGER_GET",
        &json!({ "kind": "script", "id": "fts.metronome/status.luau" }),
    )
    .unwrap();
    assert!(
        got["parts"][0]["lines"]
            .to_string()
            .contains("host.setting"),
        "{got}"
    );
}
