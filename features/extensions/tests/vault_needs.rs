//! Install on open (#710 L2, "What a project needs"): a vault names the
//! extensions it needs in `.architect/extensions.toml` and can carry small
//! ones as signed archives in `.architect/extensions/`. Opening it plans
//! what to install (required), offer (optional) and what cannot be had,
//! each with why; the install goes through one consent for all of it.
//!
//! A vault's archives are a source for its own plan only: never an update,
//! a search or another install's. One is trusted when signed by the app's
//! key or by the key the installed copy of its id is signed by, never by a
//! registry's publisher alone. A vault cannot downgrade an install or step
//! past its pin. Links, oversized and unsigned archives are refused.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use architect::action::ActionList;
use architect_extension_archive::pack;
use architect_extension_archive::sign::{SecretKey, sign_archive};
use architect_extensions::consent::{Grants, Scope};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::{FolderRegistry, InstallError, Installs, Trust};
use architect_extensions::manage::Installed;
use architect_extensions::project::{Missing, Requirement, Requirements};
use architect_extensions::vault_needs;
use architect_settings::Registry;
use semver::VersionReq;
use sha2::Digest as _;

fn hello() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extension/registry/hello")
}

/// `hello` copied as extension `id` at `version`, named `name`.
fn package(tmp: &Path, id: &str, version: &str, name: &str) -> PathBuf {
    let dir = tmp.join(format!("src-{id}-{version}"));
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
        .replace("version = \"0.1.0\"", &format!("version = \"{version}\""))
        .replace("EXAMPLE_HELLO_", &format!("{prefix}_"))
        .replace("example.hello.", &format!("{id}."));
    // What the consent prompt lists, in words.
    let manifest =
        format!("{manifest}\n[capabilities]\nreads = [\"selection\"]\nui = [\"notify\"]\n");
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    dir
}

fn signed(tmp: &Path, id: &str, version: &str, key: &SecretKey) -> Vec<u8> {
    sign_archive(&pack(&package(tmp, id, version, id)).unwrap().1, key).unwrap()
}

/// A folder registry listing each `(id, version, bytes)`.
fn registry(at: &Path, archives: &[(&str, &str, Vec<u8>)]) -> PathBuf {
    std::fs::create_dir_all(at.join("archives")).unwrap();
    let mut index = String::from("name = \"Test\"\n");
    for (id, version, bytes) in archives {
        let file = format!("archives/{id}-{version}.archext");
        std::fs::write(at.join(&file), bytes).unwrap();
        writeln!(
            index,
            "\n[[extensions]]\nid = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\nkind = \"luau\"\narchive = \"{file}\"\nsha256 = \"{}\"",
            hex::encode(sha2::Sha256::digest(bytes))
        )
        .unwrap();
    }
    std::fs::write(at.join("index.toml"), index).unwrap();
    at.to_path_buf()
}

/// An app trusting `app_key` as its own (the org key's place), with a
/// grants store when `consent`.
fn app(tmp: &Path, app_key: &SecretKey, consent: bool) -> Extensions {
    let list = ActionList::new();
    let mut host = Host::new(list, SharedSections::new(Registry::new()));
    if consent {
        host = host.with_grants(Grants::open(tmp.join("grants.json")));
    }
    let installs = Installs::new(tmp.join("installs"))
        .with_trust(Trust::none().with("the app's key", app_key.public()));
    Extensions::new(host, installs)
}

fn carry(vault: &Path, name: &str, bytes: &[u8]) {
    let dir = vault.join(vault_needs::CARRIED);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), bytes).unwrap();
}

