//! Install on open (#710 L2, "What a project needs"): a vault names the
//! extensions it needs in `.architect/extensions.toml` and can carry small
//! ones as signed archives in `.architect/extensions/`. Opening it plans
//! what to install (required), offer (optional) and what cannot be had,
//! each with why; accepting installs and loads all of it at once, without
//! a restart; an unsigned carried archive is refused, never run.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};

use architect::action::ActionList;
use architect_extension_archive::pack;
use architect_extension_archive::sign::{SecretKey, sign_archive};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::{Installs, Trust};
use architect_extensions::project::{Missing, Requirement, Requirements};
use architect_extensions::vault_needs;
use architect_settings::Registry;

fn hello() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extension/registry/hello")
}

/// `hello` copied as extension `id`, named `name`.
fn package(tmp: &Path, id: &str, name: &str) -> PathBuf {
    let dir = tmp.join(format!("src-{id}"));
    std::fs::create_dir_all(&dir).unwrap();
    for e in std::fs::read_dir(hello()).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), dir.join(e.file_name())).unwrap();
    }
    let prefix = id.to_uppercase().replace(['.', '-'], "_");
    let manifest = std::fs::read_to_string(dir.join("extension.toml"))
        .unwrap()
        .replace("id = \"example.hello\"", &format!("id = \"{id}\""))
        .replace("name = \"Hello\"", &format!("name = \"{name}\""))
        .replace("EXAMPLE_HELLO_", &format!("{prefix}_"))
        .replace("example.hello.", &format!("{id}."));
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    dir
}

fn app(tmp: &Path, key: &SecretKey) -> Extensions {
    let list = ActionList::new();
    let host = Host::new(list, SharedSections::new(Registry::new()));
    let installs = Installs::new(tmp.join("installs"))
        .with_trust(Trust::org().with("the test key", key.public()));
    Extensions::new(host, installs)
}

#[test]
fn opening_a_vault_installs_what_it_carries_and_needs_in_one_go() {
    let tmp = tempfile::tempdir().unwrap();
    let key = SecretKey::generate().unwrap();
    let vault = tmp.path().join("vault");
    let carried = vault.join(vault_needs::CARRIED);
    std::fs::create_dir_all(&carried).unwrap();
    for (id, name) in [("show.lyrics", "Lyrics Lane"), ("show.tuner", "Tuner")] {
        let bytes = sign_archive(&pack(&package(tmp.path(), id, name)).unwrap().1, &key).unwrap();
        std::fs::write(carried.join(format!("{id}.archext")), bytes).unwrap();
    }
    // One carried unsigned: refused, never offered.
    let unsigned = pack(&package(tmp.path(), "show.rogue", "Rogue")).unwrap().1;
    std::fs::write(carried.join("show.rogue.archext"), unsigned).unwrap();

    let mut needs = Requirements::default();
    needs.need(
        Requirement::new("show.lyrics")
            .named("Lyrics Lane")
            .versions("^0.1"),
    );
    needs.need(Requirement::new("show.tuner").named("Tuner").optional());
    needs.need(Requirement::new("show.rogue").named("Rogue"));
    needs.need(Requirement::new("show.gone").named("Gone"));
    vault_needs::write(&vault, &needs).unwrap();
    assert_eq!(vault_needs::read(&vault).unwrap(), needs);

    let ext = app(tmp.path(), &key);
    // What an agent reads first: EXT_NEEDS.
    let list = ext.host().list().clone();
    ext.install_actions().unwrap();
    let args = serde_json::json!({ "folder": vault.display().to_string() }).to_string();
    let out: serde_json::Value = serde_json::from_slice(
        &futures::executor::block_on(list.run_json_async("EXT_NEEDS", args.as_bytes())).unwrap(),
    )
    .unwrap();
    assert_eq!(out["install"][0]["id"], "show.lyrics", "{out}");
    assert_eq!(out["offer"][0]["id"], "show.tuner", "{out}");
    assert_eq!(out["missing"][1]["why"], "no registry here has it", "{out}");
    assert_eq!(out["refused"][0]["file"], "show.rogue.archext", "{out}");
    let opened = ext.open_vault(&vault);
    assert_eq!(opened.error, None);
    let plan = &opened.plan;
    assert_eq!(
        plan.install
            .iter()
            .map(|l| l.id.as_str())
            .collect::<Vec<_>>(),
        ["show.lyrics"]
    );
    assert_eq!(
        plan.offer.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(),
        ["show.tuner"]
    );
    assert_eq!(
        plan.prompt().as_deref(),
        Some("This project uses Lyrics Lane")
    );
    // The unsigned one is refused, with why; it and the one no registry
    // has cannot be had, and say so.
    assert_eq!(opened.refused.len(), 1, "{:?}", opened.refused);
    assert_eq!(opened.refused[0].0, "show.rogue.archext");
    assert!(opened.refused[0].1.contains("unsigned") || opened.refused[0].1.contains("signed"));
    let missing: Vec<(&str, &Missing)> = plan
        .missing
        .iter()
        .map(|(r, m)| (r.id.as_str(), m))
        .collect();
    assert_eq!(
        missing,
        [
            ("show.rogue", &Missing::NotFound),
            ("show.gone", &Missing::NotFound)
        ]
    );
    assert_eq!(Missing::NotFound.to_string(), "no registry here has it");

    // "Install all": installed and loaded now, no restart.
    let done = ext.accept(plan);
    assert!(done.iter().all(Result::is_ok), "{done:?}");
    assert!(ext.host().get("show.lyrics").is_some(), "loaded at once");
    assert!(
        ext.host().get("show.tuner").is_none(),
        "optional: offered, not installed"
    );
    assert!(ext.host().get("show.rogue").is_none(), "never run");

    // Opened again, it is ready.
    let again = ext.open_vault(&vault);
    assert_eq!(again.plan.ready, ["show.lyrics"]);
    assert!(again.plan.install.is_empty());
}

#[test]
fn a_vault_that_names_nothing_needs_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let key = SecretKey::generate().unwrap();
    let ext = app(tmp.path(), &key);
    let opened = ext.open_vault(&tmp.path().join("empty"));
    assert!(opened.is_empty(), "{opened:?}");

    let bad = tmp.path().join("bad");
    std::fs::create_dir_all(bad.join(".architect")).unwrap();
    std::fs::write(bad.join(vault_needs::NEEDS), "extensions = 3").unwrap();
    let opened = ext.open_vault(&bad);
    assert!(opened.error.unwrap().contains(".architect/extensions.toml"));
}
