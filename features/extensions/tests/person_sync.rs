//! A person's installed extensions on all their devices (#683's decision):
//! the record is signed by the device that wrote it, and applies only on a
//! device that trusts it as one of the person's own. An install, removal
//! or revoke on one device goes to the others with no prompt; a record
//! written by anyone else is refused and nothing installs; a tampered
//! archive is refused and grants nothing; the actions answer only the
//! person.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};

use architect::action::{ActionCaller, ActionList, Principal};
use architect_extensions::consent::Grants;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::{FolderRegistry, InstallError, Installs};
use architect_extensions::manage::Installed;
use architect_extensions::person_sync::{
    Applied, DeviceKeys, PersonSync, RecordError, record_path,
};
use architect_extensions::project::{Requirement, Requirements};
use architect_extensions::vault_needs;
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::json;

const GREETER: &str = "showcase.greeter";

fn showcase() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/examples/showcase-registry")
}

fn key() -> String {
    std::fs::read_to_string(showcase().join("keys/showcase.pub"))
        .unwrap()
        .lines()
        .nth(1)
        .unwrap()
        .to_owned()
}

struct Device {
    list: ActionList,
    ext: Extensions,
    keys: DeviceKeys,
}

/// One device of `person`'s, with the vault at `vault` open: its own app
/// folder, grants and key, the showcase registry among its registries.
fn device(at: &Path, name: &str, vault: &Path, person: &str) -> Device {
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_grants(Grants::open(at.join("grants.json")));
    let ext = Extensions::new(host, Installs::new(at.join("installs")))
        .with_source(FolderRegistry::new(showcase()).trusting("Showcase", key()));
    ext.install_actions().unwrap();
    ext.install_contribution_actions().unwrap();
    let keys = DeviceKeys::open(&at.join("app"), name).unwrap();
    ext.sync_person(PersonSync {
        root: vault.to_path_buf(),
        person: person.to_owned(),
        keys: keys.clone(),
    });
    Device { list, ext, keys }
}

impl Device {
    fn run(&self, id: &str, args: &serde_json::Value) -> serde_json::Value {
        let out = block_on(self.list.run_json_async(id, args.to_string().as_bytes()))
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        serde_json::from_slice(&out).unwrap_or(serde_json::Value::Null)
    }

    fn apply(&self) -> Result<Vec<Result<Applied, (String, InstallError)>>, RecordError> {
        self.ext.apply_installs(&self.ext.person_sync().unwrap())
    }

    fn granted(&self) -> Option<architect_extensions::manifest::Capabilities> {
        self.ext.host().grants().unwrap().granted(GREETER)
    }
}

#[test]
fn an_install_removal_and_revoke_on_one_device_follow_the_person() {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    let a = device(&tmp.path().join("a"), "Alice's laptop", &vault, "alice");
    let b = device(&tmp.path().join("b"), "Alice's desktop", &vault, "alice");
    // Once, the person tells each device the other is theirs.
    b.keys.trust(&a.keys.me()).unwrap();
    a.keys.trust(&b.keys.me()).unwrap();

    // A: installed with her yes, and pinned; published at once.
    a.run(
        "EXT_INSTALL",
        &json!({ "id": GREETER, "version": "=0.1.0", "grant": "device", "pin": true }),
    );
    assert!(vault.join(record_path("alice")).is_file());

    // B: installed, loaded, the same grant and pin, no prompt.
    let applied = b.apply().unwrap();
    assert!(
        matches!(&applied[0], Ok(Applied::Installed { id, .. }) if id == GREETER),
        "{applied:?}"
    );
    assert!(b.ext.host().get(GREETER).is_some());
    assert_eq!(b.granted().unwrap().reads, ["selection"]);
    assert_eq!(
        b.ext.installs().get(GREETER).unwrap().pin.as_deref(),
        Some("=0.1.0")
    );
    assert!(
        matches!(&b.apply().unwrap()[0], Ok(Applied::Here { .. })),
        "applied once"
    );

    // A revokes: B's grant goes too.
    a.run("EXT_REVOKE", &json!({ "id": GREETER }));
    let applied = b.apply().unwrap();
    assert!(
        matches!(&applied[0], Ok(Applied::Granted { .. })),
        "{applied:?}"
    );
    assert!(b.granted().is_none(), "revoked on B");

    // A removes it: B removes it.
    a.run("EXT_UNINSTALL", &json!({ "id": GREETER }));
    let applied = b.apply().unwrap();
    assert!(
        matches!(&applied[0], Ok(Applied::Removed { .. })),
        "{applied:?}"
    );
    assert!(b.ext.installs().get(GREETER).is_none());
    assert!(b.ext.host().get(GREETER).is_none());
}

