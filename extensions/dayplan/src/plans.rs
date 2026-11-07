//! Day plans in the vault, and the `DAYPLAN_*` actions: Task's
//! `get_day_plan` and `upsert_day_plan`, and the rest.
//!
//! - Templates: `<folder>/Day templates/<name>.md`, written key
//!   by key so the prose a person adds under them stays.
//! - The rules: `<folder>/Day rules.md`.
//! - Plans: `<folder>/Day plans/<date>.md`, one per edited date,
//!   all of it generated (a table of its blocks under the frontmatter).
//!   A plan of its own, rather than the daily note's frontmatter: a day
//!   is planned without making its daily note, a plan is rewritten whole
//!   without touching what is written in the daily note (open in an
//!   editor, perhaps), and "Reset to template" is deleting a file.
//!
//! A rule's `#tag` is read from the Calendar's agenda (`architect.calendar`
//! `agenda`), tracked time from the Time Tracker's entries
//! (`architect.time` `entries`), when they are there.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use chrono::{DateTime, Datelike as _, Duration, Local, NaiveDate, NaiveDateTime, Weekday};
use serde_json::{Map, Value, json};

use crate::{
    AssignmentKind, BlockAssignment, Book, Day, DayTemplate, Override, PlannedBlock, Rule,
    TimeBlock, TimeOfDay, When, book, folder, notes, rules, today, vault, with_book,
};

/// The day plans' functions, as the rest of this file names them.
mod dayplan {
    pub use crate::BlockCategory;
    pub use crate::{assign, no_tags, planned_vs_tracked, resolve, set_span, sort};
}

/// The base that finds the templates, the rules and the plans.
#[must_use]
pub fn notes_base() -> String {
    let mut out = String::from("filters:\n  or:\n");
    for t in [notes::TEMPLATE, notes::RULES, notes::PLAN] {
        let _ = writeln!(out, "    - type == \"{t}\"");
    }
    out.push_str("views:\n  - type: table\n    name: Day plans\n    order:\n");
    for k in notes::KEYS {
        let _ = writeln!(out, "      - {k}");
    }
    out
}

/// Every template, the rules and every plan in the vault.
pub(crate) async fn read() -> Result<Book, String> {
    let rows = vault::rows(&notes_base()).await?;
    let mut book = Book::default();
    let mut rules_found = false;
    for r in rows {
        match r.values.get("type").and_then(Value::as_str) {
            Some(notes::TEMPLATE) => book
                .templates
                .push(notes::template_from(&r.path, &r.values)),
            Some(notes::RULES) if !rules_found => {
                rules_found = true;
                book.rules = notes::rules_from(&r.path, &r.values);
            }
            Some(notes::PLAN) => book.plans.extend(notes::plan_from(&r.path, &r.values)),
            _ => {}
        }
    }
    book.templates.sort_by(|a, b| a.name.cmp(&b.name));
    book.plans.sort_by_key(|p| p.date);
    Ok(book)
}

/// A local time from an RFC 3339 stamp or a local `YYYY-MM-DDTHH:MM`.
fn local(text: &str) -> Option<NaiveDateTime> {
    DateTime::parse_from_rfc3339(text)
        .map(|t| t.with_timezone(&Local).naive_local())
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(text.get(..16)?, "%Y-%m-%dT%H:%M").ok())
        .or_else(|| {
            text.get(..10)?
                .parse::<NaiveDate>()
                .ok()?
                .and_hms_opt(0, 0, 0)
        })
}

/// The tags of the Calendar's events on each day of `from..to` (an
/// event's `tags`, and `#tags` written in its title); none without the
/// Calendar.
pub async fn tags_by_day(from: NaiveDate, to: NaiveDate) -> BTreeMap<NaiveDate, BTreeSet<String>> {
    let args =
        json!({ "from": from.to_string(), "to": to.to_string(), "tasks": false, "time": false });
    let Some(Ok(out)) = vault::service(crate::CALENDAR, "agenda", args).await else {
        return BTreeMap::new();
    };
    tags_of_agenda(&out)
}

