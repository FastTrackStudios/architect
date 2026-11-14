//! A person's installed extensions on all their devices (#683's decision):
//! what they trusted and installed on one device installs on their others
//! with the same grants and no new prompt; other people are prompted; a
//! tampered or wrongly signed synced archive is refused.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};

use architect::action::ActionList;
use architect_extension_archive::sign::{SecretKey, sign_archive};
use architect_extension_archive::{entries, write_entries};
use architect_extensions::consent::{Grants, Scope};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::{FolderRegistry, InstallError, Installs};
use architect_extensions::manage::Installed;
use architect_extensions::person_sync::{Applied, PersonInstalls, record_path};
use architect_extensions::project::{Requirement, Requirements};
use architect_extensions::vault_needs;
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::json;
use sha2::Digest as _;

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

/// One device: its own app folder and grants, the showcase registry
/// among its registries.
fn device(at: &Path) -> (ActionList, Extensions) {
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_grants(Grants::open(at.join("grants.json")));
    let ext = Extensions::new(host, Installs::new(at.join("installs")))
        .with_source(FolderRegistry::new(showcase()).trusting("Showcase", key()));
    ext.install_actions().unwrap();
    (list, ext)
}

#[test]
fn an_install_on_one_device_is_on_the_persons_others_with_its_grants() {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();

    // Device A: Alice installs the greeter with her yes, and pins it.
    let (list_a, a) = device(&tmp.path().join("a"));
    block_on(list_a.run_json_async(
        "EXT_INSTALL",
        json!({ "id": "showcase.greeter", "version": "=0.1.0", "grant": "device", "pin": true })
            .to_string()
            .as_bytes(),
    ))
    .unwrap();
    let record = a.publish_installs(&vault, "alice").unwrap();
    assert_eq!(record.installs.len(), 1);
    assert_eq!(record.installs[0].signer, "Showcase");
    assert_eq!(record.installs[0].pin.as_deref(), Some("=0.1.0"));
    assert!(vault.join(record_path("alice")).is_file());

    // Device B, the vault synced: installed, loaded, the same grant, no
    // prompt.
    let (_, b) = device(&tmp.path().join("b"));
    let applied = b.apply_installs(&vault, "alice").unwrap();
    assert!(
        matches!(&applied[0], Ok(Applied::Installed { id, .. }) if id == "showcase.greeter"),
        "{applied:?}"
    );
    assert!(b.host().get("showcase.greeter").is_some(), "loaded at once");
    let granted = b
        .host()
        .grants()
        .unwrap()
        .granted("showcase.greeter")
        .unwrap();
    assert_eq!(granted.reads, ["selection"]);
    assert_eq!(granted.ui, ["notify"]);
    assert_eq!(
        b.installs().get("showcase.greeter").unwrap().pin.as_deref(),
        Some("=0.1.0")
    );
    // Applied again: here already.
    let again = b.apply_installs(&vault, "alice").unwrap();
    assert!(matches!(&again[0], Ok(Applied::Here { .. })));

    // Another member: no record of theirs, nothing installed for them; the
    // vault's needs list prompts them once.
    let (_, c) = device(&tmp.path().join("c"));
    assert!(c.apply_installs(&vault, "carol").unwrap().is_empty());
    let mut needs = Requirements::default();
    needs.need(Requirement::new("showcase.greeter").versions("=0.1.0"));
    vault_needs::write(&vault, &needs).unwrap();
    let plan = c.open_vault(&vault).plan;
    assert_eq!(plan.install.len(), 1);
    let asked = c.accept_vault(&vault, &plan, None, "person");
    assert!(
        matches!(&asked[0], Ok(Installed::NeedsConsent { .. })),
        "{asked:?}"
    );
    assert!(c.installs().get("showcase.greeter").is_none());
    assert!(
        c.host()
            .grants()
            .unwrap()
            .granted("showcase.greeter")
            .is_none()
    );
    let _ = Scope::Device;
}

#[test]
fn a_tampered_or_wrongly_signed_synced_archive_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    let (list_a, a) = device(&tmp.path().join("a"));
    block_on(
        list_a.run_json_async(
            "EXT_INSTALL",
            json!({ "id": "showcase.greeter", "version": "=0.1.0", "grant": "device" })
                .to_string()
                .as_bytes(),
        ),
    )
    .unwrap();
    let record = a.publish_installs(&vault, "alice").unwrap();
    let archive = vault
        .join(vault_needs::CARRIED)
        .join(&record.installs[0].archive);
    let good = std::fs::read(&archive).unwrap();

    // Tampered: a byte changed, the record's sha256 unchanged.
    let mut bad = good.clone();
    let at = bad.len() / 2;
    bad[at] ^= 1;
    std::fs::write(&archive, &bad).unwrap();
    let (_, b) = device(&tmp.path().join("b"));
    let out = b.apply_installs(&vault, "alice").unwrap();
    assert!(
        matches!(&out[0], Err((_, InstallError::Mismatch(why))) if why.contains("sha256")),
        "{out:?}"
    );
    assert!(b.installs().get("showcase.greeter").is_none());

    // Wrongly signed: re-signed by another key, and the record edited to
    // name it (someone else wrote the vault). The device does not trust
    // that key, so it is refused, whatever the record says.
    let other = SecretKey::generate().unwrap();
    let files: Vec<_> = entries(&good)
        .unwrap()
        .into_iter()
        .filter(|(p, _)| !p.ends_with(".minisig"))
        .collect();
    let resigned = sign_archive(&write_entries(&files).unwrap(), &other).unwrap();
    std::fs::write(&archive, &resigned).unwrap();
    let mut forged: PersonInstalls = a.person_installs(&vault, "alice").unwrap();
    forged.installs[0].signer_key = other.public();
    forged.installs[0].signer = "Mallory".to_owned();
    forged.installs[0].sha256 = hex::encode(sha2::Sha256::digest(&resigned));
    std::fs::write(vault.join(record_path("alice")), forged.to_toml().unwrap()).unwrap();
    let (_, d) = device(&tmp.path().join("d"));
    let out = d.apply_installs(&vault, "alice").unwrap();
    assert!(
        matches!(&out[0], Err((_, InstallError::NotAllowed(why))) if why.contains("does not trust")),
        "{out:?}"
    );
    assert!(d.installs().get("showcase.greeter").is_none());

    // The record named the right key but the archive is signed by another:
    // refused by its signature.
    let mut mismatched = forged.clone();
    mismatched.installs[0].signer_key = key();
    mismatched.installs[0].signer = "Showcase".to_owned();
    std::fs::write(
        vault.join(record_path("alice")),
        mismatched.to_toml().unwrap(),
    )
    .unwrap();
    let out = d.apply_installs(&vault, "alice").unwrap();
    assert!(
        matches!(&out[0], Err((_, InstallError::Archive(_)))),
        "{out:?}"
    );
}

#[test]
fn a_record_through_a_link_or_for_a_bad_person_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, b) = device(&tmp.path().join("b"));
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    assert!(b.apply_installs(&vault, "../alice").is_err());
    #[cfg(unix)]
    {
        let elsewhere = tmp.path().join("elsewhere");
        std::fs::create_dir_all(elsewhere.join("people/alice")).unwrap();
        std::fs::write(elsewhere.join("people/alice/extensions.toml"), "").unwrap();
        std::os::unix::fs::symlink(&elsewhere, vault.join(".architect")).unwrap();
        let e = b.apply_installs(&vault, "alice").unwrap_err();
        assert!(e.contains("link"), "{e}");
    }
}
