//! The updater's honest pieces (#523): where an update goes for each env
//! and platform, the stamped version, the status line; and the extension
//! as the app links it, in a development build (tests are never stamped):
//! it offers and installs nothing.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::hostapi::MemoryValues;
use architect_extensions::install::Installs;
use architect_settings::Registry;
use architect_updates::model::{
    Channel, Env, Plan, Platform, ReportOnly, State, plan, release_version, status_line,
};
use futures::executor::block_on;
use serde_json::{Value, json};

fn env(vars: &[(&str, &str)], writable: bool) -> Env {
    let map: HashMap<String, OsString> = vars
        .iter()
        .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
        .collect();
    Env::from_vars(|k| map.get(k).cloned(), |_: &Path| writable)
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
fn macos_windows_and_other_cpus_report_only() {
    let e = env(&[("ARCHITECT_UPDATE_DIR", "/u"), ("APPIMAGE", "/a")], true);
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
    let e = env(&[("ARCHITECT_UPDATE_DIR", "/u"), ("APPIMAGE", "/a")], true);
    assert_eq!(
        plan(&e, &linux(), false),
        Plan::ReportOnly(ReportOnly::DevelopmentBuild)
    );
}

#[test]
fn only_a_release_version_is_stamped() {
    assert_eq!(release_version(Some("0.12.0")), Some("0.12.0".into()));
    assert_eq!(release_version(Some(" v0.12.0\n")), Some("0.12.0".into()));
    for bad in [
        None,
        Some(""),
        Some("0.12"),
        Some("main"),
        Some("0.12.0-dev"),
    ] {
        assert_eq!(release_version(bad), None, "{bad:?}");
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

#[test]
fn linked_in_a_development_build_it_offers_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_setting_values(MemoryValues::default());
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_linked(&architect_updates::Updates)
        .unwrap();
    ext.install_contribution_actions().unwrap();
    let _ = ext.sync();

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