/// [`tags_by_day`] from the Calendar's agenda.
#[must_use]
pub fn tags_of_agenda(out: &Value) -> BTreeMap<NaiveDate, BTreeSet<String>> {
    let mut by: BTreeMap<NaiveDate, BTreeSet<String>> = BTreeMap::new();
    for b in out["blocks"].as_array().into_iter().flatten() {
        let title = b["title"].as_str().unwrap_or_default();
        let words = title
            .split_whitespace()
            .filter_map(|w| w.strip_prefix('#'))
            .map(|w| {
                w.trim_end_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
            });
        let tags: BTreeSet<String> = b["tags"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|t| t.trim_start_matches('#').to_lowercase())
            .chain(words)
            .filter(|t| !t.is_empty())
            .collect();
        if tags.is_empty() {
            continue;
        }
        let (Some(start), Some(end)) = (
            b["start"].as_str().and_then(local),
            b["end"].as_str().and_then(local),
        ) else {
            continue;
        };
        let mut day = start.date();
        let last = (end - Duration::seconds(1)).date().max(day);
        while day <= last {
            by.entry(day).or_default().extend(tags.iter().cloned());
            day += Duration::days(1);
        }
    }
    by
}

/// The time tracked over `from..to`, each span from the Time Tracker's
/// entries (a running one up to now); none without it.
pub async fn tracked(from: NaiveDate, to: NaiveDate) -> Vec<(NaiveDateTime, NaiveDateTime)> {
    let args = json!({ "from": from.to_string(), "to": to.to_string() });
    let Some(Ok(out)) = vault::service(crate::TIME, "entries", args).await else {
        return Vec::new();
    };
    let now = Local::now().naive_local();
    out["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let start = local(e["start"].as_str()?)?;
            let end = e["end"].as_str().and_then(local).unwrap_or(now);
            (end > start).then_some((start, end))
        })
        .collect()
}

/// Each day of `from..to` as planned in `book`, `has` giving each day's
/// event tags.
#[must_use]
pub fn days_of(
    book: &Book,
    has: &BTreeMap<NaiveDate, BTreeSet<String>>,
    from: NaiveDate,
    to: NaiveDate,
    today: NaiveDate,
) -> Vec<Day> {
    let has_tag =
        |tag: &str, d: NaiveDate| has.get(&d).is_some_and(|t| t.contains(&tag.to_lowercase()));
    from.iter_days()
        .take_while(|d| *d < to)
        .map(|d| book.day(d, today, &has_tag))
        .collect()
}

/// Each day of `from..to` as planned now.
pub async fn days(from: NaiveDate, to: NaiveDate) -> Vec<Day> {
    let tags = tags_by_day(from, to).await;
    days_of(&book(), &tags, from, to, today())
}

/// `date` as planned now.
pub async fn day(date: NaiveDate) -> Day {
    days(date, date + Duration::days(1))
        .await
        .pop()
        .unwrap_or_else(|| Book::default().day(date, today(), &dayplan::no_tags))
}

/// The tracked spans on `date`, as minutes of the day.
#[must_use]
pub fn tracked_on(time: &[(NaiveDateTime, NaiveDateTime)], date: NaiveDate) -> Vec<(u16, u16)> {
    let (Some(a), Some(b)) = (
        date.and_hms_opt(0, 0, 0),
        (date + Duration::days(1)).and_hms_opt(0, 0, 0),
    ) else {
        return Vec::new();
    };
    let minute = |t: NaiveDateTime| {
        u16::try_from((t.clamp(a, b) - a).num_minutes().clamp(0, 24 * 60)).unwrap_or(0)
    };
    time.iter()
        .filter(|(s, e)| *s < b && *e > a)
        .map(|&(s, e)| (minute(s), minute(e)))
        .collect()
}

