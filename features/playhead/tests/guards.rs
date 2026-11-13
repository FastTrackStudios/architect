//! What keeps one extension off another's source, and the app's own off
//! every extension: the independent review of #660 (its seven failing
//! tests, kept as they were written, plus the paths it traced only), each
//! asserting the behaviour #589's contract asks for.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_pass_by_value
)]

use std::sync::{Mutex, PoisonError};

use architect::action::{ActionFuture, ActionList};
use architect_extensions::events::AppEvent;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_extensions::linked::{Builder, Extension};
use architect_playhead::deck::{Caller, Control, Deck};
use architect_playhead::model::{Spec, vault_path};
use architect_playhead::{CHANGED, Playhead, SERVICE};
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

fn ext(id: &str) -> Caller {
    Caller::Extension(id.to_owned())
}

/// The deck's `vault_path` is the one check on `media`/`art`; #589's
/// agreed gate refuses `?`, `#`, `%`.
#[test]
fn vault_path_refuses_query_fragment_and_percent() {
    for bad in [
        "Recordings/%2e%2e/%2e%2e/secret.m4a",
        "Recordings%2F..%2Fsecret.m4a",
        "Recordings/a.mp3?download=1",
        "Recordings/a.mp3#t=10",
    ] {
        assert!(!vault_path(bad), "accepted {bad:?}");
    }
}

/// An extension learns nothing of another source (a private note's title
/// on the app's own Notes source) through `SOURCES`; it hears its own
/// entry and whether it is active.
#[test]
fn sources_does_not_leak_other_sources_to_an_extension() {
    architect_playhead::ask(
        &Caller::App,
        "PLAYHEAD_SET",
        &json!({"source": "app:notes", "name": "Notes",
                "queue": [{"id": "Private/Diagnosis.md", "title": "Diagnosis call", "duration": 9}]}),
    )
    .unwrap();
    let seen = architect_playhead::ask(&ext("nosy"), "PLAYHEAD_SOURCES", &json!({})).unwrap();
    let app = architect_playhead::ask(&Caller::App, "PLAYHEAD_SOURCES", &json!({})).unwrap();
    architect_playhead::ask(
        &Caller::App,
        "PLAYHEAD_SET",
        &json!({"source": "app:notes", "queue": []}),
    )
    .unwrap();
    assert!(
        !seen.to_string().contains("Diagnosis call") && !seen.to_string().contains("app:notes"),
        "an extension read another source's item: {seen}"
    );
    assert_eq!(seen, json!({"active": false, "sources": []}));
    assert!(
        app.to_string().contains("Diagnosis call"),
        "the app sees all"
    );
}

/// An extension whose id is `vault` (or any id the app keeps) does not
/// own the app's own source.
#[test]
fn an_extension_cannot_take_the_apps_own_source() {
    let mut d = Deck::new();
    let notes = Spec::parse(&json!({"name": "Notes",
        "queue": [{"id": "Inbox/memo.md", "title": "Memo", "media": "Recordings/memo.m4a"}]}))
    .unwrap();
    d.apply(
        &Caller::App,
        Control::Set(notes.clone()),
        Some("vault"),
        0.0,
    )
    .unwrap();
    d.apply(&Caller::App, Control::Set(notes), Some("app:notes"), 0.0)
        .unwrap();
    let swap = Spec::parse(&json!({"queue": [{"id": "x", "title": "Memo",
        "media": "Private/other.m4a"}], "play": true}))
    .unwrap();
    for id in ["vault", "app", "playhead", "app:notes"] {
        assert!(
            d.apply(&ext(id), Control::Set(swap.clone()), None, 0.0)
                .is_err(),
            "extension `{id}` replaced the app's source"
        );
        assert!(d.apply(&ext(id), Control::Pause, None, 0.0).is_err());
    }
    assert_eq!(d.source("app:notes").unwrap().item().unwrap().title, "Memo");
}

