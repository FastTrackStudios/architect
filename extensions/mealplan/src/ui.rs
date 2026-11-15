//! Its views: the week, the pantry, the grocery list.
//!
//! - **Week** (`week`): each day's meals by meal. A slot's `+` opens the
//!   recipe picker (the Cookbook's `list` through the host, a search and
//!   the servings); a meal's Move picks it up and a slot's "Move here"
//!   puts it down, on a desk and a phone alike (no drag); × unplans it.
//!   A desk shows the days as seven columns, a phone as a list.
//! - **Pantry** (`pantry`): the items with their stock, `−`/`+` and ×;
//!   adding one by name, amount, unit and aisle; the substitutions.
//! - **Groceries** (`groceries`): the week's list by aisle, a line ticked
//!   by a tap (Bought also stocks the pantry), and Build to make it again.
//!
//! Every view holds only the `Meals` client. What it shows is read in a
//! future that peeks (never a signal it reads in render) every few
//! seconds and after each change it makes, and a signal is written only
//! when what was read differs, so nothing reads for ever.

use std::fmt::Display;
use std::rc::Rc;
use std::time::Duration as Wait;

use architect_extensions::hostcall::HostRequest;
use chrono::{Duration, Local, NaiveDate};
use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::client::{client, handle};
use crate::model::{Meal, monday, week_id};
use crate::service::{
    Check, Groceries, MealRef, MoveTo, PantryItem, PantrySearch, PlannedMeal, Range, Stock,
    Substitution, Week,
};
use crate::units::number;

const MUTED: &str = "color: var(--muted-foreground);";
const HEADING: &str = "font-size: 11px; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; color: var(--muted-foreground);";
const BUTTON: &str = "height: 28px; padding: 0 10px; border-radius: 7px; font-size: 12px; cursor: pointer; border: 1px solid var(--border); background: transparent; color: var(--foreground);";
const SMALL: &str = "height: 22px; min-width: 22px; padding: 0 6px; border-radius: 6px; font-size: 11px; cursor: pointer; border: 1px solid var(--border); background: transparent; color: var(--foreground);";
const PRIMARY: &str = "height: 28px; padding: 0 12px; border-radius: 7px; font-size: 12px; cursor: pointer; border: none; background: var(--primary); color: var(--primary-foreground);";
const FIELD: &str = "height: 28px; box-sizing: border-box; padding: 0 7px; border-radius: 6px; border: 1px solid var(--input); background: var(--background); color: var(--foreground); font-size: 12px; min-width: 0;";
const CARD: &str = "border: 1px solid var(--border); border-radius: 10px; background: var(--card); color: var(--card-foreground);";

/// Narrower than this, a view is laid out for a phone.
const WIDE: f64 = 760.0;

fn err(e: impl Display) -> String {
    e.to_string()
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// The width of the view's box, measured again while mounted (Blitz
/// fires no resize), as the Calendar's views do.
fn use_width(mounted: Signal<Option<Rc<MountedData>>>) -> Signal<Option<f64>> {
    let mut width = use_signal(|| None);
    use_future(move || async move {
        loop {
            if let Some(m) = mounted.peek().clone()
                && let Ok(r) = m.get_client_rect().await
            {
                let w = Some(r.size.width);
                if *width.peek() != w {
                    width.set(w);
                }
            }
            futures_timer::Delay::new(Wait::from_millis(400)).await;
        }
    });
    width
}

/// Runs `job`, then says what went wrong (or nothing) and reads again.
fn act<T: 'static>(
    mut problem: Signal<Option<String>>,
    mut nonce: Signal<u64>,
    job: impl std::future::Future<Output = Result<T, String>> + 'static,
) {
    spawn(async move {
        let said = job.await.err();
        if *problem.peek() != said {
            problem.set(said);
        }
        *nonce.write() += 1;
    });
}