#[test]
fn a_record_another_person_wrote_installs_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    // Mallory, with a device of her own, writes Alice's record (as Alice).
    let mallory = device(&tmp.path().join("m"), "Alice's laptop", &vault, "alice");
    mallory.run(
        "EXT_INSTALL",
        &json!({ "id": GREETER, "version": "=0.1.0", "grant": "device" }),
    );
    assert!(vault.join(record_path("alice")).is_file());

    // Alice's device trusts only her own devices: refused, nothing at all.
    let b = device(&tmp.path().join("b"), "Alice's desktop", &vault, "alice");
    let e = b.apply().unwrap_err();
    assert!(matches!(e, RecordError::UntrustedDevice { .. }), "{e:?}");
    assert!(b.ext.installs().get(GREETER).is_none());
    assert!(b.granted().is_none());

    // Unsigned (the signature gone): refused too.
    std::fs::remove_file(vault.join(format!("{}.minisig", record_path("alice")))).unwrap();
    assert!(matches!(b.apply().unwrap_err(), RecordError::Unsigned(_)));
    assert!(b.ext.installs().get(GREETER).is_none());

    // Another member is not in Alice's record; the vault's needs prompt
    // them once.
    let carol = device(&tmp.path().join("c"), "Carol's", &vault, "carol");
    assert!(carol.apply().unwrap().is_empty());
    let mut needs = Requirements::default();
    needs.need(Requirement::new(GREETER).versions("=0.1.0"));
    vault_needs::write(&vault, &needs).unwrap();
    let plan = carol.ext.open_vault(&vault).plan;
    let asked = carol.ext.accept_vault(&vault, &plan, None, "person");
    assert!(
        matches!(&asked[0], Ok(Installed::NeedsConsent { .. })),
        "{asked:?}"
    );
    assert!(carol.ext.installs().get(GREETER).is_none());
}

#[test]
fn a_tampered_archive_installs_nothing_and_grants_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    let a = device(&tmp.path().join("a"), "laptop", &vault, "alice");
    let b = device(&tmp.path().join("b"), "desktop", &vault, "alice");
    b.keys.trust(&a.keys.me()).unwrap();
    a.run(
        "EXT_INSTALL",
        &json!({ "id": GREETER, "version": "=0.1.0", "grant": "device" }),
    );
    let carried = vault
        .join(vault_needs::CARRIED)
        .join(format!("{GREETER}-0.1.0.archext"));
    let mut bytes = std::fs::read(&carried).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 1;
    std::fs::write(&carried, &bytes).unwrap();
    let out = b.apply().unwrap();
    assert!(
        matches!(&out[0], Err((_, InstallError::Mismatch(why))) if why.contains("sha256")),
        "{out:?}"
    );
    assert!(b.ext.installs().get(GREETER).is_none());
    assert!(b.granted().is_none(), "no grant before an install");
}

#[test]
fn the_actions_answer_only_the_person() {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    let a = device(&tmp.path().join("a"), "laptop", &vault, "alice");
    for id in ["EXT_INSTALLS_APPLY", "EXT_INSTALLS_PUBLISH"] {
        let as_mallory = ActionCaller::new(Principal::User {
            user_id: "mallory".to_owned(),
        });
        let e = block_on(a.list.run_json_async_with(&as_mallory, id, b"{}")).unwrap_err();
        assert!(e.to_string().contains("only alice"), "{id}: {e}");
        let as_alice = ActionCaller::new(Principal::User {
            user_id: "alice".to_owned(),
        });
        assert!(
            block_on(a.list.run_json_async_with(&as_alice, id, b"{}")).is_ok(),
            "{id}"
        );
    }
}
