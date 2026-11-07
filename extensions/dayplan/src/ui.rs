//! Its views (#444): the template editor and the rules.
//!
//! - **Templates** (`templates`): the templates on the left; the one
//!   picked on the right as a day (Task's `day_template_view.rs`: each
//!   block a bar in its category's colour on a 24-hour column) beside a
//!   table of its blocks to edit, with Save. A change applies from today
//!   on; past days keep the blocks they had.
//! - **Rules** (`rules`): the rules in order (first match wins) to move,
//!   add and remove, the default, the overrides chosen by hand, and the
//!   next two weeks: each day's template and why.

use std::time::Duration as Wait;

use chrono::Duration;
use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::model::{BlockCategory, TimeBlock, TimeBlockId, TimeOfDay};
use crate::rules::{Rule, When, weekday, weekday_id};

const MUTED: &str = "color: var(--muted-foreground);";
const HEADING: &str = "font-size: 11px; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; color: var(--muted-foreground);";
const BUTTON: &str = "height: 28px; padding: 0 10px; border-radius: 7px; font-size: 12px; cursor: pointer; border: 1px solid var(--border); background: transparent; color: var(--foreground);";
const PRIMARY: &str = "height: 28px; padding: 0 12px; border-radius: 7px; font-size: 12px; cursor: pointer; border: none; background: var(--primary); color: var(--primary-foreground);";
const FIELD: &str = "height: 28px; box-sizing: border-box; padding: 0 7px; border-radius: 6px; border: 1px solid var(--input); background: var(--background); color: var(--foreground); font-size: 12px;";

/// A chip of a picker: `on` when it is the one chosen.
fn chip(on: bool) -> String {
    format!(
        "height: 24px; padding: 0 9px; border-radius: 999px; font-size: 11.5px; cursor: pointer; white-space: nowrap; \
         border: 1px solid {}; background: {}; color: var(--foreground);",
        if on {
            "var(--primary)"
        } else {
            "var(--border)"
        },
        if on {
            "color-mix(in srgb, var(--primary) 18%, transparent)"
        } else {
            "transparent"
        }
    )
}

/// Re-renders the view when what it knows changes; reads again every
/// few seconds while it shows.
fn use_revision() -> u64 {
    let mut seen = use_signal(crate::revision);
    use_future(move || async move {
        let _ = crate::refresh().await;
        let mut ticks = 0_u32;
        loop {
            futures_timer::Delay::new(Wait::from_millis(250)).await;
            ticks = ticks.wrapping_add(1);
            if ticks.is_multiple_of(20) {
                let _ = crate::refresh().await;
            }
            let now = crate::revision();
            if *seen.peek() != now {
                seen.set(now);
            }
        }
    });
    seen()
}

/// The template editor.
#[component]
pub fn TemplatesView() -> Element {
    rsx! { DayPlansView { tab: "templates" } }
}

/// The rules.
#[component]
pub fn RulesView() -> Element {
    rsx! { DayPlansView { tab: "rules" } }
}

#[component]
fn DayPlansView(tab: String) -> Element {
    let _ = use_revision();
    let mut on = use_signal(|| tab.clone());
    let problem = use_signal(|| None::<String>);
    let tabs = [("templates", "Templates"), ("rules", "Rules")];
    rsx! {
        div { class: "dp-root", "data-tab": "{on}",
            style: "display: flex; flex-direction: column; height: 100%; box-sizing: border-box; color: var(--foreground); font-size: 13px;",
            div { style: "display: flex; align-items: center; gap: 10px; padding: 12px 16px; border-bottom: 1px solid var(--border);",
                span { style: "font-size: 17px; font-weight: 700;", "Day plans" }
                div { style: "display: flex; gap: 4px; margin-left: 8px;",
                    for (id, name) in tabs {
                        {
                            let bg = if on() == id { "var(--accent)" } else { "transparent" };
                            rsx! {
                                button { key: "{id}", class: "dp-tab", "data-tab": "{id}", style: "{BUTTON} background: {bg};",
                                    onclick: move |_| on.set(id.to_owned()), "{name}" }
                            }
                        }
                    }
                }
                span { style: "flex: 1;" }
                span { style: "font-size: 12px; {MUTED}", "Changes apply from today on. Past days keep what they had." }
            }
            if let Some(p) = problem() {
                div { class: "dp-problem", style: "padding: 6px 16px; color: var(--destructive);", "{p}" }
            }
            div { style: "flex: 1; min-height: 0; display: flex;",
                if on() == "rules" {
                    Rules { problem }
                } else {
                    Templates { problem }
                }
            }
        }
    }
}

