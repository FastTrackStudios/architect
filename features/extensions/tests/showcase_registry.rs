//! The showcase registry (#710 L1-L4), `examples/showcase-registry/`: its
//! signed archives install through a consent prompt that says what each
//! asks in words; installed, they turn off and on, pin, update (asking for
//! what the newer version adds) and go; its fixture vault plans "Install
//! all"; its macOS-only extension says why it cannot run elsewhere.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};

use architect::action::ActionList;
use architect_extensions::consent::Grants;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::{FolderRegistry, Installs};
use architect_extensions::project::Missing;
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

fn showcase() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/showcase-registry")
}

fn key() -> String {
    std::fs::read_to_string(showcase().join("keys/showcase.pub"))
        .unwrap()
        .lines()
        .nth(1)
        .unwrap()
        .to_owned()
}

fn app(tmp: &Path, platform: &str) -> (ActionList, Extensions) {
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_grants(Grants::open(tmp.join("grants.json")));
    host.set_platform(platform);
    let ext = Extensions::new(host, Installs::new(tmp.join("installs")))
        .with_source(FolderRegistry::new(showcase()).trusting("Showcase", key()));
    ext.install_actions().unwrap();
    ext.install_contribution_actions().unwrap();
    (list, ext)
}

fn run(list: &ActionList, id: &str, args: &Value) -> Value {
    let out = block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .unwrap_or_else(|e| panic!("{id} {args}: {e}"));
    serde_json::from_slice(&out).unwrap_or(Value::Null)
}

#[test]
fn l1_install_from_the_registry_asks_once_in_words() {
    let tmp = tempfile::tempdir().unwrap();
    let (list, ext) = app(tmp.path(), "linux");
    let found = run(&list, "EXT_SEARCH", &json!({ "query": "greet" }));
    assert_eq!(found["found"][0]["id"], "showcase.greeter", "{found}");

    // Without a person's yes: nothing installed, what it asks said.
    let asked = run(&list, "EXT_INSTALL", &json!({ "id": "showcase.greeter", "version": "=0.1.0" }));
    assert_eq!(asked["needs_consent"], true, "{asked}");
    let says: Vec<&str> = asked["asks"]
        .as_array()
        .unwrap_or_else(|| panic!("{asked}"))
        .iter()
        .map(|a| a["says"].as_str().unwrap())
        .collect();
    assert_eq!(
        says,
        ["See what you have selected and focused", "Show you notifications"],
        "{asked}"
    );
    assert!(ext.host().get("showcase.greeter").is_none());

    // With it: signed by the registry's key, installed and loaded.
    let done = run(
        &list,
        "EXT_INSTALL",
        &json!({ "id": "showcase.greeter", "version": "=0.1.0", "grant": "device" }),
    );
    assert_eq!(done["version"], "0.1.0", "{done}");
    assert_eq!(done["loaded"], true, "{done}");
    let preview = ext.preview("showcase.greeter", &semver::VersionReq::STAR).unwrap();
    assert_eq!(preview.signed_by.as_deref(), Some("Showcase"));
}

#[test]
fn l3_off_on_pin_update_and_remove() {
    let tmp = tempfile::tempdir().unwrap();
    let (list, ext) = app(tmp.path(), "linux");
    run(
        &list,
        "EXT_INSTALL",
        &json!({ "id": "showcase.greeter", "version": "=0.1.0", "grant": "device" }),
    );
    run(&list, "EXT_DISABLE", &json!({ "id": "showcase.greeter" }));
    assert!(ext.host().get("showcase.greeter").is_none());
    run(&list, "EXT_ENABLE", &json!({ "id": "showcase.greeter" }));
    assert!(ext.host().get("showcase.greeter").is_some());

    // Pinned, the update is past the pin and not taken.
    run(&list, "EXT_PIN", &json!({ "id": "showcase.greeter", "version": "=0.1.0" }));
    let check = run(&list, "EXT_UPDATE", &json!({ "check": true }));
    assert_eq!(check["available"][0]["past_pin"], "0.2.0", "{check}");

    // Unpinned: 0.2.0, with its changelog, asks for the clipboard too, and
    // waits for a person's yes.
    run(&list, "EXT_PIN", &json!({ "id": "showcase.greeter", "unpin": true }));
    let check = run(&list, "EXT_UPDATE", &json!({ "check": true }));
    let a = &check["available"][0];
    assert_eq!(a["newest"], "0.2.0", "{check}");
    assert!(a["changelog"].as_str().unwrap().contains("clipboard"), "{check}");
    assert_eq!(a["asks_more"].as_array().unwrap().len(), 1, "{check}");
    let applied = run(&list, "EXT_UPDATE", &json!({ "id": "showcase.greeter" }));
    assert_eq!(applied["needs_consent"][0]["id"], "showcase.greeter", "{applied}");
    assert_eq!(ext.installs().get("showcase.greeter").unwrap().version.to_string(), "0.1.0");

    // Removed: gone, unloaded.
    run(&list, "EXT_UNINSTALL", &json!({ "id": "showcase.greeter" }));
    assert!(ext.installs().get("showcase.greeter").is_none());
    assert!(ext.host().get("showcase.greeter").is_none());
}

#[test]
fn l2_the_fixture_vault_plans_install_all() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, ext) = app(tmp.path(), "linux");
    let opened = ext.open_vault(&showcase().join("vault"));
    let ids = |l: &[architect_extensions::index::Listing]| {
        let mut v: Vec<String> = l.iter().map(|l| l.id.clone()).collect();
        v.sort();
        v
    };
    assert_eq!(opened.refused, Vec::new());
    assert_eq!(ids(&opened.plan.install), ["showcase.tiny", "showcase.word-count"]);
    assert_eq!(ids(&opened.plan.offer), ["showcase.greeter"]);
    let missing: Vec<(String, Missing)> = opened
        .plan
        .missing
        .iter()
        .map(|(r, m)| (r.id.clone(), m.clone()))
        .collect();
    assert!(
        missing.contains(&(
            "showcase.mac-only".into(),
            Missing::CannotRun("macOS only: not on Linux".into())
        )),
        "{missing:?}"
    );
    assert!(missing.contains(&("showcase.gone".into(), Missing::NotFound)), "{missing:?}");

    let done = ext.accept(&opened.plan);
    assert!(done.iter().all(Result::is_ok), "{done:?}");
    assert!(ext.host().get("showcase.word-count").is_some());
    assert!(ext.host().get("showcase.tiny").is_some());
}

#[test]
fn l4_the_mac_only_extension_is_refused_here_and_installs_on_macos() {
    let tmp = tempfile::tempdir().unwrap();
    let (list, ext) = app(tmp.path(), "linux");
    let preview = run(&list, "EXT_PREVIEW", &json!({ "id": "showcase.mac-only" }));
    assert_eq!(preview["cannot_run"], "macOS only: not on Linux", "{preview}");
    assert_eq!(preview["runs_here"], false, "{preview}");
    assert!(
        ext.install("showcase.mac-only", &semver::VersionReq::STAR)
            .unwrap_err()
            .to_string()
            .contains("macOS only")
    );

    let tmp = tempfile::tempdir().unwrap();
    let (_, ext) = app(tmp.path(), "macos");
    ext.install("showcase.mac-only", &semver::VersionReq::STAR).unwrap();
    assert!(ext.host().get("showcase.mac-only").is_some());
}
