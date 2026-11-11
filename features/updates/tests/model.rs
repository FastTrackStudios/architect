//! The updater's honest pieces (#523): where an update goes for each env
//! and platform (an inherited `$APPIMAGE` is never replaced), the stamped
//! version and its rank, the page gate, the state machine's transitions;
//! and the extension as the app links it, in a development build (tests
//! are never stamped): it offers and installs nothing, and an outside
//! caller is asked before Install or Restart.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use architect::action::{ActionCaller, ActionError, ActionList, Principal};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::hostapi::MemoryValues;
use architect_extensions::install::Installs;
use architect_settings::Registry;
use architect_updates::model::{
    Begin, Channel, Env, Machine, Plan, Platform, RELEASES_PAGE, ReportOnly, State, newer, plan,
    release_page, release_version, status_line, version_key,
};
use futures::executor::block_on;
use serde_json::{Value, json};

/// The running binary inside the mounted image.
const EXE: &str = "/tmp/.mount_ArchXY/usr/bin/architect-desktop";

fn env_at(vars: &[(&str, &str)], exe: Option<&str>, writable: bool) -> Env {
    let map: HashMap<String, OsString> = vars
        .iter()
        .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
        .collect();
    Env::from_vars(
        |k| map.get(k).cloned(),
        exe.map(Path::new),
        |_: &Path| writable,
    )
}

/// As the `AppImage` runtime starts Architect: `APPDIR` the mount, the
/// binary under it.
fn env(vars: &[(&str, &str)], writable: bool) -> Env {
    let mut all = vars.to_vec();
    all.push(("APPDIR", "/tmp/.mount_ArchXY"));
    env_at(&all, Some(EXE), writable)
}

fn linux() -> Platform {
    Platform::new("linux", "x86_64")
}

#[test]
fn the_update_dir_wins_then_a_writable_appimage_then_report_only() {
    let both = env(
        &[
            ("ARCHITECT_UPDATE_DIR", "/u"),
            ("APPIMAGE", "/nix/store/x/A.AppImage"),
        ],
        false,
    );
    assert_eq!(
        plan(&both, &linux(), true),
        Plan::UpdateDir { dir: "/u".into() }
    );
    let plain = env(&[("APPIMAGE", "/home/me/A.AppImage")], true);
    assert_eq!(
        plan(&plain, &linux(), true),
        Plan::InPlace {
            appimage: "/home/me/A.AppImage".into()
        }
    );
    let locked = env(&[("APPIMAGE", "/opt/A.AppImage")], false);
    assert_eq!(
        plan(&locked, &linux(), true),
        Plan::ReportOnly(ReportOnly::NotWritable {
            appimage: PathBuf::from("/opt/A.AppImage")
        })
    );
    assert_eq!(
        plan(&env(&[], true), &linux(), true),
        Plan::ReportOnly(ReportOnly::NoAppImage)
    );
    // An empty variable is unset.
    assert_eq!(
        plan(
            &env(&[("ARCHITECT_UPDATE_DIR", ""), ("APPIMAGE", "")], true),
            &linux(),
            true
        ),
        Plan::ReportOnly(ReportOnly::NoAppImage)
    );
}

#[test]
fn an_inherited_appimage_is_never_replaced() {
    let foreign = |e: &Env| {
        plan(e, &linux(), true)
            == Plan::ReportOnly(ReportOnly::ForeignAppImage {
                appimage: PathBuf::from("/home/me/Editor.AppImage"),
            })
    };
    // A terminal inside another app's image: its APPIMAGE and APPDIR, and
    // Architect run from elsewhere.
    let terminal = env_at(
        &[
            ("APPIMAGE", "/home/me/Editor.AppImage"),
            ("APPDIR", "/tmp/.mount_EditAB"),
        ],
        Some("/usr/bin/architect-desktop"),
        true,
    );
    assert!(foreign(&terminal), "{terminal:?}");
    assert!(!terminal.appimage_writable, "never asked to write it");
    // No APPDIR at all.
    let bare = env_at(&[("APPIMAGE", "/home/me/Editor.AppImage")], Some(EXE), true);
    assert!(foreign(&bare));
    // No running binary known.
    let unknown = env_at(
        &[
            ("APPIMAGE", "/home/me/Editor.AppImage"),
            ("APPDIR", "/tmp/.mount_ArchXY"),
        ],
        None,
        true,
    );
    assert!(foreign(&unknown));
    // Ours, but not named like an image.
    let named = env(&[("APPIMAGE", "/home/me/architect")], true);
    assert_eq!(
        plan(&named, &linux(), true),
        Plan::ReportOnly(ReportOnly::ForeignAppImage {
            appimage: PathBuf::from("/home/me/architect")
        })
    );
}