/// An item with media the app cannot hear (every signed-in vault today;
/// a 404) and no duration ends: it is passed over, marked unplayable.
#[test]
fn an_unheard_media_item_without_duration_ends_or_stops() {
    let mut d = Deck::new(); // native: not audible
    let q = Spec::parse(&json!({"queue": [
        {"id": "a", "title": "A", "media": "R/a.m4a"},
        {"id": "b", "title": "B", "duration": 5}
    ]}))
    .unwrap();
    d.apply(&ext("s"), Control::Set(q), None, 0.0).unwrap();
    d.apply(&ext("s"), Control::Play, None, 0.0).unwrap();
    let hour = 3_600_000.0;
    let _ = d.tick(hour);
    let s = d.state("s", hour).unwrap();
    assert!(
        s["index"] == json!(1) || s["playing"] == json!(false),
        "still playing item 0 an hour in, at {} s of duration {}",
        s["time"],
        s["duration"]
    );
    assert_eq!(s["unplayable"], json!(["a"]), "{s}");
    // The last one, unplayable: stopped there, and said so.
    let q =
        Spec::parse(&json!({"queue": [{"id": "z", "title": "Z", "media": "R/z.m4a"}]})).unwrap();
    d.apply(&ext("t"), Control::Set(q), None, hour).unwrap();
    d.apply(&ext("t"), Control::Play, None, hour).unwrap();
    assert!(d.tick(hour + 100.0));
    let t = d.state("t", hour + 200.0).unwrap();
    assert_eq!(
        (
            t["playing"].clone(),
            t["unplayable"].clone(),
            t["unheard"].clone()
        ),
        (json!(false), json!(["z"]), json!(true))
    );
}

/// `SEEK` on an item of unknown length is refused; past the end of one of
/// known length, clamped.
#[test]
fn seek_is_clamped_when_length_unknown() {
    let mut d = Deck::new();
    let q =
        Spec::parse(&json!({"queue": [{"id": "a", "title": "A", "media": "R/a.m4a"}]})).unwrap();
    d.apply(&ext("s"), Control::Set(q), None, 0.0).unwrap();
    assert!(
        d.apply(&ext("s"), Control::Seek(1e12), None, 0.0).is_err(),
        "seek to 1e12 s accepted"
    );
    let q = Spec::parse(&json!({"queue": [{"id": "a", "title": "A", "duration": 10}]})).unwrap();
    d.apply(&ext("s"), Control::Set(q), None, 0.0).unwrap();
    d.apply(&ext("s"), Control::Seek(1e12), None, 0.0).unwrap();
    assert_eq!(d.state("s", 0.0).unwrap()["time"], 10.0);
}

/// A clock's length is the deck's only for media the app resolved: an
/// item whose path it would not stream reports no `duration` (#589's
/// media requirement).
#[test]
fn an_unresolved_items_media_gives_no_length() {
    let mut d = Deck::new();
    d.audible = true;
    let q = Spec::parse(&json!({"queue": [{"id": "a", "title": "A", "media": "Private/x.m4a"}]}))
        .unwrap();
    d.apply(&ext("s"), Control::Set(q), None, 0.0).unwrap();
    let reading = architect_playhead::Reading {
        duration: Some(321.0),
        ..architect_playhead::Reading::default()
    };
    d.report(&reading, 10.0);
    assert_eq!(d.state("s", 10.0).unwrap()["duration"], 0.0);
    d.report(
        &architect_playhead::Reading {
            resolved: true,
            ..reading
        },
        20.0,
    );
    assert_eq!(d.state("s", 20.0).unwrap()["duration"], 321.0);
}

// ── Through the real host ──

/// `LAB_<ID>_CALL {method, args}` calls `architect.playhead`'s `method`;
/// `LAB_<ID>_HEARD` keeps what its `app:playhead.changed` hook heard.
struct Player(&'static str, &'static str);

static HEARD: Mutex<Vec<(String, Value)>> = Mutex::new(Vec::new());

impl Extension for Player {
    fn manifest(&self) -> &'static str {
        self.0
    }
    fn build(&self, ext: &mut Builder) {
        let handle = ext.handle();
        ext.action(self.1, move |args: Value| {
            let handle = handle.clone();
            async move {
                handle
                    .service(
                        SERVICE,
                        args["method"].as_str().unwrap_or_default(),
                        args["args"].clone(),
                    )
                    .await
                    .map_err(|e| e.to_string())
            }
        });
        let me = self.1.trim_end_matches("_CALL").to_owned();
        let heard = format!("{me}_HEARD");
        ext.action(heard, move |args: Value| {
            let me = me.clone();
            async move {
                HEARD
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push((me, args["detail"].clone()));
                Ok::<_, String>(Value::Null)
            }
        });
    }
}

fn manifest(id: &str, action: &str) -> &'static str {
    let heard = action.replace("_CALL", "_HEARD");
    Box::leak(
        format!(
            r#"id = "{id}"
name = "{id}"
version = "0.1.0"
kind = "process"
api = "0.1"
[entry]
path = "x"
[uses]
"architect.playhead" = "^0.1"
[capabilities]
effect = "mutating"
events = ["playhead.changed"]
[[actions]]
id = "{action}"
name = "call"
effect = "mutating"
params = '{{"type":"object","properties":{{"method":{{"type":"string"}},"args":{{"type":"object"}}}}}}'
[[actions]]
id = "{heard}"
name = "heard"
effect = "mutating"
params = '{{"type":"object","properties":{{"event":{{"type":"string"}},"detail":{{}}}}}}'
[[hooks]]
on = "app:playhead.changed"
action = "{heard}"
"#
        )
        .into_boxed_str(),
    )
}

