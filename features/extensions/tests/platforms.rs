//! Manifest platforms for every kind (#578): an extension whose
//! `platforms` leave this one out is listed as "Linux only: not on
//! macOS", never started (a linked one's `build` never runs), its data
//! kept; a `[requires]` on what it provides names that as the reason; the
//! store refuses to install it here with the same words.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::atomic::{AtomicUsize, Ordering};

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, LoadError, SharedSections};
use architect_extensions::install::{FolderRegistry, InstallError, Installs};
use architect_extensions::linked::{Builder, Extension};
use architect_settings::Registry;
use futures::executor::block_on;
use semver::VersionReq;
use serde_json::Value;

static PIPEWIRE_BUILDS: AtomicUsize = AtomicUsize::new(0);

/// A device family that runs on Linux only (`Patchbay`'s `PipeWire` engine).
struct PipeWire;

impl Extension for PipeWire {
    fn manifest(&self) -> &'static str {
        r#"
id = "lab.pipewire"
name = "PipeWire"
version = "0.1.0"
kind = "process"
platforms = ["linux"]

[entry]
path = "pipewire-bin"

[provides]
"lab.audio-engine" = "1.0.0"
"#
    }

    fn build(&self, _: &mut Builder) {
        PIPEWIRE_BUILDS.fetch_add(1, Ordering::SeqCst);
    }
}

/// What runs anywhere and needs an engine.
struct Patchbay;

impl Extension for Patchbay {
    fn manifest(&self) -> &'static str {
        r#"
id = "lab.patchbay"
name = "Patchbay"
version = "0.1.0"
kind = "process"

[entry]
path = "patchbay-bin"

[requires]
"lab.audio-engine" = "^1"
"#
    }

    fn build(&self, _: &mut Builder) {}
}

fn app(platform: &str, tmp: &std::path::Path) -> (ActionList, Extensions) {
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_data_root(tmp.join("data"));
    host.set_platform(platform);
    let ext = Extensions::new(host, Installs::new(tmp.join("installs")))
        .with_linked(&PipeWire)
        .unwrap()
        .with_linked(&Patchbay)
        .unwrap();
    ext.install_actions().unwrap();
    (list, ext)
}

fn ext_list(list: &ActionList) -> Value {
    let out = block_on(list.run_json_async("EXT_LIST", b"")).unwrap();
    serde_json::from_slice(&out).unwrap()
}

fn row<'a>(listing: &'a Value, id: &str) -> &'a Value {
    listing["extensions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id)
        .unwrap_or_else(|| panic!("{id} is not listed: {listing}"))
}

#[test]
fn a_linux_only_linked_extension_says_so_on_macos_and_its_build_never_runs() {
    // On Linux it runs, and so does what requires its service.
    let tmp = tempfile::tempdir().unwrap();
    let before = PIPEWIRE_BUILDS.load(Ordering::SeqCst);
    let (list, ext) = app("linux", tmp.path());
    let _ = ext.sync();
    assert!(ext.host().get("lab.pipewire").is_some());
    assert!(ext.host().get("lab.patchbay").is_some());
    assert!(PIPEWIRE_BUILDS.load(Ordering::SeqCst) > before);
    assert_eq!(
        row(&ext_list(&list), "lab.pipewire")["not_here"],
        Value::Null
    );

    // On macOS it is listed, not loaded, its build never run.
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data/lab.pipewire");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("routes.json"), "{}").unwrap();
    let before = PIPEWIRE_BUILDS.load(Ordering::SeqCst);
    let (list, ext) = app("macos", tmp.path());
    let events = ext.sync();
    assert_eq!(
        PIPEWIRE_BUILDS.load(Ordering::SeqCst),
        before,
        "its build ran off its platform"
    );
    assert!(ext.host().get("lab.pipewire").is_none());
    assert!(events.iter().any(|e| matches!(
        e,
        architect_extensions::host::Event::Failed { id, error: LoadError::NotHere { .. } }
            if id == "lab.pipewire"
    )));
    let listing = ext_list(&list);
    let pw = row(&listing, "lab.pipewire");
    assert_eq!(pw["not_here"], "Linux only: not on macOS", "{pw}");
    assert_eq!(pw["platforms"], serde_json::json!(["linux"]));
    assert_eq!(pw["loaded"], false);

    // What requires its service says why: the platform, not "missing".
    assert!(ext.host().get("lab.patchbay").is_none());
    let problems: Vec<String> = ext.problems().into_iter().map(|p| p.message).collect();
    let patchbay = problems
        .iter()
        .find(|p| p.starts_with("lab.patchbay:"))
        .unwrap_or_else(|| panic!("{problems:?}"));
    assert!(
        patchbay.contains("PipeWire is Linux only: not on macOS"),
        "{patchbay}"
    );
    // `APP_STATUS`'s extensions source says it too.
    assert!(
        problems
            .iter()
            .any(|p| p == "lab.pipewire: Linux only: not on macOS"),
        "{problems:?}"
    );

    // A second sync does not try it again, and its data is where it was.
    let again = ext.sync();
    assert!(
        !again.iter().any(|e| matches!(
            e,
            architect_extensions::host::Event::Failed { id, .. } if id == "lab.pipewire"
        )),
        "{again:?}"
    );
    assert_eq!(
        std::fs::read_to_string(data.join("routes.json")).unwrap(),
        "{}"
    );
}

