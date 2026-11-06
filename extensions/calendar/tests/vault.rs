//! The Calendar against a real vault, as the app hosts it: the app's list
//! holds the vault's, Bases' and capture's actions over a folder, and the
//! extension reaches them only through the host service. Alone, linked in
//! and then as a process (its development copy); then with Tasks and the
//! Time Tracker: a task on its day, dragged to another, and tracked time
//! as a block.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_pass_by_value,
    clippy::arithmetic_side_effects,
    clippy::string_slice
)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_calendar::{Calendar, ID};
use architect_extensions::Origin;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_extensions::process::ProcessRunner;
use architect_settings::Registry;
use chrono::{Datelike as _, Local, NaiveDate, Weekday};
use futures::executor::block_on;
use serde_json::{Value, json};

/// The app's side: its list with `VAULT_*` and `BASES_*` over `dir`.
fn app_list(dir: &Path) -> Arc<ActionList> {
    let list = Arc::new(ActionList::new());
    let open = vault::actions::OpenVault::open(dir).unwrap();
    vault::actions::register(&list, &open).unwrap();
    let types: &'static architect_bases::model::ViewTypes =
        Box::leak(Box::new(architect_bases::model::ViewTypes::new()));
    architect_bases::actions::register(&list, &open, types).unwrap();
    list
}

fn run(list: &ActionList, id: &str, args: Value) -> Value {
    let out = block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .unwrap_or_else(|e| panic!("{id}: {e}"));
    serde_json::from_slice(&out).unwrap_or(Value::Null)
}

fn try_run(list: &ActionList, id: &str, args: Value) -> Result<Value, String> {
    block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
}

