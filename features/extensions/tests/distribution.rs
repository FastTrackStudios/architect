//! Distribution (#182): signed `.archext` archives from a registry, a
//! file or a peer. A good one installs and loads; an unsigned, tampered or
//! wrong-key one is refused with the reason, before anything is written;
//! a process extension with no build for this platform says so in its
//! preview; an update check says what is newer, within the pin, with its
//! changelog line.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};

use architect::action::{ActionCaller, ActionList};
use architect_extension_archive::sign::{SecretKey, sign_archive};
use architect_extension_archive::{ArchiveError, entries, pack, write_entries};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::{FolderRegistry, InstallError, Installs, Trust};
use architect_extensions::manage::Installed;
use architect_settings::Registry;
use futures::executor::block_on;
use semver::VersionReq;
use serde_json::{Value, json};
use sha2::Digest as _;

const ID: &str = "example.hello";

/// The example's Luau extension, `examples/extension/registry/hello`.
fn hello() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extension/registry/hello")
}

/// `hello` copied, at `version`.
fn hello_at(tmp: &Path, version: &str) -> PathBuf {
    let dir = tmp.join(format!("src-{version}"));
    std::fs::create_dir_all(&dir).unwrap();
    for e in std::fs::read_dir(hello()).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), dir.join(e.file_name())).unwrap();
    }
    let manifest = std::fs::read_to_string(dir.join("extension.toml"))
        .unwrap()
        .replace("version = \"0.1.0\"", &format!("version = \"{version}\""));
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    dir
}

/// A registry folder listing each `(version, archive bytes, changelog)`.
fn registry(tmp: &Path, archives: &[(&str, Vec<u8>, &str)]) -> PathBuf {
    let root = tmp.join("registry");
    std::fs::create_dir_all(root.join("archives")).unwrap();
    let mut index = String::from("name = \"Signed\"\n");
    for (version, bytes, changelog) in archives {
        let file = format!("archives/{ID}-{version}.archext");
        std::fs::write(root.join(&file), bytes).unwrap();
        let sha = hex::encode(sha2::Sha256::digest(bytes));
        index.push_str(&format!(
            "\n[[extensions]]\nid = \"{ID}\"\nname = \"Hello\"\nversion = \"{version}\"\n\
             kind = \"luau\"\narchive = \"{file}\"\nsha256 = \"{sha}\"\nchangelog = \"{changelog}\"\n"
        ));
    }
    std::fs::write(root.join("index.toml"), index).unwrap();
    root
}

fn signed(dir: &Path, key: &SecretKey) -> Vec<u8> {
    sign_archive(&pack(dir).unwrap().1, key).unwrap()
}

fn app(tmp: &Path, source: FolderRegistry) -> (ActionList, Extensions) {
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.join("installs"))).with_source(source);
    ext.install_actions().unwrap();
    (list, ext)
}

fn run(list: &ActionList, id: &str, args: &Value) -> Result<Value, String> {
    block_on(list.run_json_async_with(&ActionCaller::app(), id, args.to_string().as_bytes()))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
}

fn refused(e: InstallError) -> ArchiveError {
    match e {
        InstallError::Archive(a) => a,
        other => panic!("refused, but not as an archive: {other:?}"),
    }
}

#[test]
fn a_signed_archive_from_a_registry_installs_and_loads() {
    let tmp = tempfile::tempdir().unwrap();
    let key = SecretKey::generate().unwrap();
    let root = registry(
        tmp.path(),
        &[("0.1.0", signed(&hello_at(tmp.path(), "0.1.0"), &key), "First.")],
    );
    let (list, ext) = app(
        tmp.path(),
        FolderRegistry::new(&root).trusting("the test registry", key.public()),
    );
    let preview = ext.preview(ID, &VersionReq::STAR).unwrap();
    assert_eq!(preview.signed_by.as_deref(), Some("the test registry"));
    assert!(preview.runs_here);
    assert_eq!(preview.cannot_run, None);
    let out = run(&list, "EXT_INSTALL", &json!({ "id": ID })).unwrap();
    assert_eq!(out["version"], "0.1.0", "{out}");
    assert_eq!(out["loaded"], true, "{out}");
    let installed = ext.installs().get(ID).unwrap();
    assert!(!installed.linked, "an archive is unpacked, not linked");
    assert!(installed.dir.join("greet.luau").exists());
    assert!(!installed.dir.join("archext.json").exists());
}