// ── Templates ───────────────────────────────────────────────────────

/// A block being edited: its fields as typed.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    id: String,
    start: String,
    end: String,
    label: String,
    category: BlockCategory,
    fixed: bool,
}

impl Row {
    fn of(b: &TimeBlock) -> Self {
        Self {
            id: b.id.0.clone(),
            start: b.start.to_string(),
            end: b.end.to_string(),
            label: b.label.clone(),
            category: b.category,
            fixed: b.fixed,
        }
    }

    fn block(&self) -> Option<TimeBlock> {
        Some(TimeBlock {
            id: TimeBlockId(self.id.clone()),
            start: TimeOfDay::parse(&self.start)?,
            end: TimeOfDay::parse(&self.end)?,
            label: self.label.trim().to_owned(),
            category: self.category,
            note: None,
            fixed: self.fixed,
        })
    }
}

/// A new block's id, not one of `rows`'.
fn fresh_id(rows: &[Row]) -> String {
    (1..=rows.len() + 1)
        .map(|n| format!("block-{n}"))
        .find(|id| !rows.iter().any(|r| &r.id == id))
        .unwrap_or_default()
}

#[component]
fn Templates(problem: Signal<Option<String>>) -> Element {
    let book = crate::book();
    let mut picked = use_signal(|| None::<String>);
    let names: Vec<(String, Option<String>, usize)> = book
        .templates
        .iter()
        .map(|t| (t.name.clone(), t.color.clone(), t.blocks.len()))
        .collect();
    // The one picked, else the first.
    let name = picked().or_else(|| names.first().map(|(n, ..)| n.clone()));
    let current = name.as_deref().and_then(|n| book.template(n).cloned());
    let mut naming = use_signal(String::new);
    rsx! {
        div { class: "dp-list", style: "flex: 0 0 180px; border-right: 1px solid var(--border); padding: 12px; display: flex; flex-direction: column; gap: 4px; overflow-y: auto;",
            div { style: HEADING, "Templates" }
            for (n, colour, count) in names {
                {
                    let on = name.as_deref() == Some(n.as_str());
                    let bg = if on { "var(--accent)" } else { "transparent" };
                    let dot = colour.unwrap_or_else(|| "var(--muted-foreground)".into());
                    let pick = n.clone();
                    rsx! {
                        button { key: "{n}", class: "dp-template", "data-template": "{n}",
                            style: "display: flex; align-items: center; gap: 8px; padding: 7px 8px; border: none; border-radius: 7px; cursor: pointer; text-align: left; background: {bg}; color: var(--foreground);",
                            onclick: move |_| picked.set(Some(pick.clone())),
                            span { style: "width: 9px; height: 9px; border-radius: 3px; background: {dot};" }
                            span { style: "flex: 1; font-weight: 600;", "{n}" }
                            span { style: "font-size: 11px; {MUTED}", "{count}" }
                        }
                    }
                }
            }
            div { style: "display: flex; gap: 4px; margin-top: 8px;",
                input { class: "dp-new-name", r#type: "text", placeholder: "New template", style: "{FIELD} flex: 1; min-width: 0;",
                    value: "{naming}", oninput: move |e: FormEvent| naming.set(e.value()) }
                button { class: "dp-new", style: BUTTON,
                    onclick: move |_| {
                        let name = naming.peek().trim().to_owned();
                        if name.is_empty() { return; }
                        naming.set(String::new());
                        let mut problem = problem;
                        spawn(async move {
                            match crate::plans::template(json!({ "name": name, "blocks": [] })).await {
                                Ok(_) => { picked.set(Some(name)); problem.set(None); }
                                Err(e) => problem.set(Some(e)),
                            }
                        });
                    },
                    "Add" }
            }
        }
        if let Some(t) = current {
            Editor { key: "{t.path}", template: t, problem }
        } else {
            div { style: "flex: 1; padding: 24px; {MUTED}", "No day templates yet. Add one on the left." }
        }
    }
}

/// The pixels an hour in the day column.
const HOUR: f64 = 26.0;