fn until(what: &str, ok: impl Fn() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "waited for {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn read(dir: &Path, path: &str) -> String {
    std::fs::read_to_string(dir.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// Next week's Monday: a week with nothing else in it.
fn monday() -> NaiveDate {
    let today = Local::now().date_naive();
    let mut d = today + chrono::Duration::days(7);
    while d.weekday() != Weekday::Mon {
        d += chrono::Duration::days(1);
    }
    d
}

fn day(n: i64) -> String {
    (monday() + chrono::Duration::days(n)).to_string()
}

/// The blocks `CALENDAR_AGENDA` gives over the week from Monday.
fn week(list: &ActionList) -> Vec<Value> {
    let out = run(
        list,
        "CALENDAR_AGENDA",
        json!({ "from": day(0), "to": day(7) }),
    );
    out["blocks"].as_array().unwrap().clone()
}

fn titles_at(blocks: &[Value]) -> Vec<String> {
    blocks
        .iter()
        .map(|b| {
            format!(
                "{} {}",
                b["start"].as_str().unwrap(),
                b["title"].as_str().unwrap()
            )
        })
        .collect()
}

/// Makes, moves and ends events through `list` (the extension's), and
/// reads every one back from the markdown.
fn exercise(dir: &Path, ext: &Extensions, list: &ActionList, app: &ActionList, origin: Origin) {
    assert_eq!(ext.host().get(ID).unwrap().origin, origin);

    // A one-off: Task's shape on disk.
    let made = run(
        list,
        "CALENDAR_CREATE",
        json!({ "title": "Album release show", "start": format!("{}T20:00", day(4)), "minutes": 150,
                "location": "The Hall", "links": [{ "relation": "for", "target": "Gigs/Album release" }] }),
    );
    let show = made["path"].as_str().unwrap().to_owned();
    assert!(
        show.starts_with("Calendar/") && show.ends_with("Album release show.md"),
        "{made}"
    );
    let text = read(dir, &show);
    for line in [
        "type: scheduling-cal-event",
        "title: Album release show",
        "location: The Hall",
        "all_day: false",
        "[[Gigs/Album release]]",
    ] {
        assert!(text.contains(line), "{line:?} in\n{text}");
    }
    assert!(
        text.contains(&format!("start: {}T20:00:00", day(4)))
            || text.contains(&format!("start: '{}T20:00:00", day(4))),
        "{text}"
    );
    assert_eq!(made["minutes"], 150);

    // An external ref is made once.
    let a = run(
        list,
        "CALENDAR_CREATE",
        json!({ "title": "Synced", "start": day(2), "external_ref": "ics:abc@example" }),
    );
    let b = run(
        list,
        "CALENDAR_CREATE",
        json!({ "title": "Synced again", "start": day(2), "external_ref": { "system": "ics", "id": "abc@example" } }),
    );
    assert_eq!(a["path"], b["path"]);
    assert_eq!(b["existing"], true);
    assert_eq!(a["all_day"], true);
    assert!(
        read(dir, a["path"].as_str().unwrap()).contains(&format!("end: {}", day(3))),
        "an all-day end is the day after"
    );

    // Quick add, as said.
    let lunch = run(
        list,
        "CALENDAR_QUICK",
        json!({ "text": "Lunch with Sam 1pm for 90m", "date": day(1) }),
    );
    assert_eq!(lunch["title"], "Lunch with Sam", "{lunch}");
    assert!(
        lunch["start"]
            .as_str()
            .unwrap()
            .starts_with(&format!("{}T13:00", day(1))),
        "{lunch}"
    );
    assert_eq!(lunch["minutes"], 90);

    // A series: weekdays at 09:30 for 15 minutes.
    let standup = run(
        list,
        "CALENDAR_CREATE",
        json!({ "title": "Standup", "start": format!("{}T09:30", day(0)), "minutes": 15, "recurrence": "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR" }),
    );
    let series = standup["path"].as_str().unwrap().to_owned();
    assert_eq!(standup["repeats"], "Weekly on weekdays");
    let blocks = week(list);
    let ups = blocks.iter().filter(|b| b["title"] == "Standup").count();
    assert_eq!(ups, 5, "{}", titles_at(&blocks).join("\n"));

    // Wednesday's alone moves to 11:00: an override, the rest stay.
    let moved = run(
        list,
        "CALENDAR_RESCHEDULE",
        json!({ "path": series, "occurrence": day(2), "start": format!("{}T11:00", day(2)), "scope": "this" }),
    );
    let over = moved["path"].as_str().unwrap().to_owned();
    assert_ne!(over, series);
    let text = read(dir, &over);
    assert!(
        text.contains(&format!(
            "recurrence_id: {}",
            standup["id"].as_str().unwrap()
        )),
        "{text}"
    );
    assert!(
        text.contains(&format!("recurrence_date: '{}'", day(2)))
            || text.contains(&format!("recurrence_date: {}", day(2))),
        "{text}"
    );
    let at: Vec<String> = week(list)
        .iter()
        .filter(|b| b["title"] == "Standup")
        .map(|b| b["start"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(at.len(), 5);
    assert!(at.contains(&format!("{}T11:00", day(2))), "{at:?}");
    assert!(!at.contains(&format!("{}T09:30", day(2))), "{at:?}");

    // Thursday's is skipped: its day in the series' exdates.
    let skipped = run(
        list,
        "CALENDAR_SKIP",
        json!({ "path": series, "occurrence": day(3) }),
    );
    assert_eq!(skipped["skipped"], day(3));
    assert!(
        read(dir, &series).contains("exdates:"),
        "{}",
        read(dir, &series)
    );
    assert_eq!(
        week(list)
            .iter()
            .filter(|b| b["title"] == "Standup")
            .count(),
        4
    );

    // The whole series moves half an hour later; the override stays put.
    run(
        list,
        "CALENDAR_RESCHEDULE",
        json!({ "path": series, "occurrence": day(0), "start": format!("{}T10:00", day(0)), "scope": "series" }),
    );
    let at: Vec<String> = week(list)
        .iter()
        .filter(|b| b["title"] == "Standup")
        .map(|b| b["start"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        at.contains(&format!("{}T10:00", day(0))) && at.contains(&format!("{}T10:00", day(4))),
        "{at:?}"
    );
    assert!(at.contains(&format!("{}T11:00", day(2))), "{at:?}");

    // From Friday on, a new series at 08:00; the old one ends before.
    let split = run(
        list,
        "CALENDAR_RESCHEDULE",
        json!({ "path": series, "occurrence": day(4), "start": format!("{}T08:00", day(4)), "scope": "following" }),
    );
    assert_ne!(split["path"], series.as_str());
    assert!(
        read(dir, &series).contains("UNTIL="),
        "{}",
        read(dir, &series)
    );
    let at: Vec<String> = week(list)
        .iter()
        .filter(|b| b["title"] == "Standup")
        .map(|b| b["start"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        at.contains(&format!("{}T08:00", day(4))) && !at.contains(&format!("{}T10:00", day(4))),
        "{at:?}"
    );

    // A one-off moves; a title changes without moving it.
    let r = run(
        list,
        "CALENDAR_RESCHEDULE",
        json!({ "path": lunch["path"], "start": format!("{}T12:30", day(1)) }),
    );
    assert!(
        r["end"]
            .as_str()
            .unwrap()
            .starts_with(&format!("{}T14:00", day(1))),
        "{r}"
    );
    assert!(
        r["moved_from"]["start"]
            .as_str()
            .unwrap()
            .starts_with(&format!("{}T13:00", day(1)))
    );
    let u = run(
        list,
        "CALENDAR_UPDATE",
        json!({ "path": lunch["path"], "title": "Lunch with Sam and Alex", "color": "success" }),
    );
    assert_eq!(u["changed"], json!(["title", "color"]));
    assert!(
        u["start"]
            .as_str()
            .unwrap()
            .starts_with(&format!("{}T12:30", day(1)))
    );
    assert!(try_run(list, "CALENDAR_UPDATE", json!({ "path": lunch["path"] })).is_err());

    // Listing: by window, by link, tombstoned or not.
    let listed = run(
        list,
        "CALENDAR_LIST",
        json!({ "from": day(0), "to": day(7) }),
    );
    assert!(listed["count"].as_u64().unwrap() >= 5, "{listed}");
    let linked = run(
        list,
        "CALENDAR_LIST",
        json!({ "linked_to": "[[Album release]]" }),
    );
    assert_eq!(linked["count"], 1, "{linked}");
    run(
        list,
        "CALENDAR_UPDATE",
        json!({ "path": show, "tombstone": true }),
    );
    assert_eq!(
        run(
            list,
            "CALENDAR_LIST",
            json!({ "linked_to": "Gigs/Album release" })
        )["count"],
        0
    );
    assert_eq!(
        run(
            list,
            "CALENDAR_LIST",
            json!({ "linked_to": "Gigs/Album release", "include_tombstoned": true })
        )["count"],
        1
    );

    // From a capture, through the `event` target.
    let captured = run(
        app,
        "CAPTURE_ADD",
        json!({ "text": format!("Dentist {} at:3pm for:45m", day(3)), "target": "event", "source": "test" }),
    );
    let dentist = captured["path"].as_str().unwrap_or_default().to_owned();
    assert!(dentist.ends_with("Dentist.md"), "{captured}");
    let text = read(dir, &dentist);
    assert!(
        text.contains(&format!("start: {}T15:00:00", day(3)))
            || text.contains(&format!("start: '{}T15:00:00", day(3))),
        "{text}"
    );

    // The view's state, paged by an action.
    let shown = run(list, "CALENDAR_SHOWN", json!({}));
    assert_eq!(shown["view"], "week");
    // Its sidebar.
    let snap = run(list, "CALENDAR_NAV_SNAPSHOT", json!({}));
    assert_eq!(snap["tree"].as_array().unwrap().len(), 6, "{snap}");
    assert!(
        snap["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["container"] == "repeating"),
        "{snap}"
    );

    // Cancelled: the note goes, a series' overrides with it.
    run(list, "CALENDAR_CANCEL", json!({ "path": dentist }));
    assert!(!dir.join(&dentist).exists());
    let old = run(list, "CALENDAR_LIST", json!({}))["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["path"] == series.as_str())
        .cloned()
        .unwrap();
    let gone = run(list, "CALENDAR_CANCEL", json!({ "id": old["id"] }));
    assert_eq!(gone["overrides"], json!([over]), "{gone}");
    assert!(!dir.join(&series).exists() && !dir.join(&over).exists());
}

fn extensions(
    tmp: &Path,
    app: &Arc<ActionList>,
    runner: Option<ProcessRunner>,
    bundle: bool,
) -> (Extensions, ActionList) {
    let list = ActionList::new();
    let mut host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    if let Some(r) = runner {
        host = host.with_runner(r);
    }
    host.set_app_list((**app).clone());
    host.forward_actions_to(app);
    let mut ext = Extensions::new(host, Installs::new(tmp.join("installs")))
        .with_linked(&Calendar)
        .unwrap();
    if bundle {
        ext = ext
            .with_linked(&architect_tasks::Tasks)
            .unwrap()
            .with_linked(&architect_time_tracker::TimeTracker)
            .unwrap();
    }
    ext.install_contribution_actions().unwrap();
    // As the app has it: the registries on its list, and capture over it.
    for id in ["EXT_CAPTURE_TARGETS", "EXT_PROJECT_TYPES", "EXT_VIEW_OPEN"] {
        forward(app, &list, id);
    }
    architect_capture::actions::register(app, architect_capture::actions::Config::default(), None)
        .unwrap();
    let _ = ext.sync();
    (ext, list)
}

/// Puts `id` of `from` on `to`, run on `from` as whoever ran it (the
/// app's `forward`, `apps/ui/src/extensions.rs`).
fn forward(to: &ActionList, from: &ActionList, id: &str) {
    let info = from.info(id).unwrap();
    let (list, id) = (from.clone(), id.to_owned());
    to.register_forwarded_with(
        &info,
        Arc::new(
            move |caller: &architect::action::ActionCaller, args: &[u8]| {
                let (list, caller, id, args) =
                    (list.clone(), caller.clone(), id.clone(), args.to_vec());
                let run: architect::action::ActionFuture =
                    Box::pin(async move { list.run_json_async_with(&caller, &id, &args).await });
                Ok(run)
            },
        ),
    )
    .unwrap();
}

fn vault_in(tmp: &Path) -> std::path::PathBuf {
    let dir = tmp.join("vault");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn calendar_two_ways() {
    // Linked in.
    let tmp = tempfile::tempdir().unwrap();
    let dir = vault_in(tmp.path());
    let app = app_list(&dir);
    let (ext, list) = extensions(tmp.path(), &app, None, false);
    exercise(&dir, &ext, &list, &app, Origin::Builtin);
    let reg = ext.host().contributions();
    for kind in ["calendar", "timeline"] {
        assert_eq!(reg.base_view(kind).unwrap().owner, ID);
        assert!(
            ext.host()
                .linked_view(ID, &format!("base.{kind}"))
                .is_some()
        );
    }
    assert_eq!(
        ext.host()
            .provider(architect_calendar::SERVICE, &"^0.1".parse().unwrap())
            .unwrap()
            .0,
        ID
    );
    // Without tasks or time, it says so and shows events alone.
    let agenda = run(&list, "CALENDAR_AGENDA", json!({}));
    assert_eq!(
        (agenda["tasks"].clone(), agenda["time"].clone()),
        (json!(false), json!(false))
    );
    let _ = ext.disable(ID);
    assert!(ext.host().contributions().base_view("timeline").is_none());
    assert!(ext.host().contributions().of(ID).is_none());

    // As a process.
    let tmp = tempfile::tempdir().unwrap();
    let dir = vault_in(tmp.path());
    let dev = tmp.path().join("calendar");
    std::fs::create_dir_all(dev.join("tours")).unwrap();
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    std::fs::copy(here.join("extension.toml"), dev.join("extension.toml")).unwrap();
    std::fs::copy(
        here.join("tours/calendar-tour.styx"),
        dev.join("tours/calendar-tour.styx"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        env!("CARGO_BIN_EXE_architect-calendar"),
        dev.join("architect-calendar"),
    )
    .unwrap();
    let app = app_list(&dir);
    let runner = ProcessRunner::new().unwrap().with_socket_dir(tmp.path());
    let (ext, list) = extensions(tmp.path(), &app, Some(runner), false);
    let _ = ext.dev(&dev);
    exercise(&dir, &ext, &list, &app, Origin::Dev);
    assert!(
        ext.host().linked_view(ID, "calendar").is_none(),
        "drawn remotely now"
    );
}

/// With Tasks and the Time Tracker: a task on its day, moved by the
/// calendar, and tracked time as a block.
#[test]
fn calendar_with_tasks_and_time() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = vault_in(tmp.path());
    let app = app_list(&dir);
    let (_ext, list) = extensions(tmp.path(), &app, None, true);
    let task = run(
        &list,
        "TASKS_CREATE",
        json!({ "title": "Send the mixes", "due": day(2) }),
    );
    let path = task["task"]["path"]
        .as_str()
        .or_else(|| task["path"].as_str())
        .unwrap()
        .to_owned();
    until("the task on the calendar", || {
        week(&list)
            .iter()
            .any(|b| b["source"]["kind"] == "task" && b["source"]["path"] == path.as_str())
    });
    let b = week(&list)
        .into_iter()
        .find(|b| b["source"]["path"] == path.as_str())
        .unwrap();
    assert_eq!(
        (b["all_day"].clone(), b["start"].clone()),
        (json!(true), json!(format!("{}T00:00", day(2))))
    );
    assert_eq!(b["source"]["field"], "due");

    // Dragged to Friday at 14:00: its due date moves, with the time.
    let moved = run(
        &list,
        "CALENDAR_MOVE_TASK",
        json!({ "path": path, "date": day(4), "time": "14:00" }),
    );
    assert_eq!(moved["field"], "due", "{moved}");
    let text = read(&dir, &path);
    assert!(text.contains(&format!("{}T14:00", day(4))), "{text}");
    let b = week(&list)
        .into_iter()
        .find(|b| b["source"]["path"] == path.as_str())
        .unwrap();
    assert_eq!(
        (b["all_day"].clone(), b["end"].clone()),
        (json!(false), json!(format!("{}T14:30", day(4))))
    );

    // Time logged on Tuesday: a block, not movable here.
    run(
        &list,
        "TIME_LOG",
        json!({ "task": path, "start": format!("{}T15:00:00Z", day(1)), "end": format!("{}T16:00:00Z", day(1)), "description": "Bounce stems" }),
    );
    let at = run(
        &list,
        "CALENDAR_AGENDA",
        json!({ "from": day(0), "to": day(7) }),
    );
    assert_eq!(at["time"], true, "{at}");
    let span = at["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["source"]["kind"] == "time")
        .cloned()
        .unwrap();
    assert_eq!(span["title"], "Bounce stems");
    let entries = run(
        &list,
        "TIME_ENTRIES",
        json!({ "from": day(0), "to": day(7) }),
    );
    assert_eq!(entries["count"], 1, "{entries}");
    // Not when asked not to.
    let only = run(
        &list,
        "CALENDAR_AGENDA",
        json!({ "from": day(0), "to": day(7), "tasks": false, "time": false }),
    );
    assert!(only["blocks"].as_array().unwrap().is_empty(), "{only}");
}

/// Booking: hours, an event type, its open slots (less the calendar's
/// events), a booking checked again, on the calendar, and cancelled.
#[test]
fn booking_slots() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = vault_in(tmp.path());
    let app = app_list(&dir);
    let (_ext, list) = extensions(tmp.path(), &app, None, false);
    run(
        &list,
        "CALENDAR_SCHEDULE",
        json!({ "id": "work", "name": "Work hours", "rules": ["mon-fri 09:00-11:00"] }),
    );
    assert!(read(&dir, "Calendar/Schedules/work.md").contains("type: scheduling-schedule"));
    let t = run(
        &list,
        "CALENDAR_EVENT_TYPE",
        json!({ "title": "Intro call", "duration_min": 30, "schedule_id": "work", "location": { "kind": "phone" } }),
    );
    assert_eq!(t["id"], "intro-call", "{t}");
    let text = read(&dir, "Calendar/Event types/intro-call.md");
    assert!(
        text.contains("type: scheduling-event-type") && text.contains("duration_min: 30"),
        "{text}"
    );
    assert!(
        try_run(
            &list,
            "CALENDAR_EVENT_TYPE",
            json!({ "title": "Nope", "schedule_id": "none" })
        )
        .is_err()
    );

    // Monday: four slots; an event at 10:00 takes one.
    let slots = |list: &ActionList| -> Vec<String> {
        run(
            list,
            "CALENDAR_OPEN_SLOTS",
            json!({ "event_type": "intro-call", "from": day(0), "to": day(1) }),
        )["slots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["start"].as_str().unwrap()[11..16].to_owned())
            .collect()
    };
    assert_eq!(slots(&list), ["09:00", "09:30", "10:00", "10:30"]);
    run(
        &list,
        "CALENDAR_CREATE",
        json!({ "title": "Dentist", "start": format!("{}T10:00", day(0)), "minutes": 30 }),
    );
    assert_eq!(slots(&list), ["09:00", "09:30", "10:30"]);

    // Booked: a note, gone from the slots, on the calendar; twice is refused.
    let b = run(
        &list,
        "CALENDAR_BOOK",
        json!({ "event_type": "intro-call", "start": format!("{}T09:30", day(0)), "attendee_name": "Sam", "attendee_email": "sam@example.com" }),
    );
    let path = b["path"].as_str().unwrap().to_owned();
    let text = read(&dir, &path);
    for line in [
        "type: scheduling-booking",
        "event_type_id: intro-call",
        "attendee_name: Sam",
        "status: confirmed",
        "start_utc:",
    ] {
        assert!(text.contains(line), "{line:?} in\n{text}");
    }
    assert_eq!(slots(&list), ["09:00", "10:30"]);
    let again = try_run(
        &list,
        "CALENDAR_BOOK",
        json!({ "event_type": "intro-call", "start": format!("{}T09:30", day(0)), "attendee_name": "Alex" }),
    );
    assert!(again.unwrap_err().contains("not open"));
    let on = week(&list)
        .into_iter()
        .find(|b| b["source"]["kind"] == "booking")
        .unwrap();
    assert_eq!(on["title"], "Intro call — Sam");
    assert_eq!(on["start"], format!("{}T09:30", day(0)));

    // Cancelled: its slot is open again, and it leaves the calendar.
    let s = run(
        &list,
        "CALENDAR_BOOKING_STATUS",
        json!({ "path": path, "status": "cancelled" }),
    );
    assert_eq!(
        (s["was"].clone(), s["status"].clone()),
        (json!("confirmed"), json!("cancelled"))
    );
    assert_eq!(slots(&list), ["09:00", "09:30", "10:30"]);
    assert!(!week(&list).iter().any(|b| b["source"]["kind"] == "booking"));
    assert_eq!(
        run(&list, "CALENDAR_BOOKINGS", json!({ "status": "cancelled" }))["count"],
        1
    );
    assert_eq!(
        run(&list, "CALENDAR_EVENT_TYPES", json!({}))["event_types"][0]["schedule"],
        "Work hours"
    );
}
