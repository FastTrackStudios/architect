//! Registries a person adds, with their publishers' keys, and development
//! installs (#493): a package signed by a key its registry trusts
//! installs; one signed by another key is refused; a file's archive is
//! checked against every registered publisher; the registries, the
//! development switch and the development folders survive a restart
//! (the settings file round-trips); `EXT_DEV` loads a folder only with
//! development installs on.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use architect::action::{ActionCaller, ActionList};
use architect_extension_archive::sign::{SecretKey, sign_archive};
use architect_extension_archive::{ArchiveError, pack};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::{InstallError, Installs};
use architect_extensions::registries::{Registries, Registry};
use architect_settings::Registry as Sections;
use futures::executor::block_on;
use semver::VersionReq;
use serde_json::{Value, json};
use sha2::Digest as _;

const ID: &str = "example.hello";

fn hello() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extension/registry/hello")
}

/// `hello` copied into `tmp/name`.
fn package(tmp: &Path, name: &str) -> PathBuf {
    let dir = tmp.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    for e in std::fs::read_dir(hello()).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), dir.join(e.file_name())).unwrap();
    }
    dir
}

fn signed(dir: &Path, key: &SecretKey) -> Vec<u8> {
    sign_archive(&pack(dir).unwrap().1, key).unwrap()
}

/// A folder registry `name` listing `bytes` as hello 0.1.0.
fn registry_folder(tmp: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let root = tmp.join(name);
    std::fs::create_dir_all(root.join("archives")).unwrap();
    let file = format!("archives/{ID}-0.1.0.archext");
    std::fs::write(root.join(&file), bytes).unwrap();
    let mut index = format!("name = \"{name}\"\n");
    writeln!(
        index,
        "\n[[extensions]]\nid = \"{ID}\"\nname = \"Hello\"\nversion = \"0.1.0\"\n\
         kind = \"luau\"\narchive = \"{file}\"\nsha256 = \"{}\"",
        hex::encode(sha2::Sha256::digest(bytes))
    )
    .unwrap();
    std::fs::write(root.join("index.toml"), index).unwrap();
    root
}

fn app(tmp: &Path, registries: Registries) -> (ActionList, Extensions) {
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Sections::new()));
    let ext = Extensions::new(host, Installs::new(tmp.join("installs"))).with_registries(registries);
    ext.install_actions().unwrap();
    (list, ext)
}

fn run(list: &ActionList, id: &str, args: &Value) -> Result<Value, String> {
    block_on(list.run_json_async_with(&ActionCaller::app(), id, args.to_string().as_bytes()))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
}

#[test]
fn a_package_signed_by_a_key_its_registry_trusts_installs() {
    let tmp = tempfile::tempdir().unwrap();
    let studio = SecretKey::generate().unwrap();
    let root = registry_folder(
        tmp.path(),
        "studio",
        &signed(&package(tmp.path(), "src"), &studio),
    );
    let registries = Registries::memory(tmp.path().join("checkouts"));
    let (list, ext) = app(tmp.path(), registries);
    let added = run(
        &list,
        "EXT_REGISTRY_ADD",
        &json!({
            "name": "Studio",
            "folder": root.display().to_string(),
            "keys": [{ "name": "Studio", "key": studio.public() }],
        }),
    )
    .unwrap();
    assert_eq!(added["ok"], true, "{added}");
    let preview = ext.preview(ID, &VersionReq::STAR).unwrap();
    assert_eq!(preview.signed_by.as_deref(), Some("Studio"));
    let out = run(&list, "EXT_INSTALL", &json!({ "id": ID })).unwrap();
    assert_eq!(out["loaded"], true, "{out}");
}

#[test]
fn an_untrusted_publisher_is_refused_from_its_registry_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let (studio, stranger) = (SecretKey::generate().unwrap(), SecretKey::generate().unwrap());
    let root = registry_folder(
        tmp.path(),
        "studio",
        &signed(&package(tmp.path(), "src"), &stranger),
    );
    let registries = Registries::memory(tmp.path().join("checkouts"))
        .with_builtin(Registry::folder("Studio", &root).trusting("Studio", studio.public()));
    let (_, ext) = app(tmp.path(), registries);
    match ext.install(ID, &VersionReq::STAR) {
        Err(InstallError::Archive(ArchiveError::UntrustedKey { .. })) => {}
        other => panic!("not refused as untrusted: {other:?}"),
    }
    assert!(ext.installs().get(ID).is_none());
}

