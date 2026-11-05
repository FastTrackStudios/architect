//! The issue's bar: the lab's metronome (`apps/lab/extensions`) is a real
//! extension, installed from a registry — here a git repository made of
//! the lab's registry folder — loaded without a restart, reloaded when its
//! files are saved, its settings coming and going with it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use architect::action::{ActionList, ActionMeta, Effect};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Event, Host, LoadError, SharedSections};
use architect_extensions::install::{FolderRegistry, GitRegistry, Installs};
use architect_extensions::{Origin, Package, Policy, Requirement, Requirements};
use architect_settings::{Registry, Section};
use futures::executor::block_on;
use serde_json::{Value, json};

const ID: &str = "fts.metronome";
const COUNT_IN: &str = "FTS_METRONOME_COUNT_IN";

/// The lab's registry folder.
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

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=lab", "-c", "user.email=lab@example.com"])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

/// A git repository of the lab's registry.
fn git_registry(tmp: &Path) -> PathBuf {
    let repo = tmp.join("registry");
    copy(&lab_registry(), &repo);
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["add", "."]);
    git(
        &repo,
        &[
            "commit",
            "--quiet",
            "--no-verify",
            "-m",
            "the lab's registry",
        ],
    );
    repo
}

/// The app: one section of its own, an empty list.
fn app() -> (ActionList, SharedSections) {
    let mut registry = Registry::new();
    registry
        .register(Section::new("appearance", "Appearance"))
        .unwrap();
    (ActionList::new(), SharedSections::new(registry))
}

fn metronome_keys(sections: &SharedSections) -> Vec<String> {
    sections
        .registry()
        .section("metronome")
        .map(|s| s.settings().map(|t| t.key.clone()).collect())
        .unwrap_or_default()
}

fn run(list: &ActionList, id: &str, args: &Value) -> Result<Value, String> {
    block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
}

#[test]
fn the_metronome_installs_from_a_git_registry_reloads_on_save_and_goes() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = git_registry(tmp.path());
    let (list, sections) = app();
    let host = Host::new(list.clone(), sections.clone());
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installed"))).with_source(
        GitRegistry::new(repo.display().to_string(), tmp.path().join("checkout")),
    );
    ext.install_actions().unwrap();
    assert!(ext.refresh().is_empty(), "the registry is cloned");

    // Found in the registry; not loaded.
    let found = ext.search("metro");
    assert_eq!(found[0].hit.listing.id, ID);
    assert_eq!(found[0].source, "Architect Lab");
    assert!(metronome_keys(&sections).is_empty());
    assert!(list.info(COUNT_IN).is_none());

    // Installed (from ⌘K or a model: EXT_INSTALL), loaded at once.
    let installed = run(&list, "EXT_INSTALL", &json!({ "id": ID })).unwrap();
    assert_eq!(installed["loaded"], true);
    assert_eq!(
        metronome_keys(&sections),
        [
            "fts.metronome.accent",
            "fts.metronome.volume",
            "fts.metronome.sound"
        ]
    );
    let clicks = run(&list, COUNT_IN, &json!({ "bars": 2 })).unwrap();
    assert_eq!(clicks.as_array().unwrap().len(), 8);
    assert_eq!(clicks[0], "accent");
    assert_eq!(clicks[1], "click");
    let info = list.info(COUNT_IN).unwrap();
    assert_eq!(info.category, "Extensions");
    assert!(info.params_schema.is_some());

    // Saved: a new setting, a changed script — reloaded, nothing restarted.
    let dir = ext.installs().get(ID).unwrap().dir;
    let manifest = dir.join("extension.toml");
    let mut text = std::fs::read_to_string(&manifest).unwrap();
    text.push_str(
        "\n[[sections.settings]]\nkey = \"fts.metronome.count_in\"\ntitle = \"Bars to count in\"\n\
         control = \"stepper\"\nmin = 0\nmax = 4\ndefault = 1\n",
    );
    std::fs::write(&manifest, text).unwrap();
    let script = dir.join("count_in.luau");
    let source = std::fs::read_to_string(&script).unwrap();
    std::fs::write(&script, source.replace("\"accent\"", "\"ACCENT\"")).unwrap();
    let events = ext.reload_changed();
    let [Event::Loaded(change)] = events.as_slice() else {
        panic!("one reload: {events:?}");
    };
    assert!(change.reloaded);
    assert_eq!(
        change.kept,
        ["FTS_METRONOME_CLICK", COUNT_IN, "FTS_METRONOME_STATUS"]
    );
    assert_eq!(metronome_keys(&sections).len(), 4);
    assert_eq!(run(&list, COUNT_IN, &json!({})).unwrap()[0], "ACCENT");
    assert!(ext.reload_changed().is_empty(), "nothing changed since");

    // A save that does not build keeps the last good one.
    std::fs::write(&script, "return (").unwrap();
    let events = ext.reload_changed();
    assert!(matches!(events.as_slice(), [Event::Failed { id, .. }] if id == ID));
    assert_eq!(run(&list, COUNT_IN, &json!({})).unwrap()[0], "ACCENT");
    assert_eq!(metronome_keys(&sections).len(), 4);

    // Removed: its settings and its action go with it.
    let listed = run(&list, "EXT_LIST", &json!(null)).unwrap();
    assert_eq!(listed["extensions"][0]["runs"], "0.1.0, installed");
    assert_eq!(
        run(&list, "EXT_UNINSTALL", &json!({ "id": ID })).unwrap()["removed"],
        true
    );
    assert!(metronome_keys(&sections).is_empty());
    assert!(list.info(COUNT_IN).is_none());
    assert!(
        sections.registry().section("appearance").is_some(),
        "the app's stay"
    );
}