// The lints point into `rsx!`'s own expansion.
#[allow(clippy::redundant_clone, clippy::suboptimal_flops)]
#[component]
fn Editor(template: crate::DayTemplate, problem: Signal<Option<String>>) -> Element {
    let mut rows = use_signal(|| template.blocks.iter().map(Row::of).collect::<Vec<_>>());
    let mut description = use_signal(|| template.description.clone().unwrap_or_default());
    let mut colour = use_signal(|| template.color.clone().unwrap_or_default());
    let mut saved = use_signal(|| false);
    let mut menu = use_signal(|| None::<usize>);
    let name = template.name;
    let shown: Vec<TimeBlock> = rows().iter().filter_map(Row::block).collect();
    let bad = rows().iter().filter(|r| r.block().is_none()).count();
    let total = |c: BlockCategory| -> u16 {
        shown
            .iter()
            .filter(|b| b.category == c)
            .map(|b| {
                // A wrap past midnight counts its morning too.
                let (s, e) = (b.start.minutes_since_midnight, b.end.minutes_since_midnight);
                if e > s { e - s } else { (24 * 60 - s) + e }
            })
            .sum()
    };
    let summary = format!(
        "{} blocks · {}h allocatable · {}h meals · {}h sleep",
        shown.len(),
        f64::from(total(BlockCategory::Allocatable)) / 60.0,
        f64::from(total(BlockCategory::Meal)) / 60.0,
        f64::from(total(BlockCategory::Sleep)) / 60.0
    );
    let save = {
        let name = name.clone();
        move |_| {
            let blocks: Vec<TimeBlock> = rows.peek().iter().filter_map(Row::block).collect();
            let args = json!({
                "name": name,
                "blocks": blocks,
                "description": description.peek().clone(),
                "color": colour.peek().clone(),
            });
            let mut problem = problem;
            spawn(async move {
                match crate::plans::template(args).await {
                    Ok(_) => {
                        saved.set(true);
                        problem.set(None);
                    }
                    Err(e) => problem.set(Some(e)),
                }
            });
        }
    };
    rsx! {
        div { class: "dp-editor", "data-template": "{name}", style: "flex: 1; min-width: 0; display: flex; overflow: hidden;",
            // The day, as Task draws a template.
            div { class: "dp-day", style: "flex: 0 0 170px; overflow-y: auto; border-right: 1px solid var(--border); padding: 12px 10px;",
                div { style: "{HEADING} margin-bottom: 8px;", "The day" }
                div { style: "position: relative; height: {HOUR * 24.0}px; margin-left: 38px;",
                    for h in (0..24).step_by(2) {
                        div { key: "h{h}", style: "position: absolute; left: -38px; top: {f64::from(h) * HOUR - 6.0}px; font-size: 10px; {MUTED}", "{h:02}:00" }
                        div { style: "position: absolute; left: 0; right: 0; top: {f64::from(h) * HOUR}px; border-top: 1px solid var(--border);" }
                    }
                    // A block past midnight (sleep) is drawn to the end of the day, and
                    // its morning from 00:00.
                    for (b, tail) in shown.iter().cloned().map(|b| (b, false)).chain(shown.iter().filter(|b| b.end < b.start && b.end.minutes_since_midnight > 0).cloned().map(|b| (b, true))) {
                        {
                            let (s, e) = if tail { (0, b.end.minutes_since_midnight) } else { crate::resolve::day_span(b.start, b.end) };
                            let top = f64::from(s) / 60.0 * HOUR;
                            let height = (f64::from(e.saturating_sub(s)) / 60.0 * HOUR).max(3.0);
                            let c = b.category.colour();
                            let when = format!("{}–{}", b.start, b.end);
                            rsx! {
                                div { key: "{b.id.0}{tail}", class: "dp-bar", "data-category": "{b.category.id()}", title: "{when} {b.label}",
                                    style: "position: absolute; left: 2px; right: 2px; top: {top}px; height: {height}px; box-sizing: border-box; border-radius: 5px; \
                                            background: color-mix(in srgb, {c} 22%, var(--background)); border-left: 3px solid {c}; padding: 1px 5px; overflow: hidden; font-size: 10.5px; line-height: 13px;",
                                    if height >= 12.0 { span { style: "font-weight: 600;", "{b.label}" } }
                                    if height >= 26.0 { div { style: "{MUTED}", "{when}" } }
                                }
                            }
                        }
                    }
                }
            }
            div { style: "flex: 1; min-width: 0; overflow-y: auto; padding: 14px 16px; display: flex; flex-direction: column; gap: 10px;",
                div { style: "display: flex; align-items: center; gap: 10px;",
                    span { style: "font-size: 18px; font-weight: 700; white-space: nowrap;", "{name}" }
                    span { style: "flex: 1;" }
                    if saved() { span { class: "dp-saved", style: "font-size: 12px; {MUTED}", "Saved" } }
                    button { class: "dp-save", style: PRIMARY, disabled: bad > 0, onclick: save, "Save" }
                }
                div { style: "font-size: 12px; margin-top: -6px; {MUTED}", "{summary}" }
                div { style: "display: flex; gap: 8px;",
                    input { r#type: "text", placeholder: "What the day is for", style: "{FIELD} flex: 1;", value: "{description}",
                        oninput: move |e: FormEvent| { description.set(e.value()); saved.set(false); } }
                    input { r#type: "text", placeholder: "#3b82f6", style: "{FIELD} width: 100px;", value: "{colour}",
                        oninput: move |e: FormEvent| { colour.set(e.value()); saved.set(false); } }
                }
                if bad > 0 {
                    div { style: "font-size: 12px; color: var(--destructive);", "{bad} block(s) need times as HH:MM." }
                }
                div { class: "dp-blocks", style: "display: grid; grid-template-columns: 54px 54px minmax(90px, 1fr) 104px 34px 26px; gap: 4px 5px; align-items: center;",
                    span { style: HEADING, "Start" }
                    span { style: HEADING, "End" }
                    span { style: HEADING, "Block" }
                    span { style: HEADING, "Category" }
                    span { style: HEADING, "Fix" }
                    span {}
                    for (i, r) in rows().into_iter().enumerate() {
                        {
                            let c = r.category.colour();
                            rsx! {
                                input { key: "s{r.id}", class: "dp-start", r#type: "text", style: FIELD, value: "{r.start}",
                                    oninput: move |e: FormEvent| { if let Some(x) = rows.write().get_mut(i) { x.start = e.value(); } saved.set(false); } }
                                input { class: "dp-end", r#type: "text", style: FIELD, value: "{r.end}",
                                    oninput: move |e: FormEvent| { if let Some(x) = rows.write().get_mut(i) { x.end = e.value(); } saved.set(false); } }
                                input { class: "dp-label", r#type: "text", style: "{FIELD} border-left: 3px solid {c};", value: "{r.label}",
                                    oninput: move |e: FormEvent| { if let Some(x) = rows.write().get_mut(i) { x.label = e.value(); } saved.set(false); } }
                                div { class: "dp-category", style: "position: relative;",
                                    button { style: "{FIELD} width: 100%; display: flex; align-items: center; gap: 6px; cursor: pointer; text-align: left;",
                                        onclick: move |_| { let now = *menu.peek(); menu.set(if now == Some(i) { None } else { Some(i) }); },
                                        span { style: "width: 8px; height: 8px; border-radius: 2px; background: {c}; flex: none;" }
                                        span { style: "overflow: hidden; text-overflow: ellipsis; white-space: nowrap;", "{r.category.name()}" }
                                    }
                                    if menu() == Some(i) {
                                        div { style: "position: absolute; top: 30px; left: 0; z-index: 10; width: 150px; padding: 4px; border-radius: 8px; border: 1px solid var(--border); background: var(--popover, var(--background)); box-shadow: 0 8px 24px rgba(0,0,0,0.25);",
                                            for cat in BlockCategory::ALL {
                                                button { key: "{cat.id()}", class: "dp-category-pick", "data-category": "{cat.id()}",
                                                    style: "display: flex; align-items: center; gap: 6px; width: 100%; padding: 4px 6px; border: none; border-radius: 5px; background: transparent; color: var(--foreground); cursor: pointer; font-size: 12px; text-align: left;",
                                                    onclick: move |_| { if let Some(x) = rows.write().get_mut(i) { x.category = cat; } menu.set(None); saved.set(false); },
                                                    span { style: "width: 8px; height: 8px; border-radius: 2px; background: {cat.colour()};" }
                                                    "{cat.name()}"
                                                }
                                            }
                                        }
                                    }
                                }
                                input { r#type: "checkbox", checked: r.fixed,
                                    onchange: move |e: FormEvent| { if let Some(x) = rows.write().get_mut(i) { x.fixed = e.checked(); } saved.set(false); } }
                                button { style: "{BUTTON} padding: 0 6px;", "aria-label": "Remove",
                                    onclick: move |_| { rows.write().remove(i); saved.set(false); }, "×" }
                            }
                        }
                    }
                }
                div { style: "display: flex; gap: 6px;",
                    button { class: "dp-add", style: BUTTON,
                        onclick: move |_| {
                            let mut r = rows.write();
                            let id = fresh_id(&r);
                            let start = r.iter().filter_map(Row::block).map(|b| b.end).max().unwrap_or_else(|| TimeOfDay::new(9, 0));
                            let end = TimeOfDay::at(i64::from(start.minutes_since_midnight) + 60);
                            r.push(Row { id, start: start.to_string(), end: end.to_string(), label: "Block".into(), category: BlockCategory::Allocatable, fixed: false });
                            saved.set(false);
                        },
                        "+ Add a block" }
                    button { style: BUTTON,
                        onclick: move |_| {
                            rows.write().sort_by_key(|r| r.block().map(|b| crate::resolve::day_span(b.start, b.end).0));
                        },
                        "Sort by time" }
                }
            }
        }
    }
}