/// A day as an answer: the template and why, and each block with what it
/// is given to, its colour, and the minutes tracked in it.
#[must_use]
pub fn day_json(d: &Day, tracked: &[(u16, u16)]) -> Value {
    let vs = dayplan::planned_vs_tracked(&d.blocks, tracked);
    let folder = folder();
    let blocks: Vec<Value> = d
        .blocks
        .iter()
        .zip(vs)
        .map(|(b, (_, planned, got))| {
            let mut v = serde_json::to_value(b).unwrap_or(Value::Null);
            v["colour"] = json!(b.category.colour());
            v["minutes"] = json!(planned);
            v["tracked"] = json!(got);
            v
        })
        .collect();
    json!({
        "date": d.date.to_string(),
        "weekday": d.date.format("%A").to_string(),
        "template": d.template,
        "reason": d.reason,
        "chosen": d.chosen,
        "edited": d.edited,
        "past": d.past,
        "missing": d.missing,
        "color": d
            .template
            .as_deref()
            .and_then(|t| book().template(t).and_then(|t| t.color.clone())),
        "plan": d.edited.then(|| notes::plan_path(&folder, d.date)),
        "blocks": blocks,
    })
}

// ── Writing ─────────────────────────────────────────────────────────

/// Writes `props` to `path`: key by key when the note is there (what a
/// person wrote under it stays), else a new note with `body`.
async fn save_keys(
    path: &str,
    existing: bool,
    props: &Map<String, Value>,
    old: &[&str],
    body: &str,
) -> Result<(), String> {
    if !existing {
        return vault::create(path, props, body).await;
    }
    for (k, v) in props {
        vault::set(path, k, v).await?;
    }
    // Keys it had and has no more (a fallback taken away).
    for k in old.iter().filter(|k| !props.contains_key(**k)) {
        vault::set(path, k, &Value::Null).await?;
    }
    Ok(())
}

/// Saves `day`'s plan, all of it: the note is written again.
async fn save_plan(day: &Day) -> Result<(), String> {
    let folder = folder();
    let plan = day.plan(notes::plan_path(&folder, day.date));
    let existing = book().plan(day.date).is_some();
    if existing {
        vault::run("VAULT_DELETE", json!({ "path": plan.path })).await?;
    }
    vault::create(
        &plan.path,
        &notes::plan_props(&plan),
        &notes::plan_body(&plan),
    )
    .await?;
    with_book(|b| {
        b.plans.retain(|p| p.date != day.date);
        b.plans.push(plan);
        b.plans.sort_by_key(|p| p.date);
    });
    Ok(())
}

/// Deletes `date`'s plan, if it has one: the day is its template again.
async fn drop_plan(date: NaiveDate) -> Result<bool, String> {
    let Some(p) = book().plan(date).cloned() else {
        return Ok(false);
    };
    vault::run("VAULT_DELETE", json!({ "path": p.path })).await?;
    with_book(|b| b.plans.retain(|x| x.date != date));
    Ok(true)
}

/// Saves the rules.
async fn save_rules(r: &rules::Rules) -> Result<(), String> {
    let folder = folder();
    let mut r = r.clone();
    let existing = !r.path.is_empty() && book().rules.path == r.path;
    if r.path.is_empty() {
        r.path = notes::rules_path(&folder);
    }
    let body = "# Day rules\n\nWhich day template each day uses: the first rule that matches, \
                unless an override says otherwise. The Calendar's day plans edit them.\n";
    save_keys(
        &r.path,
        existing,
        &notes::rules_props(&r),
        &["fallback", "history"],
        body,
    )
    .await?;
    with_book(|b| b.rules = r);
    Ok(())
}

/// Saves template `t` (new when `existing` is false).
async fn save_template(t: &DayTemplate, existing: bool) -> Result<(), String> {
    let body = format!(
        "# {}\n\n{}A day template: the Calendar's day plans use it on the days its rules give it.\n",
        t.name,
        t.description
            .as_deref()
            .map_or_else(String::new, |d| format!("{d}\n\n"))
    );
    save_keys(
        &t.path,
        existing,
        &notes::template_props(t),
        &["description", "color", "history"],
        &body,
    )
    .await?;
    with_book(|b| {
        b.templates.retain(|x| x.path != t.path);
        b.templates.push(t.clone());
        b.templates.sort_by(|a, b| a.name.cmp(&b.name));
    });
    Ok(())
}