#[test]
fn macos_windows_and_other_cpus_report_only() {
    let e = env(
        &[("ARCHITECT_UPDATE_DIR", "/u"), ("APPIMAGE", "/a.AppImage")],
        true,
    );
    for (os, arch) in [
        ("macos", "aarch64"),
        ("windows", "x86_64"),
        ("linux", "aarch64"),
    ] {
        assert_eq!(
            plan(&e, &Platform::new(os, arch), true),
            Plan::ReportOnly(ReportOnly::Platform {
                os: os.into(),
                arch: arch.into()
            })
        );
    }
    assert_eq!(
        Platform::new("macos", "aarch64").asset(),
        "Architect-macos-arm64.dmg"
    );
    assert_eq!(linux().asset(), "Architect-linux-x86_64.AppImage");
}

#[test]
fn a_development_build_reports_only_whatever_the_env() {
    let e = env(
        &[("ARCHITECT_UPDATE_DIR", "/u"), ("APPIMAGE", "/a.AppImage")],
        true,
    );
    assert_eq!(
        plan(&e, &linux(), false),
        Plan::ReportOnly(ReportOnly::DevelopmentBuild)
    );
}

#[test]
fn only_a_release_or_a_nightly_version_is_stamped() {
    assert_eq!(release_version(Some("0.12.0")), Some("0.12.0".into()));
    assert_eq!(release_version(Some(" v0.12.0\n")), Some("0.12.0".into()));
    assert_eq!(
        release_version(Some("0.13.0-nightly.20261111")),
        Some("0.13.0-nightly.20261111".into())
    );
    for bad in [
        None,
        Some(""),
        Some("0.12"),
        Some("main"),
        Some("0.12.0-dev"),
        Some("0.13.0-rc.1"),
        Some("0.13.0-nightly.2026"),
    ] {
        assert_eq!(release_version(bad), None, "{bad:?}");
    }
}

#[test]
fn versions_rank_as_the_nix_launcher_ranks_them() {
    assert!(newer("0.2.10", "0.2.9"));
    assert!(!newer("0.2.9", "0.2.9"));
    // A release beats a nightly of the same X.Y.Z; a later nightly beats
    // an earlier one; the next X.Y.Z's nightly beats this release.
    assert!(newer("0.13.0", "0.13.0-nightly.20261111"));
    assert!(!newer("0.13.0-nightly.20261111", "0.13.0"));
    assert!(newer("0.13.0-nightly.20261112", "0.13.0-nightly.20261111"));
    assert!(newer("0.14.0-nightly.20261101", "0.13.0"));
    assert!(!newer("garbage", "0.1.0"));
    assert_eq!(version_key("1.2.3"), Some((1, 2, 3, 1, 0)));
}

#[test]
fn only_an_architect_release_page_is_opened() {
    let ok = "https://github.com/FastTrackStudios/architect/releases/tag/v0.13.0";
    assert_eq!(release_page(ok), ok);
    for bad in [
        "",
        "http://github.com/FastTrackStudios/architect/releases/tag/v1",
        "https://github.com/FastTrackStudios/architect/releases",
        "https://github.com/FastTrackStudios/architect/releases/",
        "https://github.com/FastTrackStudios/architect/releasesX/tag/v1",
        "https://github.com/FastTrackStudios/architect/releases/../../evil",
        "https://github.com/FastTrackStudios/architect/releases/tag/v1?x=$(rm -rf ~)",
        "https://github.com/FastTrackStudios/architect/releases/tag/v1 --flag",
        "https://evil.example/https://github.com/FastTrackStudios/architect/releases/x",
        "file:///etc/passwd",
    ] {
        assert_eq!(release_page(bad), RELEASES_PAGE, "{bad}");
    }
}

#[test]
fn nightly_is_present_and_refused() {
    assert_eq!(Channel::parse("nightly"), Channel::Nightly);
    assert_eq!(Channel::parse("anything"), Channel::Stable);
    assert!(Channel::Stable.available().is_ok());
    assert!(
        Channel::Nightly
            .available()
            .unwrap_err()
            .contains("not available yet")
    );
}

