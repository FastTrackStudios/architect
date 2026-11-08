//! Day plans (#444): templates, the rules that pick them, overrides, the
//! past kept as it was, edits kept, and the notes they are written as.

#![allow(clippy::unwrap_used)]

use architect_dayplan::notes;
use architect_dayplan::{
    AssignmentKind, BlockAssignment, BlockCategory, Book, DayPlan, DayTemplate, Override, Rule,
    Rules, Span, TimeBlock, TimeBlockId, TimeOfDay, When, no_tags,
};
use chrono::{NaiveDate, Weekday};
use serde_json::{Map, Value, json};

fn d(text: &str) -> NaiveDate {
    text.parse().unwrap()
}

fn t(text: &str) -> TimeOfDay {
    TimeOfDay::parse(text).unwrap()
}

fn block(id: &str, start: &str, end: &str, category: BlockCategory) -> TimeBlock {
    TimeBlock {
        id: TimeBlockId(id.into()),
        start: t(start),
        end: t(end),
        label: id.into(),
        category,
        note: None,
        fixed: false,
    }
}

fn template(name: &str, blocks: Vec<TimeBlock>) -> DayTemplate {
    DayTemplate {
        path: notes::template_path("Calendar", name),
        name: name.into(),
        blocks,
        ..DayTemplate::default()
    }
}

fn weekdays(template: &str, days: &[Weekday]) -> Rule {
    Rule {
        template: template.into(),
        when: When::Weekdays(days.to_vec()),
    }
}

const WORK: [Weekday; 5] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
];

/// Cody's: three allocatable blocks, meal prep, reset, sleep.
fn book() -> Book {
    Book {
        templates: vec![
            template(
                "Workday",
                vec![
                    block("reset", "06:30", "07:00", BlockCategory::Reset),
                    block("breakfast", "07:00", "07:30", BlockCategory::Meal),
                    block("block-1", "08:00", "11:00", BlockCategory::Allocatable),
                    block("lunch-prep", "12:00", "12:45", BlockCategory::Meal),
                    block("block-2", "13:00", "16:00", BlockCategory::Allocatable),
                    block("block-3", "17:00", "19:00", BlockCategory::Allocatable),
                    block("sleep", "22:30", "06:30", BlockCategory::Sleep),
                ],
            ),
            template(
                "Rest day",
                vec![block("sleep", "23:00", "07:30", BlockCategory::Sleep)],
            ),
            template(
                "Gig day",
                vec![block("load-in", "16:00", "18:00", BlockCategory::Other)],
            ),
        ],
        rules: Rules {
            rules: vec![
                Rule {
                    template: "Gig day".into(),
                    when: When::Tag("gig".into()),
                },
                weekdays("Workday", &WORK),
            ],
            fallback: Some("Rest day".into()),
            ..Rules::default()
        },
        plans: Vec::new(),
    }
}

// Fri 6, Sat 7 (today), Sun 8, Mon 9 November 2026.
const TODAY: &str = "2026-11-07";

#[test]
fn the_first_rule_that_matches_picks_the_template_and_says_why() {
    let b = book();
    let today = d(TODAY);
    let fri = b.day(d("2026-11-06"), today, &no_tags);
    assert_eq!(fri.template.as_deref(), Some("Workday"));
    assert_eq!(fri.reason, "Rule 2: Mon–Fri");
    assert!(fri.past && !fri.edited && !fri.chosen);
    assert_eq!(fri.blocks.len(), 7);
    assert_eq!(fri.blocks[0].id.0, "reset");
    assert_eq!(
        fri.blocks.last().unwrap().id.0,
        "sleep",
        "a wrap sorts last"
    );

    let sun = b.day(d("2026-11-08"), today, &no_tags);
    assert_eq!(sun.template.as_deref(), Some("Rest day"));
    assert_eq!(sun.reason, "No rule matched: the default");

    // A day with an event tagged #gig, before the weekday rule.
    let gig = |tag: &str, day: NaiveDate| tag == "gig" && day == d("2026-11-09");
    let mon = b.day(d("2026-11-09"), today, &gig);
    assert_eq!(mon.template.as_deref(), Some("Gig day"));
    assert_eq!(mon.reason, "Rule 1: a day with #gig");
}

