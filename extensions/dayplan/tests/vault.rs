//! Day Plans linked in beside the Calendar and the Time Tracker, over a
//! real vault: templates, rules and overrides written as notes, a rule's
//! `#tag` matched on the Calendar's events, a block assigned and moved,
//! a template changed with an edited day kept, tracked time against a
//! block, and "Reset to template".

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_pass_by_value,
    clippy::arithmetic_side_effects
)]

use std::path::Path;
use std::sync::Arc;

use architect::action::ActionList;
use architect_dayplan::{DayPlans, ID};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_settings::Registry;
use chrono::{Datelike as _, Duration, Local, NaiveDate, Weekday};
use futures::executor::block_on;
use serde_json::{Value, json};

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

fn read(dir: &Path, path: &str) -> String {
    std::fs::read_to_string(dir.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// Next week's Monday: every day of its week is in the future.
fn monday() -> NaiveDate {
    let mut d = Local::now().date_naive() + Duration::days(7);
    while d.weekday() != Weekday::Mon {
        d += Duration::days(1);
    }
    d
}

fn day(n: i64) -> String {
    (monday() + Duration::days(n)).to_string()
}

fn extensions(tmp: &Path, app: &Arc<ActionList>) -> (Extensions, ActionList) {
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    host.set_app_list((**app).clone());
    host.forward_actions_to(app);
    let ext = Extensions::new(host, Installs::new(tmp.join("installs")))
        .with_linked(&DayPlans)
        .unwrap()
        .with_linked(&architect_calendar::Calendar)
        .unwrap()
        .with_linked(&architect_time_tracker::TimeTracker)
        .unwrap();
    ext.install_contribution_actions().unwrap();
    let _ = ext.sync();
    (ext, list)
}

fn block(id: &str, start: &str, end: &str, category: &str) -> Value {
    json!({ "id": id, "start": start, "end": end, "label": id, "category": category })
}

fn workday() -> Value {
    json!([
        block("reset", "06:30", "07:00", "reset"),
        block("block-1", "08:00", "11:00", "allocatable"),
        block("lunch-prep", "12:00", "12:45", "meal"),
        block("block-2", "13:00", "16:00", "allocatable"),
        block("block-3", "17:00", "19:00", "allocatable"),
        block("sleep", "22:30", "06:30", "sleep"),
    ])
}

fn ids(day: &Value) -> Vec<String> {
    day["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_str().unwrap().to_owned())
        .collect()
}

fn find<'a>(day: &'a Value, id: &str) -> &'a Value {
    day["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["id"] == id)
        .unwrap_or_else(|| panic!("no {id} in {day}"))
}

#[test]
fn day_plans_over_a_vault() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("vault");
    std::fs::create_dir_all(&dir).unwrap();
    let app = app_list(&dir);
    let (ext, list) = extensions(tmp.path(), &app);
    assert!(ext.host().get(ID).is_some());

    // Templates and rules, as notes.
    run(
        &list,
        "DAYPLAN_TEMPLATE",
        json!({ "name": "Workday", "blocks": workday(), "description": "Three blocks.", "color": "#3b82f6" }),
    );
    run(
        &list,
        "DAYPLAN_TEMPLATE",
        json!({ "name": "Rest day", "blocks": [block("sleep", "23:00", "07:30", "sleep")] }),
    );
    run(
        &list,
        "DAYPLAN_TEMPLATE",
        json!({ "name": "Gig day", "blocks": [block("load-in", "16:00", "18:00", "other")] }),
    );
    let text = read(&dir, "Calendar/Day templates/Workday.md");
    for line in ["type: day-template", "name: Workday", "Three blocks."] {
        assert!(text.contains(line), "{line:?} in\n{text}");
    }
    let rules = run(
        &list,
        "DAYPLAN_TEMPLATE_RULES",
        json!({ "rules": [
            { "template": "gig day", "tag": "#gig" },
            { "template": "Workday", "weekdays": ["mon", "tue", "wed", "thu", "fri"] }
        ], "fallback": "Rest day" }),
    );
    assert_eq!(rules["rules"][1]["words"], "Mon–Fri", "{rules}");
    assert_eq!(
        rules["rules"][0]["template"], "Gig day",
        "the name as the template has it"
    );
    assert!(read(&dir, "Calendar/Day rules.md").contains("type: day-template-rules"));
    let bad = try_run(
        &list,
        "DAYPLAN_TEMPLATE_RULES",
        json!({ "rules": [{ "template": "Nope", "weekdays": ["mon"] }] }),
    );
    assert!(bad.unwrap_err().contains("no day template \"Nope\""));

    // Monday is a workday, by rule 2.
    let mon = run(&list, "DAYPLAN_GET", json!({ "date": day(0) }));
    assert_eq!(mon["template"], "Workday", "{mon}");
    assert_eq!(mon["reason"], "Rule 2: Mon–Fri");
    assert_eq!(mon["edited"], false);
    assert_eq!(ids(&mon)[0], "reset");
    assert_eq!(find(&mon, "block-1")["colour"], "#3b82f6");
    assert_eq!(
        run(&list, "DAYPLAN_RESOLVE", json!({ "date": day(5) }))["template"],
        "Rest day"
    );

    // A gig on Tuesday: the tag rule comes first.
    run(
        &list,
        "CALENDAR_CREATE",
        json!({ "title": "Show at The Hall #gig", "start": format!("{}T20:00", day(1)) }),
    );
    let tue = run(&list, "DAYPLAN_RESOLVE", json!({ "date": day(1) }));
    assert_eq!(tue["template"], "Gig day", "{tue}");
    assert_eq!(tue["reason"], "Rule 1: a day with #gig");

    // Block 1 given to a project, moved: Monday has a plan of its own.
    let mon = run(
        &list,
        "DAYPLAN_ASSIGN",
        json!({ "date": day(0), "block": "block-1", "kind": "project", "path": "Projects/Album/project.md", "title": "Album" }),
    );
    assert_eq!(mon["edited"], true);
    assert_eq!(find(&mon, "block-1")["assignment"]["title"], "Album");
    run(
        &list,
        "DAYPLAN_ASSIGN",
        json!({ "date": day(0), "block": "lunch-prep", "kind": "recipe", "path": "Recipes/Lentil soup.md" }),
    );
    let mon = run(
        &list,
        "DAYPLAN_SET_BLOCK",
        json!({ "date": day(0), "block": "block-3", "start": "17:30", "end": "19:30" }),
    );
    assert_eq!(find(&mon, "block-3")["start"], "17:30");
    assert_eq!(
        find(&mon, "lunch-prep")["assignment"]["title"],
        "Lentil soup"
    );
    let plan = format!("Calendar/Day plans/{}.md", day(0));
    let text = read(&dir, &plan);
    for line in [
        "type: day-plan",
        "template: Workday",
        "[[Projects/Album/project\\|Album]]",
    ] {
        assert!(text.contains(line), "{line:?} in\n{text}");
    }
    assert!(
        try_run(
            &list,
            "DAYPLAN_ASSIGN",
            json!({ "date": day(0), "block": "block-1", "kind": "task" })
        )
        .is_err(),
        "a task needs its note"
    );

    // The workday changes from today on: Thursday follows, Monday keeps
    // its edits.
    let mut changed = workday();
    changed[1]["start"] = json!("09:00");
    run(
        &list,
        "DAYPLAN_TEMPLATE",
        json!({ "name": "Workday", "blocks": changed }),
    );
    assert!(read(&dir, "Calendar/Day templates/Workday.md").contains("history:"));
    let thu = run(&list, "DAYPLAN_GET", json!({ "date": day(3) }));
    assert_eq!(find(&thu, "block-1")["start"], "09:00");
    let mon = run(&list, "DAYPLAN_GET", json!({ "date": day(0) }));
    assert_eq!(find(&mon, "block-1")["start"], "08:00");
    assert_eq!(mon["reason"], "Rule 2: Mon–Fri");

    // Overrides: Wednesday by hand, a whole week, cleared.
    let o = run(
        &list,
        "DAYPLAN_OVERRIDE",
        json!({ "date": day(2), "template": "Rest day" }),
    );
    assert_eq!(o["days"][0]["reason"], "Override for this day", "{o}");
    let wed = run(&list, "DAYPLAN_GET", json!({ "date": day(2) }));
    assert_eq!(
        (wed["template"].as_str(), wed["chosen"].as_bool()),
        (Some("Rest day"), Some(true))
    );
    run(
        &list,
        "DAYPLAN_OVERRIDE",
        json!({ "week": day(7), "template": "none" }),
    );
    let next = run(&list, "DAYPLAN_GET", json!({ "date": day(8) }));
    assert!(next["template"].is_null() && next["blocks"].as_array().unwrap().is_empty());
    run(
        &list,
        "DAYPLAN_OVERRIDE",
        json!({ "date": day(2), "clear": true }),
    );
    assert_eq!(
        run(&list, "DAYPLAN_RESOLVE", json!({ "date": day(2) }))["template"],
        "Workday"
    );

    // Friday becomes a rest day by its own weekday rule.
    let r = run(
        &list,
        "DAYPLAN_SET_WEEKDAY",
        json!({ "weekdays": ["fri"], "template": "Rest day" }),
    );
    let words: Vec<&str> = r["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["words"].as_str().unwrap())
        .collect();
    assert_eq!(words, ["a day with #gig", "Fri", "Mon–Thu"]);
    assert_eq!(
        run(&list, "DAYPLAN_RESOLVE", json!({ "date": day(4) }))["reason"],
        "Rule 2: Fri"
    );

    // An hour tracked on Thursday morning: inside block 1 (09:00–11:00).
    run(
        &app,
        "VAULT_CREATE",
        json!({ "path": "Notes/Work.md", "body": "Work." }),
    );
    run(
        &list,
        "TIME_LOG",
        json!({ "note": "Notes/Work.md", "start": format!("{}T09:30:00{}", day(3), Local::now().format("%:z")),
                "end": format!("{}T10:30:00{}", day(3), Local::now().format("%:z")) }),
    );
    let thu = run(&list, "DAYPLAN_GET", json!({ "date": day(3) }));
    assert_eq!(find(&thu, "block-1")["tracked"], 60, "{thu}");
    assert_eq!(find(&thu, "block-1")["minutes"], 120);

    // Several days at once, for the Calendar's views.
    let week = run(
        &list,
        "DAYPLAN_DAYS",
        json!({ "from": day(0), "to": day(7) }),
    );
    assert_eq!(week["days"].as_array().unwrap().len(), 7);
    let templates = run(&list, "DAYPLAN_TEMPLATES", json!({}));
    let work = templates["templates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "Workday")
        .unwrap();
    assert_eq!(work["used_by"], json!(["Rule 3: Mon–Thu"]));
    assert_eq!(work["allocatable_minutes"], 7 * 60);

    // Reset to template: the plan goes.
    let reset = run(&list, "DAYPLAN_RESET", json!({ "date": day(0) }));
    assert_eq!(
        (reset["reset"].as_bool(), reset["edited"].as_bool()),
        (Some(true), Some(false))
    );
    assert!(!dir.join(&plan).exists());
    assert_eq!(find(&reset, "block-1")["start"], "09:00");

    // A rename: the rules follow.
    run(
        &list,
        "DAYPLAN_TEMPLATE",
        json!({ "name": "Rest day", "rename": "Easy day" }),
    );
    assert!(dir.join("Calendar/Day templates/Easy day.md").exists());
    assert_eq!(
        run(&list, "DAYPLAN_RESOLVE", json!({ "date": day(4) }))["template"],
        "Easy day"
    );
}