#[test]
fn opening_a_vault_plans_and_one_grant_installs_all_of_it() {
    let tmp = tempfile::tempdir().unwrap();
    let key = SecretKey::generate().unwrap();
    let vault = tmp.path().join("vault");
    carry(
        &vault,
        "lyrics.archext",
        &signed(tmp.path(), "show.lyrics", "0.1.0", &key),
    );
    carry(
        &vault,
        "tuner.archext",
        &signed(tmp.path(), "show.tuner", "0.1.0", &key),
    );
    carry(
        &vault,
        "rogue.archext",
        &pack(&package(tmp.path(), "show.rogue", "0.1.0", "Rogue"))
            .unwrap()
            .1,
    );
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

    let ext = app(tmp.path(), &key, true);
    ext.install_actions().unwrap();
    // What an agent reads first: EXT_NEEDS, which changes nothing.
    let list = ext.host().list().clone();
    let args = serde_json::json!({ "folder": vault.display().to_string() }).to_string();
    let out: serde_json::Value = serde_json::from_slice(
        &futures::executor::block_on(list.run_json_async("EXT_NEEDS", args.as_bytes())).unwrap(),
    )
    .unwrap();
    assert_eq!(out["install"][0]["id"], "show.lyrics", "{out}");
    assert_eq!(out["offer"][0]["id"], "show.tuner", "{out}");
    assert_eq!(out["refused"][0]["file"], "rogue.archext", "{out}");
    assert!(
        ext.offers().is_empty(),
        "the vault's archives are no source of the app's"
    );

    let opened = ext.open_vault(&vault);
    let plan = &opened.plan;
    assert_eq!(
        plan.prompt().as_deref(),
        Some("This project uses show.lyrics")
    );
    assert!(
        opened.refused[0].1.contains("unsigned"),
        "{:?}",
        opened.refused
    );
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

    // The prompt's previews say, in words, what each asks.
    let previews = ext.previews_for(Some(&vault), plan);
    let says: Vec<&str> = previews[0]
        .as_ref()
        .unwrap()
        .asks
        .iter()
        .map(|a| a.says.as_str())
        .collect();
    assert_eq!(
        says,
        [
            "See what you have selected and focused",
            "Show you notifications"
        ]
    );

    // No grant: nothing installed, each one waits for the person's yes.
    let waiting = ext.accept_vault(&vault, plan, None, "person");
    assert!(
        waiting
            .iter()
            .all(|r| matches!(r, Ok(Installed::NeedsConsent { .. }))),
        "{waiting:?}"
    );
    assert!(ext.installs().get("show.lyrics").is_none());
    assert!(ext.host().get("show.lyrics").is_none());

    // One grant: installed and loaded at once; the optional one is not.
    let done = ext.accept_vault(&vault, plan, Some(&Scope::Device), "person");
    assert!(
        done.iter().all(|r| matches!(r, Ok(Installed::Done { .. }))),
        "{done:?}"
    );
    assert!(ext.host().get("show.lyrics").is_some());
    assert!(ext.host().get("show.tuner").is_none());
    assert_eq!(ext.open_vault(&vault).plan.ready, ["show.lyrics"]);
}

#[test]
fn a_vaults_archive_is_never_an_update_and_a_publishers_key_is_not_enough() {
    let tmp = tempfile::tempdir().unwrap();
    let app_key = SecretKey::generate().unwrap();
    let publisher = SecretKey::generate().unwrap();
    let reg = registry(
        &tmp.path().join("reg"),
        &[(
            "acme.greeter",
            "0.1.0",
            signed(tmp.path(), "acme.greeter", "0.1.0", &publisher),
        )],
    );
    let ext = app(tmp.path(), &app_key, false)
        .with_source(FolderRegistry::new(&reg).trusting("Acme", publisher.public()));
    ext.install("acme.greeter", &VersionReq::STAR).unwrap();
    assert!(
        ext.installs()
            .get("acme.greeter")
            .unwrap()
            .signer_key
            .is_some()
    );

    // A vault carries 9.9.9 signed by the publisher (the installed copy's
    // key: trusted for its id), and a new id signed by the same publisher
    // (no installed copy: not trusted).
    let vault = tmp.path().join("vault");
    carry(
        &vault,
        "greeter.archext",
        &signed(tmp.path(), "acme.greeter", "9.9.9", &publisher),
    );
    carry(
        &vault,
        "other.archext",
        &signed(tmp.path(), "acme.other", "0.1.0", &publisher),
    );
    let mut needs = Requirements::default();
    needs.need(Requirement::new("acme.greeter").versions(">=9"));
    needs.need(Requirement::new("acme.other"));
    vault_needs::write(&vault, &needs).unwrap();
    let opened = ext.open_vault(&vault);
    assert_eq!(opened.plan.install.len(), 1, "{opened:?}");
    assert_eq!(opened.plan.install[0].version.to_string(), "9.9.9");
    assert!(
        opened
            .refused
            .iter()
            .any(|(f, why)| f == "other.archext" && why.contains("not trusted")),
        "{:?}",
        opened.refused
    );

    // Opened, it is still no source: an update, a search and an install by
    // id see the registry's only.
    let updates = ext.updates(None);
    assert!(updates.iter().all(|a| a.newest != "9.9.9"), "{updates:?}");
    assert!(
        ext.search("greeter")
            .iter()
            .all(|f| f.hit.listing.version.to_string() == "0.1.0")
    );
    assert!(ext.install("acme.other", &VersionReq::STAR).is_err());
    assert_eq!(
        ext.installs()
            .get("acme.greeter")
            .unwrap()
            .version
            .to_string(),
        "0.1.0"
    );
}

#[test]
fn an_update_signed_by_another_key_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let app_key = SecretKey::generate().unwrap();
    let (first, second) = (
        SecretKey::generate().unwrap(),
        SecretKey::generate().unwrap(),
    );
    let a = registry(
        &tmp.path().join("a"),
        &[(
            "acme.greeter",
            "0.1.0",
            signed(tmp.path(), "acme.greeter", "0.1.0", &first),
        )],
    );
    let b = registry(
        &tmp.path().join("b"),
        &[(
            "acme.greeter",
            "0.2.0",
            signed(tmp.path(), "acme.greeter", "0.2.0", &second),
        )],
    );
    let ext = app(tmp.path(), &app_key, false)
        .with_source(FolderRegistry::new(&a).trusting("First", first.public()))
        .with_source(FolderRegistry::new(&b).trusting("Second", second.public()));
    ext.install("acme.greeter", &VersionReq::parse("=0.1.0").unwrap())
        .unwrap();
    let e = ext.install("acme.greeter", &VersionReq::STAR).unwrap_err();
    assert!(matches!(e, InstallError::SignerChanged { .. }), "{e:?}");
    assert!(
        e.to_string().contains("First") && e.to_string().contains("Second"),
        "{e}"
    );
    assert_eq!(
        ext.installs()
            .get("acme.greeter")
            .unwrap()
            .version
            .to_string(),
        "0.1.0"
    );
}

