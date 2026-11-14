//! The extension showcase's host-service examples (#710,
//! `examples/showcase`), each loaded from its own folder as a development
//! copy, as `just showcase` loads it:
//!
//! - the Timers sampler (H11): its `every` hook ticks on the host's clock
//!   and counts in its status item, its cron hook runs on a weekday
//!   morning and not at the weekend, and its worker counts primes;
//! - Secrets and fetch (H5, H6): a call reaches the one host it names
//!   (a local server, no real network), a token the person enters once is
//!   put into the request by the host and never handed back, and another
//!   host is refused;
//! - the Services sampler (H12): it loads only after the extensions it
//!   `[requires]` and calls Calendar, Day Plans and Tasks through their
//!   services (stand-ins here, with the services' own names).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use architect::action::{ActionCaller, ActionError, ActionList};
use architect_extensions::events::FakeClock;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::hostui::FakeUi;
use architect_extensions::install::Installs;
use architect_extensions::net::Secrets;
use architect_extensions::{Origin, Package};
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

#[path = "support/server.rs"]
mod server;

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/showcase")
        .join(name)
}

fn run(list: &ActionList, id: &str, args: &Value) -> Result<Value, ActionError> {
    block_on(list.run_json_async_with(&ActionCaller::app(), id, args.to_string().as_bytes()))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
}

