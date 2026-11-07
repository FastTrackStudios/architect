//! Day plans as notes: their frontmatter read and written, Task's
//! `task:features/scheduling/scheduling/src/{parse,write}/day_template.rs`
//! and `day_plan.rs` in the calendar's folder.
//!
//! ```yaml
//! # Calendar/Day templates/Workday.md
//! type: day-template
//! name: Workday
//! color: "#3b82f6"
//! blocks:
//!   - { id: reset, start: "06:30", end: "07:00", label: Morning reset, category: reset }
//!   - { id: block-1, start: "08:00", end: "11:00", label: Block 1, category: allocatable }
//!   - { id: lunch, start: "12:00", end: "12:45", label: Lunch prep, category: meal }
//! history:   # the blocks before a change, until the day before it
//!   - { until: 2026-11-06, blocks: [...] }
//!
//! # Calendar/Day rules.md
//! type: day-template-rules
//! rules:
//!   - { template: Gig day, tag: gig }
//!   - { template: Workday, weekdays: [mon, tue, wed, thu, fri] }
//!   - { template: Studio day, rrule: "FREQ=WEEKLY;INTERVAL=2;BYDAY=SA", start: 2026-11-07 }
//! fallback: Rest day
//! overrides:
//!   - { from: 2026-11-12, to: 2026-11-12, span: day, template: Studio day }
//!
//! # Calendar/Day plans/2026-11-12.md
//! type: day-plan
//! date: 2026-11-12
//! template: Workday
//! reason: "Rule 2: Mon–Fri"
//! blocks:
//!   - { id: block-1, start: "08:00", end: "11:00", label: Block 1, category: allocatable,
//!       assignment: { kind: project, title: Album, path: Projects/Album/project.md } }
//! ```
//!
//! The body under a template's or a plan's frontmatter is a table of its
//! blocks, written for people; the frontmatter is what is read.

use std::fmt::Write as _;

use chrono::NaiveDate;
use serde_json::{Map, Value, json};

use crate::model::{DayPlan, DayTemplate, PlannedBlock, Revision, TimeBlock};
use crate::rules::{Override, Rule, Rules, RulesRevision};

/// A template's `type`.
pub const TEMPLATE: &str = "day-template";
/// The rules note's.
pub const RULES: &str = "day-template-rules";
/// A plan's.
pub const PLAN: &str = "day-plan";

/// The keys the three are read by.
pub const KEYS: &[&str] = &[
    "type",
    "name",
    "description",
    "color",
    "blocks",
    "history",
    "rules",
    "fallback",
    "overrides",
    "date",
    "template",
    "reason",
];

/// The folder under the calendar's that holds the templates.
pub const TEMPLATES_FOLDER: &str = "Day templates";
/// … the plans.
pub const PLANS_FOLDER: &str = "Day plans";
/// The rules note's name.
pub const RULES_NOTE: &str = "Day rules.md";

fn under(folder: &str, sub: &str) -> String {
    let f = folder.trim_matches('/');
    if f.is_empty() {
        sub.to_owned()
    } else {
        format!("{f}/{sub}")
    }
}

/// Where template `name` is written.
#[must_use]
pub fn template_path(folder: &str, name: &str) -> String {
    let safe: String = name
        .trim()
        .chars()
        .map(|c| {
            if "/\\:*?\"<>|#^[]".contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect();
    under(folder, &format!("{TEMPLATES_FOLDER}/{safe}.md"))
}

/// Where `date`'s plan is written.
#[must_use]
pub fn plan_path(folder: &str, date: NaiveDate) -> String {
    under(folder, &format!("{PLANS_FOLDER}/{date}.md"))
}

/// Where the rules are.
#[must_use]
pub fn rules_path(folder: &str) -> String {
    under(folder, RULES_NOTE)
}

/// A note's name without its folder and extension.
fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name)
}

fn text(v: &Map<String, Value>, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Each element of list `k` that reads as a `T`; the rest are skipped.
fn each<T: serde::de::DeserializeOwned>(v: &Map<String, Value>, k: &str) -> Vec<T> {
    v.get(k)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|x| serde_json::from_value(x.clone()).ok())
        .collect()
}

fn date(v: &Map<String, Value>, k: &str) -> Option<NaiveDate> {
    text(v, k)?.get(..10)?.parse().ok()
}