#[test]
fn rrules_and_date_ranges_pick_days() {
    let every_other_sat = Rule {
        template: "Studio day".into(),
        when: When::Rrule {
            rule: "FREQ=WEEKLY;INTERVAL=2;BYDAY=SA".into(),
            start: d("2026-11-07"),
        },
    };
    assert!(every_other_sat.matches(d("2026-11-07"), &no_tags));
    assert!(!every_other_sat.matches(d("2026-11-14"), &no_tags));
    assert!(every_other_sat.matches(d("2026-11-21"), &no_tags));
    assert!(
        !every_other_sat.matches(d("2026-10-31"), &no_tags),
        "before its start"
    );

    let tour = Rule {
        template: "Road day".into(),
        when: When::Dates {
            from: d("2026-12-01"),
            to: d("2026-12-07"),
        },
    };
    assert!(tour.matches(d("2026-12-07"), &no_tags));
    assert!(!tour.matches(d("2026-12-08"), &no_tags));
    assert_eq!(tour.words(), "1 Dec – 7 Dec 2026");
}

#[test]
fn overrides_beat_rules_and_a_days_beats_a_weeks() {
    let mut b = book();
    let today = d(TODAY);
    b.rules.set_override(
        Override::week(d("2026-11-09"), Some("Rest day".into())),
        false,
    );
    b.rules.set_override(
        Override::day(d("2026-11-12"), Some("Gig day".into())),
        false,
    );
    b.rules
        .set_override(Override::day(d("2026-11-13"), None), false);

    let mon = b.day(d("2026-11-09"), today, &no_tags);
    assert_eq!(mon.template.as_deref(), Some("Rest day"));
    assert_eq!(mon.reason, "Override for the week of 9 Nov");
    assert!(mon.chosen);
    let thu = b.day(d("2026-11-12"), today, &no_tags);
    assert_eq!(thu.template.as_deref(), Some("Gig day"));
    assert_eq!(thu.reason, "Override for this day");
    let fri = b.day(d("2026-11-13"), today, &no_tags);
    assert_eq!(fri.template, None, "cleared by hand");
    assert!(fri.blocks.is_empty());

    // Cleared again: the week's applies.
    b.rules
        .set_override(Override::day(d("2026-11-13"), None), true);
    let fri = b.day(d("2026-11-13"), today, &no_tags);
    assert_eq!(fri.template.as_deref(), Some("Rest day"));
    assert_eq!(
        b.rules.override_on(d("2026-11-13")).unwrap().span,
        Span::Week
    );
}

#[test]
fn a_rule_change_reaches_today_and_after_and_never_the_past() {
    let mut b = book();
    let today = d(TODAY);
    let yesterday = b.day(d("2026-11-06"), today, &no_tags);
    let monday = b.day(d("2026-11-09"), today, &no_tags);
    assert_eq!(monday.template.as_deref(), Some("Workday"));

    // Every day is a rest day from now on.
    let rules = vec![weekdays(
        "Rest day",
        &[
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
            Weekday::Sat,
            Weekday::Sun,
        ],
    )];
    b.rules.change(rules.clone(), None, today);

    assert_eq!(
        b.day(d("2026-11-06"), today, &no_tags),
        yesterday,
        "yesterday's template and plan are as they were"
    );
    let monday = b.day(d("2026-11-09"), today, &no_tags);
    assert_eq!(monday.template.as_deref(), Some("Rest day"));
    assert_eq!(monday.reason, "Rule 1: every day");
    assert_eq!(
        b.day(d("2026-11-07"), today, &no_tags).template.as_deref(),
        Some("Rest day"),
        "today follows the new rules"
    );

    // A second change today keeps the first history, not today's rules.
    b.rules
        .change(vec![weekdays("Workday", &WORK)], None, today);
    assert_eq!(b.rules.history.len(), 1);
    assert_eq!(b.day(d("2026-11-06"), today, &no_tags), yesterday);

    // A day later, another change: the day before keeps the second set.
    let tomorrow = d("2026-11-08");
    b.rules.change(rules, None, tomorrow);
    assert_eq!(b.rules.history.len(), 2);
    assert_eq!(
        b.day(d("2026-11-07"), tomorrow, &no_tags).template,
        None,
        "Saturday under the second set: no rule, no fallback"
    );
    assert_eq!(b.day(d("2026-11-06"), tomorrow, &no_tags), {
        let mut y = yesterday;
        y.past = true;
        y
    });
}