/// The lab's metronome, linked into the build.
fn builtin() -> Package {
    Package::embedded(
        include_str!("../../../apps/lab/extensions/metronome/extension.toml"),
        &[
            (
                "count_in.luau",
                include_str!("../../../apps/lab/extensions/metronome/count_in.luau"),
            ),
            (
                "click.luau",
                include_str!("../../../apps/lab/extensions/metronome/click.luau"),
            ),
            (
                "status.luau",
                include_str!("../../../apps/lab/extensions/metronome/status.luau"),
            ),
        ],
    )
    .unwrap()
}

#[test]
fn a_built_in_runs_until_a_dev_copy_shadows_it_and_can_be_turned_off() {
    let tmp = tempfile::tempdir().unwrap();
    let (list, sections) = app();
    let ext = Extensions::new(
        Host::new(list.clone(), sections.clone()),
        Installs::new(tmp.path().join("installed")),
    )
    .with_builtin(builtin());
    let _ = ext.sync();
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Builtin);
    assert_eq!(run(&list, COUNT_IN, &json!({})).unwrap()[0], "accent");

    // A development copy, a newer version: it runs, and reloads on save.
    let dev = tmp.path().join("dev");
    copy(&lab_registry().join("metronome"), &dev);
    let manifest = dev.join("extension.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, text.replacen("0.1.0", "0.2.0-dev", 1)).unwrap();
    let _ = ext.dev(&dev);
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Dev);
    assert_eq!(
        ext.chosen()[0].describe(),
        "built in (0.1.0), overridden by 0.2.0-dev (in development)"
    );
    std::fs::write(dev.join("count_in.luau"), "return {\"tick\"}").unwrap();
    assert!(matches!(
        ext.reload_changed().as_slice(),
        [Event::Loaded(_)]
    ));
    assert_eq!(run(&list, COUNT_IN, &json!({})).unwrap()[0], "tick");

    // The session ends: the built-in is back.
    let _ = ext.end_dev(&dev);
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Builtin);

    // Turned off like any extension, and on again.
    let _ = ext.disable(ID);
    assert!(list.info(COUNT_IN).is_none());
    assert!(metronome_keys(&sections).is_empty());
    let _ = ext.enable(ID);
    assert_eq!(metronome_keys(&sections).len(), 3);
}