/// An extension whose declared work names the playhead's actions: a
/// preset that empties `lab.alpha`'s source, and a hook that pauses
/// whatever is active, on an app event it hears.
struct Evil;
impl Extension for Evil {
    fn manifest(&self) -> &'static str {
        r#"id = "lab.evil"
name = "evil"
version = "0.1.0"
kind = "process"
api = "0.1"
[entry]
path = "x"
[capabilities]
effect = "mutating"
actions = ["PLAYHEAD_"]
events = ["lab.poke"]
[[actions]]
id = "LAB_EVIL_STOP"
name = "Play"
effect = "mutating"
preset = { action = "PLAYHEAD_SET", args = '{"source":"lab.alpha","queue":[]}' }
[[hooks]]
on = "app:lab.poke"
action = "PLAYHEAD_PAUSE"
"#
    }
    fn build(&self, _: &mut Builder) {}
}

fn host_with(others: &[&dyn Extension]) -> (tempfile::TempDir, ActionList, Extensions) {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    // Hooks are kept and run by the test ([`run_hooks`]), in order.
    let host = Host::new(list.clone(), SharedSections::new(Registry::new())).with_spawn(|f| {
        SPAWNED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(f);
    });
    let mut ext = Extensions::new(host, Installs::new(tmp.path().join("i")))
        .with_linked(&Playhead)
        .unwrap();
    for e in others {
        ext = ext.with_linked(*e).unwrap();
    }
    ext.install_contribution_actions().unwrap();
    let _ = ext.sync();
    (tmp, list, ext)
}

static SPAWNED: Mutex<Vec<ActionFuture>> = Mutex::new(Vec::new());

/// Runs the hooks started so far.
fn run_hooks() {
    loop {
        let next = std::mem::take(&mut *SPAWNED.lock().unwrap_or_else(PoisonError::into_inner));
        if next.is_empty() {
            return;
        }
        for f in next {
            let _ = block_on(f);
        }
    }
}

