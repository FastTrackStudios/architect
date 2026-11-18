//! An extension's `kind = "process"` view (#838), not linked: a tiny
//! `data` extension whose only view is the lab's demo program, loaded as
//! an installed copy (refused until its program is granted, then its
//! frames arrive), stopped when it unloads, then loaded as a development
//! copy (frames again, no grant needed) and started again when its
//! program's file changes, as `EXT_DEV`'s reload does.
//!
//! Frames over shared memory, with no window: the session the app's pane
//! would start (`extension::open`), greeted as a pane's widget greets it.
//! Needs a Vulkan adapter for the program to draw on; without one (a CI
//! runner with no GPU) it says so and passes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc,
    clippy::arithmetic_side_effects
)]
#![cfg(all(
    target_os = "linux",
    feature = "guest",
    feature = "host",
    feature = "demo",
    feature = "extension"
))]

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::consent::{Grants, Scope};
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::manifest::Capabilities;
use architect_extensions::{Origin, Package};
use architect_process_view::extension::{self, PREFIX};
use architect_process_view::host::{Session, Status, find};
use architect_process_view::wire::{Adapter, Transport};
use architect_workspace::{KindSpec, ViewKinds};

const ID: &str = "lab.orbits";
const VIEW: &str = "orbits";
const PANE: &str = "pane-1";
const PROGRAM: &str = "bin/orbits";

/// The package: the manifest and the demo program, copied in (a program
/// runs from its package only: a link out of it is refused).
fn package(dir: &Path) {
    package_with(dir, "", "");
}

/// [`package`], its view's `args` line `args` and `tail` after the view.
fn package_with(dir: &Path, args: &str, tail: &str) {
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    let bin = dir.join(PROGRAM);
    std::fs::copy(env!("CARGO_BIN_EXE_process-view-demo"), &bin).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let manifest = format!(
        r#"id = "{ID}"
name = "Orbits"
version = "1.0.0"
kind = "data"

[capabilities]
programs = ["{PROGRAM}"]

[[views]]
id = "{VIEW}"
title = "Orbits"
kind = "process"
program = "{PROGRAM}"
{args}
{tail}
"#
    );
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
}

fn host(grants: &Grants) -> Host {
    let mut kinds = ViewKinds::new();
    kinds.register(KindSpec::process()).unwrap();
    Host::new(ActionList::new(), SharedSections::default())
        .with_view_kinds(kinds)
        .with_grants(grants.clone())
}

/// What happened when a pane showed the view.
enum Shown {
    Frames,
    /// The program could not draw here (no GPU), and why.
    NoGpu(String),
}