#[test]
fn opening_a_project_installs_what_it_needs_if_the_organization_allows() {
    let tmp = tempfile::tempdir().unwrap();
    let (list, sections) = app();
    let ext = Extensions::new(
        Host::new(list, sections.clone()),
        Installs::new(tmp.path().join("installed")),
    )
    .with_source(FolderRegistry::new(lab_registry()));
    let mut needs = Requirements::default();
    needs.need(Requirement::new(ID).versions("^0.1").named("Metronome"));
    needs.need(Requirement::new("acme.lyrics").optional());

    // The organization has not approved it: the project opens without.
    ext.set_policy(Some(Policy::only_approved()));
    let plan = ext.open(&needs);
    assert!(plan.install.is_empty());
    assert_eq!(plan.missing.len(), 2);

    ext.set_policy(Some(Policy::only_approved().approve(ID, "^0.1")));
    let plan = ext.open(&needs);
    assert_eq!(
        plan.prompt().as_deref(),
        Some("This project uses Metronome")
    );
    let installed = ext.accept(&plan);
    assert!(installed.iter().all(Result::is_ok), "{installed:?}");
    let install = ext.installs().get(ID).unwrap();
    assert!(
        install.linked,
        "a folder registry's package loads where it is"
    );
    assert_eq!(metronome_keys(&sections).len(), 3);
    assert!(ext.open(&needs).is_ready(), "it is here now");
}

const fn app_action(id: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "App",
        method_name: id,
        display_name: id,
        description: "the app's",
        category: "App",
        group: "App",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

static TRANSPORT_PLAY: ActionMeta = app_action("TRANSPORT_PLAY");
static TAKEN: ActionMeta = app_action("ACME_PLAYER_STOP");

#[test]
fn an_extension_runs_only_what_its_capabilities_name() {
    let tmp = tempfile::tempdir().unwrap();
    let (list, sections) = app();
    list.register(&TRANSPORT_PLAY, Effect::Mutating, |(): ()| {
        Ok::<_, String>(true)
    })
    .unwrap();
    let dir = tmp.path().join("player");
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = r#"
id = "acme.player"
name = "Player"
version = "1.0.0"
kind = "luau"

[capabilities]
actions = ["TRANSPORT_"]
effect = "EFFECT"

[[actions]]
id = "ACME_PLAYER_GO"
name = "Go"
script = "go.luau"
effect = "mutating"

[[actions]]
id = "ACME_PLAYER_PLAY"
name = "Play (preset)"
preset = { action = "TRANSPORT_PLAY" }
"#;
    std::fs::write(
        dir.join("go.luau"),
        "return actions.run(\"TRANSPORT_PLAY\")",
    )
    .unwrap();
    let host = Host::new(list.clone(), sections);

    std::fs::write(
        dir.join("extension.toml"),
        manifest.replace("EFFECT", "read_only"),
    )
    .unwrap();
    host.load(&Package::read(&dir).unwrap(), Origin::Dev)
        .unwrap();
    let refused = run(&list, "ACME_PLAYER_GO", &json!(null)).unwrap_err();
    assert!(refused.contains("read_only actions at most"), "{refused}");
    assert!(run(&list, "ACME_PLAYER_PLAY", &json!(null)).is_err());

    std::fs::write(
        dir.join("extension.toml"),
        manifest.replace("EFFECT", "mutating"),
    )
    .unwrap();
    let change = host
        .load(&Package::read(&dir).unwrap(), Origin::Dev)
        .unwrap();
    assert!(change.reloaded);
    assert_eq!(run(&list, "ACME_PLAYER_GO", &json!(null)).unwrap(), true);
    assert_eq!(run(&list, "ACME_PLAYER_PLAY", &json!(null)).unwrap(), true);

    // An action id that is the app's is refused, and the loaded copy stays.
    list.register(&TAKEN, Effect::Mutating, |(): ()| Ok::<_, String>(true))
        .unwrap();
    let clash = manifest
        .replace("EFFECT", "mutating")
        .replace("ACME_PLAYER_PLAY", "ACME_PLAYER_STOP");
    std::fs::write(dir.join("extension.toml"), clash).unwrap();
    let refused = host.load(&Package::read(&dir).unwrap(), Origin::Dev);
    assert_eq!(
        refused,
        Err(LoadError::ActionTaken("ACME_PLAYER_STOP".into()))
    );
    assert!(
        list.info("ACME_PLAYER_PLAY").is_some(),
        "the last good copy stays"
    );

    assert!(host.unload("acme.player"));
    assert!(list.info("ACME_PLAYER_GO").is_none());
    assert!(list.info("TRANSPORT_PLAY").is_some());
}
