//! Installs where there is no folder (#495): a browser keeps installs,
//! grants and installed packages in its storage, through the same install
//! path a desk takes (`Installs`, `Grants`, `Extensions::open` /
//! `accept`), on a `MemDisk` whose files the web app keeps. A registry
//! compiled into the app hands its packages over as files; nothing needs
//! `std::fs`. What the browser has no runner for is "Can't run here" with
//! why, before anything installs.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use architect::action::ActionList;
use architect_extensions::bundled::{BundledPackage, BundledSource};
use architect_extensions::consent::{Grants, Scope};
use architect_extensions::disk::{Disk, MemDisk};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_extensions::manage::Installed;
use architect_extensions::manifest::Kind;
use architect_extensions::package::Files;
use architect_extensions::project::{Missing, Requirement, Requirements};
use architect_settings::Registry;

const NOTES: &[u8] = br#"id = "x.notes"
name = "Notes"
version = "0.1.0"
kind = "data"

[capabilities]
ui = ["notify"]
"#;

const PAGE: &[u8] = b"export default function Notes() { return <div>notes</div>; }\n";

const CLICKER: &[u8] = br#"id = "x.clicker"
name = "Clicker"
version = "0.1.0"
kind = "process"

[entry]
path = "bin/clicker"
"#;

fn source() -> BundledSource {
    BundledSource::new(
        "Built in",
        vec![
            BundledPackage {
                folder: "notes".into(),
                files: vec![
                    ("extension.toml".into(), NOTES),
                    ("pages/notes.tsx".into(), PAGE),
                ],
            },
            BundledPackage {
                folder: "clicker".into(),
                files: vec![("extension.toml".into(), CLICKER)],
            },
        ],
    )
}

/// A browser's app: its installs and grants on `disk`, the web platform.
fn app(disk: &MemDisk) -> Extensions {
    let disk: Arc<dyn Disk> = Arc::new(disk.clone());
    let host = Host::new(ActionList::new(), SharedSections::new(Registry::new()))
        .with_grants(Grants::open_on(Arc::clone(&disk), "/architect/grants.json"));
    host.set_platform("web");
    Extensions::new(host, Installs::new("/architect/extensions").with_disk(disk))
        .with_source(source())
}

fn needs() -> Requirements {
    Requirements {
        extensions: vec![Requirement::new("x.notes"), Requirement::new("x.clicker")],
    }
}

#[test]
fn a_browser_installs_through_one_consent_and_keeps_it_in_its_storage() {
    // What the web app keeps in the browser's storage: every change.
    let kept = Arc::new(Mutex::new(BTreeMap::new()));
    let k = Arc::clone(&kept);
    let disk = MemDisk::new().on_change(move |files| {
        *k.lock().unwrap() = files.clone();
    });
    let ext = app(&disk);
    let plan = ext.open(&needs());
    let offered: Vec<&str> = plan.install.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(offered, ["x.notes"], "{plan:?}");
    // The process cannot run in a browser: said before anything installs.
    let (_, why) = plan
        .missing
        .iter()
        .find(|(r, _)| r.id == "x.clicker")
        .expect("the process is listed as what cannot be had");
    let Missing::CannotRun(why) = why else {
        panic!("{why:?}");
    };
    assert!(why.contains("desk"), "{why}");

    // The prompt reads each from the built-in registry, nothing written.
    let previews = ext.previews_for(None, &plan);
    assert!(previews.iter().all(Result::is_ok), "{previews:?}");
    assert!(kept.lock().unwrap().is_empty());

    // Nothing before the one yes.
    let waiting = ext.accept(&plan, None, "person");
    assert!(
        waiting
            .iter()
            .all(|r| matches!(r, Ok(Installed::NeedsConsent { .. }))),
        "{waiting:?}"
    );
    assert!(ext.installs().get("x.notes").is_none());

    let done = ext.accept(&plan, Some(&Scope::Device), "person");
    assert!(
        matches!(done.as_slice(), [Ok(Installed::Done { loaded: true, .. })]),
        "{done:?}"
    );
    // Running, read from the browser's storage.
    let loaded = ext.host().get("x.notes").expect("loaded");
    assert_eq!(loaded.dir, None, "no folder of the operating system's");
    let package = ext
        .chosen()
        .into_iter()
        .find(|c| c.runs.id == "x.notes")
        .unwrap()
        .runs
        .package;
    assert!(matches!(package.files, Files::Disk(_)));
    assert_eq!(package.text("pages/notes.tsx").unwrap().as_bytes(), PAGE);
    assert!(package.text("../grants.json").is_err());
    let texts = package.texts().unwrap();
    assert!(
        texts.iter().any(|(p, _)| p == "pages/notes.tsx"),
        "{texts:?}"
    );

    // Its install and its grant are what the browser keeps.
    let files = kept.lock().unwrap().clone();
    assert!(
        files.contains_key("architect/extensions/x.notes/installed.toml"),
        "{:?}",
        files.keys()
    );
    assert!(files.contains_key("architect/extensions/x.notes/pages/notes.tsx"));
    assert!(files.contains_key("architect/grants.json"));
    assert!(
        !files.keys().any(|f| f.contains(".incoming")),
        "{:?}",
        files.keys()
    );

    // The page loads again: the same, from what was kept, no new prompt.
    let again = app(&MemDisk::with_files(files));
    for e in again.sync() {
        if let architect_extensions::host::Event::Failed { id, error } = e {
            panic!("{id}: {error}");
        }
    }
    assert!(
        again.host().get("x.notes").is_some(),
        "running after a reload"
    );
    assert!(
        again
            .host()
            .grants()
            .unwrap()
            .needs("x.notes", &loaded.manifest.capabilities)
            .is_empty()
    );
    assert!(
        again.open(&needs()).install.is_empty(),
        "nothing left to install"
    );

    // Removed: gone from the storage too.
    assert!(again.uninstall("x.notes").unwrap());
    assert!(again.installs().get("x.notes").is_none());
}

#[test]
fn what_a_browser_has_no_runner_for_says_why() {
    use architect_extensions::host::{kind_cannot_run, why_no_runner};
    let luau = why_no_runner(Kind::Luau, "web");
    assert!(luau.contains("Luau") && luau.contains("#812"), "{luau}");
    assert_eq!(kind_cannot_run(Kind::Luau, false, "web"), Some(luau));
    // A browser that runs it, and a desk, say nothing here.
    assert_eq!(kind_cannot_run(Kind::TypeScript, true, "web"), None);
    assert_eq!(kind_cannot_run(Kind::Process, false, "linux"), None);
}