// ── Arguments ───────────────────────────────────────────────────────

fn str_arg<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn date_arg(args: &Value, k: &str) -> Result<Option<NaiveDate>, String> {
    match str_arg(args, k) {
        None => Ok(None),
        Some("today") => Ok(Some(today())),
        Some("tomorrow") => Ok(Some(today() + Duration::days(1))),
        Some("yesterday") => Ok(Some(today() - Duration::days(1))),
        Some(s) => s
            .get(..10)
            .and_then(|d| d.parse().ok())
            .map(Some)
            .ok_or_else(|| format!("{k}: a date, YYYY-MM-DD ({s:?})")),
    }
}

fn the_date(args: &Value) -> Result<NaiveDate, String> {
    Ok(date_arg(args, "date")?.unwrap_or_else(today))
}

fn time_arg(args: &Value, k: &str) -> Result<TimeOfDay, String> {
    let s = str_arg(args, k).ok_or_else(|| format!("{k}: a time, HH:MM"))?;
    TimeOfDay::parse(s).ok_or_else(|| format!("{k}: a time, HH:MM ({s:?})"))
}

/// A template named `name`, as the rules and overrides name it.
fn template_named(name: &str) -> Result<String, String> {
    let b = book();
    b.template(name).map(|t| t.name.clone()).ok_or_else(|| {
        let names: Vec<String> = b.templates.iter().map(|t| t.name.clone()).collect();
        format!("no day template {name:?} (there are: {})", names.join(", "))
    })
}

/// `template`: a template's name, or `none` / null for no template.
fn template_arg(args: &Value) -> Result<Option<String>, String> {
    match str_arg(args, "template") {
        None | Some("none" | "None") => Ok(None),
        Some(t) => template_named(t).map(Some),
    }
}

/// Reads the vault again before an action: a note written elsewhere (an
/// agent's, another device's) counts.
async fn fresh() {
    let _ = crate::refresh().await;
}

async fn answer(d: &Day) -> Value {
    let time = tracked(d.date, d.date + Duration::days(1)).await;
    day_json(d, &tracked_on(&time, d.date))
}

/// Several days as answers, the time read once.
async fn answers(list: &[Day]) -> Vec<Value> {
    let (Some(head), Some(tail)) = (list.first(), list.last()) else {
        return Vec::new();
    };
    let time = tracked(head.date, tail.date + Duration::days(1)).await;
    list.iter()
        .map(|d| day_json(d, &tracked_on(&time, d.date)))
        .collect()
}

// ── Actions ─────────────────────────────────────────────────────────

/// `DAYPLAN_GET`: Task's `get_day_plan`. `date` (today by default), or
/// `from`..`to` (`to` exclusive, at most 62 days) for several.
pub async fn get(args: Value) -> Result<Value, String> {
    fresh().await;
    if let (Some(from), Some(to)) = (date_arg(&args, "from")?, date_arg(&args, "to")?) {
        if to <= from || (to - from).num_days() > 62 {
            return Err("from..to: one to 62 days".into());
        }
        return Ok(json!({ "days": answers(&days(from, to).await).await }));
    }
    Ok(answer(&day(the_date(&args)?).await).await)
}

/// `DAYPLAN_DAYS`: each day of `from`..`to` (at most 62) as planned.
///
/// For the Calendar's day and week views (its service's `days`): the
/// template and why, the blocks, and the time tracked in each. Read from
/// what it last read, without the vault again.
pub async fn days_action(args: Value) -> Result<Value, String> {
    if !crate::cache_read() {
        let _ = crate::refresh().await;
    }
    let from = date_arg(&args, "from")?.unwrap_or_else(today);
    let to = date_arg(&args, "to")?.unwrap_or(from + Duration::days(7));
    if to <= from || (to - from).num_days() > 62 {
        return Err("from..to: one to 62 days".into());
    }
    Ok(json!({ "days": answers(&days(from, to).await).await, "revision": crate::revision() }))
}