#[test]
fn an_override_is_the_one_thing_that_changes_a_past_day() {
    let mut b = book();
    let today = d(TODAY);
    b.rules.set_override(
        Override::day(d("2026-11-05"), Some("Rest day".into())),
        false,
    );
    let thu = b.day(d("2026-11-05"), today, &no_tags);
    assert_eq!(thu.template.as_deref(), Some("Rest day"));
    assert!(thu.past && thu.chosen);
}

#[test]
fn a_template_change_reaches_unedited_days_from_today_and_edited_ones_keep_theirs() {
    let mut b = book();
    let today = d(TODAY);
    let friday = b.day(d("2026-11-06"), today, &no_tags);

    // Monday is edited: block 1 given to the Album.
    let mut monday = b.day(d("2026-11-10"), today, &no_tags);
    architect_dayplan::assign(
        &mut monday,
        "block-1",
        Some(BlockAssignment {
            kind: AssignmentKind::Project,
            title: "Album".into(),
            path: Some("Projects/Album/project.md".into()),
        }),
    )
    .unwrap();
    let saved = monday.plan(notes::plan_path("Calendar", monday.date));
    assert_eq!(saved.path, "Calendar/Day plans/2026-11-10.md");
    b.plans.push(saved);

    // The workday's first block starts later from today on.
    let mut blocks = b.templates[0].blocks.clone();
    blocks[2].start = t("09:00");
    b.templates[0].change_blocks(blocks.clone(), today);
    b.templates[0].change_blocks(blocks, today);
    assert_eq!(
        b.templates[0].history.len(),
        1,
        "the same blocks twice: kept once"
    );

    assert_eq!(
        b.day(d("2026-11-06"), today, &no_tags),
        friday,
        "the past is kept"
    );
    let wed = b.day(d("2026-11-11"), today, &no_tags);
    assert_eq!(wed.block("block-1").unwrap().start, t("09:00"));
    let tue = b.day(d("2026-11-10"), today, &no_tags);
    assert!(tue.edited);
    let kept = tue.block("block-1").unwrap();
    assert_eq!(kept.start, t("08:00"), "an edited day keeps its edits");
    assert_eq!(kept.title(), "Album");
    assert_eq!(tue.reason, "Rule 2: Mon–Fri");

    // Reset to template: its plan goes.
    b.plans.clear();
    assert!(!b.day(d("2026-11-10"), today, &no_tags).edited);
}

#[test]
fn a_block_moves_and_tracked_time_is_set_against_it() {
    let b = book();
    let mut day = b.day(d("2026-11-09"), d(TODAY), &no_tags);
    architect_dayplan::set_span(&mut day, "block-3", t("05:00"), t("06:00")).unwrap();
    assert_eq!(day.blocks[0].id.0, "block-3", "sorted again");
    assert!(architect_dayplan::set_span(&mut day, "nope", t("05:00"), t("06:00")).is_err());

    // 08:30–09:30 and 10:30–12:30 tracked: 1h30 of block 1's three hours.
    let got = architect_dayplan::planned_vs_tracked(&day.blocks, &[(510, 570), (630, 750)]);
    let b1 = got.iter().find(|(id, ..)| id == "block-1").unwrap();
    assert_eq!((b1.1, b1.2), (180, 90));
    let lunch = got.iter().find(|(id, ..)| id == "lunch-prep").unwrap();
    assert_eq!((lunch.1, lunch.2), (45, 30));
}