#[test]
fn an_unsigned_archive_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let key = SecretKey::generate().unwrap();
    let unsigned = pack(&hello_at(tmp.path(), "0.1.0")).unwrap().1;
    let root = registry(tmp.path(), &[("0.1.0", unsigned, "")]);
    let (_, ext) = app(tmp.path(), FolderRegistry::new(&root).trusting("k", key.public()));
    let e = refused(ext.install(ID, &VersionReq::STAR).unwrap_err());
    assert!(matches!(e, ArchiveError::Unsigned { .. }), "{e:?}");
    // The preview is refused the same way: nothing unsigned is read as if
    // it were what it says.
    assert!(ext.preview(ID, &VersionReq::STAR).is_err());
    assert!(ext.installs().get(ID).is_none());
}

#[test]
fn a_tampered_archive_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let key = SecretKey::generate().unwrap();
    let mut files = entries(&signed(&hello_at(tmp.path(), "0.1.0"), &key)).unwrap();
    let (_, script) = files.iter_mut().find(|(p, _)| p == "greet.luau").unwrap();
    script[0] ^= 0x01;
    let tampered = write_entries(&files).unwrap();
    // Listed with the tampered archive's own hash: only the signature
    // stands between it and an install.
    let root = registry(tmp.path(), &[("0.1.0", tampered, "")]);
    let (_, ext) = app(tmp.path(), FolderRegistry::new(&root).trusting("k", key.public()));
    let e = refused(ext.install(ID, &VersionReq::STAR).unwrap_err());
    assert!(
        matches!(&e, ArchiveError::FileTampered { path, .. } if path == "greet.luau"),
        "{e:?}"
    );
    assert!(ext.installs().get(ID).is_none());
}