/// Block `v` of an upsert: a template block's keys, an assignment.
fn block_of(v: &Value, n: usize) -> Result<PlannedBlock, String> {
    let mut v = v.clone();
    if v.get("id").is_none_or(Value::is_null) {
        v["id"] = json!(format!("block-{}", n + 1));
    }
    if v.get("label").is_none_or(Value::is_null) {
        v["label"] = json!("Block");
    }
    serde_json::from_value(v).map_err(|e| format!("block {}: {e}", n + 1))
}

/// `DAYPLAN_UPSERT`: Task's `upsert_day_plan`.
///
/// Saves `date`'s plan as `blocks` (each `{id, start, end, label, category, assignment, fixed}`);
/// `merge` fills the rest of the day from its template; `reconcile`
/// reflows the blocks around the fixed ones (and `obstacles`,
/// `[["HH:MM","HH:MM"]]`) and says what moved.
pub async fn upsert(args: Value) -> Result<Value, String> {
    fresh().await;
    let date = the_date(&args)?;
    let mut d = day(date).await;
    if let Some(list) = args.get("blocks").and_then(Value::as_array) {
        let mut blocks = list
            .iter()
            .enumerate()
            .map(|(i, v)| block_of(v, i))
            .collect::<Result<Vec<_>, _>>()?;
        if args.get("merge").and_then(Value::as_bool).unwrap_or(false) {
            let tpl: Vec<TimeBlock> = d
                .template
                .as_deref()
                .and_then(|t| book().template(t).map(|t| t.blocks_on(date).to_vec()))
                .unwrap_or_default();
            blocks = dayplan::resolve::merge_template(&blocks, &tpl)
                .into_iter()
                .map(|r| r.block)
                .collect();
        }
        d.blocks = blocks;
    }
    if args.get("template").is_some() {
        d.template = template_arg(&args)?;
    }
    let changes = if args
        .get("reconcile")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let obstacles: Vec<(u16, u16)> = args
            .get("obstacles")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|o| {
                let s = TimeOfDay::parse(o.get(0)?.as_str()?)?;
                let e = TimeOfDay::parse(o.get(1)?.as_str()?)?;
                Some(dayplan::resolve::day_span(s, e))
            })
            .collect();
        let out = dayplan::resolve::reconcile(d.blocks.clone(), &obstacles);
        d.blocks = out.blocks;
        serde_json::to_value(&out.changes).unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    dayplan::sort(&mut d.blocks);
    save_plan(&d).await?;
    let mut out = answer(&day(date).await).await;
    out["changes"] = changes;
    Ok(out)
}

/// `DAYPLAN_ASSIGN`: gives block `block` of `date` to a `task`, a
/// `project` or a `recipe` (its note's `path`) or a `label` (`title`);
/// `kind: none` takes it away.
pub async fn assign(args: Value) -> Result<Value, String> {
    fresh().await;
    let date = the_date(&args)?;
    let id = str_arg(&args, "block").ok_or("block: the block's id")?;
    let kind = str_arg(&args, "kind").unwrap_or("label");
    let what = if kind.eq_ignore_ascii_case("none") {
        None
    } else {
        let kind =
            AssignmentKind::parse(kind).ok_or("kind: task, project, recipe, label or none")?;
        let path = str_arg(&args, "path").map(str::to_owned);
        let title = str_arg(&args, "title")
            .map(str::to_owned)
            .or_else(|| path.as_deref().map(|p| stem(p).to_owned()))
            .ok_or("title: what it is given to (or a note's path)")?;
        if kind != AssignmentKind::Label && path.is_none() {
            return Err(format!("path: the {kind:?} note's path").to_lowercase());
        }
        Some(BlockAssignment { kind, title, path })
    };
    let mut d = day(date).await;
    dayplan::assign(&mut d, id, what)?;
    save_plan(&d).await?;
    Ok(answer(&day(date).await).await)
}