fn run(list: &ActionList, id: &str, a: Value) -> Result<Value, String> {
    block_on(list.run_json_async(id, a.to_string().as_bytes()))
        .map(|o| serde_json::from_slice::<Value>(&o).unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
}

#[test]
fn service_sources_from_a_linked_extension() {
    let alpha = Player(manifest("lab.alpha", "LAB_ALPHA_CALL"), "LAB_ALPHA_CALL");
    let beta = Player(manifest("lab.beta", "LAB_BETA_CALL"), "LAB_BETA_CALL");
    let (_tmp, list, _ext) = host_with(&[&alpha, &beta]);
    run(
        &list,
        "LAB_ALPHA_CALL",
        json!({"method": "set", "args": {"name": "Alpha",
        "queue": [{"id": "1", "title": "Alpha secret title", "duration": 60}]}}),
    )
    .unwrap();
    let seen = run(
        &list,
        "LAB_BETA_CALL",
        json!({"method": "sources", "args": {}}),
    )
    .unwrap();
    run(
        &list,
        "LAB_ALPHA_CALL",
        json!({"method": "set", "args": {"queue": []}}),
    )
    .unwrap();
    assert!(
        !seen.to_string().contains("Alpha secret title"),
        "lab.beta read lab.alpha's item through the service: {seen}"
    );
}

/// What ⌘K or a favourite's press does: run an extension's preset as the
/// app. It runs as its extension, so it moves only its own source.
#[test]
fn a_preset_run_by_the_app_cannot_move_another_source() {
    let alpha = Player(manifest("lab.alpha", "LAB_ALPHA_CALL"), "LAB_ALPHA_CALL");
    let (_tmp, list, _ext) = host_with(&[&alpha, &Evil]);
    run(
        &list,
        "LAB_ALPHA_CALL",
        json!({"method": "set", "args": {"name": "Alpha",
        "queue": [{"id": "1", "title": "A", "duration": 60}]}}),
    )
    .unwrap();
    let out = run(&list, "LAB_EVIL_STOP", json!({}));
    let alpha = run(
        &list,
        "LAB_ALPHA_CALL",
        json!({"method": "state", "args": {}}),
    )
    .unwrap();
    let _ = run(
        &list,
        "LAB_ALPHA_CALL",
        json!({"method": "set", "args": {"queue": []}}),
    );
    assert!(
        !alpha.is_null(),
        "lab.evil's preset removed lab.alpha's source (preset said {out:?})"
    );
    assert!(out.unwrap_err().contains("not your source"));
}

/// A hook, run with no press at all, reaches the playhead as its
/// extension, never as the app: it cannot pause the active source.
#[test]
fn a_hook_cannot_move_another_source() {
    let alpha = Player(manifest("lab.alpha", "LAB_ALPHA_CALL"), "LAB_ALPHA_CALL");
    let (_tmp, list, ext) = host_with(&[&alpha, &Evil]);
    ext.host()
        .define_event(AppEvent {
            name: "lab.poke".to_owned(),
            title: "Poke".to_owned(),
            ..AppEvent::default()
        })
        .unwrap();
    run(
        &list,
        "LAB_ALPHA_CALL",
        json!({"method": "set", "args": {"name": "Alpha",
        "queue": [{"id": "1", "title": "A", "duration": 60}], "play": true}}),
    )
    .unwrap();
    assert_eq!(ext.host().raise_app("lab.poke", &json!({})), Ok(1));
    run_hooks();
    let alpha = run(
        &list,
        "LAB_ALPHA_CALL",
        json!({"method": "state", "args": {}}),
    )
    .unwrap();
    let _ = run(
        &list,
        "LAB_ALPHA_CALL",
        json!({"method": "set", "args": {"queue": []}}),
    );
    assert_eq!(
        alpha["playing"],
        json!(true),
        "lab.evil's hook paused lab.alpha: {alpha}"
    );
}

/// `playhead.changed` goes per owner: an extension hears its own source's
/// state, and of another's change only `{active}`.
#[test]
fn each_extension_hears_only_its_own_source_change() {
    let alpha = Player(manifest("lab.alpha", "LAB_ALPHA_CALL"), "LAB_ALPHA_CALL");
    let beta = Player(manifest("lab.beta", "LAB_BETA_CALL"), "LAB_BETA_CALL");
    let (_tmp, list, ext) = host_with(&[&alpha, &beta]);
    let host = ext.host().clone();
    host.define_event(AppEvent {
        name: CHANGED.to_owned(),
        title: "Now Playing changes".to_owned(),
        ..AppEvent::default()
    })
    .unwrap();
    // As the app wires it (`apps/ui/src/extensions.rs`).
    architect_playhead::on_change("app.event", move |s| {
        let _ = architect_playhead::raise_changed(&host, s);
    });
    run(&list, "LAB_ALPHA_CALL", json!({"method": "set", "args": {"name": "Alpha",
        "queue": [{"id": "Private/a.md", "title": "Alpha secret title", "media": "Private/a.m4a", "duration": 60}]}}))
    .unwrap();
    run_hooks();
    let heard = std::mem::take(&mut *HEARD.lock().unwrap_or_else(PoisonError::into_inner));
    let _ = run(
        &list,
        "LAB_ALPHA_CALL",
        json!({"method": "set", "args": {"queue": []}}),
    );
    let of = |who: &str| -> Vec<Value> {
        heard
            .iter()
            .filter(|(w, _)| w == who)
            .map(|(_, d)| d.clone())
            .collect()
    };
    let (a, b) = (of("LAB_ALPHA"), of("LAB_BETA"));
    assert_eq!(a.len(), 1, "{heard:?}");
    assert_eq!(a[0]["title"], "Alpha secret title");
    assert_eq!(b, [json!({"active": false})], "lab.beta heard {b:?}");
}

/// A favourite's declared `PLAYHEAD_TOGGLE`, pressed in the browser:
/// as its declarer, so it plays the declarer's own source (its face's),
/// never pauses whichever other source is active.
#[test]
fn a_favourites_press_in_the_browser_moves_its_declarers_source() {
    let set = |id: &str| {
        architect_playhead::ask(
            &ext(id),
            "PLAYHEAD_SET",
            &json!({"queue": [{"id": "1", "title": id, "duration": 60}]}),
        )
        .unwrap();
    };
    set("lab.fav.a");
    set("lab.fav.b");
    architect_playhead::ask(&ext("lab.fav.b"), "PLAYHEAD_PLAY", &json!({})).unwrap();
    assert!(architect_playhead::press_as("lab.fav.a", "PLAYHEAD_TOGGLE").is_some());
    let (a, b) = (
        architect_playhead::playing("lab.fav.a"),
        architect_playhead::playing("lab.fav.b"),
    );
    for id in ["lab.fav.a", "lab.fav.b"] {
        architect_playhead::ask(&ext(id), "PLAYHEAD_SET", &json!({"queue": []})).unwrap();
    }
    assert!(a && !b, "a {a}, b {b}");
    assert!(architect_playhead::press_as("lab.fav.a", "APP_SETTINGS").is_none());
}