#[test]
fn a_wrong_key_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let signer = SecretKey::generate().unwrap();
    let trusted = SecretKey::generate().unwrap();
    let root = registry(
        tmp.path(),
        &[("0.1.0", signed(&hello_at(tmp.path(), "0.1.0"), &signer), "")],
    );
    let (_, ext) = app(
        tmp.path(),
        FolderRegistry::new(&root).trusting("k", trusted.public()),
    );
    let e = refused(ext.install(ID, &VersionReq::STAR).unwrap_err());
    match e {
        ArchiveError::UntrustedKey { key_id, trusted: names } => {
            assert_eq!(key_id, signer.key_id());
            // The org key is trusted by default, and the registry's own.
            assert_eq!(names.len(), 2, "{names:?}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_pinned_hash_that_does_not_match_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let key = SecretKey::generate().unwrap();
    let root = registry(
        tmp.path(),
        &[("0.1.0", signed(&hello_at(tmp.path(), "0.1.0"), &key), "")],
    );
    // Another (validly signed) archive put in its place.
    std::fs::write(
        root.join(format!("archives/{ID}-0.1.0.archext")),
        signed(&hello_at(tmp.path(), "0.1.0"), &SecretKey::generate().unwrap()),
    )
    .unwrap();
    let (_, ext) = app(tmp.path(), FolderRegistry::new(&root).trusting("k", key.public()));
    let e = ext.install(ID, &VersionReq::STAR).unwrap_err();
    assert!(
        matches!(&e, InstallError::Mismatch(m) if m.contains("sha256")),
        "{e:?}"
    );
}

#[test]
fn an_archive_from_a_file_is_checked_the_same_way() {
    let tmp = tempfile::tempdir().unwrap();
    let key = SecretKey::generate().unwrap();
    let file = tmp.path().join("hello.archext");
    std::fs::write(&file, signed(&hello_at(tmp.path(), "0.1.0"), &key)).unwrap();
    // Trusting only the org key: refused.
    let (list, _) = app(tmp.path(), FolderRegistry::new(tmp.path().join("none")));
    let e = run(&list, "EXT_INSTALL_FILE", &json!({ "path": file })).unwrap_err();
    assert!(e.contains("does not trust"), "{e}");
    // Trusting the key: installed and loaded.
    let tmp2 = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    let installs = Installs::new(tmp2.path().join("installs"))
        .with_trust(Trust::org().with("a peer's key", key.public()));
    let ext = Extensions::new(host, installs);
    ext.install_actions().unwrap();
    let out = run(&list, "EXT_INSTALL_FILE", &json!({ "path": file })).unwrap();
    assert_eq!(out["id"], ID, "{out}");
    assert_eq!(out["loaded"], true, "{out}");
    // And through the method, as a peer's bytes.
    let again = ext
        .install_archive(std::fs::read(&file).unwrap(), "peer bob", None, "person")
        .unwrap();
    assert!(matches!(again, Installed::Done { .. }));
    assert_eq!(ext.installs().get(ID).unwrap().source, "peer bob");
}

#[test]
fn no_build_for_this_platform_is_said_before_installing() {
    let tmp = tempfile::tempdir().unwrap();
    let key = SecretKey::generate().unwrap();
    let dir = tmp.path().join("tool");
    // A process extension built only for a platform no machine runs.
    std::fs::create_dir_all(dir.join("bin/riscv64gc-unknown-none-elf")).unwrap();
    std::fs::write(
        dir.join("extension.toml"),
        "id = \"acme.tool\"\nname = \"Tool\"\nversion = \"1.0.0\"\nkind = \"process\"\n\
         [entry]\npath = \"bin/tool\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("bin/riscv64gc-unknown-none-elf/tool"), "x").unwrap();
    let archive = signed(&dir, &key);
    let root = tmp.path().join("registry");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("tool.archext"), &archive).unwrap();
    std::fs::write(
        root.join("index.toml"),
        "[[extensions]]\nid = \"acme.tool\"\nversion = \"1.0.0\"\nkind = \"process\"\n\
         archive = \"tool.archext\"\nplatforms = [\"riscv64gc-unknown-none-elf\"]\n",
    )
    .unwrap();
    let (_, ext) = app(tmp.path(), FolderRegistry::new(&root).trusting("k", key.public()));
    let preview = ext.preview("acme.tool", &VersionReq::STAR).unwrap();
    assert!(!preview.runs_here);
    let why = preview.cannot_run.unwrap();
    assert!(why.starts_with("no build for this platform"), "{why}");
    assert!(why.contains("riscv64gc-unknown-none-elf"), "{why}");
    let e = refused(ext.install("acme.tool", &VersionReq::STAR).unwrap_err());
    assert!(matches!(e, ArchiveError::NoBuild { .. }), "{e:?}");
}

#[test]
fn an_update_check_says_what_is_newer_within_the_pin() {
    let tmp = tempfile::tempdir().unwrap();
    let key = SecretKey::generate().unwrap();
    let root = registry(
        tmp.path(),
        &[
            ("0.1.0", signed(&hello_at(tmp.path(), "0.1.0"), &key), "First."),
            ("0.2.0", signed(&hello_at(tmp.path(), "0.2.0"), &key), "Louder."),
        ],
    );
    let (list, ext) = app(tmp.path(), FolderRegistry::new(&root).trusting("k", key.public()));
    run(&list, "EXT_INSTALL", &json!({ "id": ID, "version": "=0.1.0" })).unwrap();
    let out = run(&list, "EXT_UPDATE", &json!({ "check": true })).unwrap();
    let a = &out["available"][0];
    assert_eq!(a["id"], ID, "{out}");
    assert_eq!(a["installed"], "0.1.0");
    assert_eq!(a["newest"], "0.2.0");
    assert_eq!(a["changelog"], "Louder.");
    assert_eq!(a["asks_more"], json!([]));
    // Nothing was installed by the check.
    assert_eq!(ext.installs().get(ID).unwrap().version.to_string(), "0.1.0");

    // Pinned at 0.1: the newer one is past the pin, not offered.
    run(&list, "EXT_PIN", &json!({ "id": ID })).unwrap();
    let out = run(&list, "EXT_UPDATE", &json!({ "check": true })).unwrap();
    let a = &out["available"][0];
    assert_eq!(a["newest"], "0.1.0", "{out}");
    assert_eq!(a["past_pin"], "0.2.0", "{out}");

    // Unpinned, the update applies.
    run(&list, "EXT_PIN", &json!({ "id": ID, "unpin": true })).unwrap();
    let out = run(&list, "EXT_UPDATE", &json!({})).unwrap();
    assert_eq!(out["updated"][0]["to"], "0.2.0", "{out}");
}
