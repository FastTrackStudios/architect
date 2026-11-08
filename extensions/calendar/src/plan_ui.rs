//! Day plans in the day and week views (#444).
//!
//! Each day's plan blocks drawn under its events in their categories' colours, with the time
//! tracked in each against the time planned; the day's template on its
//! head (why, and a menu to choose another by hand); a block's panel to
//! give it to a task, a project, a recipe or a label, or to reset the day
//! to its template. Everything goes to Day Plans through its service
//! (`architect.dayplan`).

use chrono::{NaiveDate, NaiveDateTime};
use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::plan::{DAYPLAN, PlanBlock, PlanDay};

const MUTED: &str = "color: var(--muted-foreground);";
const BUTTON: &str = "padding: 3px 10px; border-radius: 7px; font-size: 12px; cursor: pointer; border: 1px solid var(--border); background: transparent; color: var(--foreground);";
const FIELD: &str = "height: 28px; box-sizing: border-box; padding: 0 8px; border-radius: 7px; border: 1px solid var(--input); background: var(--background); color: var(--foreground); font-size: 12px;";

/// A plan block picked: its day and its id.
pub type PickedPlan = Option<(NaiveDate, String)>;

/// Minutes in words: `1h 20m`, `45m`.
#[must_use]
pub fn minutes_words(m: u32) -> String {
    match (m / 60, m % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// Planned against tracked, for a block: `1h 20m of 3h`; nothing when
/// nothing is tracked on a day still to come.
#[must_use]
pub fn tracked_words(b: &PlanBlock, past_or_today: bool) -> Option<String> {
    (b.tracked > 0 || past_or_today).then(|| {
        format!(
            "{} of {}",
            minutes_words(b.tracked),
            minutes_words(b.minutes)
        )
    })
}

/// Runs Day Plans' `method` with `args`, then reads the plans again.
async fn call(method: &str, args: Value) -> Result<Value, String> {
    let out = crate::vault::service(DAYPLAN, method, args)
        .await
        .ok_or("Day Plans is not installed")??;
    let around = crate::shown().cursor;
    crate::refresh_plans(
        around - chrono::Duration::days(30),
        around + chrono::Duration::days(32),
    )
    .await;
    Ok(out)
}

/// A day's plan blocks, behind its events: `top0` the first minute
/// drawn, `px` the pixels a minute. Only their labels take a click, so
/// a sweep over them still makes an event.
#[component]
pub fn PlanLayer(
    day: NaiveDate,
    plan: Option<PlanDay>,
    /// The day before's, for a block that runs into this morning.
    before: Option<PlanDay>,
    top0: i64,
    px: f64,
    picked: Signal<PickedPlan>,
) -> Element {
    let midnight = crate::layout::midnight(day);
    let at = |t: NaiveDateTime| (t - midnight).num_minutes();
    let mut drawn: Vec<(String, PlanBlock, i64, i64, bool)> = Vec::new();
    if let Some(p) = &before {
        for b in &p.blocks {
            if let Some((s, e)) = b.tail(p.date) {
                drawn.push((format!("tail:{}", b.id), b.clone(), at(s), at(e), true));
            }
        }
    }
    let past_or_today = day <= crate::today();
    if let Some(p) = &plan {
        for b in &p.blocks {
            if let Some((s, e)) = b.span(day) {
                drawn.push((b.id.clone(), b.clone(), at(s), at(e), false));
            }
        }
    }
    let chosen = picked();
    rsx! {
        for (key, b, s, e, tail) in drawn {
            {
                let start = s.max(top0);
                #[allow(clippy::cast_precision_loss, clippy::as_conversions)]
                let (top, height) = ((start - top0) as f64 * px, ((e - start) as f64 * px).max(0.0));
                let colour = b.colour.clone();
                let id = b.id.clone();
                let on = chosen.as_ref().is_some_and(|(d, i)| *d == day && *i == id && !tail);
                let title = b.title().to_owned();
                let label = if b.assignment.is_some() { format!("{} · {title}", b.label) } else { title };
                let vs = tracked_words(&b, past_or_today);
                let ratio = if b.minutes > 0 { f64::from(b.tracked.min(b.minutes)) / f64::from(b.minutes) } else { 0.0 };
                let tip = format!("{} · {} ({})", b.when(), b.label, b.category);
                let outline = if on { "2px solid var(--primary)" } else { "none" };
                // A tall block's label at its foot: what starts with it (an
                // event, tracked time) covers its top.
                let place = if height >= 40.0 { "bottom: 2px;" } else { "top: 2px;" };
                let bar = ratio * 100.0;
                rsx! {
                    if height > 0.0 {
                        div { key: "{key}", class: "cal-plan", "data-block": "{b.id}", "data-category": "{b.category}",
                            style: "position: absolute; left: 0; right: 0; top: {top}px; height: {height}px; box-sizing: border-box; \
                                    background: color-mix(in srgb, {colour} 13%, transparent); border-left: 3px solid {colour}; \
                                    border-top: 1px solid color-mix(in srgb, {colour} 35%, transparent); pointer-events: none; z-index: 0; overflow: hidden;",
                            if !tail && height >= 17.0 {
                                button { class: "cal-plan-label", title: "{tip}",
                                    style: "pointer-events: auto; position: absolute; left: 3px; {place} max-width: calc(100% - 8px); \
                                            display: flex; flex-direction: column; align-items: flex-start; border: none; border-radius: 5px; padding: 1px 5px; cursor: pointer; text-align: left; \
                                            font-size: 10.5px; line-height: 14px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; \
                                            background: color-mix(in srgb, {colour} 22%, var(--background)); color: var(--foreground); \
                                            outline: {outline};",
                                    onpointerdown: move |e: PointerEvent| e.stop_propagation(),
                                    onclick: move |_| { let mut p = picked; p.set(Some((day, id.clone()))); },
                                    span { style: "font-weight: 600; max-width: 100%; overflow: hidden; text-overflow: ellipsis;", "{label}" }
                                    if let Some(v) = vs.filter(|_| height >= 40.0) {
                                        span { class: "cal-plan-vs", style: "{MUTED} font-variant-numeric: tabular-nums;", "{v}" }
                                    }
                                }
                                if b.minutes > 0 && ratio > 0.0 {
                                    div { class: "cal-plan-bar", style: "position: absolute; left: 0; top: 0; bottom: 0; width: 3px; margin-left: -3px; background: linear-gradient(to bottom, var(--foreground) {bar}%, transparent 0); opacity: 0.55;" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The templates, as Day Plans has them: their names.
fn use_templates() -> Signal<Vec<String>> {
    let mut names = use_signal(Vec::new);
    use_future(move || async move {
        if let Some(Ok(v)) = crate::vault::service(DAYPLAN, "templates", json!({})).await {
            let list: Vec<String> = v["templates"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| t["name"].as_str().map(str::to_owned))
                .collect();
            names.set(list);
        }
    });
    names
}

/// A day's template on its head: its name in its colour, why in its
/// tooltip, a menu to choose another for that day by hand, go back to
/// the rules, or reset an edited day.
#[component]
pub fn TemplateBadge(day: NaiveDate, plan: PlanDay, problem: Signal<Option<String>>) -> Element {
    let mut open = use_signal(|| false);
    let names = use_templates();
    let colour = plan
        .color
        .clone()
        .unwrap_or_else(|| "var(--muted-foreground)".into());
    let name = plan
        .template
        .clone()
        .unwrap_or_else(|| "No template".into());
    let why = plan.reason.clone();
    let mark = if plan.chosen { "✎ " } else { "" };
    let edited = plan.edited;
    let tip = format!("{name}: {why}{}", if edited { " · edited" } else { "" });
    let mut choose = move |template: Value, clear: bool| {
        open.set(false);
        let mut problem = problem;
        spawn(async move {
            let args = json!({ "date": day.to_string(), "template": template, "clear": clear });
            if let Err(e) = call("override", args).await {
                problem.set(Some(e));
            }
        });
    };
    rsx! {
        div { class: "cal-template", style: "position: relative; display: flex; justify-content: center; margin-top: 3px;",
            button { class: "cal-template-badge", "data-date": "{day}", "data-chosen": "{plan.chosen}",
                title: "{tip}",
                style: "display: inline-flex; align-items: center; gap: 4px; max-width: calc(100% - 8px); padding: 1px 8px; border-radius: 999px; \
                        font-size: 10.5px; cursor: pointer; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; \
                        border: 1px solid color-mix(in srgb, {colour} 50%, transparent); background: color-mix(in srgb, {colour} 14%, transparent); color: var(--foreground);",
                onclick: move |_| open.toggle(),
                span { style: "width: 6px; height: 6px; border-radius: 3px; background: {colour}; flex: none;" }
                "{mark}{name}"
                if edited { span { style: "{MUTED}", "·edited" } }
            }
            if open() {
                div { class: "cal-template-menu",
                    style: "position: absolute; top: 24px; z-index: 20; min-width: 200px; padding: 6px; border-radius: 10px; text-align: left; \
                            border: 1px solid var(--border); background: var(--popover, var(--background)); box-shadow: 0 8px 24px rgba(0,0,0,0.18);",
                    div { style: "font-size: 11px; padding: 4px 6px; {MUTED}", "{why}" }
                    for n in names() {
                        {
                            let on = plan.template.as_deref() == Some(n.as_str());
                            let bg = if on { "var(--accent)" } else { "transparent" };
                            let pick = n.clone();
                            rsx! {
                                button { key: "{n}", class: "cal-template-pick", "data-template": "{n}",
                                    style: "display: block; width: 100%; text-align: left; padding: 5px 8px; border: none; border-radius: 6px; cursor: pointer; font-size: 12px; \
                                            background: {bg}; color: var(--foreground);",
                                    onclick: move |_| choose(json!(pick.clone()), false),
                                    "Use {n} this day"
                                }
                            }
                        }
                    }
                    button { style: "display: block; width: 100%; text-align: left; padding: 5px 8px; border: none; border-radius: 6px; cursor: pointer; font-size: 12px; background: transparent; color: var(--foreground);",
                        onclick: move |_| choose(json!("none"), false), "No template this day" }
                    if plan.chosen {
                        button { style: "display: block; width: 100%; text-align: left; padding: 5px 8px; border: none; border-radius: 6px; cursor: pointer; font-size: 12px; background: transparent; color: var(--foreground);",
                            onclick: move |_| choose(Value::Null, true), "Back to the rules" }
                    }
                    if edited {
                        button { class: "cal-template-reset", style: "display: block; width: 100%; text-align: left; padding: 5px 8px; border: none; border-radius: 6px; cursor: pointer; font-size: 12px; background: transparent; color: var(--destructive);",
                            onclick: move |_| {
                                open.set(false);
                                let mut problem = problem;
                                spawn(async move {
                                    if let Err(e) = call("reset", json!({ "date": day.to_string() })).await {
                                        problem.set(Some(e));
                                    }
                                });
                            },
                            "Reset to template"
                        }
                    }
                    button { style: "display: block; width: 100%; text-align: left; padding: 5px 8px; border: none; border-radius: 6px; cursor: pointer; font-size: 12px; background: transparent; {MUTED}",
                        onclick: move |_| { open.set(false); spawn(async move { let _ = crate::vault::run("EXT_VIEW_OPEN", json!({ "extension": "dayplan", "view": "templates" })).await; }); },
                        "Edit templates and rules…" }
                }
            }
        }
    }
}

/// What a block can be given to: each kind's choices, `(title, path)`.
async fn choices(kind: &str) -> Vec<(String, String)> {
    let rows = |v: &Value, list: &str| -> Vec<(String, String)> {
        v[list]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| {
                let path = t["path"].as_str()?.to_owned();
                let title = t["title"]
                    .as_str()
                    .map_or_else(|| crate::model::stem(&path).to_owned(), str::to_owned);
                Some((title, path))
            })
            .collect()
    };
    match kind {
        "task" => {
            match crate::vault::service("architect.tasks", "list", json!({ "view": "all" })).await {
                Some(Ok(v)) => rows(&v, "tasks"),
                _ => Vec::new(),
            }
        }
        "project" => match crate::vault::service("architect.projects", "list", json!({})).await {
            Some(Ok(v)) => rows(&v, "projects"),
            _ => Vec::new(),
        },
        "recipe" => {
            let base = "filters:\n  or:\n    - type == \"recipe\"\n    - type == \"meal\"\nviews:\n  - type: table\n    name: Recipes\n    order:\n      - title\n";
            crate::vault::rows(base)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|r| (r.name.trim_end_matches(".md").to_owned(), r.path))
                .collect()
        }
        _ => Vec::new(),
    }
}

/// A plan block's panel: when, what it is given to, and how to change it.
#[component]
pub fn PlanPanel(
    picked: Signal<PickedPlan>,
    plans: Vec<PlanDay>,
    problem: Signal<Option<String>>,
) -> Element {
    let Some((day, id)) = picked() else {
        return rsx! {};
    };
    let plan = plans.iter().find(|p| p.date == day).cloned();
    let block = plan
        .as_ref()
        .and_then(|p| p.blocks.iter().find(|b| b.id == id).cloned());
    let mut kind = use_signal(String::new);
    let mut text = use_signal(String::new);
    let mut options = use_signal(Vec::<(String, String)>::new);
    let mut was = use_signal(|| (day, id.clone()));
    let default_kind = block.as_ref().map_or("label", |b| {
        if b.category == "meal" {
            "recipe"
        } else {
            "project"
        }
    });
    if *was.peek() != (day, id.clone()) || kind.peek().is_empty() {
        was.set((day, id.clone()));
        kind.set(default_kind.to_owned());
        text.set(String::new());
    }
    use_effect(move || {
        let k = kind();
        spawn(async move {
            options.set(choices(&k).await);
        });
    });
    let Some(b) = block else {
        return rsx! {};
    };
    let plan = plan.unwrap_or_default();
    let give = move |title: String, path: Option<String>| {
        let k = kind.peek().clone();
        let id = id.clone();
        let mut problem = problem;
        spawn(async move {
            let args = json!({ "date": day.to_string(), "block": id, "kind": k, "title": title, "path": path });
            if let Err(e) = call("assign", args).await {
                problem.set(Some(e));
            }
        });
    };
    let clear = {
        let id = b.id.clone();
        move |_| {
            let id = id.clone();
            let mut problem = problem;
            spawn(async move {
                let args = json!({ "date": day.to_string(), "block": id, "kind": "none" });
                if let Err(e) = call("assign", args).await {
                    problem.set(Some(e));
                }
            });
        }
    };
    let filter = text().to_lowercase();
    let shown = options()
        .into_iter()
        .filter(|(t, _)| filter.is_empty() || t.to_lowercase().contains(&filter))
        .take(8);
    let given = b.assignment.clone();
    let when = format!(
        "{} · {} · {}{}",
        day.format("%A %-d %B"),
        b.when(),
        b.category,
        if b.fixed { " · fixed" } else { "" }
    );
    let template_words = plan.template_words();
    let tracked = format!(
        "Tracked {} of {} planned",
        minutes_words(b.tracked),
        minutes_words(b.minutes)
    );
    let now_kind = kind();
    let give_label = give.clone();
    let placeholder = if now_kind == "label" {
        "A label, then Enter"
    } else {
        "Find one…"
    };
    rsx! {
        div { class: "cal-plan-panel",
            style: "flex: 0 0 300px; border-left: 1px solid var(--border); padding: 14px 16px; overflow-y: auto; display: flex; flex-direction: column; gap: 10px; box-sizing: border-box;",
            div { style: "display: flex; align-items: center; gap: 8px;",
                span { style: "width: 10px; height: 10px; border-radius: 3px; background: {b.colour};" }
                span { style: "font-weight: 700; font-size: 15px; flex: 1;", "{b.label}" }
                button { style: BUTTON, "aria-label": "Close", onclick: move |_| { let mut p = picked; p.set(None); }, "×" }
            }
            div { style: "font-size: 12px; {MUTED}", "{when}" }
            div { style: "font-size: 12px; {MUTED}", "{template_words}" }
            div { class: "cal-plan-tracked", style: "font-size: 12px;", "{tracked}" }
            div { style: "font-size: 11px; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; {MUTED}", "Given to" }
            if let Some(a) = given {
                div { class: "cal-plan-given", style: "display: flex; align-items: center; gap: 8px; padding: 6px 8px; border-radius: 8px; border: 1px solid var(--border);",
                    span { style: "font-size: 11px; {MUTED}", "{a.kind}" }
                    if let Some(p) = a.path.clone() {
                        button { style: "flex: 1; text-align: left; border: none; background: transparent; color: var(--foreground); cursor: pointer; font-size: 13px; font-weight: 600; padding: 0;",
                            onclick: move |_| { let p = p.clone(); spawn(async move { if let Some(h) = crate::vault::handle() { let _ = h.host(architect_extensions::hostcall::HostRequest::Open { path: p }).await; } }); },
                            "{a.title}" }
                    } else {
                        span { style: "flex: 1; font-weight: 600;", "{a.title}" }
                    }
                    button { style: BUTTON, onclick: clear, "Clear" }
                }
            } else {
                div { style: "font-size: 12px; {MUTED}", "Nothing yet." }
            }
            div { style: "display: flex; gap: 4px; flex-wrap: wrap;",
                for k in ["project", "task", "recipe", "label"] {
                    {
                        let bg = if now_kind == k { "var(--accent)" } else { "transparent" };
                        rsx! {
                            button { key: "{k}", class: "cal-plan-kind", "data-kind": "{k}",
                                style: "{BUTTON} background: {bg};",
                                onclick: move |_| kind.set(k.to_owned()), "{k}" }
                        }
                    }
                }
            }
            input { class: "cal-plan-text", r#type: "text", style: "{FIELD} width: 100%;",
                placeholder,
                value: "{text}",
                oninput: move |e: FormEvent| text.set(e.value()),
                onkeydown: move |e: KeyboardEvent| {
                    e.stop_propagation();
                    if e.key() == Key::Named(NamedKey::Enter) && kind.peek().as_str() == "label" {
                        let t = text.peek().trim().to_owned();
                        if !t.is_empty() { give_label(t, None); text.set(String::new()); }
                    }
                },
            }
            if now_kind != "label" {
                div { style: "display: flex; flex-direction: column; gap: 2px;",
                    for (t, p) in shown {
                        {
                            let give = give.clone();
                            let (tt, pp) = (t.clone(), p.clone());
                            rsx! {
                                button { key: "{p}", class: "cal-plan-option",
                                    style: "text-align: left; border: none; border-radius: 6px; padding: 5px 8px; background: transparent; color: var(--foreground); cursor: pointer; font-size: 12px;",
                                    onclick: move |_| give(tt.clone(), Some(pp.clone())),
                                    "{t}"
                                }
                            }
                        }
                    }
                }
            }
            if plan.edited {
                button { class: "cal-plan-reset", style: "{BUTTON} color: var(--destructive); margin-top: 6px;",
                    onclick: move |_| {
                        let mut problem = problem;
                        spawn(async move {
                            if let Err(e) = call("reset", json!({ "date": day.to_string() })).await {
                                problem.set(Some(e));
                            }
                        });
                    },
                    "Reset this day to its template"
                }
            }
        }
    }
}