/// Greets `session` as a pane would and takes its frames until `n` came.
fn frames(session: &Arc<Session>, n: usize) -> Shown {
    let adapter = Adapter {
        vendor: 0,
        device: 0,
        name: "test".to_owned(),
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut got = 0;
    while got < n {
        session.greet_without_window(320, 200, adapter.clone());
        got += session.take_frames_without_window();
        if let Status::Stopped(why) = session.snapshot().status
            && got == 0
            && session.snapshot().generation >= 4
        {
            // Started and restarted as a process extension is (three
            // more times), and never drew: no GPU here.
            return Shown::NoGpu(why);
        }
        assert!(
            Instant::now() < deadline,
            "{n} frames within 30 s: {got}, {:?}",
            session.snapshot().status
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let snap = session.snapshot();
    assert_eq!(snap.status, Status::Running);
    assert_eq!(snap.transport, Some(Transport::Shm));
    assert_eq!(snap.size, Some((320, 200)));
    Shown::Frames
}

/// The sessions are the process's (one app, one host): a test's `reap`
/// would end another's, so they take turns.
fn alone() -> std::sync::MutexGuard<'static, ()> {
    static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    ONE.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn key() -> String {
    extension::key(ID, VIEW, PANE)
}

#[test]
fn an_installed_then_development_copy_shows_its_process_view() {
    let _alone = alone();
    let tmp = tempfile::tempdir().unwrap();
    let installed = tmp.path().join("installed");
    package(&installed);
    let grants = Grants::in_memory();
    let host = host(&grants);

    // Installed: loads, and its view says why until its program is granted.
    host.load(&Package::read(&installed).unwrap(), Origin::Installed)
        .unwrap();
    let refused = extension::open(&host, ID, VIEW, PANE).unwrap_err();
    assert!(refused.contains("not granted"), "{refused}");
    assert!(find(&key()).is_none(), "nothing started");
    grants.grant(
        ID,
        "1.0.0",
        Capabilities {
            programs: vec![PROGRAM.to_owned()],
            ..Capabilities::default()
        },
        Scope::Device,
        "test",
    );
    let session = extension::open(&host, ID, VIEW, PANE).unwrap();
    assert!(session.key().starts_with(PREFIX));
    match frames(&session, 3) {
        Shown::Frames => {}
        Shown::NoGpu(why) => {
            eprintln!("the program could not draw here (no GPU?): {why}: skipped");
            extension::reap(&host);
            return;
        }
    }
    let first = session.snapshot().pid;

    // Its grant taken back: the next check stops it.
    grants.revoke(ID, None);
    let ended = extension::reap(&host);
    assert_eq!(ended, [key()]);
    assert!(matches!(session.snapshot().status, Status::Stopped(_)));
    grants.grant(
        ID,
        "1.0.0",
        Capabilities {
            programs: vec![PROGRAM.to_owned()],
            ..Capabilities::default()
        },
        Scope::Device,
        "test",
    );

    // Unloaded: stopped, its session gone.
    let session = extension::open(&host, ID, VIEW, PANE).unwrap();
    assert!(matches!(frames(&session, 1), Shown::Frames));
    assert!(host.unload(ID));
    assert_eq!(extension::reap(&host), [key()]);
    assert!(find(&key()).is_none());
    assert!(matches!(session.snapshot().status, Status::Stopped(_)));

    // A development copy: no grant asked (it runs with what it asks).
    let dev = tmp.path().join("dev");
    package(&dev);
    grants.revoke(ID, None);
    host.load(&Package::read(&dev).unwrap(), Origin::Dev)
        .unwrap();
    let session = extension::open(&host, ID, VIEW, PANE).unwrap();
    assert!(matches!(frames(&session, 3), Shown::Frames));
    let before = session.snapshot();
    assert_ne!(before.pid, first, "a program of its own");

    // Rebuilt: a new file; the reload (`EXT_DEV`'s) starts it again in the
    // same pane's session.
    std::thread::sleep(Duration::from_millis(20));
    let bin = dev.join(PROGRAM);
    let mut bytes = std::fs::read(&bin).unwrap();
    bytes.extend_from_slice(b"rebuilt");
    // As a build does: a new file put in its place (the old one runs).
    let fresh = dev.join("bin/.orbits.new");
    std::fs::write(&fresh, bytes).unwrap();
    std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&fresh, &bin).unwrap();
    host.load(&Package::read(&dev).unwrap(), Origin::Dev)
        .unwrap();
    let again = extension::open(&host, ID, VIEW, PANE).unwrap();
    assert!(Arc::ptr_eq(&again, &session), "the pane's session");
    let after = session.snapshot();
    assert!(after.generation > before.generation, "started again");
    assert_ne!(after.pid, before.pid);
    assert!(matches!(frames(&session, 3), Shown::Frames));

    // Its pane closed for good: parked, then reaped (here: at once).
    extension::reap(&host);
    assert!(find(&key()).is_some(), "shown: kept");
    architect_process_view::host::park(&key());
    let ended = architect_process_view::host::reap(Duration::ZERO, |_| true);
    assert_eq!(ended, [key()]);
    assert!(matches!(session.snapshot().status, Status::Stopped(_)));
}

/// A program killed is started again, as a process extension's is, and
/// the pane says why it died meanwhile.
#[test]
fn a_program_that_dies_is_started_again_and_says_why() {
    let _alone = alone();
    let tmp = tempfile::tempdir().unwrap();
    let dev = tmp.path().join("dev");
    package(&dev);
    let host = host(&Grants::in_memory());
    host.load(&Package::read(&dev).unwrap(), Origin::Dev)
        .unwrap();
    let pane = "pane-supervised";
    let session = extension::open(&host, ID, VIEW, pane).unwrap();
    match frames(&session, 1) {
        Shown::Frames => {}
        Shown::NoGpu(why) => {
            eprintln!("the program could not draw here (no GPU?): {why}: skipped");
            architect_process_view::host::end(&extension::key(ID, VIEW, pane));
            return;
        }
    }
    let pid = session.snapshot().pid.unwrap();
    rustix::process::kill_process(
        rustix::process::Pid::from_raw(i32::try_from(pid).unwrap()).unwrap(),
        rustix::process::Signal::KILL,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut said = None;
    loop {
        let snap = session.snapshot();
        if let Status::Stopped(why) = &snap.status {
            said.get_or_insert_with(|| why.clone());
        }
        if snap.pid.is_some_and(|p| p != pid) {
            break;
        }
        assert!(Instant::now() < deadline, "not started again: {snap:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
    if let Some(why) = said {
        assert!(why.contains("signal 9"), "{why}");
        assert!(why.contains("starting it again (1 of 3)"), "{why}");
    }
    assert!(matches!(frames(&session, 1), Shown::Frames));
    architect_process_view::host::end(&extension::key(ID, VIEW, pane));
}

/// An argument naming a setting gets its value, and a change of the
/// setting starts the program again with the new one (`refresh`).
#[test]
fn a_setting_its_arguments_name_reaches_the_program_and_restarts_it() {
    use architect_extensions::hostapi::MemoryValues;
    use architect_extensions::hostapi::SettingValues as _;
    let _alone = alone();
    let tmp = tempfile::tempdir().unwrap();
    let dev = tmp.path().join("dev");
    package_with(
        &dev,
        r#"args = ["--clock={{setting:lab.orbits.clock}}", "{{setting:lab.orbits.show}}"]"#,
        r#"[[sections]]
id = "orbits"
title = "Orbits"

[[sections.settings]]
key = "lab.orbits.clock"
title = "Clock"
control = "text"
default = "internal"

[[sections.settings]]
key = "lab.orbits.show"
title = "Show"
control = "text"
default = ""
"#,
    );
    let host = host(&Grants::in_memory());
    let values = Arc::new(MemoryValues::default());
    host.set_setting_values(values.clone());
    host.load(&Package::read(&dev).unwrap(), Origin::Dev)
        .unwrap();
    let pane = "pane-settings";
    let key = extension::key(ID, VIEW, pane);
    let session = extension::open(&host, ID, VIEW, pane).unwrap();
    // The default, and an empty setting alone left out.
    assert_eq!(session.program().args, ["--clock=internal"]);
    if let Shown::NoGpu(why) = frames(&session, 1) {
        eprintln!("the program could not draw here (no GPU?): {why}: skipped");
        architect_process_view::host::end(&key);
        return;
    }
    let before = session.snapshot();
    assert!(extension::refresh(&host).is_empty(), "nothing changed");
    values
        .set("lab.orbits.clock", serde_json::json!("midi:IAC"))
        .unwrap();
    values
        .set("lab.orbits.show", serde_json::json!("recital.ig-show"))
        .unwrap();
    assert_eq!(extension::refresh(&host), [key.clone()]);
    assert_eq!(
        session.program().args,
        ["--clock=midi:IAC", "recital.ig-show"]
    );
    let after = session.snapshot();
    assert!(after.generation > before.generation, "started again");
    assert_ne!(after.pid, before.pid);
    assert!(matches!(frames(&session, 1), Shown::Frames));
    architect_process_view::host::end(&key);
}

/// A pane gone from its workspace (closed) stops its program at the next
/// check; one still in it (moving to another group or a pop-out window, or
/// a tab behind another) keeps it running, the same program, however long
/// it is away; one with no workspace to ask stops after the backstop. A
/// close and a reopen before the check keep the program too.
#[test]
fn a_program_stops_when_its_pane_leaves_the_workspace_not_when_it_moves() {
    use std::cell::Cell;
    use std::rc::Rc;
    let _alone = alone();
    let tmp = tempfile::tempdir().unwrap();
    let dev = tmp.path().join("dev");
    package(&dev);
    let host = host(&Grants::in_memory());
    host.load(&Package::read(&dev).unwrap(), Origin::Dev)
        .unwrap();
    let pane = "pane-moving";
    let key = extension::key(ID, VIEW, pane);
    let present = Rc::new(Cell::new(true));
    let there = present.clone();
    extension::register_pane(&key, move || there.get());
    let session = extension::open(&host, ID, VIEW, pane).unwrap();
    if let Shown::NoGpu(why) = frames(&session, 1) {
        eprintln!("the program could not draw here (no GPU?): {why}: skipped");
        architect_process_view::host::end(&key);
        return;
    }
    let pid = session.snapshot().pid;

    // A move to a pop-out: unmounted here, mounted there, the pane in the
    // workspace all along; away longer than the backstop.
    architect_process_view::host::park(&key);
    std::thread::sleep(extension::BACKSTOP + Duration::from_millis(300));
    assert!(extension::reap(&host).is_empty(), "moving: kept");
    architect_process_view::host::unpark(&key);
    assert_eq!(session.snapshot().pid, pid, "the same program");
    assert!(matches!(frames(&session, 1), Shown::Frames));

    // A close and a reopen before the check: the same program.
    architect_process_view::host::park(&key);
    present.set(false);
    present.set(true);
    architect_process_view::host::unpark(&key);
    assert!(extension::reap(&host).is_empty());
    assert_eq!(session.snapshot().pid, pid);

    // Closed: gone at the next check.
    architect_process_view::host::park(&key);
    present.set(false);
    assert_eq!(extension::reap(&host), [key.clone()]);
    assert!(find(&key).is_none());
    assert!(matches!(session.snapshot().status, Status::Stopped(_)));

    // Nothing to ask (no workspace): the backstop.
    let unknown = "pane-nowhere";
    let k2 = extension::key(ID, VIEW, unknown);
    let s2 = extension::open(&host, ID, VIEW, unknown).unwrap();
    architect_process_view::host::park(&k2);
    assert!(extension::reap(&host).is_empty(), "within the backstop");
    std::thread::sleep(extension::BACKSTOP + Duration::from_millis(300));
    assert_eq!(extension::reap(&host), [k2]);
    assert!(matches!(s2.snapshot().status, Status::Stopped(_)));
}