/// `DAYPLAN_SET_BLOCK`: moves or resizes block `block` of `date` to
/// `start`..`end`, or changes its `label`, `category` or `fixed`.
pub async fn set_block(args: Value) -> Result<Value, String> {
    fresh().await;
    let date = the_date(&args)?;
    let id = str_arg(&args, "block").ok_or("block: the block's id")?;
    let mut d = day(date).await;
    if args.get("start").is_some() || args.get("end").is_some() {
        let b = d
            .block(id)
            .ok_or_else(|| format!("{date} has no block {id:?}"))?;
        let start = if args.get("start").is_some() {
            time_arg(&args, "start")?
        } else {
            b.start
        };
        let end = if args.get("end").is_some() {
            time_arg(&args, "end")?
        } else {
            b.end
        };
        dayplan::set_span(&mut d, id, start, end)?;
    }
    if let Some(b) = d.blocks.iter_mut().find(|b| b.id.0 == id) {
        if let Some(l) = str_arg(&args, "label") {
            l.clone_into(&mut b.label);
        }
        if let Some(c) = str_arg(&args, "category") {
            b.category = dayplan::BlockCategory::parse(c);
        }
        if let Some(f) = args.get("fixed").and_then(Value::as_bool) {
            b.fixed = f;
        }
    }
    save_plan(&d).await?;
    Ok(answer(&day(date).await).await)
}

/// `DAYPLAN_RESET`: "Reset to template": `date`'s plan goes, and the day
/// is its template's again.
pub async fn reset(args: Value) -> Result<Value, String> {
    fresh().await;
    let date = the_date(&args)?;
    let dropped = drop_plan(date).await?;
    let mut out = answer(&day(date).await).await;
    out["reset"] = json!(dropped);
    Ok(out)
}

fn template_json(t: &DayTemplate, r: &rules::Rules) -> Value {
    let used: Vec<String> = r
        .rules
        .iter()
        .enumerate()
        .filter(|(_, x)| t.is(&x.template))
        .map(|(i, x)| format!("Rule {}: {}", i + 1, x.words()))
        .chain(
            r.fallback
                .as_deref()
                .filter(|f| t.is(f))
                .map(|_| "The default".to_owned()),
        )
        .collect();
    let minutes = |c: dayplan::BlockCategory| -> u32 {
        t.blocks
            .iter()
            .filter(|b| b.category == c)
            .map(|b| {
                let (s, e) = dayplan::resolve::day_span(b.start, b.end);
                u32::from(e.saturating_sub(s))
            })
            .sum()
    };
    json!({
        "name": t.name,
        "path": t.path,
        "description": t.description,
        "color": t.color,
        "blocks": t.blocks,
        "revisions": t.history.len(),
        "used_by": used,
        "allocatable_minutes": minutes(dayplan::BlockCategory::Allocatable),
        "meal_minutes": minutes(dayplan::BlockCategory::Meal),
    })
}

/// `DAYPLAN_TEMPLATES`: the day templates, each with its blocks and the
/// rules that use it.
pub async fn templates(_: Value) -> Result<Value, String> {
    fresh().await;
    let b = book();
    let list: Vec<Value> = b
        .templates
        .iter()
        .map(|t| template_json(t, &b.rules))
        .collect();
    Ok(json!({ "templates": list }))
}

