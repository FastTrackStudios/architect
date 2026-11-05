//! `EXT_CHECK`, `EXT_CLONE` and the extensions' problems (#268): the lab's
//! metronome checks clean; a broken copy fails with the stage, file, field
//! and line; a built-in one is cloned into the development folder under a
//! new namespace; a dev folder whose manifest breaks is a problem with its
//! file and field while the last good copy runs.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};

use architect::action::ActionList;
use architect_extensions::check::Checker;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_extensions::{Origin, Package};
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

fn metronome() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/lab/extensions/metronome")
}

fn copy_of(dir: &Path, to: &Path) -> PathBuf {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(dir).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
    }
    to.to_path_buf()
}

fn edit(file: &Path, from: &str, to: &str) {
    let text = std::fs::read_to_string(file).unwrap();
    assert!(text.contains(from), "{from:?} is not in {}", file.display());
    std::fs::write(file, text.replacen(from, to, 1)).unwrap();
}

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

fn run(list: &ActionList, id: &str, args: &Value) -> Result<Value, String> {
    block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
}

#[test]
fn the_metronome_checks_clean_and_a_broken_copy_says_exactly_why() {
    let report = Checker::new().check(&metronome());
    assert!(report.ok, "{report:?}");
    assert_eq!(report.id.as_deref(), Some("fts.metronome"));
    assert_eq!(
        report.actions,
        [
            "FTS_METRONOME_COUNT_IN",
            "FTS_METRONOME_STATUS",
            "FTS_METRONOME_CLICK"
        ]
    );

    let tmp = tempfile::tempdir().unwrap();
    // A manifest field of the wrong kind: the field and its line.
    let bad = copy_of(&metronome(), &tmp.path().join("bad-effect"));
    edit(
        &bad.join("extension.toml"),
        "group = \"Metronome\"",
        "group = \"Metronome\"\neffect = \"sometimes\"",
    );
    let report = Checker::new().check(&bad);
    assert!(!report.ok);
    let p = &report.problems[0];
    assert_eq!(
        (p.stage.as_str(), p.file.as_deref(), p.field.as_deref()),
        ("manifest", Some("extension.toml"), Some("effect")),
        "{p:?}"
    );
    assert!(p.line.is_some_and(|l| l > 10), "{p:?}");
    assert!(p.message.contains("sometimes"), "{p:?}");
    assert!(
        report.summary().starts_with("manifest (extension.toml:"),
        "{}",
        report.summary()
    );

    // A script that is not Luau: its file and line.
    let bad = copy_of(&metronome(), &tmp.path().join("bad-script"));
    std::fs::write(
        bad.join("count_in.luau"),
        "local x = 1\nlocal y = = 2\nreturn x\n",
    )
    .unwrap();
    let report = Checker::new().check(&bad);
    let p = &report.problems[0];
    assert_eq!(
        (p.stage.as_str(), p.file.as_deref(), p.line),
        ("script", Some("count_in.luau"), Some(2)),
        "{p:?}"
    );

    // An action outside its namespace: the action's id field.
    let bad = copy_of(&metronome(), &tmp.path().join("bad-namespace"));
    edit(
        &bad.join("extension.toml"),
        "id = \"FTS_METRONOME_COUNT_IN\"",
        "id = \"OTHER_COUNT_IN\"",
    );
    let report = Checker::new().check(&bad);
    assert_eq!(
        report.problems[0].field.as_deref(),
        Some("actions[OTHER_COUNT_IN].id"),
        "{report:?}"
    );

    // No folder at all.
    let report = Checker::new().check(&tmp.path().join("nowhere"));
    assert_eq!(report.problems[0].stage, "files", "{report:?}");
}

#[test]
fn ext_check_and_ext_clone_are_actions_and_a_broken_dev_copy_is_a_problem() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let ext = Extensions::new(
        Host::new(list.clone(), SharedSections::new(Registry::new())),
        Installs::new(tmp.path().join("installed")),
    )
    .with_builtin(builtin())
    .with_dev_root(tmp.path().join("dev"));
    ext.install_actions().unwrap();
    let _ = ext.sync();

    let checked = run(
        &list,
        "EXT_CHECK",
        &json!({ "folder": metronome().display().to_string() }),
    )
    .unwrap();
    assert_eq!(checked["ok"], true, "{checked}");
    let typo = run(&list, "EXT_CHECK", &json!({ "fodler": "x" })).unwrap_err();
    assert!(typo.contains("valid arguments: folder"), "{typo}");

    // The built-in metronome, cloned: a new id, its action in the new
    // namespace, running from the dev folder; the built-in one as it was.
    let cloned = run(
        &list,
        "EXT_CLONE",
        &json!({ "id": "fts.metronome", "as": "my.metronome" }),
    )
    .unwrap();
    assert_eq!(cloned["id"], "my.metronome", "{cloned}");
    let folder = PathBuf::from(cloned["folder"].as_str().unwrap());
    let manifest = std::fs::read_to_string(folder.join("extension.toml")).unwrap();
    assert!(
        manifest.starts_with("# A copy of fts.metronome"),
        "{manifest}"
    );
    assert!(folder.join("count_in.luau").is_file());
    assert_eq!(ext.host().get("my.metronome").unwrap().origin, Origin::Dev);
    assert_eq!(
        run(&list, "MY_METRONOME_COUNT_IN", &json!({})).unwrap()[0],
        "accent"
    );
    assert_eq!(
        ext.host().get("fts.metronome").unwrap().origin,
        Origin::Builtin
    );
    assert!(
        run(
            &list,
            "EXT_CLONE",
            &json!({ "id": "fts.metronome", "as": "my.metronome" })
        )
        .unwrap_err()
        .contains("exists")
    );
    assert!(ext.problems().is_empty(), "{:?}", ext.problems());

    // The copy's manifest breaks: its last good copy runs, and the problem
    // names the file and field.
    edit(
        &folder.join("extension.toml"),
        "kind = \"luau\"",
        "kind = \"lua\"",
    );
    let _ = ext.reload_changed();
    assert!(
        ext.host().get("my.metronome").is_some(),
        "the last good copy runs"
    );
    let problems = ext.problems();
    assert_eq!(problems.len(), 1, "{problems:?}");
    let p = &problems[0];
    assert_eq!(p.source, "extension");
    assert_eq!(
        p.file.as_deref(),
        Some(folder.join("extension.toml").display().to_string().as_str()),
        "{p:?}"
    );
    assert_eq!(p.field.as_deref(), Some("kind"), "{p:?}");
    assert!(p.message.contains("line "), "{p:?}");
}
