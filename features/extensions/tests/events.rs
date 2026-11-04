//! Timers, events and background extensions (#308), on a fake clock:
//!
//! - a manifest's scheduled hooks (`every`, `at`) run when the host's
//!   clock says they are due, and one made at run time
//!   (`host.subscribe`) too;
//! - an extension reacts to a document's save (raised on the host, and
//!   on the Action List as the document layer raises it), to the window's
//!   focus, and to an event the app defines, typed — which one that does
//!   not ask for it, or was not granted it, does not hear;
//! - a background extension starts on its first event and stops once
//!   idle its timeout, its state in its log and in `APP_STATUS`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::events::{BackgroundState, DocEvent, FakeClock, TypedEvent};
use architect_extensions::host::{Host, LoadError, SharedSections};
use architect_extensions::{Origin, Package};
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

const ID: &str = "lab.pulse";

/// 2026-11-04, a Wednesday, 08:00 UTC.
const WED_8: u64 = 1_793_750_400_000 + 8 * 3_600_000;

const MANIFEST: &str = r#"
id = "lab.pulse"
name = "Pulse"
version = "0.1.0"
kind = "luau"

[capabilities]
events = ["transport.*"]

[[actions]]
id = "LAB_PULSE_NOTE"
name = "Note what ran it"
params = '{"type":"object"}'
script = "note.luau"

[[actions]]
id = "LAB_PULSE_SUBSCRIBE"
name = "Make a hook"
params = '{"type":"object"}'
script = "subscribe.luau"

[[hooks]]
every = "5m"
script = "hooks.luau"
function = "every"

[[hooks]]
at = "09:30"
action = "LAB_PULSE_NOTE"

[[hooks]]
on = "doc.saved"
script = "hooks.luau"
function = "saved"

[[hooks]]
on = "app.focused"
script = "hooks.luau"
function = "focused"

[[hooks]]
on = "app:transport.changed"
script = "hooks.luau"
function = "transport"
"#;

const HOOKS: &str = r#"
return {
    every = function(event)
        ext.state{ key = "every", text = ctx.event .. " " .. event.detail.every }
    end,
    saved = function(event)
        ext.state{ key = "saved", text = event.detail.path }
    end,
    focused = function(event)
        ext.state{ key = "focused", text = ctx.event }
    end,
    transport = function(event)
        ext.state{ key = "transport", text = tostring(event.detail.playing) .. " at " .. tostring(event.detail.bar) }
    end,
}
"#;

const NOTE: &str = r#"
ext.state{ key = "note", text = (ctx.event or "none") .. " " .. (ctx.detail or "") }
return ctx.source
"#;

const SUBSCRIBE: &str = "
return host.subscribe(args)
";

fn write(dir: &Path, manifest: &str) {
    std::fs::create_dir_all(dir).unwrap();
    for (name, text) in [
        ("extension.toml", manifest),
        ("hooks.luau", HOOKS),
        ("note.luau", NOTE),
        ("subscribe.luau", SUBSCRIBE),
    ] {
        std::fs::write(dir.join(name), text).unwrap();
    }
}

/// A host on a fake clock, its hooks run as they start.
fn host(clock: &FakeClock) -> Host {
    Host::new(ActionList::new(), SharedSections::new(Registry::new()))
        .with_spawn(|work| {
            let _ = block_on(work);
        })
        .with_clock(clock.clone())
}

/// Loads it from `dir`: why not, when it does not read or load.
fn load(host: &Host, dir: &Path, manifest: &str) -> Result<(), String> {
    write(dir, manifest);
    let package = Package::read(dir).map_err(|e| e.to_string())?;
    host.load(&package, Origin::Dev)
        .map(|_| ())
        .map_err(|e: LoadError| e.to_string())
}

fn state(host: &Host, key: &str) -> Option<String> {
    host.contributions()
        .state(architect_extensions::host::now_ms())
        .into_iter()
        .find(|s| s.source == format!("ext:{ID}") && s.key == key)
        .map(|s| s.text)
}

fn run(host: &Host, id: &str, args: &Value) -> Value {
    let out = block_on(host.list().run_json_async(id, args.to_string().as_bytes()))
        .unwrap_or_else(|e| panic!("{id}: {e}"));
    serde_json::from_slice(&out).unwrap()
}

fn logs(host: &Host) -> Vec<String> {
    host.logs()
        .since(ID, None, 100)
        .into_iter()
        .map(|l| format!("{} {}", l.source, l.text))
        .collect()
}