/// A template from its note's frontmatter.
#[must_use]
pub fn template_from(path: &str, v: &Map<String, Value>) -> DayTemplate {
    let mut history: Vec<Revision> = each(v, "history");
    history.sort_by_key(|r| r.until);
    DayTemplate {
        path: path.to_owned(),
        name: text(v, "name").unwrap_or_else(|| stem(path).to_owned()),
        description: text(v, "description"),
        color: text(v, "color"),
        blocks: each(v, "blocks"),
        history,
    }
}

/// The rules from their note's frontmatter.
#[must_use]
pub fn rules_from(path: &str, v: &Map<String, Value>) -> Rules {
    let mut history: Vec<RulesRevision> = each(v, "history");
    history.sort_by_key(|r| r.until);
    Rules {
        path: path.to_owned(),
        rules: each(v, "rules"),
        fallback: text(v, "fallback"),
        history,
        overrides: each(v, "overrides"),
    }
}

/// A plan from its note's frontmatter (its date from the frontmatter, else
/// its name).
#[must_use]
pub fn plan_from(path: &str, v: &Map<String, Value>) -> Option<DayPlan> {
    Some(DayPlan {
        path: path.to_owned(),
        date: date(v, "date").or_else(|| stem(path).parse().ok())?,
        template: text(v, "template"),
        reason: text(v, "reason"),
        blocks: each(v, "blocks"),
    })
}

fn to_value<T: serde::Serialize>(t: &T) -> Value {
    serde_json::to_value(t).unwrap_or(Value::Null)
}

/// A template's frontmatter.
#[must_use]
pub fn template_props(t: &DayTemplate) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("type".into(), json!(TEMPLATE));
    m.insert("name".into(), json!(t.name));
    if let Some(d) = &t.description {
        m.insert("description".into(), json!(d));
    }
    if let Some(c) = &t.color {
        m.insert("color".into(), json!(c));
    }
    m.insert("blocks".into(), to_value(&t.blocks));
    if !t.history.is_empty() {
        m.insert("history".into(), to_value(&t.history));
    }
    m
}

/// The rules' frontmatter.
#[must_use]
pub fn rules_props(r: &Rules) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("type".into(), json!(RULES));
    m.insert("rules".into(), to_value::<Vec<Rule>>(&r.rules));
    if let Some(f) = &r.fallback {
        m.insert("fallback".into(), json!(f));
    }
    if !r.history.is_empty() {
        m.insert("history".into(), to_value(&r.history));
    }
    m.insert("overrides".into(), to_value::<Vec<Override>>(&r.overrides));
    m
}

/// A plan's frontmatter.
#[must_use]
pub fn plan_props(p: &DayPlan) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("type".into(), json!(PLAN));
    m.insert("date".into(), json!(p.date.to_string()));
    if let Some(t) = &p.template {
        m.insert("template".into(), json!(t));
    }
    if let Some(r) = &p.reason {
        m.insert("reason".into(), json!(r));
    }
    m.insert("blocks".into(), to_value(&p.blocks));
    m
}

fn cell(s: &str) -> String {
    s.replace('|', "\\|")
}

/// A template's body: its blocks as a table.
#[must_use]
pub fn template_body(t: &DayTemplate) -> String {
    let mut out = format!("# {}\n\n", t.name);
    if let Some(d) = &t.description {
        let _ = writeln!(out, "{d}\n");
    }
    out.push_str("| Time | Block | Category |\n|---|---|---|\n");
    for b in &t.blocks {
        block_row(&mut out, b);
    }
    out
}

fn block_row(out: &mut String, b: &TimeBlock) {
    let _ = writeln!(
        out,
        "| {}–{} | {}{} | {} |",
        b.start,
        b.end,
        cell(&b.label),
        if b.fixed { " (fixed)" } else { "" },
        b.category.name()
    );
}

/// A plan's body: its blocks and what each is given to.
#[must_use]
pub fn plan_body(p: &DayPlan) -> String {
    let mut out = format!("# Plan for {}\n\n", p.date.format("%A, %-d %B %Y"));
    if let Some(t) = &p.template {
        let _ = writeln!(out, "From the {t} template.\n");
    }
    out.push_str("| Time | Block | For |\n|---|---|---|\n");
    for b in &p.blocks {
        plan_row(&mut out, b);
    }
    out
}

fn plan_row(out: &mut String, b: &PlannedBlock) {
    let what = b.assignment.as_ref().map_or_else(String::new, |a| {
        a.path.as_deref().map_or_else(
            || cell(&a.title),
            |p| format!("[[{}\\|{}]]", p.trim_end_matches(".md"), cell(&a.title)),
        )
    });
    let _ = writeln!(
        out,
        "| {}–{} | {} | {what} |",
        b.start,
        b.end,
        cell(&b.label)
    );
}