fn available() -> State {
    State::Available {
        version: "1.3.0".into(),
        notes_url: String::new(),
        signed: true,
        plan: Plan::UpdateDir { dir: "/u".into() },
    }
}

#[test]
fn a_late_check_never_replaces_an_install() {
    let mut m = Machine::new();
    assert!(m.begin_check());
    assert!(!m.begin_check(), "one check at a time");
    // A second caller's check finished first: Available.
    assert!(m.finish_check(available(), Some("t1".into())));
    // Install starts; the first check's answer arrives late.
    assert!(matches!(m.begin_install(), Ok(Begin::Start { .. })));
    assert!(!m.finish_check(State::UpToDate { newest: None }, Some("t2".into())));
    assert!(matches!(m.state, State::Downloading { .. }));
    assert_eq!(m.last_check.as_deref(), Some("t1"));
    // A new check may not start while downloading or once ready.
    assert!(!m.begin_check());
    assert!(m.finish_install(State::Ready {
        version: "1.3.0".into(),
        plan: Plan::UpdateDir { dir: "/u".into() },
    }));
    assert!(!m.begin_check());
    assert!(matches!(m.state, State::Ready { .. }));
}

#[test]
fn a_second_install_starts_nothing() {
    let mut m = Machine {
        state: available(),
        ..Machine::new()
    };
    assert_eq!(
        m.begin_install(),
        Ok(Begin::Start {
            version: "1.3.0".into(),
            plan: Plan::UpdateDir { dir: "/u".into() }
        })
    );
    assert_eq!(m.begin_install(), Ok(Begin::Already));
    // Progress moves only the download it belongs to.
    assert!(m.progress("1.3.0", 50, Some(100)));
    assert!(!m.progress("1.3.0", 50, Some(100)), "same percent");
    assert!(!m.progress("9.9.9", 99, Some(100)));
    // Install only from Available.
    for state in [
        State::Idle,
        State::Checking,
        State::UpToDate { newest: None },
        State::Failed { reason: "x".into() },
    ] {
        let mut m = Machine {
            state: state.clone(),
            ..Machine::new()
        };
        assert!(m.begin_install().is_err(), "{state:?}");
        assert_eq!(m.state, state, "unchanged");
    }
    let mut report = Machine {
        state: State::Available {
            version: "1.3.0".into(),
            notes_url: "https://evil.example/x".into(),
            signed: true,
            plan: Plan::ReportOnly(ReportOnly::NoAppImage),
        },
        ..Machine::new()
    };
    let why = report.begin_install().unwrap_err();
    assert!(why.ends_with(RELEASES_PAGE), "{why}");
    // A finished install is not overwritten by a stray finish.
    let mut idle = Machine::new();
    assert!(!idle.finish_install(State::Failed { reason: "x".into() }));
    assert_eq!(idle.state, State::Idle);
}

#[test]
fn the_status_line_offers_install_then_restart() {
    let available = |signed, plan| State::Available {
        version: "1.3.0".into(),
        notes_url: String::new(),
        signed,
        plan,
    };
    let dir = Plan::UpdateDir { dir: "/u".into() };
    let line = status_line(&available(true, dir.clone())).unwrap();
    assert_eq!(line.text, "Update 1.3.0 available");
    assert_eq!(line.action, "UPDATES_INSTALL");
    assert_eq!(
        status_line(&available(false, dir.clone())).unwrap().action,
        "UPDATES_PAGE"
    );
    assert_eq!(
        status_line(&available(true, Plan::ReportOnly(ReportOnly::NoAppImage)))
            .unwrap()
            .action,
        "UPDATES_PAGE"
    );
    let ready = status_line(&State::Ready {
        version: "1.3.0".into(),
        plan: dir,
    })
    .unwrap();
    assert_eq!(
        (ready.text.as_str(), ready.action),
        ("Restart to finish", "UPDATES_RESTART")
    );
    let down = status_line(&State::Downloading {
        version: "1.3.0".into(),
        received: 50,
        total: Some(200),
    })
    .unwrap();
    assert_eq!(down.text, "Downloading 1.3.0… 25%");
    for quiet in [
        State::Idle,
        State::DevelopmentBuild,
        State::Checking,
        State::UpToDate { newest: None },
        State::Failed {
            reason: "offline".into(),
        },
    ] {
        assert_eq!(status_line(&quiet), None, "{quiet:?}");
    }
}