// ── Rules ───────────────────────────────────────────────────────────

/// A rule as its form has it: the template, the kind, the value typed.
fn rule_value(r: &Rule) -> (String, String) {
    match &r.when {
        When::Weekdays(d) => (
            "weekdays".into(),
            d.iter()
                .map(|d| weekday_id(*d))
                .collect::<Vec<_>>()
                .join(", "),
        ),
        When::Rrule { rule, start } => ("rrule".into(), format!("{rule} from {start}")),
        When::Dates { from, to } => ("dates".into(), format!("{from} to {to}")),
        When::Tag(t) => ("tag".into(), format!("#{t}")),
    }
}

/// A rule from its form: `None` when the value does not read.
fn rule_of(template: &str, kind: &str, value: &str) -> Option<Rule> {
    let v = value.trim();
    let when = match kind {
        "weekdays" => When::Weekdays(
            v.split([',', ' '])
                .filter(|s| !s.is_empty())
                .map(weekday)
                .collect::<Option<Vec<_>>>()?,
        ),
        "rrule" => {
            let (rule, start) = v.split_once(" from ")?;
            architect_calendar::recur::Rule::parse(rule.trim())?;
            When::Rrule {
                rule: rule.trim().to_owned(),
                start: start.trim().parse().ok()?,
            }
        }
        "dates" => {
            let (from, to) = v.split_once(" to ")?;
            When::Dates {
                from: from.trim().parse().ok()?,
                to: to.trim().parse().ok()?,
            }
        }
        "tag" => When::Tag(v.trim_start_matches('#').to_owned()),
        _ => return None,
    };
    Some(Rule {
        template: template.to_owned(),
        when,
    })
}

