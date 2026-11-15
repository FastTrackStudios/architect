//! Its views.
//!
//! The cookbook (find a recipe, read it scaled), cook mode (step by
//! step, with timers), the editor (Cooklang with a live preview), and
//! the recipe card a `type: recipe` note draws at its head (an
//! `[[embeds]]` claim).
//!
//! The views hold only the `Recipes` client ([`crate::client`]); what
//! they show is read in futures, never while drawing. Layouts wrap, so a
//! phone's narrow pane stacks what a desk shows side by side.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::model::{CookStep, Recipe};
use crate::scale::{number, scaled};
use crate::service::{RecipeSummary, Save, Search};

const MUTED: &str = "color: var(--muted-foreground);";
const HEADING: &str = "font-size: 11px; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; color: var(--muted-foreground);";
const BUTTON: &str = "min-height: 32px; padding: 0 12px; border-radius: 8px; font-size: 13px; cursor: pointer; border: 1px solid var(--border); background: transparent; color: var(--foreground);";
const PRIMARY: &str = "min-height: 32px; padding: 0 14px; border-radius: 8px; font-size: 13px; cursor: pointer; border: none; background: var(--primary); color: var(--primary-foreground);";
const FIELD: &str = "height: 32px; box-sizing: border-box; padding: 0 9px; border-radius: 8px; border: 1px solid var(--input); background: var(--background); color: var(--foreground); font-size: 13px;";
const CARD: &str = "border: 1px solid var(--border); border-radius: 10px; padding: 12px 14px; background: var(--card, transparent);";

// ── What the cook and edit views show ───────────────────────────────

/// The recipe cook mode and the editor show: set by `COOKBOOK_OPEN` and
/// the views' buttons.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Open {
    /// Its vault path; none for a recipe not saved yet.
    pub path: Option<String>,
    /// Its Cooklang source.
    pub source: String,
    /// Scaled to this many servings.
    pub servings: Option<f64>,
}

static OPEN: Mutex<Option<Open>> = Mutex::new(None);
static OPEN_REVISION: AtomicU64 = AtomicU64::new(0);

/// Shows `open` in cook mode and the editor.
pub fn set_open(open: Open) {
    *OPEN.lock().unwrap_or_else(PoisonError::into_inner) = Some(open);
    OPEN_REVISION.fetch_add(1, Ordering::Relaxed);
}