/// `DAYPLAN_TEMPLATE`: makes template `name` or changes it: `blocks` (from
/// today on: the days before keep the ones it had), `description`,
/// `color`, `rename` (the rules follow).
pub async fn template(args: Value) -> Result<Value, String> {
    fresh().await;
    let name = str_arg(&args, "name").ok_or("name: the template's name")?;
    let folder = folder();
    let found = book().template(name).cloned();
    let existing = found.is_some();
    let mut t = found.unwrap_or_else(|| DayTemplate {
        path: notes::template_path(&folder, name),
        name: name.to_owned(),
        ..DayTemplate::default()
    });
    if let Some(list) = args.get("blocks").and_then(Value::as_array) {
        let blocks: Vec<TimeBlock> = list
            .iter()
            .enumerate()
            .map(|(i, v)| {
                block_of(v, i).map(|b| TimeBlock {
                    id: b.id,
                    start: b.start,
                    end: b.end,
                    label: b.label,
                    category: b.category,
                    note: b.note,
                    fixed: b.fixed,
                })
            })
            .collect::<Result<_, _>>()?;
        if existing {
            t.change_blocks(blocks, today());
        } else {
            t.blocks = blocks;
        }
    }
    if let Some(v) = args.get("description") {
        t.description = v
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
    }
    if let Some(v) = args.get("color") {
        t.color = v
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
    }
    save_template(&t, existing).await?;
    if let Some(to) = str_arg(&args, "rename").filter(|n| !t.is(n)) {
        let new_path = notes::template_path(&folder, to);
        vault::run("VAULT_RENAME", json!({ "from": t.path, "to": new_path })).await?;
        vault::set(&new_path, "name", &json!(to)).await?;
        let mut r = book().rules;
        let old = t.name.clone();
        r.rename_template(&old, to);
        with_book(|b| {
            for p in &mut b.plans {
                if p.template
                    .as_deref()
                    .is_some_and(|x| x.eq_ignore_ascii_case(&old))
                {
                    p.template = Some(to.to_owned());
                }
            }
            if let Some(x) = b.templates.iter_mut().find(|x| x.path == t.path) {
                x.path.clone_from(&new_path);
                to.clone_into(&mut x.name);
            }
        });
        if !r.path.is_empty() {
            save_rules(&r).await?;
        }
        t.path = new_path;
        to.clone_into(&mut t.name);
    }
    let r = book().rules;
    Ok(template_json(&t, &r))
}

fn rules_json(r: &rules::Rules) -> Value {
    let list: Vec<Value> = r
        .rules
        .iter()
        .enumerate()
        .map(|(i, x)| {
            let mut v = serde_json::to_value(x).unwrap_or(Value::Null);
            v["number"] = json!(i + 1);
            v["words"] = json!(x.words());
            v
        })
        .collect();
    let overrides: Vec<Value> = r
        .overrides
        .iter()
        .map(|o| {
            let mut v = serde_json::to_value(o).unwrap_or(Value::Null);
            v["words"] = json!(
                rules::Reason::Override {
                    span: o.span,
                    from: o.from,
                    to: o.to
                }
                .words()
            );
            v
        })
        .collect();
    json!({
        "path": r.path,
        "rules": list,
        "fallback": r.fallback,
        "overrides": overrides,
        "revisions": r.history.len(),
    })
}

/// `DAYPLAN_TEMPLATE_RULES`: the rules, in order, the default and the
/// overrides; with `rules` (and `fallback`), sets them from today on (the
/// days before keep the ones they had).
pub async fn template_rules(args: Value) -> Result<Value, String> {
    fresh().await;
    let mut r = book().rules;
    if let Some(list) = args.get("rules").and_then(Value::as_array) {
        let mut new = Vec::new();
        for (i, v) in list.iter().enumerate() {
            let rule: Rule =
                serde_json::from_value(v.clone()).map_err(|e| format!("rule {}: {e}", i + 1))?;
            new.push(Rule {
                template: template_named(&rule.template)?,
                when: rule.when,
            });
        }
        let fallback = match args.get("fallback") {
            Some(Value::String(f)) if !f.trim().is_empty() && f != "none" => {
                Some(template_named(f)?)
            }
            Some(_) => None,
            None => r.fallback.clone(),
        };
        r.change(new, fallback, today());
        save_rules(&r).await?;
    }
    Ok(rules_json(&r))
}

