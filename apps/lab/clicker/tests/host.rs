//! The host service over a process's own connection (#178): the lab's
//! clicker, run as its program, asks the host for its setting and runs
//! `APP_STATUS` through `guest::host`; the host checks the call against
//! its capabilities and sees it as the clicker; what the program says on
//! stderr is in its log (`EXT_LOGS`).
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use architect::action::{ActionCaller, ActionList, PermissionEngine, Principal};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::hostapi::{MemoryValues, SettingValues};
use architect_extensions::hostcall::{HostError, HostRequest};
use architect_extensions::install::Installs;
use architect_extensions::process::ProcessRunner;
use futures::executor::block_on;
use serde_json::{Value, json};

fn clicker(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("extension.toml"),
        include_str!("../extension.toml"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        env!("CARGO_BIN_EXE_architect-lab-clicker"),
        dir.join("architect-lab-clicker"),
    )
    .unwrap();
    dir.to_path_buf()
}

/// Lets everyone run, remembering who asked for what.
#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Vec<(Principal, String)>>>);

impl PermissionEngine for Seen {
    fn check(
        &self,
        _: &Principal,
        _: &architect_permissions::Resource,
        _: &architect_permissions::Action,
    ) -> architect::action::Decision {
        architect::action::Decision::Allow
    }

    fn authorize(&self, who: &Principal, id: &str) -> architect::action::Decision {
        self.0.lock().unwrap().push((who.clone(), id.to_owned()));
        architect::action::Decision::Allow
    }
}

#[test]
fn the_clicker_asks_the_host_over_its_own_connection() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    architect::action::app_status::register(&list).unwrap();
    let seen = Seen::default();
    list.set_authorizer(seen.clone());
    let values = MemoryValues::default();
    values.set("lab.clicker.step", json!(3)).unwrap();
    let runner = ProcessRunner::new().unwrap().with_socket_dir(tmp.path());
    let host = Host::new(
        list.clone(),
        SharedSections::new(architect_settings::Registry::new()),
    )
    .with_runner(runner)
    .with_setting_values(values);
    let ext = Extensions::new(host.clone(), Installs::new(tmp.path().join("installs")));
    ext.install_actions().unwrap();
    let events = ext.dev(clicker(&tmp.path().join("clicker")));
    assert!(host.get("lab.clicker").is_some(), "{events:?}");

    let out =
        block_on(list.run_json_async_with(&ActionCaller::app(), "LAB_CLICKER_PROBE", b"")).unwrap();
    let out: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(out["step"], 3, "its setting, from the host: {out}");
    assert_eq!(out["problems"], 0, "{out}");
    assert_ne!(out["pid"].as_u64(), Some(u64::from(std::process::id())));
    let calls = seen.0.lock().unwrap().clone();
    assert!(
        calls.contains(&(
            Principal::extension("lab.clicker", None),
            "APP_STATUS".to_owned()
        )),
        "the gate saw the clicker: {calls:?}"
    );

    // Outside its capabilities: refused, here as over the wire.
    match block_on(host.api("lab.clicker").call(HostRequest::Selection)) {
        Err(HostError::Refused { capability, .. }) => assert_eq!(capability, "reads.selection"),
        other => panic!("not refused: {other:?}"),
    }

    // Its stderr, and its start, in its log.
    let logs = block_on(list.run_json_async("EXT_LOGS", br#"{"id":"lab.clicker"}"#)).unwrap();
    let logs: Value = serde_json::from_slice(&logs).unwrap();
    let lines = logs["lines"].as_array().unwrap();
    assert!(
        lines
            .iter()
            .any(|l| l["source"] == "stderr"
                && l["text"].as_str().unwrap().contains("probe: step 3")),
        "{logs}"
    );
    assert!(lines.iter().any(|l| l["source"] == "process"), "{logs}");
}