/// What cook mode and the editor show.
#[must_use]
pub fn open() -> Option<Open> {
    OPEN.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Moves on when the views' recipe or the cookbook changes.
static REVISION: AtomicU64 = AtomicU64::new(0);

/// Tells the views the cookbook changed.
pub fn bump() {
    REVISION.fetch_add(1, Ordering::Relaxed);
}

fn revisions() -> (u64, u64) {
    (
        REVISION.load(Ordering::Relaxed),
        OPEN_REVISION.load(Ordering::Relaxed),
    )
}

/// Draws the view again when [`bump`] or [`set_open`] is called, and
/// every five seconds while it shows (an edit made elsewhere).
fn use_revision() -> (u64, u64, u32) {
    let mut seen = use_signal(|| {
        let (r, o) = revisions();
        (r, o, 0)
    });
    use_future(move || async move {
        let mut ticks = 0_u32;
        loop {
            futures_timer::Delay::new(Duration::from_millis(250)).await;
            ticks = ticks.wrapping_add(1);
            let (r, o) = revisions();
            let now = (r, o, ticks / 20);
            if *seen.peek() != now {
                seen.set(now);
            }
        }
    });
    seen()
}

/// `0:45`, `4:00`, `1:02:30`.
#[must_use]
pub fn clock(seconds: u32) -> String {
    let (h, m, s) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// A step as a cook reads it: `[[Hot Honey]]{3}` is "Hot Honey" (an
/// alias, `[[Hot Honey|the honey]]`, its own words).
#[must_use]
pub fn as_read(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for link in crate::wiki::scan_links(text) {
        let end = link.start.saturating_add(link.len);
        if let (Some(before), true) = (text.get(at..link.start), end <= text.len()) {
            out.push_str(before);
            out.push_str(&link.display);
            at = end;
        }
    }
    out.push_str(text.get(at..).unwrap_or_default());
    out
}

/// A recipe as a note's frontmatter (`props`) and Cooklang body.
#[must_use]
pub fn source_of(props: &Value, body: &str) -> String {
    props.as_object().filter(|m| !m.is_empty()).map_or_else(
        || body.to_owned(),
        |m| {
            let yaml = serde_yaml::to_string(m).unwrap_or_default();
            format!("---\n{}---\n{body}", yaml.trim_start_matches("---\n"))
        },
    )
}

fn spawn_action(id: &'static str, args: Value) {
    spawn(async move {
        let _ = run_app(id, args).await;
    });
}

async fn run_app(id: &str, args: Value) -> Result<Value, String> {
    use architect_extensions::hostcall::HostRequest;
    let h = crate::client::handle().ok_or("the cookbook is not running")?;
    h.host(HostRequest::Run {
        action: id.to_owned(),
        args,
        ticket: None,
    })
    .await
    .map_err(|e| e.to_string())
}

/// Opens one of its views on `open`.
fn show(view: &'static str, open: Open) {
    set_open(open);
    spawn_action(
        "EXT_VIEW_OPEN",
        json!({ "extension": crate::ID, "view": view }),
    );
}

// ── The cookbook ─────────────────────────────────────────────────────

/// Find a recipe; read it scaled.
#[component]
pub fn CookbookView() -> Element {
    let rev = use_revision();
    let mut text = use_signal(String::new);
    let mut found = use_signal(Vec::<RecipeSummary>::new);
    let mut problem = use_signal(|| None::<String>);
    let mut picked = use_signal(|| None::<String>);
    let mut recipe = use_signal(|| None::<Recipe>);
    let mut importing = use_signal(|| false);
    let mut from = use_signal(String::new);

    // The list, again when the text or the cookbook changes.
    use_effect(move || {
        let q = text();
        let _ = rev;
        spawn(async move {
            let got = async {
                let c = crate::client::client().await.map_err(|e| e.to_string())?;
                c.list(Search {
                    text: Some(q).filter(|q| !q.trim().is_empty()),
                    ..Search::default()
                })
                .await
                .map_err(|e| architect::vox_into_app(e).to_string())
            }
            .await;
            match got {
                Ok(list) => {
                    if *found.peek() != list {
                        found.set(list);
                    }
                    problem.set(None);
                }
                Err(e) => problem.set(Some(e)),
            }
        });
    });
    // The recipe picked.
    use_effect(move || {
        let Some(path) = picked() else {
            recipe.set(None);
            return;
        };
        let _ = rev;
        spawn(async move {
            let got = async {
                let c = crate::client::client().await.map_err(|e| e.to_string())?;
                c.get(path, None)
                    .await
                    .map_err(|e| architect::vox_into_app(e).to_string())
            }
            .await;
            match got {
                Ok(r) => {
                    if recipe.peek().as_ref() != Some(&r) {
                        recipe.set(Some(r));
                    }
                }
                Err(e) => problem.set(Some(e)),
            }
        });
    });

    let list = found();
    let mounted = use_signal(|| None::<std::rc::Rc<MountedData>>);
    let width = use_width(mounted);
    // A phone's pane shows the list or the recipe, not both.
    let phone = width().is_some_and(|w| w < WIDE);
    let show_list = !phone || picked().is_none();
    let show_detail = !phone || picked().is_some();
    rsx! {
        div { class: "cb-root", "data-phone": "{phone}",
            onmounted: move |e: MountedEvent| { let mut m = mounted; m.set(Some(e.data())); },
            style: "display: flex; flex-direction: column; height: 100%; box-sizing: border-box; color: var(--foreground); font-size: 14px;",
            div { style: "display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: 12px 16px; border-bottom: 1px solid var(--border);",
                // A phone's shell titles the pane already.
                if !phone {
                    span { style: "font-size: 18px; font-weight: 700; margin-right: 4px;", "Cookbook" }
                }
                input { class: "cb-search", style: "{FIELD} flex: 1 1 160px; min-width: 120px;",
                    placeholder: "Find a recipe, a tag, a course…",
                    value: "{text}",
                    oninput: move |e| text.set(e.value()) }
                button { class: "cb-new", style: "{BUTTON}",
                    onclick: move |_| show("edit", Open { path: None, source: NEW.to_owned(), servings: None }),
                    "New" }
                button { class: "cb-import-open", style: "{BUTTON}",
                    onclick: move |_| importing.set(!importing()),
                    "Import" }
                button { class: "cb-base", style: "{BUTTON}",
                    onclick: move |_| spawn_action("COOKBOOK_BASE", Value::Null),
                    "As a base" }
            }
            if importing() {
                div { class: "cb-import", style: "display: flex; flex-wrap: wrap; gap: 8px; padding: 8px 16px; border-bottom: 1px solid var(--border);",
                    input { class: "cb-import-from", style: "{FIELD} flex: 1 1 220px;",
                        placeholder: "A recipe page's address, or a .cook file's text",
                        value: "{from}",
                        oninput: move |e| from.set(e.value()) }
                    button { class: "cb-import-go", style: "{PRIMARY}", disabled: from().trim().is_empty(),
                        onclick: move |_| {
                            let given = from().trim().to_owned();
                            let is_url = given.starts_with("http://") || given.starts_with("https://");
                            let import = crate::service::Import {
                                url: is_url.then(|| given.clone()),
                                text: (!is_url).then(|| given.clone()),
                                path: None,
                                save: true,
                            };
                            spawn(async move {
                                let got = async {
                                    let c = crate::client::client().await.map_err(|e| e.to_string())?;
                                    c.import(import).await.map_err(|e| architect::vox_into_app(e).to_string())
                                }.await;
                                match got {
                                    Ok(done) => {
                                        problem.set(None);
                                        from.set(String::new());
                                        importing.set(false);
                                        picked.set(Some(done.path));
                                        bump();
                                    }
                                    Err(e) => problem.set(Some(e)),
                                }
                            });
                        },
                        "Import" }
                }
            }
            if let Some(p) = problem() {
                div { class: "cb-problem", style: "padding: 6px 16px; color: var(--destructive);", "{p}" }
            }
            div { style: "flex: 1; min-height: 0; overflow: auto; display: flex; flex-wrap: wrap; align-content: flex-start; gap: 16px; padding: 12px 16px;",
                if show_list {
                div { class: "cb-list", style: "flex: 1 1 240px; max-width: 100%; display: flex; flex-direction: column; gap: 6px;",
                    if list.is_empty() {
                        div { style: "{MUTED} padding: 8px 0;",
                            if text().trim().is_empty() { "No recipes yet. New writes one in Cooklang." } else { "Nothing matches." }
                        }
                    }
                    for s in list {
                        {
                            let on = picked().as_deref() == Some(s.path.as_str());
                            let bg = if on { "var(--accent)" } else { "transparent" };
                            let path = s.path.clone();
                            let meta = summary_line(&s);
                            rsx! {
                                button { key: "{s.path}", class: "cb-row", "data-path": "{s.path}",
                                    style: "text-align: left; padding: 8px 10px; border-radius: 8px; border: 1px solid var(--border); background: {bg}; color: var(--foreground); cursor: pointer;",
                                    onclick: move |_| picked.set(Some(path.clone())),
                                    div { style: "font-weight: 600;", "{s.name}" }
                                    if !meta.is_empty() {
                                        div { style: "font-size: 12px; {MUTED}", "{meta}" }
                                    }
                                }
                            }
                        }
                    }
                }
                }
                if show_detail {
                div { class: "cb-detail", style: "flex: 3 1 340px; max-width: 100%; min-width: 0; display: flex; flex-direction: column; gap: 10px;",
                    if phone {
                        button { class: "cb-back-to-list", style: "{BUTTON} align-self: flex-start;",
                            onclick: move |_| picked.set(None), "‹ All recipes" }
                    }
                    if let Some(r) = recipe() {
                        RecipeCard { key: "{r.path}", recipe: r.clone(), path: Some(r.path) }
                    } else {
                        div { style: "{MUTED} padding: 8px 0;", "Pick a recipe to read it." }
                    }
                }
                }
            }
        }
    }
}

/// Below this a pane is a phone's: one thing at a time.
const WIDE: f64 = 760.0;

/// The width of the view's box, measured again while mounted (Blitz
/// fires no resize), as the Calendar's and Mealplan's views do.
fn use_width(mounted: Signal<Option<std::rc::Rc<MountedData>>>) -> Signal<Option<f64>> {
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
            futures_timer::Delay::new(Duration::from_millis(400)).await;
        }
    });
    width
}