/// `DAYPLAN_SET_WEEKDAY`: template `template` on `weekdays` (`[mon, tue]`,
/// or one `weekday`), from today on: those days leave the weekday rules
/// that had them, and one rule for them takes the first one's place.
pub async fn set_weekday(args: Value) -> Result<Value, String> {
    fresh().await;
    let names: Vec<String> = match args.get("weekdays") {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        _ => str_arg(&args, "weekday")
            .map(|w| w.split(',').map(str::to_owned).collect())
            .unwrap_or_default(),
    };
    let days: Vec<Weekday> = names
        .iter()
        .map(|n| rules::weekday(n).ok_or_else(|| format!("not a weekday: {n}")))
        .collect::<Result<_, _>>()?;
    if days.is_empty() {
        return Err("weekdays: [mon, tue, …]".into());
    }
    let template = template_named(str_arg(&args, "template").ok_or("template: its name")?)?;
    let mut r = book().rules;
    let mut list = r.rules.clone();
    let at = list
        .iter()
        .position(|x| matches!(x.when, When::Weekdays(_)))
        .unwrap_or(list.len());
    for x in &mut list {
        if let When::Weekdays(w) = &mut x.when {
            w.retain(|d| !days.contains(d));
        }
    }
    list.retain(|x| !matches!(&x.when, When::Weekdays(w) if w.is_empty()));
    let at = at.min(list.len());
    // The same template's weekday rule takes them in.
    if let Some(same) = list
        .iter_mut()
        .find(|x| x.template == template && matches!(x.when, When::Weekdays(_)))
    {
        if let When::Weekdays(w) = &mut same.when {
            w.extend(days);
            w.sort_by_key(Weekday::num_days_from_monday);
        }
    } else {
        list.insert(
            at,
            Rule {
                template,
                when: When::Weekdays(days),
            },
        );
    }
    let fallback = r.fallback.clone();
    r.change(list, fallback, today());
    save_rules(&r).await?;
    Ok(rules_json(&r))
}

/// `DAYPLAN_OVERRIDE`: a template chosen by hand.
///
/// Template `template` (`none`: no template) for `date`, the `week`
/// starting on a date, or `from`..=`to`; `clear` takes the override
/// away. It beats every rule, and is the one thing
/// that changes a past day. A day's own override starts its plan again
/// from the template chosen.
pub async fn set_override(args: Value) -> Result<Value, String> {
    fresh().await;
    let template = template_arg(&args)?;
    let clear = args.get("clear").and_then(Value::as_bool).unwrap_or(false);
    let o = if let Some(start) = date_arg(&args, "week")? {
        Override::week(start, template)
    } else if let (Some(from), Some(to)) = (date_arg(&args, "from")?, date_arg(&args, "to")?) {
        if to < from {
            return Err("to is before from".into());
        }
        if from == to {
            Override::day(from, template)
        } else {
            Override {
                from,
                to,
                span: rules::Span::Range,
                template,
            }
        }
    } else {
        Override::day(the_date(&args)?, template)
    };
    let mut r = book().rules;
    r.set_override(o.clone(), clear);
    save_rules(&r).await?;
    if o.span == rules::Span::Day {
        drop_plan(o.from).await?;
    }
    let to = o.to + Duration::days(1);
    let days: Vec<Value> = days(o.from, to)
        .await
        .iter()
        .map(|d| json!({ "date": d.date.to_string(), "template": d.template, "reason": d.reason }))
        .collect();
    Ok(json!({ "override": o, "cleared": clear, "days": days }))
}

/// `DAYPLAN_RESOLVE`: which template `date` (or each day of
/// `from`..`to`) uses, and why.
pub async fn resolve(args: Value) -> Result<Value, String> {
    fresh().await;
    let (from, to) = match (date_arg(&args, "from")?, date_arg(&args, "to")?) {
        (Some(f), Some(t)) if t > f && (t - f).num_days() <= 366 => (f, t),
        (Some(_), Some(_)) => return Err("from..to: one to 366 days".into()),
        _ => {
            let d = the_date(&args)?;
            (d, d + Duration::days(1))
        }
    };
    let list: Vec<Value> = days(from, to)
        .await
        .iter()
        .map(|d| {
            json!({
                "date": d.date.to_string(),
                "weekday": d.date.weekday().to_string(),
                "template": d.template,
                "reason": d.reason,
                "chosen": d.chosen,
                "edited": d.edited,
                "past": d.past,
            })
        })
        .collect();
    if list.len() == 1 {
        return Ok(list.into_iter().next().unwrap_or(Value::Null));
    }
    Ok(json!({ "days": list }))
}

/// A note's name without its folder and extension.
fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name)
}