#[test]
fn a_files_archive_is_checked_against_the_registered_publishers() {
    let tmp = tempfile::tempdir().unwrap();
    let (studio, stranger) = (SecretKey::generate().unwrap(), SecretKey::generate().unwrap());
    let src = package(tmp.path(), "src");
    let good = tmp.path().join("good.archext");
    std::fs::write(&good, signed(&src, &studio)).unwrap();
    let bad = tmp.path().join("bad.archext");
    std::fs::write(&bad, signed(&src, &stranger)).unwrap();
    let root = registry_folder(tmp.path(), "studio", b"");
    let registries = Registries::memory(tmp.path().join("checkouts"))
        .with_builtin(Registry::folder("Studio", &root).trusting("Studio", studio.public()));
    let (list, _) = app(tmp.path(), registries);
    let refused = run(
        &list,
        "EXT_INSTALL_FILE",
        &json!({ "path": bad.display().to_string() }),
    )
    .unwrap_err();
    assert!(refused.contains("trust") || refused.contains("key"), "{refused}");
    let out = run(
        &list,
        "EXT_INSTALL_FILE",
        &json!({ "path": good.display().to_string() }),
    )
    .unwrap();
    assert_eq!(out["loaded"], true, "{out}");
}

#[test]
fn registries_and_development_installs_round_trip_through_the_settings_file() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("extension-registries.json");
    let key = SecretKey::generate().unwrap();
    let dev = package(tmp.path(), "dev-hello");
    {
        let registries = Registries::open(&file, tmp.path().join("checkouts"))
            .with_builtin(Registry::git("FastTrackStudios", "https://example.invalid/x.git"));
        registries
            .add(Registry::git("Studio", "https://example.invalid/studio.git").trusting("Studio", key.public()))
            .unwrap();
        registries.set_dev_installs(true).unwrap();
        let (list, ext) = app(tmp.path(), registries);
        let out = run(&list, "EXT_DEV", &json!({ "folder": dev.display().to_string() })).unwrap();
        assert_eq!(out["id"], ID, "{out}");
        assert_eq!(out["loaded"], true, "{out}");
        assert!(ext.host().get(ID).is_some());
    }
    // The app's own registry is never written; the person's is.
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(!text.contains("FastTrackStudios"), "{text}");
    let again = Registries::open(&file, tmp.path().join("checkouts"));
    let listed = again.list();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].registry.name, "Studio");
    assert_eq!(listed[0].registry.keys[0].key, key.public());
    assert!(again.dev_installs());
    assert_eq!(again.dev_folders(), vec![std::fs::canonicalize(&dev).unwrap()]);
    // On the next start the development copy loads again.
    let (list, ext) = app(&tmp.path().join("next"), again);
    let _ = ext.sync();
    assert!(ext.host().get(ID).is_some(), "the development copy loads on start");
    let listed = run(&list, "EXT_REGISTRIES", &json!(null)).unwrap();
    assert_eq!(listed["registries"][0]["name"], "Studio", "{listed}");
    assert_eq!(listed["dev_installs"], true);
}

#[test]
fn ext_dev_is_refused_with_development_installs_off() {
    let tmp = tempfile::tempdir().unwrap();
    let dev = package(tmp.path(), "dev-hello");
    let (list, ext) = app(tmp.path(), Registries::memory(tmp.path().join("checkouts")));
    let e = run(&list, "EXT_DEV", &json!({ "folder": dev.display().to_string() })).unwrap_err();
    assert!(e.contains("development installs are off"), "{e}");
    assert!(ext.host().get(ID).is_none());
    // A debug build has them on whatever the setting says.
    let forced = Registries::memory(tmp.path().join("checkouts")).with_dev_forced(true);
    let (list, ext) = app(&tmp.path().join("forced"), forced);
    run(&list, "EXT_DEV", &json!({ "folder": dev.display().to_string() })).unwrap();
    assert!(ext.host().get(ID).is_some());
}

#[test]
fn a_registry_with_a_bad_key_or_no_place_is_not_added() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Registries::memory(tmp.path());
    assert!(r.add(Registry::git("X", "https://x").trusting("X", "not a key")).is_err());
    assert!(r.add(Registry { name: "Y".into(), git: None, folder: None, keys: vec![] }).is_err());
    assert!(r.list().is_empty());
}