/// `value` read by `load(key)` whenever `key` or `nonce` moves, and every
/// few seconds: written only when it differs. `None` until first read;
/// a failure is `Err` (for an empty state).
fn use_read<K, T, F, Fut>(
    key: Signal<K>,
    nonce: Signal<u64>,
    load: F,
) -> Signal<Option<Result<T, String>>>
where
    K: Clone + PartialEq + 'static,
    T: PartialEq + 'static,
    F: Fn(K) -> Fut + 'static,
    Fut: std::future::Future<Output = Result<T, String>> + 'static,
{
    let mut value = use_signal(|| None::<Result<T, String>>);
    let load = Rc::new(load);
    use_future(move || {
        let load = load.clone();
        async move {
            let mut seen: Option<(K, u64)> = None;
            let mut ticks = 0_u32;
            loop {
                let now = (key.peek().clone(), *nonce.peek());
                if seen.as_ref() != Some(&now) || ticks.is_multiple_of(20) {
                    seen = Some(now.clone());
                    let got = load(now.0).await;
                    if value.peek().as_ref() != Some(&got) {
                        value.set(Some(got));
                    }
                }
                ticks = ticks.wrapping_add(1);
                futures_timer::Delay::new(Wait::from_millis(250)).await;
            }
        }
    });
    value
}

fn open_note(path: String) {
    spawn(async move {
        if let Some(h) = handle() {
            let _ = h.host(HostRequest::Open { path }).await;
        }
    });
}

/// The week view.
#[component]
pub fn WeekView() -> Element {
    rsx! { MealplanView { tab: "week" } }
}

/// The pantry and the substitutions.
#[component]
pub fn PantryView() -> Element {
    rsx! { MealplanView { tab: "pantry" } }
}

/// The grocery list.
#[component]
pub fn GroceriesView() -> Element {
    rsx! { MealplanView { tab: "groceries" } }
}

#[component]
fn MealplanView(tab: String) -> Element {
    let mut on = use_signal(|| tab.clone());
    let mounted = use_signal(|| None::<Rc<MountedData>>);
    let width = use_width(mounted);
    let phone = width().is_some_and(|w| w < WIDE);
    let problem = use_signal(|| None::<String>);
    let nonce = use_signal(|| 0_u64);
    let mut start = use_signal(|| monday(today()));
    let tabs = [
        ("week", "Week"),
        ("pantry", "Pantry"),
        ("groceries", "Groceries"),
    ];
    let s = start();
    let label = week_id(s);
    let span = format!(
        "{} – {}",
        s.format("%-d %b"),
        (s + Duration::days(6)).format("%-d %b")
    );
    let pad = if phone { "10px 12px" } else { "12px 16px" };
    rsx! {
        div { class: "mp-root", "data-tab": "{on}", "data-phone": "{phone}",
            onmounted: move |e: MountedEvent| { let mut m = mounted; m.set(Some(e.data())); },
            style: "position: relative; display: flex; flex-direction: column; height: 100%; box-sizing: border-box; color: var(--foreground); font-size: 13px; background: var(--background);",
            div { style: "display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: {pad}; border-bottom: 1px solid var(--border);",
                if !phone {
                    span { style: "font-size: 17px; font-weight: 700; margin-right: 6px;", "Mealplan" }
                }
                div { style: "display: flex; gap: 4px;",
                    for (id, name) in tabs {
                        {
                            let bg = if on() == id { "var(--accent)" } else { "transparent" };
                            rsx! {
                                button { key: "{id}", class: "mp-tab", "data-tab": "{id}", style: "{BUTTON} background: {bg};",
                                    onclick: move |_| on.set(id.to_owned()), "{name}" }
                            }
                        }
                    }
                }
                span { style: "flex: 1;" }
                if on() != "pantry" {
                    div { style: "display: flex; align-items: center; gap: 4px;",
                        button { class: "mp-prev", style: SMALL, onclick: move |_| { let s = *start.peek(); start.set(s - Duration::days(7)); }, "‹" }
                        button { class: "mp-this-week", style: "{SMALL} min-width: 0;", title: "This week",
                            onclick: move |_| start.set(monday(today())),
                            span { style: "font-weight: 600;", "{label}" }
                            if !phone { span { style: MUTED, " · {span}" } }
                        }
                        button { class: "mp-next", style: SMALL, onclick: move |_| { let s = *start.peek(); start.set(s + Duration::days(7)); }, "›" }
                    }
                }
            }
            if let Some(p) = problem() {
                div { class: "mp-problem", style: "padding: 6px 16px; color: var(--destructive); font-size: 12px;", "{p}" }
            }
            div { style: "flex: 1; min-height: 0; overflow: auto;",
                match on().as_str() {
                    "pantry" => rsx! { Pantry { phone, problem, nonce } },
                    "groceries" => rsx! { GroceryList { phone, start, problem, nonce } },
                    _ => rsx! { WeekPlan { phone, start, problem, nonce } },
                }
            }
        }
    }
}