fn map(v: &Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

#[test]
fn templates_rules_and_plans_round_trip_through_their_notes() {
    let mut b = book();
    let today = d(TODAY);
    let mut tpl = b.templates[0].clone();
    tpl.color = Some("#3b82f6".into());
    tpl.blocks[3].fixed = true;
    tpl.change_blocks(
        vec![block("all", "00:00", "24:00", BlockCategory::Other)],
        today,
    );
    let props = notes::template_props(&tpl);
    assert_eq!(props["type"], "day-template");
    assert_eq!(props["blocks"][0]["start"], "00:00");
    assert_eq!(props["blocks"][0]["end"], "24:00");
    assert_eq!(notes::template_from(&tpl.path, &props), tpl);
    let body = notes::template_body(&b.templates[0]);
    assert!(
        body.contains("| 08:00–11:00 | block-1 | Allocatable |"),
        "{body}"
    );

    b.rules
        .change(vec![weekdays("Workday", &WORK)], None, today);
    b.rules
        .set_override(Override::day(d("2026-11-12"), None), false);
    b.rules.path = notes::rules_path("Calendar");
    let props = notes::rules_props(&b.rules);
    assert_eq!(
        props["rules"][0],
        json!({"template": "Workday", "weekdays": ["mon", "tue", "wed", "thu", "fri"]})
    );
    assert_eq!(
        props["history"][0]["rules"][0],
        json!({"template": "Gig day", "tag": "gig"})
    );
    assert_eq!(notes::rules_from(&b.rules.path, &props), b.rules);

    let mut day = b.day(d("2026-11-10"), today, &no_tags);
    architect_dayplan::assign(
        &mut day,
        "lunch-prep",
        Some(BlockAssignment {
            kind: AssignmentKind::Recipe,
            title: "Lentil soup".into(),
            path: Some("Recipes/Lentil soup.md".into()),
        }),
    )
    .unwrap();
    let plan: DayPlan = day.plan(notes::plan_path("Calendar", day.date));
    let props = notes::plan_props(&plan);
    assert_eq!(props["blocks"][3]["assignment"]["kind"], "recipe");
    assert_eq!(notes::plan_from(&plan.path, &props), Some(plan.clone()));
    assert!(
        notes::plan_body(&plan)
            .contains("| 12:00–12:45 | lunch-prep | [[Recipes/Lentil soup\\|Lentil soup]] |")
    );

    // What the vault reads back from YAML: an unquoted 07:30 is a number
    // of minutes (YAML 1.1's base 60), a category in any case, a bad
    // block skipped.
    let read = notes::template_from(
        "Calendar/Day templates/Quick.md",
        &map(&json!({
            "blocks": [
                {"id": "a", "start": 450, "end": "08:00", "label": "A", "category": "Wind-down"},
                {"id": "b", "start": "25:00", "end": "08:00", "label": "B"},
                {"id": "c", "start": "09:00", "end": "10:00", "label": "C", "category": "lunch?"}
            ]
        })),
    );
    assert_eq!(read.name, "Quick");
    assert_eq!(read.blocks.len(), 2);
    assert_eq!(read.blocks[0].start, t("07:30"));
    assert_eq!(read.blocks[1].category, BlockCategory::Other);
    assert_eq!(BlockCategory::parse("Wind-down"), BlockCategory::WindDown);
}

#[test]
fn a_renamed_template_is_renamed_in_the_rules_and_overrides() {
    let mut r = book().rules;
    r.change(
        vec![weekdays("Workday", &WORK)],
        Some("Workday".into()),
        d(TODAY),
    );
    r.set_override(
        Override::day(d("2026-11-12"), Some("Workday".into())),
        false,
    );
    r.rename_template("workday", "Office day");
    assert_eq!(r.rules[0].template, "Office day");
    assert_eq!(r.fallback.as_deref(), Some("Office day"));
    assert_eq!(r.overrides[0].template.as_deref(), Some("Office day"));
    assert_eq!(r.history[0].rules[1].template, "Office day");
}