// The lints point into `rsx!`'s own expansion.
#[allow(clippy::redundant_clone)]
#[component]
fn Rules(problem: Signal<Option<String>>) -> Element {
    let book = crate::book();
    let names: Vec<String> = book.templates.iter().map(|t| t.name.clone()).collect();
    let rules = book.rules;
    let mut days = use_signal(Vec::<Value>::new);
    let revision = crate::revision();
    use_effect(use_reactive!(|revision| {
        let _ = revision;
        spawn(async move {
            // What it knows, not the vault again: reading moves the
            // revision this follows.
            let from = crate::today();
            let list = crate::plans::days(from, from + Duration::days(14)).await;
            days.set(
                list.iter()
                    .map(|d| json!({ "date": d.date.to_string(), "template": d.template, "reason": d.reason, "chosen": d.chosen, "edited": d.edited }))
                    .collect(),
            );
        });
    }));
    let save = move |list: Vec<Rule>, fallback: Option<String>| {
        let mut problem = problem;
        spawn(async move {
            let args =
                json!({ "rules": list, "fallback": fallback.unwrap_or_else(|| "none".into()) });
            match crate::plans::template_rules(args).await {
                Ok(_) => problem.set(None),
                Err(e) => problem.set(Some(e)),
            }
        });
    };
    let mut new_template = use_signal(|| names.first().cloned().unwrap_or_default());
    let mut new_kind = use_signal(|| "weekdays".to_owned());
    let mut new_value = use_signal(|| "sat, sun".to_owned());
    let list = rules.rules.clone();
    let fallback = rules.fallback.clone();
    let overrides = rules.overrides;
    rsx! {
        div { class: "dp-rules", style: "flex: 1; min-width: 0; overflow-y: auto; padding: 14px 16px; display: flex; flex-direction: column; gap: 10px;",
            div { style: HEADING, "Rules, in order: the first that matches a day wins" }
            for (i, r) in list.iter().cloned().enumerate() {
                {
                    let (kind, value) = rule_value(&r);
                    let words = r.words();
                    let (up, down, gone) = (list.clone(), list.clone(), list.clone());
                    let (f1, f2, f3) = (fallback.clone(), fallback.clone(), fallback.clone());
                    let n = list.len();
                    rsx! {
                        div { key: "{i}", class: "dp-rule", "data-template": "{r.template}",
                            style: "display: flex; align-items: center; gap: 8px; padding: 7px 10px; border: 1px solid var(--border); border-radius: 8px;",
                            span { style: "font-weight: 700; width: 18px; {MUTED}", "{i + 1}" }
                            span { style: "font-weight: 600; white-space: nowrap;", "{r.template}" }
                            span { title: "{kind}: {value}", style: "flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; {MUTED}", "{words}" }
                            button { style: BUTTON, disabled: i == 0, "aria-label": "Up",
                                onclick: move |_| { let mut l = up.clone(); l.swap(i, i - 1); save(l, f1.clone()); }, "↑" }
                            button { style: BUTTON, disabled: i + 1 == n, "aria-label": "Down",
                                onclick: move |_| { let mut l = down.clone(); l.swap(i, i + 1); save(l, f2.clone()); }, "↓" }
                            button { style: BUTTON, "aria-label": "Remove",
                                onclick: move |_| { let mut l = gone.clone(); l.remove(i); save(l, f3.clone()); }, "×" }
                        }
                    }
                }
            }
            div { style: "{HEADING} margin-top: 6px;", "Add a rule" }
            div { class: "dp-add-rule", style: "display: flex; align-items: center; gap: 6px; flex-wrap: wrap;",
                for n in names.clone() {
                    {
                        let style = chip(new_template() == n);
                        let pick = n.clone();
                        rsx! { button { key: "{n}", style, onclick: move |_| new_template.set(pick.clone()), "{n}" } }
                    }
                }
            }
            div { style: "display: flex; align-items: center; gap: 6px; flex-wrap: wrap;",
                for (k, label) in [("weekdays", "On weekdays"), ("rrule", "Repeating"), ("dates", "Between dates"), ("tag", "Tagged event")] {
                    {
                        let style = chip(new_kind() == k);
                        rsx! {
                            button { key: "{k}", style,
                                onclick: move |_| {
                                    new_value.set(match k {
                                        "rrule" => format!("FREQ=WEEKLY;INTERVAL=2;BYDAY=SA from {}", crate::today()),
                                        "dates" => format!("{} to {}", crate::today(), crate::today() + Duration::days(6)),
                                        "tag" => "#gig".into(),
                                        _ => "mon, tue, wed, thu, fri".into(),
                                    });
                                    new_kind.set(k.to_owned());
                                },
                                "{label}"
                            }
                        }
                    }
                }
            }
            div { style: "display: flex; align-items: center; gap: 6px;",
                input { r#type: "text", style: "{FIELD} flex: 1; min-width: 0;", value: "{new_value}", oninput: move |e: FormEvent| new_value.set(e.value()) }
                button { class: "dp-add-rule-go", style: BUTTON,
                    onclick: {
                        let (list, fallback) = (list.clone(), fallback.clone());
                        move |_| {
                            if let Some(r) = rule_of(&new_template.peek(), &new_kind.peek(), &new_value.peek()) {
                                let mut l = list.clone();
                                l.push(r);
                                save(l, fallback.clone());
                            } else {
                                let mut p = problem;
                                p.set(Some("That rule does not read: weekdays like mon, tue; an RRULE then \"from\" a date; two dates with \"to\"; a #tag.".into()));
                            }
                        }
                    },
                    "+ Add rule" }
            }
            div { style: "display: flex; align-items: center; gap: 8px;",
                span { style: HEADING, "Otherwise" }
                div { class: "dp-fallback", style: "display: flex; gap: 6px; flex-wrap: wrap;",
                    for n in std::iter::once("none".to_owned()).chain(names.clone()) {
                        {
                            let on = if n == "none" { fallback.is_none() } else { fallback.as_deref() == Some(n.as_str()) };
                            let style = chip(on);
                            let list = list.clone();
                            let label = if n == "none" { "No template".to_owned() } else { n.clone() };
                            let pick = n.clone();
                            rsx! {
                                button { key: "{n}", style,
                                    onclick: move |_| save(list.clone(), (pick != "none").then(|| pick.clone())),
                                    "{label}"
                                }
                            }
                        }
                    }
                }
            }
            div { style: "{HEADING} margin-top: 8px;", "Chosen by hand" }
            if overrides.is_empty() {
                div { style: "font-size: 12px; {MUTED}", "None. Choose a day's template on its head in the calendar." }
            }
            for o in overrides.iter().cloned() {
                {
                    let words = crate::Reason::Override { span: o.span, from: o.from, to: o.to }.words();
                    let what = o.template.clone().unwrap_or_else(|| "No template".into());
                    let when = if o.from == o.to { o.from.format("%a %-d %b %Y").to_string() } else { format!("{} – {}", o.from.format("%-d %b"), o.to.format("%-d %b %Y")) };
                    rsx! {
                        div { key: "{o.from}{o.to}", class: "dp-override",
                            style: "display: flex; align-items: center; gap: 8px; padding: 7px 10px; border: 1px dashed var(--border); border-radius: 8px;",
                            span { style: "font-weight: 600; min-width: 160px;", "{when}" }
                            span { style: "flex: 1;", "{what}" }
                            span { style: "font-size: 11px; {MUTED}", "{words}" }
                            button { style: BUTTON,
                                onclick: move |_| {
                                    let args = match o.span {
                                        crate::Span::Week => json!({ "week": o.from.to_string(), "clear": true }),
                                        crate::Span::Day => json!({ "date": o.from.to_string(), "clear": true }),
                                        crate::Span::Range => json!({ "from": o.from.to_string(), "to": o.to.to_string(), "clear": true }),
                                    };
                                    let mut problem = problem;
                                    spawn(async move { if let Err(e) = crate::plans::set_override(args).await { problem.set(Some(e)); } });
                                },
                                "Clear" }
                        }
                    }
                }
            }
        }
        div { class: "dp-upcoming", style: "flex: 0 0 270px; border-left: 1px solid var(--border); overflow-y: auto; padding: 14px 16px;",
            div { style: "{HEADING} margin-bottom: 8px;", "The next two weeks" }
            for d in days() {
                {
                    let date = d["date"].as_str().unwrap_or_default().to_owned();
                    let label = date.parse::<chrono::NaiveDate>().map_or_else(|_| date.clone(), |x| x.format("%a %-d %b").to_string());
                    let template = d["template"].as_str().unwrap_or("No template").to_owned();
                    let reason = d["reason"].as_str().unwrap_or_default().to_owned();
                    let chosen = d["chosen"].as_bool().unwrap_or(false);
                    let edited = d["edited"].as_bool().unwrap_or(false);
                    let colour = crate::book().template(&template).and_then(|t| t.color.clone()).unwrap_or_else(|| "var(--muted-foreground)".into());
                    let border = if chosen { "1px dashed var(--primary)" } else { "1px solid transparent" };
                    rsx! {
                        div { key: "{date}", class: "dp-day-row", "data-date": "{date}", "data-chosen": "{chosen}",
                            style: "display: grid; grid-template-columns: 82px 1fr; gap: 2px 8px; padding: 6px 8px; border-radius: 8px; border: {border};",
                            span { style: "font-weight: 600;", "{label}" }
                            span { style: "display: flex; align-items: center; gap: 6px; font-weight: 600;",
                                span { style: "width: 8px; height: 8px; border-radius: 3px; background: {colour};" }
                                "{template}"
                                if edited { span { style: "font-size: 11px; font-weight: 400; {MUTED}", "edited" } }
                            }
                            span {}
                            span { style: "font-size: 11.5px; {MUTED}", "{reason}" }
                        }
                    }
                }
            }
        }
    }
}