#[test]
fn a_process_extension_in_an_app_with_no_process_runner_is_not_here_either() {
    // The web and iOS hosts run no processes: the same state, not a crash.
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("desk-only");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(
        pkg.join("extension.toml"),
        "id = \"lab.desk\"\nname = \"Desk only\"\nversion = \"0.1.0\"\nkind = \"process\"\n\n[entry]\npath = \"desk-bin\"\n",
    )
    .unwrap();
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    host.set_platform("ios");
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")));
    ext.install_actions().unwrap();
    let _ = ext.dev(&pkg);
    let _ = ext.sync();
    assert!(ext.host().get("lab.desk").is_none());
    let listing = ext_list(&list);
    let desk = row(&listing, "lab.desk");
    assert_eq!(
        desk["not_here"], "this app does not run process extensions",
        "{desk}"
    );
}

#[test]
fn the_store_refuses_to_install_an_extension_not_for_this_platform() {
    let tmp = tempfile::tempdir().unwrap();
    let registry = tmp.path().join("registry");
    let pkg = registry.join("linux-tool");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(
        pkg.join("extension.toml"),
        "id = \"lab.linux-tool\"\nname = \"Linux tool\"\nversion = \"0.1.0\"\nkind = \"data\"\nplatforms = [\"linux\"]\n",
    )
    .unwrap();
    std::fs::write(
        registry.join("index.toml"),
        "name = \"test\"\n\n[[extensions]]\nid = \"lab.linux-tool\"\nname = \"Linux tool\"\nversion = \"0.1.0\"\nkind = \"data\"\npath = \"linux-tool\"\n",
    )
    .unwrap();

    let list = ActionList::new();
    let host = Host::new(list, SharedSections::new(Registry::new()));
    host.set_platform("windows");
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_source(FolderRegistry::new(&registry));
    let err = ext
        .install("lab.linux-tool", &VersionReq::STAR)
        .unwrap_err();
    assert_eq!(
        err,
        InstallError::NotHere {
            platform: "windows".into(),
            why: "Linux only".into(),
        }
    );
    assert_eq!(err.to_string(), "Linux only: not on Windows");
    assert!(
        ext.installs().get("lab.linux-tool").is_none(),
        "nothing written"
    );

    // Where it runs, it installs.
    let tmp2 = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list, SharedSections::new(Registry::new()));
    host.set_platform("linux");
    let ext = Extensions::new(host, Installs::new(tmp2.path().join("installs")))
        .with_source(FolderRegistry::new(&registry));
    ext.install("lab.linux-tool", &VersionReq::STAR).unwrap();
}

#[test]
fn an_unknown_platform_is_refused_by_the_manifest_check() {
    let e = architect_extensions::manifest::Manifest::parse(
        "id = \"x\"\nname = \"X\"\nversion = \"0.1.0\"\nkind = \"data\"\nplatforms = [\"darwin\"]\n",
    )
    .unwrap_err()
    .to_string();
    assert!(e.contains("darwin") && e.contains("macos"), "{e}");
}