#[test]
fn a_vault_cannot_downgrade_an_install_or_step_past_its_pin() {
    let tmp = tempfile::tempdir().unwrap();
    let key = SecretKey::generate().unwrap();
    let reg = registry(
        &tmp.path().join("reg"),
        &[
            (
                "acme.greeter",
                "0.1.0",
                signed(tmp.path(), "acme.greeter", "0.1.0", &key),
            ),
            (
                "acme.greeter",
                "0.2.0",
                signed(tmp.path(), "acme.greeter", "0.2.0", &key),
            ),
            (
                "acme.greeter",
                "0.3.0",
                signed(tmp.path(), "acme.greeter", "0.3.0", &key),
            ),
        ],
    );
    let ext = app(tmp.path(), &key, false).with_source(FolderRegistry::new(&reg));
    ext.install("acme.greeter", &VersionReq::parse("=0.2.0").unwrap())
        .unwrap();
    ext.installs()
        .set_pin("acme.greeter", Some("~0.2"))
        .unwrap();

    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    let mut needs = Requirements::default();
    needs.need(Requirement::new("acme.greeter").versions("=0.1.0"));
    vault_needs::write(&vault, &needs).unwrap();
    let plan = ext.open_vault(&vault).plan;
    assert!(plan.install.is_empty());
    assert!(
        matches!(&plan.missing[0].1, Missing::WantsOlder { installed, .. } if installed == "0.2.0"),
        "{plan:?}"
    );
    assert!(plan.missing[0].1.to_string().contains("wants an older one"));

    let mut needs = Requirements::default();
    needs.need(Requirement::new("acme.greeter").versions("=0.3.0"));
    vault_needs::write(&vault, &needs).unwrap();
    let plan = ext.open_vault(&vault).plan;
    assert!(
        matches!(&plan.missing[0].1, Missing::OutsidePin { pin, .. } if pin == "~0.2"),
        "{plan:?}"
    );

    // Not by name: refused. By name: the person's to do.
    let e = ext
        .install("acme.greeter", &VersionReq::parse("=0.1.0").unwrap())
        .unwrap_err();
    assert!(matches!(e, InstallError::Downgrade { .. }), "{e:?}");
    ext.install_with_in(
        "acme.greeter",
        &VersionReq::parse("=0.1.0").unwrap(),
        None,
        "person",
        false,
        &[],
        true,
    )
    .unwrap();
    assert_eq!(
        ext.installs()
            .get("acme.greeter")
            .unwrap()
            .version
            .to_string(),
        "0.1.0"
    );
}

#[test]
fn links_and_oversized_archives_are_refused_unread() {
    let tmp = tempfile::tempdir().unwrap();
    let key = SecretKey::generate().unwrap();
    let vault = tmp.path().join("vault");
    let mut needs = Requirements::default();
    needs.need(Requirement::new("show.big"));
    vault_needs::write(&vault, &needs).unwrap();
    // Too large: refused by its size, never opened.
    let dir = vault.join(vault_needs::CARRIED);
    std::fs::create_dir_all(&dir).unwrap();
    let f = std::fs::File::create(dir.join("big.archext")).unwrap();
    f.set_len(vault_needs::MAX_ARCHIVE + 1).unwrap();
    #[cfg(unix)]
    {
        let elsewhere = tmp.path().join("elsewhere.archext");
        std::fs::write(&elsewhere, signed(tmp.path(), "show.big", "0.1.0", &key)).unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.join("linked.archext")).unwrap();
    }
    let ext = app(tmp.path(), &key, false);
    let opened = ext.open_vault(&vault);
    assert!(
        opened
            .refused
            .iter()
            .any(|(f, why)| f == "big.archext" && why.contains("more than")),
        "{:?}",
        opened.refused
    );
    #[cfg(unix)]
    assert!(
        opened
            .refused
            .iter()
            .any(|(f, why)| f == "linked.archext" && why.contains("link")),
        "{:?}",
        opened.refused
    );
    assert!(opened.plan.install.is_empty());
}

#[test]
fn a_vault_that_names_nothing_needs_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let key = SecretKey::generate().unwrap();
    let ext = app(tmp.path(), &key, false);
    assert!(ext.open_vault(&tmp.path().join("empty")).is_empty());

    let bad = tmp.path().join("bad");
    std::fs::create_dir_all(bad.join(".architect")).unwrap();
    std::fs::write(bad.join(vault_needs::NEEDS), "extensions = 3").unwrap();
    let opened = ext.open_vault(&bad);
    assert!(opened.error.unwrap().contains(".architect/extensions.toml"));
}