// ── The week ────────────────────────────────────────────────────────

/// A slot of the week: a day's meal.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Slot {
    date: String,
    meal: Meal,
}

/// A meal picked up to move.
#[derive(Clone, Debug, PartialEq)]
struct Moving {
    from: MealRef,
    name: String,
}

#[component]
fn WeekPlan(
    phone: bool,
    start: Signal<NaiveDate>,
    problem: Signal<Option<String>>,
    nonce: Signal<u64>,
) -> Element {
    let week = use_read(start, nonce, |s: NaiveDate| async move {
        let c = client().await.map_err(err)?;
        c.week(Range {
            week: Some(week_id(s)),
            ..Range::default()
        })
        .await
        .map_err(architect::vox_into_app)
        .map_err(err)
    });
    let mut picking = use_signal(|| None::<Slot>);
    let mut moving = use_signal(|| None::<Moving>);
    let week: Option<Week> = match week() {
        Some(Ok(w)) => Some(w),
        Some(Err(e)) => {
            return rsx! { div { style: "padding: 16px; {MUTED}", "{e}" } };
        }
        None => None,
    };
    let Some(week) = week else {
        return rsx! { div { style: "padding: 16px; {MUTED}", "Reading the week…" } };
    };
    let today = today().to_string();
    let cols = if phone {
        "display: flex; flex-direction: column; gap: 10px; padding: 10px 12px;".to_owned()
    } else {
        "display: grid; grid-template-columns: repeat(7, minmax(0, 1fr)); gap: 8px; padding: 12px 16px;".to_owned()
    };
    rsx! {
        div { style: "display: flex; flex-direction: column; min-height: 100%;",
            if let Some(m) = moving() {
                div { class: "mp-moving", style: "display: flex; align-items: center; gap: 8px; padding: 6px 16px; background: color-mix(in srgb, var(--primary) 12%, transparent); font-size: 12px;",
                    span { "Moving {m.name}: choose “Move here” on a slot." }
                    span { style: "flex: 1;" }
                    button { style: SMALL, onclick: move |_| moving.set(None), "Cancel" }
                }
            }
            div { class: "mp-week", style: "{cols}",
                for day in week.days.iter().cloned() {
                    {
                        let date = NaiveDate::parse_from_str(&day.date, "%Y-%m-%d").ok();
                        let head = date.map(|d| d.format(if phone { "%A %-d %B" } else { "%a %-d" }).to_string()).unwrap_or_default();
                        let ring = if day.date == today { "border-color: var(--primary);" } else { "" };
                        rsx! {
                            div { key: "{day.date}", class: "mp-day", "data-date": "{day.date}", style: "{CARD} {ring} padding: 8px; display: flex; flex-direction: column; gap: 6px; min-width: 0;",
                                div { style: "font-weight: 700; font-size: 12.5px;", "{head}" }
                                for meal in Meal::ALL {
                                    {
                                        let here: Vec<PlannedMeal> = day.meals.iter().filter(|m| Meal::parse(&m.meal) == Some(meal)).cloned().collect();
                                        let slot_add = Slot { date: day.date.clone(), meal };
                                        let slot_move = slot_add.clone();
                                        let show = !here.is_empty() || !phone || meal == Meal::Dinner || moving().is_some();
                                        rsx! {
                                            if show {
                                                div { key: "{meal.as_str()}", class: "mp-slot", "data-meal": "{meal.as_str()}",
                                                    style: "display: flex; flex-direction: column; gap: 3px;",
                                                    div { style: "display: flex; align-items: center; gap: 4px;",
                                                        span { style: "{HEADING} flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;", "{meal.title()}" }
                                                        if moving().is_some() {
                                                            button { class: "mp-move-here", style: SMALL,
                                                                onclick: move |_| {
                                                                    let Some(m) = moving.peek().clone() else { return };
                                                                    moving.set(None);
                                                                    let to = MoveTo { date: Some(slot_move.date.clone()), meal: Some(slot_move.meal.as_str().to_owned()) };
                                                                    act(problem, nonce, async move {
                                                                        let c = client().await.map_err(err)?;
                                                                        c.move_meal(m.from, to).await.map_err(architect::vox_into_app).map_err(err)
                                                                    });
                                                                },
                                                                "Move here"
                                                            }
                                                        } else {
                                                            button { class: "mp-add", style: SMALL, title: "Plan a recipe",
                                                                onclick: move |_| picking.set(Some(slot_add.clone())), "+" }
                                                        }
                                                    }
                                                    for m in here {
                                                        {
                                                            let r = MealRef { date: m.date.clone(), meal: m.meal.clone(), recipe: Some(m.recipe.clone()) };
                                                            let r2 = r.clone();
                                                            // `×4`, when it says.
                                                            let path = m.recipe.clone();
                                                            let name = m.name.clone();
                                                            let serves = m.servings.map(|s| format!("{} ", number(s))).unwrap_or_default();
                                                            rsx! {
                                                                div { key: "{m.recipe}", class: "mp-meal", "data-recipe": "{m.recipe}",
                                                                    style: "display: flex; flex-direction: column; gap: 3px; padding: 5px 6px; border-radius: 7px; background: color-mix(in srgb, var(--primary) 12%, transparent); min-width: 0;",
                                                                    div { class: "mp-meal-name", title: "Open the recipe",
                                                                        style: "font-size: 12px; font-weight: 600; line-height: 1.3; cursor: pointer; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
                                                                        onclick: move |_| open_note(path.clone()), "{m.name}" }
                                                                    div { style: "display: flex; align-items: center; gap: 3px;",
                                                                    span { style: "{MUTED} font-size: 11px; flex: 1; min-width: 0;", if serves.is_empty() { "" } else { "×{serves}" } }
                                                                    button { class: "mp-move", style: SMALL, title: "Move",
                                                                        onclick: move |_| moving.set(Some(Moving { from: r.clone(), name: name.clone() })), "Move" }
                                                                    button { class: "mp-unplan", style: SMALL, title: "Unplan",
                                                                        onclick: move |_| {
                                                                            let r = r2.clone();
                                                                            act(problem, nonce, async move {
                                                                                let c = client().await.map_err(err)?;
                                                                                c.unplan(r).await.map_err(architect::vox_into_app).map_err(err)
                                                                            });
                                                                        },
                                                                        "×"
                                                                    }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if let Some(slot) = picking() {
                RecipePicker { phone, slot, picking, problem, nonce }
            }
        }
    }
}

/// A recipe as the Cookbook lists it.
#[derive(Clone, Debug, PartialEq)]
struct Listed {
    path: String,
    name: String,
    servings: Option<f64>,
}

/// The Cookbook's recipes whose name has `text` (`architect.cookbook`
/// `list`, through the host).
async fn recipes(text: String) -> Result<Vec<Listed>, String> {
    let h = handle().ok_or("Mealplan is not running")?;
    let args = if text.trim().is_empty() {
        json!({ "limit": 50 })
    } else {
        json!({ "text": text.trim(), "limit": 50 })
    };
    let v = h
        .service(crate::files::COOKBOOK, "list", args)
        .await
        .map_err(err)?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            Some(Listed {
                path: r.get("path")?.as_str()?.to_owned(),
                name: r
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                servings: r.get("servings").and_then(crate::model::number_of),
            })
        })
        .collect())
}

#[component]
fn RecipePicker(
    phone: bool,
    slot: Slot,
    picking: Signal<Option<Slot>>,
    problem: Signal<Option<String>>,
    nonce: Signal<u64>,
) -> Element {
    let mut search = use_signal(String::new);
    let mut servings = use_signal(|| 2.0_f64);
    let found = use_read(search, nonce, recipes);
    let date = NaiveDate::parse_from_str(&slot.date, "%Y-%m-%d")
        .map(|d| d.format("%A %-d %B").to_string())
        .unwrap_or_default();
    let title = format!("{} on {date}", slot.meal.title());
    let place = if phone {
        "position: absolute; left: 0; right: 0; bottom: 0; max-height: 75%; border-radius: 14px 14px 0 0;"
    } else {
        "position: absolute; top: 0; right: 0; bottom: 0; width: 320px; max-width: 90%; border-radius: 0;"
    };
    let n = servings();
    rsx! {
        div { class: "mp-picker-scrim", style: "position: absolute; inset: 0; background: rgba(0, 0, 0, 0.3); z-index: 9;",
            onclick: move |_| picking.set(None) }
        div { class: "mp-picker", style: "{place} z-index: 10; box-sizing: border-box; display: flex; flex-direction: column; gap: 8px; padding: 12px; background: var(--popover); color: var(--popover-foreground); border: 1px solid var(--border); box-shadow: 0 8px 30px rgba(0,0,0,0.25);",
            div { style: "display: flex; align-items: center; gap: 8px;",
                span { style: "font-weight: 700; flex: 1;", "{title}" }
                button { style: SMALL, onclick: move |_| picking.set(None), "×" }
            }
            input { class: "mp-search", r#type: "text", placeholder: "Find a recipe", style: "{FIELD} width: 100%;",
                value: "{search}", oninput: move |e: FormEvent| search.set(e.value()) }
            div { style: "display: flex; align-items: center; gap: 6px; font-size: 12px;",
                span { style: MUTED, "Servings" }
                button { style: SMALL, onclick: move |_| { let v = (*servings.peek() - 1.0).max(1.0); servings.set(v); }, "−" }
                span { class: "mp-servings", style: "min-width: 20px; text-align: center; font-weight: 600;", "{number(n)}" }
                button { style: SMALL, onclick: move |_| { let v = *servings.peek() + 1.0; servings.set(v); }, "+" }
            }
            div { style: "flex: 1; min-height: 0; overflow: auto; display: flex; flex-direction: column; gap: 2px;",
                match found() {
                    None => rsx! { span { style: MUTED, "Reading the Cookbook…" } },
                    Some(Err(e)) => rsx! { span { style: "color: var(--destructive);", "{e}" } },
                    Some(Ok(list)) if list.is_empty() => rsx! { span { style: MUTED, "No recipes found." } },
                    Some(Ok(list)) => rsx! {
                        for r in list {
                            {
                                let slot = slot.clone();
                                let path = r.path.clone();
                                let own = r.servings.map(|s| format!("serves {}", number(s))).unwrap_or_default();
                                rsx! {
                                    button { key: "{r.path}", class: "mp-recipe", "data-path": "{r.path}",
                                        style: "display: flex; align-items: baseline; gap: 6px; text-align: left; padding: 7px 8px; border-radius: 7px; border: none; background: transparent; color: var(--foreground); cursor: pointer; font-size: 13px;",
                                        onclick: move |_| {
                                            let m = PlannedMeal {
                                                date: slot.date.clone(),
                                                meal: slot.meal.as_str().to_owned(),
                                                recipe: path.clone(),
                                                servings: Some(*servings.peek()),
                                                ..PlannedMeal::default()
                                            };
                                            picking.set(None);
                                            act(problem, nonce, async move {
                                                let c = client().await.map_err(err)?;
                                                c.plan(m).await.map_err(architect::vox_into_app).map_err(err)
                                            });
                                        },
                                        span { style: "flex: 1;", "{r.name}" }
                                        span { style: "{MUTED} font-size: 11px;", "{own}" }
                                    }
                                }
                            }
                        }
                    },
                }
            }
        }
    }
}

// ── The pantry ──────────────────────────────────────────────────────

/// How much `−`/`+` moves an item: 100 of grams or millilitres, else 1.
fn step(unit: &str) -> f64 {
    match unit.trim().to_ascii_lowercase().as_str() {
        "g" | "ml" => 100.0,
        _ => 1.0,
    }
}

#[component]
fn Pantry(phone: bool, problem: Signal<Option<String>>, nonce: Signal<u64>) -> Element {
    let none = use_signal(|| ());
    let items = use_read(none, nonce, |()| async move {
        let c = client().await.map_err(err)?;
        c.pantry(PantrySearch::default())
            .await
            .map_err(architect::vox_into_app)
            .map_err(err)
    });
    let subs = use_read(none, nonce, |()| async move {
        let c = client().await.map_err(err)?;
        c.substitutions()
            .await
            .map_err(architect::vox_into_app)
            .map_err(err)
    });
    let mut name = use_signal(String::new);
    let mut qty = use_signal(String::new);
    let mut unit = use_signal(String::new);
    let mut aisle = use_signal(String::new);
    let mut from = use_signal(String::new);
    let mut to = use_signal(String::new);
    let mut ratio = use_signal(|| "1".to_owned());
    let mut add = move || {
        let n = name.peek().trim().to_owned();
        if n.is_empty() {
            return;
        }
        let change = Stock {
            name: n,
            qty: qty.peek().trim().parse().ok(),
            unit: Some(unit.peek().trim().to_owned()).filter(|u| !u.is_empty()),
            aisle: Some(aisle.peek().trim().to_owned()).filter(|a| !a.is_empty()),
            ..Stock::default()
        };
        name.set(String::new());
        qty.set(String::new());
        act(problem, nonce, async move {
            let c = client().await.map_err(err)?;
            c.stock(change)
                .await
                .map_err(architect::vox_into_app)
                .map_err(err)
        });
    };
    let mut substitute = move || {
        let (f, t) = (from.peek().trim().to_owned(), to.peek().trim().to_owned());
        if f.is_empty() || t.is_empty() {
            return;
        }
        let rule = Substitution {
            from: f,
            to: t,
            ratio: ratio.peek().trim().parse().unwrap_or(1.0),
            ..Substitution::default()
        };
        from.set(String::new());
        to.set(String::new());
        act(problem, nonce, async move {
            let c = client().await.map_err(err)?;
            c.substitute(rule)
                .await
                .map_err(architect::vox_into_app)
                .map_err(err)
        });
    };
    let wrap = if phone { "flex-wrap: wrap;" } else { "" };
    rsx! {
        div { class: "mp-pantry", style: "display: flex; flex-direction: column; gap: 14px; padding: 12px 16px; max-width: 820px;",
            div { style: "display: flex; gap: 6px; {wrap}",
                input { class: "mp-item-name", r#type: "text", placeholder: "Item", style: "{FIELD} flex: 2;",
                    value: "{name}", oninput: move |e: FormEvent| name.set(e.value()),
                    onkeydown: move |e: KeyboardEvent| if e.key() == Key::Named(NamedKey::Enter) { add() } }
                input { r#type: "text", placeholder: "Amount", style: "{FIELD} flex: 1; width: 70px;",
                    value: "{qty}", oninput: move |e: FormEvent| qty.set(e.value()) }
                input { r#type: "text", placeholder: "Unit", style: "{FIELD} flex: 1; width: 60px;",
                    value: "{unit}", oninput: move |e: FormEvent| unit.set(e.value()) }
                input { r#type: "text", placeholder: "Aisle", style: "{FIELD} flex: 1; width: 90px;",
                    value: "{aisle}", oninput: move |e: FormEvent| aisle.set(e.value()) }
                button { class: "mp-item-add", style: PRIMARY, onclick: move |_| add(), "Add" }
            }
            match items() {
                None => rsx! { span { style: MUTED, "Reading the pantry…" } },
                Some(Err(e)) => rsx! { span { style: "color: var(--destructive);", "{e}" } },
                Some(Ok(list)) if list.is_empty() => rsx! { span { style: MUTED, "The pantry is empty. Add what you have, and the grocery list leaves it off." } },
                Some(Ok(list)) => rsx! {
                    div { style: "{CARD} display: flex; flex-direction: column;",
                        for item in list {
                            PantryRow { key: "{item.path}", item, problem, nonce }
                        }
                    }
                },
            }
            div { style: "display: flex; flex-direction: column; gap: 6px;",
                span { style: HEADING, "Substitutions" }
                match subs() {
                    Some(Ok(list)) if !list.is_empty() => rsx! {
                        for s in list {
                            {
                                let (f, t) = (s.from.clone(), s.to.clone());
                                let always = if s.always { " · always" } else { "" };
                                rsx! {
                                    div { key: "{s.from}→{s.to}", class: "mp-sub", style: "display: flex; align-items: center; gap: 6px; font-size: 12.5px;",
                                        span { style: "flex: 1;", "{s.from}: {number(s.ratio)} × {s.to}{always}" }
                                        button { style: SMALL, title: "Remove",
                                            onclick: move |_| {
                                                let (f, t) = (f.clone(), t.clone());
                                                act(problem, nonce, async move {
                                                    let c = client().await.map_err(err)?;
                                                    c.unsubstitute(f, Some(t)).await.map_err(architect::vox_into_app).map_err(err)
                                                });
                                            },
                                            "×"
                                        }
                                    }
                                }
                            }
                        }
                    },
                    _ => rsx! { span { style: "{MUTED} font-size: 12px;", "None yet: what stands in for what when the pantry has it." } },
                }
                div { style: "display: flex; gap: 6px; {wrap}",
                    input { r#type: "text", placeholder: "Instead of", style: "{FIELD} flex: 2;",
                        value: "{from}", oninput: move |e: FormEvent| from.set(e.value()) }
                    input { r#type: "text", placeholder: "use", style: "{FIELD} flex: 2;",
                        value: "{to}", oninput: move |e: FormEvent| to.set(e.value()) }
                    input { r#type: "text", placeholder: "ratio", style: "{FIELD} flex: 1; width: 50px;",
                        value: "{ratio}", oninput: move |e: FormEvent| ratio.set(e.value()) }
                    button { class: "mp-sub-add", style: BUTTON, onclick: move |_| substitute(), "Add" }
                }
            }
        }
    }
}

/// One pantry item. It makes no signals: a keyed row's own would be
/// captured by the list's closures and outlive it.
#[component]
fn PantryRow(item: PantryItem, problem: Signal<Option<String>>, nonce: Signal<u64>) -> Element {
    let amount = item.qty.map_or_else(
        || {
            if item.staple {
                "always".to_owned()
            } else {
                "—".to_owned()
            }
        },
        |q| format!("{} {}", number(q), item.unit).trim().to_owned(),
    );
    let by = step(&item.unit);
    let (up, down, gone) = (item.name.clone(), item.name.clone(), item.name.clone());
    let unit = item.unit.clone();
    let unit2 = item.unit.clone();
    let flag = if item.expired {
        "expired"
    } else if item.low {
        "low"
    } else {
        ""
    };
    let aisle = item.aisle.clone().unwrap_or_default();
    let path = item.path.clone();
    rsx! {
        div { class: "mp-item", "data-name": "{item.name}",
            style: "display: flex; align-items: center; gap: 8px; padding: 7px 10px; border-bottom: 1px solid var(--border);",
            div { style: "flex: 1; min-width: 0; display: flex; align-items: baseline; gap: 8px; cursor: pointer; overflow: hidden;",
                title: "Open its note",
                onclick: move |_| open_note(path.clone()),
                span { style: "font-size: 13px; white-space: nowrap;", "{item.name}" }
                if !aisle.is_empty() { span { style: "{MUTED} font-size: 11px; white-space: nowrap;", "{aisle}" } }
            }
            if !flag.is_empty() {
                span { style: "font-size: 11px; color: var(--destructive);", "{flag}" }
            }
            button { style: SMALL, title: "Less",
                onclick: move |_| {
                    let change = Stock { name: down.clone(), add: Some(-by), unit: Some(unit.clone()), ..Stock::default() };
                    act(problem, nonce, async move {
                        let c = client().await.map_err(err)?;
                        c.stock(change).await.map_err(architect::vox_into_app).map_err(err)
                    });
                },
                "−"
            }
            span { class: "mp-qty", style: "min-width: 64px; text-align: center; font-variant-numeric: tabular-nums;", "{amount}" }
            button { style: SMALL, title: "More",
                onclick: move |_| {
                    let change = Stock { name: up.clone(), add: Some(by), unit: Some(unit2.clone()), ..Stock::default() };
                    act(problem, nonce, async move {
                        let c = client().await.map_err(err)?;
                        c.stock(change).await.map_err(architect::vox_into_app).map_err(err)
                    });
                },
                "+"
            }
            button { style: SMALL, title: "Remove from the pantry",
                onclick: move |_| {
                    let n = gone.clone();
                    act(problem, nonce, async move {
                        let c = client().await.map_err(err)?;
                        c.remove_item(n).await.map_err(architect::vox_into_app).map_err(err)
                    });
                },
                "×"
            }
        }
    }
}

// ── The grocery list ────────────────────────────────────────────────

#[component]
fn GroceryList(
    phone: bool,
    start: Signal<NaiveDate>,
    problem: Signal<Option<String>>,
    nonce: Signal<u64>,
) -> Element {
    let list = use_read(start, nonce, |s: NaiveDate| async move {
        let c = client().await.map_err(err)?;
        c.grocery_list(Range {
            week: Some(week_id(s)),
            ..Range::default()
        })
        .await
        .map_err(architect::vox_into_app)
        .map_err(err)
    });
    let build = move |_| {
        let week = week_id(*start.peek());
        act(problem, nonce, async move {
            let c = client().await.map_err(err)?;
            c.groceries(Range {
                week: Some(week),
                ..Range::default()
            })
            .await
            .map_err(architect::vox_into_app)
            .map_err(err)
        });
    };
    let list: Option<Result<Groceries, String>> = list();
    let pad = if phone { "10px 12px" } else { "12px 16px" };
    rsx! {
        div { class: "mp-groceries", style: "display: flex; flex-direction: column; gap: 12px; padding: {pad}; max-width: 720px;",
            div { style: "display: flex; align-items: center; gap: 8px;",
                span { style: "{MUTED} font-size: 12px; flex: 1;", "From the week's meals, less the pantry. Ticks stay when it is built again." }
                button { class: "mp-build", style: PRIMARY, onclick: build,
                    if matches!(list, Some(Ok(_))) { "Build again" } else { "Build the list" }
                }
            }
            match list {
                None => rsx! { span { style: MUTED, "Reading the list…" } },
                Some(Err(_)) => rsx! { span { style: MUTED, "No list for this week yet. Plan some meals, then build it." } },
                Some(Ok(g)) => rsx! {
                    if !g.substituted.is_empty() {
                        span { style: "{MUTED} font-size: 12px;", "Using instead: {g.substituted.join(\", \")}" }
                    }
                    if g.aisles.is_empty() {
                        span { style: MUTED, "Nothing to buy: the pantry has it all." }
                    }
                    for a in g.aisles.iter().cloned() {
                        div { key: "{a.aisle}", class: "mp-aisle", style: "display: flex; flex-direction: column; gap: 2px;",
                            span { style: HEADING, "{a.aisle}" }
                            for i in a.items {
                                {
                                    let path = g.path.clone();
                                    let path2 = g.path.clone();
                                    let text = i.text.clone();
                                    let text2 = i.text.clone();
                                    let checked = i.checked;
                                    let strike = if checked { "text-decoration: line-through; color: var(--muted-foreground);" } else { "" };
                                    rsx! {
                                        div { key: "{i.text}", class: "mp-line", "data-checked": "{checked}",
                                            style: "display: flex; align-items: center; gap: 8px; padding: 5px 2px; border-bottom: 1px solid var(--border);",
                                            button { class: "mp-tick", style: "flex: 1; min-width: 0; display: flex; align-items: center; gap: 8px; text-align: left; border: none; background: transparent; color: var(--foreground); cursor: pointer; font-size: 13.5px; padding: 0;",
                                                onclick: move |_| {
                                                    let ch = Check { path: Some(path.clone()), item: text.clone(), checked: !checked, ..Check::default() };
                                                    act(problem, nonce, async move {
                                                        let c = client().await.map_err(err)?;
                                                        c.check(ch).await.map_err(architect::vox_into_app).map_err(err)
                                                    });
                                                },
                                                Tick { on: checked }
                                                span { style: "{strike}", "{i.text}" }
                                            }
                                            if !checked && !i.own {
                                                button { class: "mp-bought", style: SMALL, title: "Bought: tick it and add it to the pantry",
                                                    onclick: move |_| {
                                                        let ch = Check { path: Some(path2.clone()), item: text2.clone(), checked: true, bought: true, ..Check::default() };
                                                        act(problem, nonce, async move {
                                                            let c = client().await.map_err(err)?;
                                                            c.check(ch).await.map_err(architect::vox_into_app).map_err(err)
                                                        });
                                                    },
                                                    "Bought"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
            }
        }
    }
}

/// A checkbox drawn, not a glyph (a phone's fonts may have no ☐).
#[component]
fn Tick(on: bool) -> Element {
    let fill = if on { "var(--primary)" } else { "transparent" };
    let edge = if on {
        "var(--primary)"
    } else {
        "var(--muted-foreground)"
    };
    rsx! {
        span { class: "mp-tick-box", "data-on": "{on}",
            style: "display: inline-block; flex: none; width: 16px; height: 16px; box-sizing: border-box; border-radius: 4px; border: 2px solid {edge}; background: {fill};" }
    }
}