/// A copy of example `name` in `to`, each `from` in its files made `with`.
fn copy(name: &str, to: &Path, from: &str, with: &str) -> PathBuf {
    let dir = to.join(name);
    for entry in walk(&example(name)) {
        let rel = entry.strip_prefix(example(name)).unwrap();
        let dst = dir.join(rel);
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        let text = std::fs::read_to_string(&entry).unwrap();
        std::fs::write(dst, text.replace(from, with)).unwrap();
    }
    dir
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

// ---- the Timers sampler -------------------------------------------------

/// 2026-11-04, a Wednesday, 08:00 UTC.
const WED_8: u64 = 1_793_750_400_000 + 8 * 3_600_000;

#[test]
fn the_timers_samplers_hooks_tick_on_the_clock_and_its_worker_counts() {
    let tmp = tempfile::tempdir().unwrap();
    let clock = FakeClock::at(WED_8);
    let list = ActionList::new();
    let ui = FakeUi::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_spawn(|work| {
            let _ = block_on(work);
        })
        .with_clock(clock.clone())
        .with_data_root(tmp.path().join("data"))
        .with_ui(ui.clone());
    host.load(&Package::read(&example("timers")).unwrap(), Origin::Dev)
        .unwrap();
    let ticks = |host: &Host| {
        host.contributions()
            .status(architect_extensions::host::now_ms())
            .into_iter()
            .find(|s| s.item.id == "ticks")
            .unwrap()
            .item
            .text
    };
    assert_eq!(ticks(&host), "No ticks yet");

    // Every ten seconds, on the host's clock.
    assert!(host.tick().ran.is_empty(), "nothing is due yet");
    clock.advance(Duration::from_secs(10));
    let ran = host.tick().ran;
    assert_eq!(ran.len(), 1, "{ran:?}");
    assert_eq!(ticks(&host), "1 tick");
    clock.advance(Duration::from_secs(10));
    host.tick();
    assert_eq!(ticks(&host), "2 ticks");

    // The cron hook: 09:00 on a weekday.
    clock.set(WED_8 + 60 * 60_000);
    let ran = host.tick().ran;
    assert!(ran.iter().any(|r| r.contains("0 9 * * 1-5")), "{ran:?}");
    assert!(
        ui.notices()
            .iter()
            .any(|n| n.text == "Good morning from the Timers sampler")
    );
    // Friday 09:00 (Thursday's, missed while the clock jumped, runs once),
    // then Saturday 09:00: not a weekday.
    clock.set(WED_8 + 2 * 86_400_000 + 60 * 60_000);
    let ran = host.tick().ran;
    assert!(ran.iter().any(|r| r.contains("0 9 * * 1-5")), "{ran:?}");
    clock.set(WED_8 + 3 * 86_400_000 + 60 * 60_000);
    let ran = host.tick().ran;
    assert!(!ran.iter().any(|r| r.contains("0 9 * * 1-5")), "{ran:?}");

    // Its worker: the primes below 1000.
    let out = run(&list, "SHOWCASE_TIMERS_PRIMES", &json!({ "n": 1000 })).unwrap();
    assert_eq!(out, 168);
    assert!(
        ui.notices()
            .iter()
            .any(|n| n.text == "168 primes below 1000")
    );
}

// ---- Secrets and fetch --------------------------------------------------

#[test]
fn secrets_and_fetch_reaches_its_host_with_the_token_the_host_puts_in() {
    let server = server::serve();
    let tmp = tempfile::tempdir().unwrap();
    let here = format!("127.0.0.1:{}", server.port);
    let dir = copy("secrets-fetch", tmp.path(), "127.0.0.1:8765", &here);
    let list = ActionList::new();
    let ui = FakeUi::new().answer(json!("sk-showcase-1"));
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_data_root(tmp.path().join("data"))
        .with_secrets(Secrets::open(tmp.path().join("app/extension-secrets.json")));
    host.set_ui(Arc::new(ui.clone()));
    host.load(&Package::read(&dir).unwrap(), Origin::Dev)
        .unwrap();
    let base = format!("http://{here}");

    // H6: no token yet; the call reaches its host, with no Authorization.
    let out = run(&list, "SHOWCASE_FETCH_CALL", &json!({ "base": base })).unwrap();
    assert!(out["status"].is_number(), "{out}");
    let heard = server.heard.to("/echo");
    assert_eq!(heard.len(), 1);
    assert!(
        !heard[0].1.contains_key("authorization"),
        "{:?}",
        heard[0].1
    );

    // H5: the token, asked once in the host's own prompt and kept.
    assert_eq!(
        run(&list, "SHOWCASE_FETCH_SET", &Value::Null).unwrap(),
        true
    );
    assert_eq!(
        run(&list, "SHOWCASE_FETCH_SET", &Value::Null).unwrap(),
        true
    );
    assert_eq!(ui.asked().len(), 1, "asked once");
    run(&list, "SHOWCASE_FETCH_CALL", &json!({ "base": base })).unwrap();
    let heard = server.heard.to("/echo");
    assert_eq!(
        heard[1].1.get("authorization").map(String::as_str),
        Some("Bearer sk-showcase-1"),
        "the host put it in"
    );
    // Never in what the extension wrote down or could read.
    let kept = std::fs::read_to_string(tmp.path().join("app/extension-secrets.json")).unwrap();
    assert!(kept.contains("sk-showcase-1"));
    for file in walk(&dir).into_iter().chain(walk(&tmp.path().join("data"))) {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        assert!(!text.contains("sk-showcase-1"), "{}", file.display());
    }

    // Nor in anything it said to the person.
    assert!(
        ui.notices()
            .iter()
            .all(|n| !n.text.contains("sk-showcase-1")),
        "{:?}",
        ui.notices()
    );

    // H6: a host it does not name is refused before anything is sent.
    let said = run(&list, "SHOWCASE_FETCH_ELSEWHERE", &Value::Null).unwrap();
    assert!(said.as_str().unwrap().contains("refused"), "{said}");
}

// ---- the Services sampler -----------------------------------------------

/// A stand-in for a first-party extension: `id` providing `service`, its
/// methods its actions answering `answers` (method → JSON) and keeping
/// what they were asked in its data.
fn provider(tmp: &Path, id: &str, service: &str, answers: &[(&str, &str)]) -> PathBuf {
    let dir = tmp.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    let prefix = id.to_uppercase();
    let mut manifest = format!(
        "id = \"{id}\"\nname = \"{id}\"\nversion = \"0.1.0\"\nkind = \"luau\"\n\n[provides]\n\"{service}\" = \"0.1.0\"\n"
    );
    for (method, answer) in answers {
        let action = format!("{prefix}_{}", method.to_uppercase());
        let _ = write!(
            manifest,
            "\n[[actions]]\nid = \"{action}\"\nname = \"{action}\"\nscript = \"{method}.luau\"\nparams = '{{\"type\":\"object\",\"properties\":{{\"from\":{{}},\"to\":{{}},\"date\":{{}},\"view\":{{}},\"title\":{{}},\"start\":{{}}}}}}'\n"
        );
        std::fs::write(
            dir.join(format!("{method}.luau")),
            format!("host.set_data(\"asked_{method}\", args)\nreturn {answer}\n"),
        )
        .unwrap();
    }
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    dir
}

#[test]
fn the_services_sampler_calls_calendar_day_plans_and_tasks_through_their_services() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_data_root(tmp.path().join("data"))
        .with_ui(FakeUi::new());
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")));

    // Without its providers it does not load: it [requires] them.
    let _ = ext.dev(&example("services"));
    assert!(ext.host().get("showcase.services").is_none());

    for dir in [
        provider(
            tmp.path(),
            "calendar",
            "architect.calendar",
            &[
                ("list", r#"{ { title = "Standup" }, { title = "Lunch" } }"#),
                ("create", r#"{ path = "Calendar/Showcase review.md" }"#),
            ],
        ),
        provider(
            tmp.path(),
            "dayplan",
            "architect.dayplan",
            &[("get", r#"{ blocks = { { title = "Deep work" } } }"#)],
        ),
        provider(
            tmp.path(),
            "tasks",
            "architect.tasks",
            &[
                (
                    "list",
                    r#"{ { title = "Ship it" }, { title = "Review" }, { title = "Plan" } }"#,
                ),
                ("create", r#"{ path = "Tasks/Showcase review.md" }"#),
            ],
        ),
    ] {
        let _ = ext.dev(&dir);
    }
    let _ = ext.dev(&example("services"));
    assert!(
        ext.host().get("showcase.services").is_some(),
        "{:?}",
        ext.problems()
    );

    let today = run(&list, "SHOWCASE_SERVICES_TODAY", &Value::Null).unwrap();
    assert_eq!(today, json!({ "events": 2, "blocks": 1, "tasks": 3 }));
    let planned = run(
        &list,
        "SHOWCASE_SERVICES_PLAN",
        &json!({ "title": "Showcase review", "start": "2026-11-14T15:00" }),
    )
    .unwrap();
    assert_eq!(planned["task"]["path"], "Tasks/Showcase review.md");
    assert_eq!(planned["event"]["path"], "Calendar/Showcase review.md");
}

// ---- Surface ------------------------------------------------------------

#[test]
fn surface_sends_only_to_the_virtual_port_and_hears_its_own_osc() {
    use architect_input::{PortKind, PortMessage, Ports, UdpOsc, VirtualPorts};
    // A port the test picks, in place of the example's 9031.
    let free = std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let tmp = tempfile::tempdir().unwrap();
    let dir = copy("surface", tmp.path(), "9031", &free.to_string());
    let midi = VirtualPorts::new(PortKind::Midi);
    let virtual_port = midi.add("Lab Surface");
    // A person's device beside it: never sent to.
    let hardware = midi.add("A real keyboard");
    let list = ActionList::new();
    let ui = FakeUi::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_spawn(|work| {
            let _ = block_on(work);
        })
        .with_data_root(tmp.path().join("data"))
        .with_ports(Ports::new().with_midi(midi).with_osc(UdpOsc))
        .with_ui(ui.clone());
    host.load(&Package::read(&dir).unwrap(), Origin::Dev)
        .unwrap();

    let out = run(&list, "SHOWCASE_SURFACE_PING", &Value::Null).unwrap();
    assert_eq!(out["sent_midi"], true);
    // CC 7 = 100 on channel 1, on the virtual port only.
    assert_eq!(virtual_port.take(), [PortMessage::Midi(vec![0xB0, 7, 100])]);
    assert!(
        hardware.take().is_empty(),
        "the person's device heard nothing"
    );
    // Its capability names only the virtual port: another is refused.
    let request: architect_extensions::hostcall::HostRequest = serde_json::from_value(
        json!({"op": "open_port", "kind": "midi", "name": "A real keyboard"}),
    )
    .unwrap();
    let refused = block_on(host.api("showcase.surface").call(request));
    assert!(refused.is_err(), "{refused:?}");

    // Its own OSC, heard back by its port.message hook.
    let started = std::time::Instant::now();
    while !ui
        .notices()
        .iter()
        .any(|n| n.text == "OSC heard: /showcase/ping")
    {
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            ui.notices()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