#[test]
fn scheduled_hooks_run_when_the_clock_says() {
    let tmp = tempfile::tempdir().unwrap();
    let clock = FakeClock::at(WED_8);
    let host = host(&clock);
    load(&host, &tmp.path().join("pulse"), MANIFEST).unwrap();
    let next = host.next_runs(ID);
    assert!(
        next.contains(&("every 5m/hooks.luau#every".to_owned(), WED_8 + 300_000)),
        "{next:?}"
    );
    assert!(host.tick().ran.is_empty(), "nothing is due yet");

    clock.advance(Duration::from_secs(299));
    assert!(host.tick().ran.is_empty(), "a second early");
    clock.advance(Duration::from_secs(1));
    assert_eq!(host.tick().ran, ["lab.pulse every 5m"]);
    assert_eq!(state(&host, "every").as_deref(), Some("schedule 5m"));
    assert!(host.tick().ran.is_empty(), "once, then five minutes on");

    // 09:30: the `at` hook's action, told it was the schedule.
    clock.set(WED_8 + 90 * 60_000);
    let ran = host.tick().ran;
    assert!(ran.contains(&"lab.pulse at 09:30".to_owned()), "{ran:?}");
    let note = state(&host, "note").unwrap();
    assert!(
        note.starts_with("schedule ") && note.contains("09:30"),
        "{note}"
    );

    // One made at run time, through the host service: every minute.
    let key = run(
        &host,
        "LAB_PULSE_SUBSCRIBE",
        &json!({ "every": "1m", "action": "LAB_PULSE_NOTE" }),
    );
    assert_eq!(key, json!("every 1m/LAB_PULSE_NOTE"));
    clock.advance(Duration::from_secs(60));
    assert_eq!(host.tick().ran, ["lab.pulse every 1m"]);
    assert!(host.unsubscribe(ID, "every 1m/LAB_PULSE_NOTE"));
    clock.advance(Duration::from_secs(60));
    assert!(host.tick().ran.is_empty(), "taken off");

    // A schedule that does not read is refused, at load and at run time.
    let bad = MANIFEST.replace("every = \"5m\"", "every = \"soon\"");
    let other = Host::new(ActionList::new(), SharedSections::new(Registry::new()));
    assert!(load(&other, &tmp.path().join("bad"), &bad).is_err());
    let refused = block_on(host.list().run_json_async(
        "LAB_PULSE_SUBSCRIBE",
        br#"{"at":"25:00","action":"LAB_PULSE_NOTE"}"#,
    ));
    assert!(refused.is_err(), "{refused:?}");
}

#[derive(serde::Serialize)]
struct TransportChanged {
    playing: bool,
    bar: u32,
}

impl TypedEvent for TransportChanged {
    const NAME: &'static str = "transport.changed";
    const TITLE: &'static str = "Transport changes";
    const DESCRIPTION: &'static str = "Play, stop and the bar it is at.";
}