fn run(list: &ActionList, id: &str) -> Result<Value, String> {
    block_on(list.run_json_async(id, b"null"))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
}

fn app(dir: &Path) -> (ActionList, Extensions) {
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_setting_values(MemoryValues::default());
    let ext = Extensions::new(host, Installs::new(dir.join("installs")))
        .with_linked(&architect_updates::Updates)
        .unwrap();
    ext.install_contribution_actions().unwrap();
    let _ = ext.sync();
    (list, ext)
}

#[test]
fn linked_in_a_development_build_it_offers_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let (list, _ext) = app(tmp.path());

    assert_eq!(
        architect_updates::release_version(),
        None,
        "tests are never stamped"
    );
    let state = run(&list, "UPDATES_STATE").unwrap();
    assert_eq!(state["development_build"], json!(true));
    assert_eq!(state["current"], Value::Null);
    assert_eq!(state["update"]["state"], json!("development_build"));
    let checked = run(&list, "UPDATES_CHECK").unwrap();
    assert_eq!(checked["update"]["state"], json!("development_build"));
    assert_eq!(checked["last_check"], Value::Null, "never asked");
    let refused = run(&list, "UPDATES_INSTALL").unwrap_err();
    assert!(refused.contains("development build"), "{refused}");
}

#[test]
fn an_outside_caller_is_asked_before_install_or_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let (list, _ext) = app(tmp.path());
    list.require_confirmation(Duration::from_secs(60));
    let agent = ActionCaller::new(Principal::Service {
        name: "agent".to_owned(),
    });
    for id in ["UPDATES_INSTALL", "UPDATES_RESTART"] {
        let out = block_on(list.run_json_async_with(&agent, id, b"null"));
        assert!(
            matches!(out, Err(ActionError::ConfirmationRequired { .. })),
            "{id}: {out:?}"
        );
    }
    // The app itself (its status item, a key, the menu) is not asked: the
    // person pressing it is the approval. In a development build Install
    // then refuses on its own terms.
    let own = run(&list, "UPDATES_INSTALL").unwrap_err();
    assert!(own.contains("development build"), "{own}");
    // Reads are never asked.
    assert!(block_on(list.run_json_async_with(&agent, "UPDATES_STATE", b"null")).is_ok());
}

#[cfg(unix)]
mod restart {
    use std::path::Path;
    use std::process::Command;
    use std::time::{Duration, Instant};

    use architect_updates::restart::start_after_exit;

    /// A program that leaves `marker` behind when it runs.
    fn program(dir: &Path, marker: &Path) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let p = dir.join("app.AppImage");
        std::fs::write(&p, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn appears(path: &Path, within: Duration) -> bool {
        let started = Instant::now();
        while started.elapsed() < within {
            if path.exists() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        path.exists()
    }

    #[test]
    fn it_starts_the_app_once_the_old_one_exited() {
        let tmp = tempfile::tempdir().unwrap();
        let marker = tmp.path().join("started");
        let mut old = Command::new("sleep").arg("0.3").spawn().unwrap();
        let w = start_after_exit(
            &program(tmp.path(), &marker),
            old.id(),
            Duration::from_secs(10),
        )
        .unwrap();
        old.wait().unwrap();
        assert!(appears(&marker, Duration::from_secs(5)));
        w.detach();
    }

    #[test]
    fn a_quit_that_never_comes_starts_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let marker = tmp.path().join("started");
        // This test process never exits within the bound.
        let w = start_after_exit(
            &program(tmp.path(), &marker),
            std::process::id(),
            Duration::from_millis(400),
        )
        .unwrap();
        assert!(!appears(&marker, Duration::from_secs(2)));
        w.detach();
    }

    #[test]
    fn a_cancelled_watcher_starts_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let marker = tmp.path().join("started");
        let mut old = Command::new("sleep").arg("0.5").spawn().unwrap();
        let w = start_after_exit(
            &program(tmp.path(), &marker),
            old.id(),
            Duration::from_secs(10),
        )
        .unwrap();
        w.cancel();
        old.wait().unwrap();
        assert!(!appears(&marker, Duration::from_secs(1)));
    }
}