/// A new recipe's first text.
pub const NEW: &str = "---\ntitle: New recipe\nservings: 2\ntags: []\n---\nMix @flour{200%g} with @milk{300%ml}.\n\nCook for ~{10%minutes}.\n";

fn summary_line(s: &RecipeSummary) -> String {
    let mut bits = Vec::new();
    if let Some(c) = &s.course {
        bits.push(c.clone());
    }
    if let Some(v) = &s.servings {
        bits.push(format!("serves {v}"));
    }
    let total = s
        .prep_minutes
        .unwrap_or(0)
        .saturating_add(s.cook_minutes.unwrap_or(0));
    if total > 0 {
        bits.push(format!("{total} min"));
    }
    for t in s.tags.iter().take(3) {
        bits.push(format!("#{t}"));
    }
    bits.join(" · ")
}

// ── The recipe card ─────────────────────────────────────────────────

/// A recipe to read: scale it, its ingredients, its steps with their
/// timers, its cookware; Cook and Edit when it is a file.
#[component]
pub fn RecipeCard(recipe: Recipe, path: Option<String>) -> Element {
    let base = recipe.servings.map_or(1.0, f64::from);
    let mut servings = use_signal(|| base);
    let shown = scaled(recipe.clone(), Some(servings()));
    let source = recipe.source;
    let sections = sections(&shown.cook_steps.0);
    let total = shown
        .prep_minutes
        .unwrap_or(0)
        .saturating_add(shown.cook_minutes.unwrap_or(0));
    rsx! {
        div { class: "cb-card", style: "display: flex; flex-direction: column; gap: 14px;",
            div { style: "display: flex; flex-wrap: wrap; align-items: baseline; gap: 8px;",
                span { class: "cb-name", style: "font-size: 20px; font-weight: 700;", "{shown.name}" }
                if let Some(c) = &shown.course { span { style: "font-size: 12px; {MUTED}", "{c}" } }
                if let Some(c) = &shown.cuisine { span { style: "font-size: 12px; {MUTED}", "{c}" } }
                if total > 0 { span { style: "font-size: 12px; {MUTED}", "{total} min" } }
            }
            if let Some(d) = &shown.description {
                div { style: "{MUTED}", "{d}" }
            }
            div { style: "display: flex; flex-wrap: wrap; align-items: center; gap: 8px;",
                span { style: "{HEADING}", "Serves" }
                button { class: "cb-less", style: "{BUTTON} width: 32px; padding: 0;",
                    onclick: move |_| { let s = servings(); if s > 1.0 { servings.set(s - 1.0); } }, "−" }
                span { class: "cb-servings", style: "min-width: 28px; text-align: center; font-weight: 700;", "{number(servings())}" }
                button { class: "cb-more", style: "{BUTTON} width: 32px; padding: 0;",
                    onclick: move |_| servings.set(servings() + 1.0), "+" }
                span { style: "flex: 1;" }
                {
                    let path_c = path.clone();
                    let source_c = source.clone();
                    rsx! {
                        button { class: "cb-cook", style: "{PRIMARY}",
                            onclick: move |_| show("cook", Open { path: path_c.clone(), source: source_c.clone(), servings: Some(servings()) }),
                            "Cook" }
                    }
                }
                if path.is_some() {
                    {
                        let path_e = path;
                        let source_e = source;
                        rsx! {
                            button { class: "cb-edit", style: "{BUTTON}",
                                onclick: move |_| show("edit", Open { path: path_e.clone(), source: source_e.clone(), servings: None }),
                                "Edit" }
                        }
                    }
                }
            }
            div { style: "display: flex; flex-wrap: wrap; gap: 16px; align-items: flex-start;",
                div { class: "cb-ingredients", style: "{CARD} flex: 1 1 220px;",
                    div { style: "{HEADING} margin-bottom: 6px;", "Ingredients" }
                    for (i, ing) in shown.ingredients.iter().enumerate() {
                        div { key: "{i}", class: "cb-ingredient", style: "display: flex; gap: 8px; padding: 3px 0;",
                            span { style: "min-width: 72px; font-variant-numeric: tabular-nums; font-weight: 600;",
                                "{ing.qty_display.clone().unwrap_or_default()} {ing.unit}" }
                            span {
                                if ing.is_recipe_ref { "↪ " }
                                "{ing.name}"
                                if ing.optional { span { style: "{MUTED}", " (optional)" } }
                                if let Some(n) = &ing.note { span { style: "{MUTED}", " — {n}" } }
                            }
                        }
                    }
                    if !shown.cookware.is_empty() {
                        div { style: "{HEADING} margin: 10px 0 4px;", "Cookware" }
                        div { style: "{MUTED}", "{shown.cookware.join(\", \")}" }
                    }
                }
                div { class: "cb-steps", style: "flex: 2 1 300px; display: flex; flex-direction: column; gap: 10px;",
                    for (si, (label, steps)) in sections.into_iter().enumerate() {
                        div { key: "{si}", style: "display: flex; flex-direction: column; gap: 8px;",
                            if let Some(l) = label { div { style: "{HEADING}", "{l}" } }
                            for (n, step) in steps {
                                StepRow { key: "{n}", n, step: step, name: shown.name.clone() }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The steps by their `= Section`, numbered from one across them.
fn sections(steps: &[CookStep]) -> Vec<(Option<String>, Vec<(usize, CookStep)>)> {
    let mut out: Vec<(Option<String>, Vec<(usize, CookStep)>)> = Vec::new();
    for (i, s) in steps.iter().enumerate() {
        let n = i.saturating_add(1);
        match out.last_mut() {
            Some((label, list)) if *label == s.section => list.push((n, s.clone())),
            _ => out.push((s.section.clone(), vec![(n, s.clone())])),
        }
    }
    out
}

#[component]
fn StepRow(n: usize, step: CookStep, name: String) -> Element {
    rsx! {
        div { class: "cb-step", style: "display: flex; gap: 10px;",
            span { style: "min-width: 22px; font-weight: 700; {MUTED}", "{n}" }
            div { style: "display: flex; flex-direction: column; gap: 4px; flex: 1;",
                span { style: "line-height: 1.45;", "{as_read(&step.text)}" }
                for note in step.notes.iter() {
                    span { style: "font-size: 12px; {MUTED}", "{note}" }
                }
                if !step.timers.is_empty() {
                    div { style: "display: flex; flex-wrap: wrap; gap: 6px;",
                        for (ti, t) in step.timers.iter().enumerate() {
                            {
                                let label = t.name.clone().unwrap_or_else(|| format!("{name}, step {n}"));
                                let seconds = t.seconds;
                                rsx! {
                                    button { key: "{ti}", class: "cb-timer", style: "{BUTTON} min-height: 26px; font-size: 12px;",
                                        onclick: move |_| spawn_action("COOKBOOK_TIMER", json!({ "seconds": seconds, "label": label.clone() })),
                                        "⏱ {clock(seconds)}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ── The embed: a `type: recipe` note's head ─────────────────────────

/// The recipe card a recipe note draws at its head (its claim
/// `note_type = ["recipe"]`): its frontmatter and Cooklang body parsed
/// in place, nothing read.
#[component]
pub fn RecipeEmbed() -> Element {
    let Some(input) = architect_extensions::linked::use_embed() else {
        return rsx! {};
    };
    let source = source_of(&input.props, &input.source);
    match crate::parse::parse_cook("Recipe.cook", &source) {
        Ok(r) => rsx! {
            div { class: "cb-embed", style: "padding: 4px 0;",
                RecipeCard { recipe: r, path: None }
            }
        },
        Err(e) => rsx! { div { style: "{MUTED}", "Not a recipe Cooklang reads: {e}" } },
    }
}

// ── Cook mode ───────────────────────────────────────────────────────

/// Step by step: gather what it needs, then one step at a time, big,
/// with its timers.
#[component]
pub fn CookView() -> Element {
    let _ = use_revision();
    let Some(o) = open() else {
        return rsx! { div { style: "padding: 24px; {MUTED}", "Open a recipe and press Cook." } };
    };
    let recipe =
        match crate::parse::parse_cook(o.path.as_deref().unwrap_or("Recipe.cook"), &o.source) {
            Ok(r) => scaled(r, o.servings),
            Err(e) => return rsx! { div { style: "padding: 24px;", "{e}" } },
        };
    rsx! { Cook { key: "{o.path.clone().unwrap_or_default()}{o.servings.unwrap_or(0.0)}", recipe } }
}

#[component]
fn Cook(recipe: Recipe) -> Element {
    // Page 0 gathers; page n is step n.
    let mut page = use_signal(|| 0_usize);
    let mut have = use_signal(Vec::<usize>::new);
    let steps = recipe.cook_steps.0.clone();
    let last = steps.len();
    let p = page().min(last);
    rsx! {
        div { class: "cb-cook-root", "data-page": "{p}",
            style: "display: flex; flex-direction: column; height: 100%; box-sizing: border-box; color: var(--foreground);",
            div { style: "display: flex; align-items: center; gap: 10px; padding: 12px 16px; border-bottom: 1px solid var(--border);",
                span { style: "font-size: 17px; font-weight: 700; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;", "{recipe.name}" }
                span { style: "font-size: 12px; {MUTED}", if p == 0 { "Gather" } else { "Step {p} of {last}" } }
            }
            div { style: "flex: 1; min-height: 0; overflow: auto; padding: 18px 16px; display: flex; flex-direction: column; gap: 14px;",
                if p == 0 {
                    div { style: "{HEADING}", "Gather" }
                    for (i, ing) in recipe.ingredients.iter().enumerate() {
                        {
                            let got = have().contains(&i);
                            let mark = if got { "☑" } else { "☐" };
                            let deco = if got { "line-through" } else { "none" };
                            rsx! {
                                button { key: "{i}", class: "cb-gather",
                                    style: "text-align: left; display: flex; gap: 10px; align-items: center; min-height: 40px; padding: 4px 8px; border-radius: 8px; border: 1px solid var(--border); background: transparent; color: var(--foreground); font-size: 16px; cursor: pointer; text-decoration: {deco};",
                                    onclick: move |_| { let mut h = have.write(); if let Some(at) = h.iter().position(|x| *x == i) { h.remove(at); } else { h.push(i); } },
                                    span { "{mark}" }
                                    span { style: "font-weight: 600; min-width: 80px;", "{ing.qty_display.clone().unwrap_or_default()} {ing.unit}" }
                                    span { "{ing.name}" }
                                }
                            }
                        }
                    }
                } else if let Some(step) = steps.get(p.saturating_sub(1)) {
                    if let Some(s) = &step.section { div { style: "{HEADING}", "{s}" } }
                    div { class: "cb-cook-step", style: "font-size: 22px; line-height: 1.5;", "{as_read(&step.text)}" }
                    for note in step.notes.iter() { div { style: "{MUTED}", "{note}" } }
                    for (ti, t) in step.timers.iter().enumerate() {
                        {
                            let label = t.name.clone().unwrap_or_else(|| format!("{}, step {p}", recipe.name));
                            let seconds = t.seconds;
                            rsx! {
                                Countdown { key: "{p}-{ti}", label, seconds }
                            }
                        }
                    }
                }
            }
            div { style: "display: flex; gap: 10px; padding: 12px 16px; border-top: 1px solid var(--border);",
                button { class: "cb-back", style: "{BUTTON} flex: 1; min-height: 44px;", disabled: p == 0,
                    onclick: move |_| page.set(p.saturating_sub(1)), "Back" }
                button { class: "cb-next", style: "{PRIMARY} flex: 2; min-height: 44px;", disabled: p >= last,
                    onclick: move |_| page.set((p + 1).min(last)),
                    if p == 0 { "Start" } else if p >= last { "Done" } else { "Next" } }
            }
        }
    }
}

/// One timer: Start counts it down here and schedules the OS's
/// notification for its end (a toast where there is none).
#[component]
fn Countdown(label: String, seconds: u32) -> Element {
    let mut left = use_signal(|| None::<u32>);
    let running = left().is_some();
    let shown = left().unwrap_or(seconds);
    let label_c = label.clone();
    let label_d = label.clone();
    rsx! {
        div { class: "cb-countdown", style: "{CARD} display: flex; align-items: center; gap: 12px;",
            span { style: "font-size: 28px; font-weight: 700; font-variant-numeric: tabular-nums;", "{clock(shown)}" }
            span { style: "flex: 1; {MUTED}", "{label}" }
            if !running {
                button { class: "cb-start", style: "{PRIMARY} min-height: 40px;",
                    onclick: move |_| {
                        let label_d = label_d.clone();
                        left.set(Some(seconds));
                        spawn_action("COOKBOOK_TIMER", json!({ "seconds": seconds, "label": label_c.clone() }));
                        spawn(async move {
                            loop {
                                futures_timer::Delay::new(Duration::from_secs(1)).await;
                                let now = left.peek().unwrap_or(0);
                                if now <= 1 {
                                    left.set(Some(0));
                                    done(&label_d);
                                    break;
                                }
                                left.set(Some(now - 1));
                            }
                        });
                    },
                    "Start" }
            } else if shown == 0 {
                span { style: "font-weight: 700; color: var(--primary);", "Done" }
            }
        }
    }
}

/// A timer that ran out while its view showed: a toast in the app (the
/// device's notification, where there is one, was scheduled at Start).
fn done(label: &str) {
    use architect_extensions::hostcall::HostRequest;
    let Some(h) = crate::client::handle() else {
        return;
    };
    let text = format!("{label}: done");
    spawn(async move {
        let _ = h
            .host(HostRequest::Notify {
                text,
                kind: Some("success".to_owned()),
                title: Some("Timer".to_owned()),
            })
            .await;
    });
}

// ── The editor ──────────────────────────────────────────────────────

/// Cooklang, with what it reads as beside it; Save writes it (one undo
/// step).
#[component]
pub fn EditView() -> Element {
    let rev = use_revision();
    let Some(o) = open() else {
        return rsx! { div { style: "padding: 24px; {MUTED}", "Open a recipe and press Edit, or New." } };
    };
    rsx! { Edit { key: "{rev.1}", open: o } }
}

#[component]
fn Edit(open: Open) -> Element {
    let mut source = use_signal(|| open.source.clone());
    let mut said = use_signal(|| None::<(bool, String)>);
    let path = open.path.clone();
    let preview = crate::parse::check(path.as_deref().unwrap_or("Recipe.cook"), &source());
    let title = path.clone().unwrap_or_else(|| "New recipe".to_owned());
    rsx! {
        div { class: "cb-edit-root",
            style: "display: flex; flex-direction: column; height: 100%; box-sizing: border-box; color: var(--foreground);",
            div { style: "display: flex; flex-wrap: wrap; align-items: center; gap: 10px; padding: 12px 16px; border-bottom: 1px solid var(--border);",
                span { style: "font-size: 16px; font-weight: 700; flex: 1; min-width: 0;", "{title}" }
                if let Some((ok, text)) = said() {
                    span { class: "cb-said", style: if ok { "font-size: 12px; color: var(--muted-foreground);" } else { "font-size: 12px; color: var(--destructive);" }, "{text}" }
                }
                {
                    let path_s = path;
                    rsx! {
                        button { class: "cb-save", style: "{PRIMARY}", disabled: preview.is_err(),
                            onclick: move |_| {
                                let save = Save { path: path_s.clone(), name: None, source: source(), create: path_s.is_none() };
                                spawn(async move {
                                    let got = async {
                                        let c = crate::client::client().await.map_err(|e| e.to_string())?;
                                        c.save(save).await.map_err(|e| architect::vox_into_app(e).to_string())
                                    }.await;
                                    match got {
                                        Ok(r) => {
                                            said.set(Some((true, format!("Saved {}", r.path))));
                                            set_open(Open { path: Some(r.path.clone()), source: r.source, servings: None });
                                            bump();
                                        }
                                        Err(e) => said.set(Some((false, e))),
                                    }
                                });
                            },
                            "Save" }
                    }
                }
            }
            div { style: "flex: 1; min-height: 0; display: flex; flex-wrap: wrap; gap: 12px; padding: 12px 16px; overflow: auto;",
                textarea { class: "cb-source",
                    style: "flex: 1 1 320px; min-height: 280px; box-sizing: border-box; padding: 10px; border-radius: 8px; border: 1px solid var(--input); background: var(--background); color: var(--foreground); font-family: var(--font-mono, monospace); font-size: 13px; line-height: 1.5;",
                    initial_value: "{open.source}",
                    oninput: move |e| source.set(e.value()) }
                div { class: "cb-preview", style: "flex: 1 1 280px; min-width: 0;",
                    match preview {
                        Ok(r) => rsx! { RecipeCard { recipe: r, path: None } },
                        Err(e) => rsx! { div { style: "color: var(--destructive);", "{e}" } },
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_recipe_link_reads_as_its_name() {
        assert_eq!(
            as_read("Drizzle [[Hot Honey]]{3} over, with [[Slaw|the slaw]]."),
            "Drizzle Hot Honey over, with the slaw."
        );
    }

    #[test]
    fn clocks_read_as_kitchen_timers() {
        assert_eq!(clock(45), "0:45");
        assert_eq!(clock(240), "4:00");
        assert_eq!(clock(3750), "1:02:30");
    }

    #[test]
    fn a_notes_frontmatter_and_body_make_its_cooklang() {
        let s = source_of(
            &json!({ "servings": 2, "title": "Toast" }),
            "Toast @bread{2%slices}.\n",
        );
        let r = crate::parse::parse_cook("x.cook", &s).unwrap();
        assert_eq!(r.name, "Toast");
        assert_eq!(r.servings, Some(2));
        assert_eq!(source_of(&Value::Null, "x"), "x");
    }

    #[test]
    fn steps_group_under_their_sections() {
        let r = crate::parse::parse_cook(
            "x.cook",
            "= Prep\n\nChop @onion{1}.\n\n= Cook\n\nFry it.\n\nServe.\n",
        )
        .unwrap();
        let s = sections(&r.cook_steps.0);
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].1.len(), 2);
        assert_eq!(s[1].1[0].0, 2);
    }
}