#[test]
fn an_extension_reacts_to_a_save_its_focus_and_an_app_event() {
    let tmp = tempfile::tempdir().unwrap();
    let clock = FakeClock::at(WED_8);
    let host = host(&clock);
    load(&host, &tmp.path().join("pulse"), MANIFEST).unwrap();

    // A document's save, raised on the host.
    let saved = DocEvent::Saved {
        path: "notes/a.md".into(),
    };
    assert_eq!(host.raise_doc(&saved), 1);
    assert_eq!(state(&host, "saved").as_deref(), Some("notes/a.md"));

    // And raised by the document layer on the app's list, which the host
    // watches.
    let app = ActionList::new();
    host.watch(&app);
    DocEvent::Saved {
        path: "notes/b.md".into(),
    }
    .raise_on(&app);
    let start = Instant::now();
    while state(&host, "saved").as_deref() != Some("notes/b.md") {
        assert!(start.elapsed() < Duration::from_secs(10), "the list's save");
        std::thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(host.focus(true, None), 1);
    assert_eq!(state(&host, "focused").as_deref(), Some("app.focused"));
    assert_eq!(host.focus(false, None), 0, "nothing on app.blurred");

    // An app-defined event: raised once defined, typed.
    let early = host.raise(&TransportChanged {
        playing: true,
        bar: 1,
    });
    assert!(early.is_err(), "not defined yet");
    host.define::<TransportChanged>().unwrap();
    assert_eq!(
        host.raise(&TransportChanged {
            playing: true,
            bar: 5
        }),
        Ok(1)
    );
    assert_eq!(state(&host, "transport").as_deref(), Some("true at 5"));
    let heard = host.events_for(ID);
    assert_eq!(heard["app"][0]["name"], "app:transport.changed");
    assert_eq!(heard["app"][0]["title"], "Transport changes");

    // One that does not ask to hear it is refused as it loads.
    let deaf = MANIFEST.replace("events = [\"transport.*\"]", "");
    let other = Host::new(ActionList::new(), SharedSections::new(Registry::new()));
    let why = load(&other, &tmp.path().join("deaf"), &deaf).unwrap_err();
    assert!(why.contains("transport.changed"), "{why}");
}

#[test]
fn an_app_event_an_installed_extension_was_not_granted_is_not_heard() {
    let tmp = tempfile::tempdir().unwrap();
    let clock = FakeClock::at(WED_8);
    let grants = architect_extensions::consent::Grants::in_memory();
    let host = host(&clock).with_grants(grants);
    let dir = tmp.path().join("pulse");
    write(&dir, MANIFEST);
    host.load(&Package::read(&dir).unwrap(), Origin::Installed)
        .unwrap();
    host.define::<TransportChanged>().unwrap();
    assert_eq!(
        host.raise(&TransportChanged {
            playing: false,
            bar: 2
        }),
        Ok(0)
    );
    assert_eq!(state(&host, "transport"), None);
    assert!(
        logs(&host)
            .iter()
            .any(|l| l.contains("refused") && l.contains("transport.changed")),
        "{:?}",
        logs(&host)
    );
}

const BACKGROUND: &str = r#"
id = "lab.pulse"
name = "Pulse"
version = "0.1.0"
kind = "luau"

[background]
idle = "30s"

[[actions]]
id = "LAB_PULSE_NOTE"
name = "Note what ran it"
params = '{"type":"object"}'
script = "note.luau"

[[hooks]]
on = "doc.saved"
script = "hooks.luau"
function = "saved"
"#;

#[test]
fn a_background_extension_starts_on_its_first_event_and_stops_when_idle() {
    let tmp = tempfile::tempdir().unwrap();
    let clock = FakeClock::at(WED_8);
    let host = host(&clock);
    load(&host, &tmp.path().join("pulse"), BACKGROUND).unwrap();
    let status = |host: &Host| host.background()[0].clone();
    assert_eq!(status(&host).state, BackgroundState::Stopped);
    assert_eq!(status(&host).starts, 0, "not started as it loads");

    // Its first event starts it.
    host.raise_doc(&DocEvent::Saved {
        path: "notes/a.md".into(),
    });
    assert_eq!(state(&host, "saved").as_deref(), Some("notes/a.md"));
    let now = status(&host);
    assert_eq!(
        (now.state, now.starts, now.started_by.as_deref()),
        (BackgroundState::Running, 1, Some("hook doc.saved"))
    );
    assert_eq!(now.stops_at_ms, Some(WED_8 + 30_000));

    // Used again before its timeout: it runs on, its timeout from then.
    clock.advance(Duration::from_secs(20));
    assert_eq!(run(&host, "LAB_PULSE_NOTE", &json!({})), json!("app"));
    clock.advance(Duration::from_secs(20));
    assert!(host.tick().stopped.is_empty(), "used 20 s ago");
    clock.advance(Duration::from_secs(10));
    assert_eq!(host.tick().stopped, ["lab.pulse"]);
    assert_eq!(status(&host).state, BackgroundState::Stopped);

    // Its state, in its log and in APP_STATUS.
    let lines = logs(&host);
    assert!(
        lines
            .iter()
            .any(|l| l == "background started (hook doc.saved)"),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l == "background stopped: idle for 30 s"),
        "{lines:?}"
    );
    let sections = host.list().status().read();
    let (_, background) = sections
        .iter()
        .find(|(name, _)| name == "background")
        .expect("APP_STATUS has a background section");
    let background: Value = serde_json::from_str(background).unwrap();
    assert_eq!(background[0]["id"], ID);
    assert_eq!(background[0]["state"], "stopped");

    // The next event starts it again.
    assert_eq!(run(&host, "LAB_PULSE_NOTE", &json!({})), json!("app"));
    let again = status(&host);
    assert_eq!(
        (again.state, again.starts, again.started_by.as_deref()),
        (BackgroundState::Running, 2, Some("action LAB_PULSE_NOTE"))
    );

    // Unloaded, its record goes.
    assert!(host.unload(ID));
    assert!(host.background().is_empty());
}
