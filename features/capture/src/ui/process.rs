//! Processing: turning one capture into what it actually is (#445).
//!
//! Ported from Task (`task:crates/ui/src/process.rs`), its look and
//! behaviour kept, onto architect's inbox (`INBOX_*`) and targets.
//! Capture is the cheap half: a line lands in the inbox with no
//! questions asked. Processing makes the system trustworthy: every
//! capture comes back once, and leaves as a task with somewhere to live,
//! a note, a later, or nothing.
//!
//! [`ProcessCard`] is that one decision, used by the inbox deck
//! ([`ProcessDeck`]), by "Process now" in the composer, and by the
//! navigator's Inbox view (#443). It works in two [`Mode`]s, the person's
//! choice, remembered on this device:
//!
//! - **Fast**: the card and its outcomes side by side (Task's card): do it
//!   now, add to a project, write a note, snooze; keep as reference, a
//!   project, already done, delete.
//! - **Clarify**: the questions in order (Task's first card, `bf21391a`):
//!   *is it actionable?* If not: a note, someday, or delete. If so: *do it
//!   now* (two minutes, the clock started), *waiting on someone*, *make it
//!   a task*, or *it's a project*.
//!
//! Either way, "make it a task" opens [`TaskShape`]: the capture read the
//! way quick add reads it into an editable title, project, contexts, due
//! date and estimate, so a task leaves the inbox already filed. Made
//! through the `task` target (the Tasks extension's `TASKS_CAPTURE`); its
//! clock through the Time Tracker's `TIME_START`.

// Days, seconds and a deck's place: small counts, never near an edge.
#![allow(clippy::arithmetic_side_effects, clippy::as_conversions)]

use std::sync::atomic::{AtomicU8, Ordering};

use architect::action::ActionCaller;
use chrono::{Datelike as _, Duration, Local};
use dioxus::prelude::*;
use lucide_dioxus as lc;
use serde_json::{Map, Value, json};

use super::CaptureUi;
use super::style::{self, CaptureCss, FIELD_RESET, Look, RESET, TRUNCATE, use_look, wash};
use crate::actions::{self, Capturer};
use crate::inbox::{self, Item};
use crate::target;

// ── mode ────────────────────────────────────────────────────────────

/// How much the card asks before it acts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Every outcome on the card at once.
    Fast,
    /// The GTD questions, one at a time.
    Clarify,
}

/// This run's choice: 0 unread, 1 Fast, 2 Clarify.
static MODE: AtomicU8 = AtomicU8::new(0);

/// Where the choice is kept on this device: `capture-mode` in the app's
/// config folder (`ARCHITECT_CONFIG_DIR`), when there is one.
fn mode_file() -> Option<std::path::PathBuf> {
    std::env::var_os("ARCHITECT_CONFIG_DIR")
        .filter(|d| !d.is_empty())
        .map(|d| std::path::PathBuf::from(d).join("capture-mode"))
}

impl Mode {
    /// This device's choice; Fast until one is made.
    #[must_use]
    pub fn stored() -> Self {
        let mut n = MODE.load(Ordering::Relaxed);
        if n == 0 {
            n = match mode_file().and_then(|f| std::fs::read_to_string(f).ok()) {
                Some(s) if s.trim() == "clarify" => 2,
                _ => 1,
            };
            MODE.store(n, Ordering::Relaxed);
        }
        if n == 2 { Self::Clarify } else { Self::Fast }
    }

    /// Remembers this choice on this device.
    pub fn remember(self) {
        MODE.store(if self == Self::Clarify { 2 } else { 1 }, Ordering::Relaxed);
        if let Some(f) = mode_file() {
            let _ = std::fs::write(f, self.as_str());
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Clarify => "clarify",
        }
    }
}

/// What processing an item did: the deck says so and moves on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Became a task, a project or a target's thing: what was made.
    Filed(String),
    /// Became a note.
    Noted,
    /// Put off; what it says ("Back in a week").
    Later(String),
    /// Marked handled.
    Done,
    /// Deleted.
    Deleted,
    /// Left for another pass.
    Skipped,
}

impl Outcome {
    /// What the deck says it did.
    #[must_use]
    pub fn said(&self) -> Option<String> {
        match self {
            Self::Filed(s) | Self::Later(s) => Some(s.clone()),
            Self::Noted => Some("Saved as a note".to_owned()),
            Self::Done => Some("Marked done".to_owned()),
            Self::Deleted => Some("Deleted".to_owned()),
            Self::Skipped => None,
        }
    }
}

/// Human-friendly interval label ("tomorrow", "3 days", "2 weeks").
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
pub fn fmt_interval(days: i64) -> String {
    if days <= 0 {
        "today".to_owned()
    } else if days == 1 {
        "tomorrow".to_owned()
    } else if days < 7 {
        format!("{days} days")
    } else if days < 30 {
        format!("{} weeks", (days as f64 / 7.0).round() as i64)
    } else if days < 365 {
        format!("{} months", (days as f64 / 30.0).round() as i64)
    } else {
        format!("{:.1} years", days as f64 / 365.0)
    }
}

/// How far Snooze puts it off: Soon, Maybe, Someday (Task's spaced
/// resurfacing, as fixed steps: architect's inbox keeps no review count).
pub const LATER_DAYS: [i64; 3] = [1, 7, 30];

/// Context names offered as one-tap chips: the person's own, then the
/// ones a day plan looks for, without repeats, each with its `@`.
#[must_use]
pub fn context_suggestions(existing: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |raw: &str| {
        let c = format!(
            "@{}",
            raw.trim().trim_start_matches('@').to_ascii_lowercase()
        );
        if c.len() > 1 && !out.contains(&c) {
            out.push(c);
        }
    };
    for c in existing {
        push(c);
    }
    for c in crate::vocabulary::DEFAULT_CONTEXTS {
        push(c);
    }
    out
}

/// A capture's whole text: its title line and its body.
#[must_use]
pub fn whole_text(item: &Item) -> String {
    if item.title.is_empty() || item.body.starts_with(&item.title) {
        item.body.clone()
    } else if item.body.is_empty() {
        item.title.clone()
    } else {
        format!("{}\n{}", item.title, item.body)
    }
}

/// A rewritten note's body: your words, then the capture's own beneath
/// as a quote, the paper trail back to the thought.
#[must_use]
pub fn rewritten_note(body: &str, original: &str) -> String {
    let quoted: Vec<String> = original.trim().lines().map(|l| format!("> {l}")).collect();
    let body = body.trim();
    if body.is_empty() {
        quoted.join("\n")
    } else {
        format!("{body}\n\n{}", quoted.join("\n"))
    }
}

/// Where a capture came from, at a glance: an icon and a word (the
/// navigator's Inbox rows, the card's meta line).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Email,
    Watch,
    Agent,
    /// A highlight from something read.
    Literature,
    /// The capture window, over another app.
    Window,
    /// Your own passing thought: the default.
    Thought,
}

impl Origin {
    #[must_use]
    pub fn of(item: &Item) -> Self {
        match item.source.as_str() {
            "email" | "mail" => Self::Email,
            "watch" => Self::Watch,
            "agent" | "mcp" | "llm" => Self::Agent,
            "readwise" | "kindle" => Self::Literature,
            "window" => Self::Window,
            _ => Self::Thought,
        }
    }

    /// The word for it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Email => "Email",
            Self::Watch => "Watch",
            Self::Agent => "Agent",
            Self::Literature => "Reading",
            Self::Window => "Window",
            Self::Thought => "Thought",
        }
    }

    /// Its Lucide icon.
    #[must_use]
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Email => "mail",
            Self::Watch => "watch",
            Self::Agent => "bot",
            Self::Literature => "book-open",
            Self::Window => "app-window",
            Self::Thought => "lightbulb",
        }
    }
}

/// The icon tile for where a capture came from (Task's inbox rows).
#[component]
pub fn OriginTile(origin: Origin) -> Element {
    let l = use_look();
    let (bg, ink) = match origin {
        Origin::Email => (wash(&l.info, 15), l.info.clone()),
        Origin::Agent => (wash(&l.brand, 15), l.brand.clone()),
        Origin::Literature => (wash(&l.warn, 15), l.warn.clone()),
        _ => (wash(&l.ink, 7), l.ink_2.clone()),
    };
    rsx! {
        span {
            style: "display: flex; width: 32px; height: 32px; flex-shrink: 0; align-items: center; justify-content: center; \
                    border-radius: 10px; background: {bg}; color: {ink};",
            "aria-label": "{origin.label()}",
            match origin {
                Origin::Email => rsx! { lc::Mail { size: 16 } },
                Origin::Watch => rsx! { lc::Watch { size: 16 } },
                Origin::Agent => rsx! { lc::Bot { size: 16 } },
                Origin::Literature => rsx! { lc::BookOpen { size: 16 } },
                Origin::Window => rsx! { lc::AppWindow { size: 16 } },
                Origin::Thought => rsx! { lc::Lightbulb { size: 16 } },
            }
        }
    }
}

/// How long ago, the way a list says it: "now", "5m", "3h", "Yesterday",
/// "4d", then the date.
#[must_use]
pub fn ago(created: &str, now: chrono::DateTime<chrono::FixedOffset>) -> String {
    let Ok(then) = chrono::DateTime::parse_from_rfc3339(created) else {
        return created.get(..10).unwrap_or(created).to_owned();
    };
    let secs = (now - then).num_seconds().max(0);
    if secs < 60 {
        "now".to_owned()
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else if secs < 2 * 86_400 {
        "Yesterday".to_owned()
    } else if secs < 7 * 86_400 {
        format!("{}d", secs / 86_400)
    } else {
        then.format("%b %-d").to_string()
    }
}

// ── the card ────────────────────────────────────────────────────────

/// Where the card is in its questions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// Fast: every outcome. Clarify: "is it actionable?".
    Decide,
    /// Clarify, actionable: what kind of next step.
    Actionable,
    /// Clarify, not actionable: a note, someday, or gone.
    NotActionable,
    /// Choosing when it comes back.
    Later,
    /// Shaping the task.
    Shape(Shape),
    /// Rewriting it as a note in your own words, under a parent.
    Rewrite,
    /// Doing it now: the task exists and its clock is running.
    Doing,
}

/// What kind of task the shaping form makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Task,
    Waiting,
}

/// A terminal decision the card carries out itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Decision {
    Note,
    Later(usize),
    Done,
    Delete,
    DoNow,
    Project,
}

/// What a key on the card picked.
#[derive(Clone, Copy)]
enum Pick {
    Step(Step),
    Decide(Decision),
    Skip,
}

/// What a key does on step `step` in `mode` (never while typing in a
/// form): Fast `d` do now, `t`/`p` task, `s`/`l` later, `n` write a note,
/// `r` reference, `c` done, `x` delete; Clarify `y` / `n`, then the tiles'
/// letters; space skips.
fn key_pick(step: Step, mode: Mode, c: &str, skippable: bool) -> Option<Pick> {
    Some(match (step, mode, c) {
        (Step::Decide, _, " ") if skippable => Pick::Skip,
        (Step::Decide, Mode::Fast, "d") | (Step::Actionable, _, "d") => {
            Pick::Decide(Decision::DoNow)
        }
        (Step::Decide, Mode::Fast, "t" | "p") | (Step::Actionable, _, "t") => {
            Pick::Step(Step::Shape(Shape::Task))
        }
        (Step::Decide, Mode::Fast, "s" | "l") => Pick::Step(Step::Later),
        (Step::Decide, Mode::Fast, "n") => Pick::Step(Step::Rewrite),
        (Step::Decide, Mode::Fast, "r") | (Step::NotActionable, _, "n") => {
            Pick::Decide(Decision::Note)
        }
        (Step::Decide, Mode::Fast, "c") => Pick::Decide(Decision::Done),
        (Step::Decide, Mode::Fast, "x") | (Step::NotActionable, _, "x") => {
            Pick::Decide(Decision::Delete)
        }
        (Step::Decide, Mode::Clarify, "y") => Pick::Step(Step::Actionable),
        (Step::Decide, Mode::Clarify, "n") => Pick::Step(Step::NotActionable),
        (Step::Actionable, _, "w") => Pick::Step(Step::Shape(Shape::Waiting)),
        (Step::Actionable, _, "p") => Pick::Decide(Decision::Project),
        (Step::NotActionable, _, "s") => Pick::Decide(Decision::Later(2)),
        _ => return None,
    })
}

/// One inbox item, processed. `on_done` fires once with what happened;
/// the card does every write itself, through the capture's actions.
#[allow(clippy::too_many_lines)]
#[component]
pub fn ProcessCard(
    capture: CaptureUi,
    item: Item,
    mode: Signal<Mode>,
    on_done: EventHandler<Outcome>,
    /// Offer "Skip" (the deck does; a single fresh capture doesn't).
    #[props(default)]
    skippable: bool,
) -> Element {
    let l = use_look();
    let phone = style::phone();
    let cap = capture.capturer();
    // Callers key the card by the item's path, so each item starts here.
    let mut step = use_signal(|| Step::Decide);
    // The task "do it now" made (its path, its title), while its clock
    // runs on this card, and when it started.
    let mut doing = use_signal(|| None::<(String, String)>);
    let mut started = use_signal(|| None::<chrono::DateTime<Local>>);
    let mut said = use_signal(|| None::<String>);
    let has_task = cap.has("TASKS_CAPTURE");
    let has_project = cap.has("PROJECTS_CREATE");

    let origin = Origin::of(&item);
    let date = item.created.get(..10).unwrap_or(&item.created).to_owned();
    let source = item.source.clone();
    let body = whole_text(&item);

    // "Do it now", finished or paused: the clock stops, the task records
    // how it ended, and the card moves on.
    let finish = {
        let cap = cap.clone();
        use_callback(move |status: &'static str| {
            let Some((path, title)) = doing.peek().clone() else {
                return;
            };
            let cap = cap.clone();
            spawn(async move {
                let app = ActionCaller::app();
                if cap.has("TIME_STOP") {
                    let _ = cap.call(&app, "TIME_STOP", json!({})).await;
                }
                let _ = cap
                    .call(
                        &app,
                        "TASKS_SET_STATUS",
                        json!({ "path": path, "status": status }),
                    )
                    .await;
                capture.changed();
                let what = if status == "done" { "Done" } else { "Paused" };
                on_done.call(Outcome::Filed(format!("{what}: {title}")));
            });
        })
    };

    // Every terminal decision, in one place.
    let decide = {
        let item = item.clone();
        let cap = cap;
        use_callback(move |d: Decision| {
            let item = item.clone();
            let cap = cap.clone();
            spawn(async move {
                let app = ActionCaller::app();
                let path = item.path.clone();
                let out: Result<Option<Outcome>, String> = async {
                    Ok(Some(match d {
                        Decision::Note => {
                            cap.process(&app, &path, target::NOTE, None, &Map::new())
                                .await?;
                            Outcome::Noted
                        }
                        Decision::Later(i) => {
                            let days = LATER_DAYS.get(i).copied().unwrap_or(1);
                            let day = actions::day_after(days).format("%Y-%m-%d").to_string();
                            cap.defer(&app, &path, Some(&day)).await?;
                            let when = fmt_interval(days);
                            Outcome::Later(if when == "tomorrow" {
                                "Back tomorrow".to_owned()
                            } else {
                                format!("Back in {when}")
                            })
                        }
                        Decision::Done => {
                            cap.done(&app, &path).await?;
                            Outcome::Done
                        }
                        Decision::Delete => {
                            cap.discard(&app, &path).await?;
                            Outcome::Deleted
                        }
                        Decision::DoNow => {
                            let mut chips = Map::new();
                            chips.insert("status".to_owned(), json!("in-progress"));
                            let made = cap.process(&app, &path, "task", None, &chips).await?;
                            let task = made
                                .get("path")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned();
                            let title = made
                                .get("title")
                                .and_then(Value::as_str)
                                .map_or_else(|| item.label().to_owned(), str::to_owned);
                            if cap.has("TIME_START") {
                                let _ = cap
                                    .call(
                                        &app,
                                        "TIME_START",
                                        json!({ "task": task, "description": title }),
                                    )
                                    .await;
                            }
                            // The card stays on it, clock running, until it
                            // is done or paused.
                            doing.set(Some((task, title)));
                            started.set(Some(Local::now()));
                            step.set(Step::Doing);
                            return Ok(None);
                        }
                        Decision::Project => {
                            let title = item.label().to_owned();
                            cap.call(&app, "PROJECTS_CREATE", json!({ "title": title }))
                                .await?;
                            cap.discard(&app, &path).await?;
                            Outcome::Filed(format!("New project: {title}"))
                        }
                    }))
                }
                .await;
                match out {
                    Ok(Some(o)) => {
                        capture.changed();
                        on_done.call(o);
                    }
                    Ok(None) => capture.changed(),
                    Err(e) => said.set(Some(e)),
                }
            });
        })
    };

    let on_key = move |e: KeyboardEvent| {
        let Key::Character(c) = e.key() else { return };
        if let Some(p) = key_pick(step(), mode(), c.as_str(), skippable) {
            e.prevent_default();
            match p {
                Pick::Step(s) => step.set(s),
                Pick::Decide(d) => decide.call(d),
                Pick::Skip => on_done.call(Outcome::Skipped),
            }
        }
    };

    let card_pad = if phone { 20 } else { 24 };
    let body_size = if phone { 18 } else { 20 };
    let quiet_btn = format!(
        "{RESET} display: flex; height: 40px; align-items: center; gap: 6px; border-radius: 999px; padding: 0 12px; \
         font-size: 14px; color: {}; --cap-h: {}; --cap-a: {};",
        l.ink_2,
        wash(&l.hover, 50),
        l.hover
    );
    let danger_btn = format!(
        "{RESET} display: flex; height: 40px; align-items: center; gap: 6px; border-radius: 999px; padding: 0 12px; \
         font-size: 14px; color: {}; --cap-h: {}; --cap-a: {};",
        l.danger,
        wash(&l.danger, 10),
        wash(&l.danger, 10)
    );
    let grid2 = "display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px;";
    let grid3 = "display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 8px;";

    rsx! {
        CaptureCss {}
        div {
            style: "display: flex; min-height: 0; width: 100%; flex-direction: column; gap: 16px; outline: none; color: {l.ink}; font-family: {l.font};",
            tabindex: "0",
            autofocus: true,
            onkeydown: on_key,
            // ── the capture ──
            article { style: "display: flex; flex-direction: column; gap: 12px; border-radius: 16px; border: 1px solid {l.rule}; \
                              background: {l.sheet}; padding: {card_pad}px; box-shadow: {l.lift};",
                div { style: "display: flex; flex-wrap: wrap; align-items: center; column-gap: 8px; row-gap: 4px; font-size: 11px; \
                              font-weight: 600; text-transform: uppercase; letter-spacing: 0.16em; color: {l.ink_2};",
                    span { "{origin.label()}" }
                    span { style: "color: {l.rule};", "·" }
                    span { "{date}" }
                    if !source.is_empty() && source != "ui" && source != "cli" {
                        span { style: "color: {l.rule};", "·" }
                        span { "via {source}" }
                    }
                }
                p { id: "inbox-title",
                    style: "margin: 0; white-space: pre-wrap; overflow-wrap: break-word; font-size: {body_size}px; font-weight: 500; \
                            line-height: 1.625; color: {l.ink};",
                    "{body}"
                }
            }
            if let Some(e) = said() {
                div { style: "font-size: 13px; color: {l.danger};", "{e}" }
            }

            // ── the decision ──
            match step() {
                Step::Decide => match mode() {
                    // Read it, then decide in one tap: do it now, file it as
                    // the next step of a project, snooze it, or make it
                    // knowledge. Or let it go.
                    Mode::Fast => rsx! {
                        div { style: "{grid2}",
                            if has_task {
                                Choice { icon: rsx! { lc::Play { size: 18 } }, label: "Do it now", hint: "Two minutes — the clock starts", primary: true,
                                    on_pick: move |()| decide.call(Decision::DoNow) }
                                Choice { icon: rsx! { lc::FolderKanban { size: 18 } }, label: "Add to a project", hint: "Its next step, with a date",
                                    on_pick: move |()| step.set(Step::Shape(Shape::Task)) }
                            }
                            Choice { icon: rsx! { lc::FileText { size: 18 } }, label: "Write a note", hint: "In your own words, under a parent", primary: !has_task,
                                on_pick: move |()| step.set(Step::Rewrite) }
                            Choice { icon: rsx! { lc::Clock { size: 18 } }, label: "Snooze", hint: "Read it again later",
                                on_pick: move |()| step.set(Step::Later) }
                        }
                        div { style: "display: flex; flex-wrap: wrap; align-items: center; gap: 4px;",
                            button { r#type: "button", class: "cap-h cap-a", style: "{quiet_btn}",
                                onclick: move |_| decide.call(Decision::Note),
                                lc::Archive { size: 15 }
                                "Keep as reference"
                            }
                            if has_project {
                                button { r#type: "button", class: "cap-h cap-a", style: "{quiet_btn}",
                                    onclick: move |_| decide.call(Decision::Project),
                                    lc::FolderKanban { size: 15 }
                                    "It's a project"
                                }
                            }
                            button { r#type: "button", class: "cap-h cap-a", style: "{quiet_btn}",
                                onclick: move |_| decide.call(Decision::Done),
                                lc::Check { size: 15 }
                                "Already done"
                            }
                            button { r#type: "button", class: "cap-h cap-a", style: "{danger_btn}",
                                onclick: move |_| decide.call(Decision::Delete),
                                lc::Trash { size: 15 }
                                "Delete"
                            }
                            if skippable {
                                button { r#type: "button", class: "cap-h cap-a", style: "{quiet_btn} margin-left: auto;",
                                    onclick: move |_| on_done.call(Outcome::Skipped),
                                    "Skip"
                                    lc::ArrowRight { size: 15 }
                                }
                            }
                        }
                    },
                    Mode::Clarify => rsx! {
                        Question { text: "Is this actionable?" }
                        div { style: "{grid2}",
                            Choice { icon: rsx! { lc::Check { size: 18 } }, label: "Yes", hint: "There's something to do", primary: true,
                                on_pick: move |()| step.set(Step::Actionable) }
                            Choice { icon: rsx! { lc::X { size: 18 } }, label: "No", hint: "Reference, an idea, or nothing",
                                on_pick: move |()| step.set(Step::NotActionable) }
                        }
                        if skippable {
                            button {
                                r#type: "button",
                                class: "cap-h",
                                style: "{RESET} align-self: center; border-radius: 6px; padding: 8px 12px; font-size: 12px; color: {l.ink_2}; \
                                        --cap-h: {l.hover}; --cap-hc: {l.ink};",
                                onclick: move |_| on_done.call(Outcome::Skipped),
                                "Skip for now"
                            }
                        }
                    },
                },
                Step::Actionable => rsx! {
                    Question { text: "What's the next step?", on_back: move |()| step.set(Step::Decide) }
                    div { style: "{grid2}",
                        if has_task {
                            Choice { icon: rsx! { lc::Play { size: 18 } }, label: "Do it now", hint: "Two minutes or less — the clock starts", primary: true,
                                on_pick: move |()| decide.call(Decision::DoNow) }
                            Choice { icon: rsx! { lc::ListTodo { size: 18 } }, label: "Make it a task", hint: "Give it a context and a date",
                                on_pick: move |()| step.set(Step::Shape(Shape::Task)) }
                            Choice { icon: rsx! { lc::Hourglass { size: 18 } }, label: "Waiting on someone", hint: "Track it until they come back",
                                on_pick: move |()| step.set(Step::Shape(Shape::Waiting)) }
                        }
                        if has_project {
                            Choice { icon: rsx! { lc::FolderKanban { size: 18 } }, label: "It's a project", hint: "More than one step",
                                on_pick: move |()| decide.call(Decision::Project) }
                        }
                        if !has_task && !has_project {
                            Choice { icon: rsx! { lc::FileText { size: 18 } }, label: "Write a note", hint: "Nothing here makes tasks yet: keep it as a note",
                                primary: true, on_pick: move |()| step.set(Step::Rewrite) }
                        }
                    }
                },
                Step::NotActionable => rsx! {
                    Question { text: "Keep it?", on_back: move |()| step.set(Step::Decide) }
                    div { style: "{grid3}",
                        Choice { icon: rsx! { lc::FileText { size: 18 } }, label: "Note", hint: "Reference", primary: true,
                            on_pick: move |()| decide.call(Decision::Note) }
                        Choice { icon: rsx! { lc::Archive { size: 18 } }, label: "Someday", hint: "Maybe, later",
                            on_pick: move |()| decide.call(Decision::Later(2)) }
                        Choice { icon: rsx! { lc::Trash { size: 18 } }, label: "Delete", hint: "Let it go", danger: true,
                            on_pick: move |()| decide.call(Decision::Delete) }
                    }
                },
                Step::Doing => {
                    let title = doing.read().as_ref().map(|(_, t)| t.clone()).unwrap_or_default();
                    rsx! {
                        div { style: "display: flex; flex-direction: column; gap: 12px; border-radius: 16px; border: 1px solid {wash(&l.brand, 30)}; \
                                      background: {wash(&l.brand, 10)}; padding: 16px;",
                            div { style: "display: flex; align-items: center; gap: 12px;",
                                span { style: "position: relative; display: flex; width: 12px; height: 12px;",
                                    span { class: "cap-ping", style: "position: absolute; display: inline-flex; width: 100%; height: 100%; border-radius: 999px; background: {wash(&l.brand, 60)};" }
                                    span { style: "position: relative; display: inline-flex; width: 12px; height: 12px; border-radius: 999px; background: {l.brand};" }
                                }
                                span { style: "min-width: 0; flex: 1; {TRUNCATE} font-size: 16px; font-weight: 500; color: {l.ink};", "Doing: {title}" }
                                ElapsedClock { since: started() }
                            }
                            p { style: "margin: 0; font-size: 13px; color: {l.ink_2};",
                                "It's a task now, and the clock is running. Finish it, or pause and it stays on your list."
                            }
                            div { style: "{grid2}",
                                button {
                                    r#type: "button",
                                    style: "{RESET} display: flex; height: 48px; align-items: center; justify-content: center; gap: 8px; border-radius: 12px; \
                                            background: {l.brand}; color: {l.brand_ink}; font-size: 15px; font-weight: 600;",
                                    onclick: move |_| finish.call("done"),
                                    lc::Check { size: 18 }
                                    "Done"
                                }
                                button {
                                    r#type: "button",
                                    class: "cap-a",
                                    style: "{RESET} display: flex; height: 48px; align-items: center; justify-content: center; gap: 8px; border-radius: 12px; \
                                            background: {wash(&l.ink, 8)}; color: {l.ink}; font-size: 15px; font-weight: 500; --cap-a: {wash(&l.ink, 12)};",
                                    onclick: move |_| finish.call("open"),
                                    lc::Hourglass { size: 18 }
                                    "Pause"
                                }
                            }
                        }
                    }
                }
                Step::Rewrite => rsx! {
                    NoteRewrite {
                        key: "{item.path}",
                        capture,
                        item: item,
                        on_back: move |()| step.set(Step::Decide),
                        on_done,
                    }
                },
                Step::Later => {
                    let [soon, mid, far] = LATER_DAYS.map(fmt_interval);
                    rsx! {
                        Question { text: "When should it come back?", on_back: move |()| step.set(Step::Decide) }
                        div { style: "{grid3}",
                            Choice { label: "Soon", hint: "{soon}", primary: true,
                                on_pick: move |()| decide.call(Decision::Later(0)) }
                            Choice { label: "Maybe", hint: "{mid}",
                                on_pick: move |()| decide.call(Decision::Later(1)) }
                            Choice { label: "Someday", hint: "{far}",
                                on_pick: move |()| decide.call(Decision::Later(2)) }
                        }
                    }
                }
                Step::Shape(shape) => rsx! {
                    TaskShape {
                        key: "{item.path}",
                        capture,
                        item: item,
                        waiting: shape == Shape::Waiting,
                        on_back: move |()| step.set(Step::Decide),
                        on_done,
                    }
                },
            }
        }
    }
}

/// Time since `since`, ticking each second: the clock "do it now"
/// started.
#[component]
pub fn ElapsedClock(since: Option<chrono::DateTime<Local>>) -> Element {
    let l = use_look();
    let mut now = use_signal(Local::now);
    use_future(move || async move {
        loop {
            futures_timer::Delay::new(std::time::Duration::from_secs(1)).await;
            now.set(Local::now());
        }
    });
    let secs = since.map_or(0, |s| (now() - s).num_seconds().max(0));
    let text = if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    };
    rsx! { span { style: "flex-shrink: 0; font-family: monospace; font-size: 15px; font-variant-numeric: tabular-nums; color: {l.brand};", "{text}" } }
}

/// The question a clarify step asks, with a way back to the last one.
#[component]
fn Question(text: &'static str, on_back: Option<EventHandler<()>>) -> Element {
    let l = use_look();
    rsx! {
        div { style: "display: flex; align-items: center; gap: 8px;",
            if let Some(back) = on_back {
                button {
                    r#type: "button",
                    class: "cap-h",
                    style: "{RESET} border-radius: 6px; padding: 4px 8px; font-size: 12px; color: {l.ink_2}; --cap-h: {l.hover}; --cap-hc: {l.ink};",
                    onclick: move |_| back.call(()),
                    "‹ Back"
                }
            }
            h2 { style: "margin: 0; font-size: 16px; font-weight: 600; color: {l.ink};", "{text}" }
        }
    }
}

/// One outcome, as a tile big enough to hit with a thumb.
#[component]
fn Choice(
    label: &'static str,
    hint: String,
    on_pick: EventHandler<()>,
    #[props(default)] icon: Option<Element>,
    #[props(default)] primary: bool,
    #[props(default)] danger: bool,
) -> Element {
    let l = use_look();
    let tone = tone(&l, primary, danger);
    let icon_ink = if danger {
        l.danger.clone()
    } else if primary {
        l.brand.clone()
    } else {
        l.ink_2.clone()
    };
    rsx! {
        button {
            r#type: "button",
            class: "cap-h cap-a",
            style: "{RESET} display: flex; min-height: 72px; flex-direction: column; align-items: flex-start; justify-content: center; gap: 4px; \
                    border-radius: 12px; padding: 10px 12px; text-align: left; {tone}",
            onclick: move |_| on_pick.call(()),
            span { style: "display: flex; align-items: center; gap: 8px; font-size: 14px; font-weight: 500; color: {l.ink};",
                if let Some(icon) = icon {
                    span { style: "display: flex; color: {icon_ink};", {icon} }
                }
                "{label}"
            }
            span { style: "font-size: 12px; line-height: 1.375; color: {l.ink_2};", "{hint}" }
        }
    }
}

/// A tile's border, fill and washes.
fn tone(l: &Look, primary: bool, danger: bool) -> String {
    if primary {
        format!(
            "border: 1px solid {}; background: {}; --cap-h: {}; --cap-a: {};",
            wash(&l.brand, 50),
            wash(&l.brand, 10),
            wash(&l.brand, 20),
            wash(&l.brand, 20)
        )
    } else if danger {
        format!(
            "border: 1px solid {}; background: {}; --cap-h: {}; --cap-a: {};",
            wash(&l.rule, 70),
            wash(&l.sheet, 60),
            wash(&l.danger, 10),
            wash(&l.danger, 10)
        )
    } else {
        format!(
            "border: 1px solid {}; background: {}; --cap-h: {}; --cap-a: {};",
            wash(&l.rule, 70),
            wash(&l.sheet, 60),
            wash(&l.hover, 40),
            wash(&l.hover, 50)
        )
    }
}

/// A capture rewritten as a note in your own words: the step that turns
/// a fleeting thought into knowledge. The title is the note's name (a
/// sentence that says what it means); the body is yours; the original
/// words are kept beneath as a quote; and the note is placed under one
/// parent note (its `up`), so related thinking sits together.
#[allow(clippy::too_many_lines)]
#[component]
fn NoteRewrite(
    capture: CaptureUi,
    item: Item,
    on_back: EventHandler<()>,
    on_done: EventHandler<Outcome>,
) -> Element {
    let l = use_look();
    let first = item.label().to_owned();
    let rest = if item.title.is_empty() {
        item.body.lines().skip(1).collect::<Vec<_>>().join("\n")
    } else {
        item.body.clone()
    };
    let mut title = use_signal(|| first.clone());
    let mut body = use_signal(|| rest.trim().to_owned());
    let mut parent = use_signal(|| None::<String>);
    let mut query = use_signal(String::new);
    let mut said = use_signal(|| None::<String>);
    let pages = use_resource(move || {
        let cap = capture.capturer();
        async move {
            cap.call(&ActionCaller::app(), "VAULT_LIST", json!({}))
                .await
                .unwrap_or_default()
        }
    });
    let q = query().to_lowercase();
    let folder = capture.capturer().config().folder.clone();
    #[allow(clippy::needless_collect)]
    let matches: Vec<String> = pages
        .read()
        .as_ref()
        .and_then(|v| v.get("notes").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|n| {
            let path = n.get("path").and_then(Value::as_str)?;
            if !crate::vocabulary::is_note(path) || path.starts_with(&format!("{folder}/")) {
                return None;
            }
            Some(
                n.get("title")
                    .and_then(Value::as_str)
                    .map_or_else(|| crate::vocabulary::bare_name(path), str::to_owned),
            )
        })
        .filter(|t| q.is_empty() || t.to_lowercase().contains(&q))
        .take(6)
        .collect();
    let original = whole_text(&item);
    let can_save = !title.read().trim().is_empty();
    let field = format!(
        "{FIELD_RESET} width: 100%; border-radius: 12px; background: {}; padding: 0 14px; font-size: 16px; color: {}; --cap-ph: {};",
        wash(&l.ink, 6),
        l.ink,
        l.ink_3
    );
    let save = {
        let item = item;
        let original = original.clone();
        move |_| {
            let t = title.peek().trim().to_owned();
            if t.is_empty() {
                return;
            }
            let md = rewritten_note(&body.peek(), &original);
            let up = parent.peek().clone();
            let cap = capture.capturer();
            let path = item.path.clone();
            spawn(async move {
                let app = ActionCaller::app();
                let notes = cap.config().notes.trim_matches('/').to_owned();
                let name = inbox::file_name(&t);
                let at = if notes.is_empty() {
                    name
                } else {
                    format!("{notes}/{name}")
                };
                let mut props = Map::new();
                if let Some(p) = up {
                    props.insert("up".to_owned(), json!(format!("[[{p}]]")));
                }
                props.insert("created".to_owned(), json!(Local::now().to_rfc3339()));
                let out = async {
                    cap.call(
                        &app,
                        "VAULT_CREATE",
                        json!({ "path": at, "body": md, "properties": props }),
                    )
                    .await?;
                    cap.discard(&app, &path).await
                }
                .await;
                match out {
                    Ok(_) => {
                        capture.changed();
                        on_done.call(Outcome::Noted);
                    }
                    Err(e) => said.set(Some(e)),
                }
            });
        }
    };

    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 12px;",
            div { style: "display: flex; align-items: center; gap: 8px;",
                button {
                    r#type: "button",
                    class: "cap-a",
                    style: "{RESET} display: flex; width: 36px; height: 36px; align-items: center; justify-content: center; border-radius: 999px; \
                            color: {l.ink_2}; --cap-a: {l.hover};",
                    "aria-label": "Back",
                    onclick: move |_| on_back.call(()),
                    lc::ArrowLeft { size: 16 }
                }
                span { style: "font-size: 15px; font-weight: 600; color: {l.ink};", "Write it in your own words" }
            }
            input {
                class: "cap-in",
                style: "{field} height: 48px; font-weight: 500;",
                placeholder: "What it means, as a sentence",
                "aria-label": "Note title",
                value: "{title}",
                oninput: move |e| title.set(e.value()),
            }
            textarea {
                class: "cap-in",
                style: "{field} min-height: 128px; padding-top: 12px; padding-bottom: 12px; line-height: 24px; resize: vertical;",
                placeholder: "Explain it so you'd understand it in a year.",
                "aria-label": "Note body",
                value: "{body}",
                oninput: move |e| body.set(e.value()),
                {body.peek().clone()}
            }
            // ── the parent ──
            div { style: "display: flex; flex-direction: column; gap: 8px; border-radius: 16px; background: {wash(&l.ink, 4)}; padding: 12px;",
                span { style: "font-size: 13px; font-weight: 500; color: {l.ink_2};", "Place it under" }
                if let Some(ptitle) = parent() {
                    div { style: "display: flex; align-items: center; gap: 8px;",
                        span { style: "display: flex; height: 36px; min-width: 0; align-items: center; gap: 6px; border-radius: 999px; \
                                       background: {wash(&l.brand, 15)}; padding: 0 12px; font-size: 14px; font-weight: 500; color: {l.brand};",
                            lc::FileText { size: 14 }
                            span { style: "{TRUNCATE}", "{ptitle}" }
                        }
                        button {
                            r#type: "button",
                            style: "{RESET} font-size: 13px; color: {l.ink_2};",
                            onclick: move |_| parent.set(None),
                            "Change"
                        }
                    }
                } else {
                    input {
                        class: "cap-in",
                        style: "{field} height: 40px; font-size: 15px;",
                        placeholder: "Find a related note…",
                        "aria-label": "Parent note",
                        value: "{query}",
                        oninput: move |e| query.set(e.value()),
                    }
                    div { style: "display: flex; flex-direction: column;",
                        for ptitle in matches {
                            button {
                                key: "{ptitle}",
                                r#type: "button",
                                class: "cap-a cap-h",
                                style: "{RESET} display: flex; min-height: 40px; align-items: center; gap: 8px; border-radius: 8px; padding: 0 8px; \
                                        text-align: left; font-size: 15px; color: {l.ink}; --cap-a: {l.hover}; --cap-h: {wash(&l.hover, 50)};",
                                onclick: {
                                    let p = ptitle;
                                    move |_| parent.set(Some(p.clone()))
                                },
                                span { style: "display: flex; flex-shrink: 0; color: {l.ink_2};", lc::FileText { size: 14 } }
                                span { style: "{TRUNCATE}", "{ptitle}" }
                            }
                        }
                    }
                }
            }
            // ── the original, kept ──
            div { style: "border-radius: 12px; border-left: 2px solid {l.rule}; padding: 4px 12px; font-size: 14px; line-height: 20px; color: {l.ink_2};",
                span { style: "display: block; padding-bottom: 4px; font-size: 11px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.12em;", "Original capture" }
                span { style: "white-space: pre-wrap;", "{original}" }
            }
            if let Some(e) = said() {
                div { style: "font-size: 13px; color: {l.danger};", "{e}" }
            }
            button {
                r#type: "button",
                class: "cap-btn",
                style: "{RESET} display: flex; height: 48px; align-items: center; justify-content: center; gap: 8px; border-radius: 12px; \
                        background: {l.brand}; color: {l.brand_ink}; font-size: 15px; font-weight: 600;",
                disabled: !can_save,
                onclick: save,
                lc::Check { size: 18 }
                "Save note"
            }
        }
    }
}

// ── shaping the task ────────────────────────────────────────────────

/// The task a capture becomes, pre-filled from what it said and editable
/// before it's made: title, project, contexts, due date, estimate, and,
/// for "waiting on someone", who.
#[allow(clippy::too_many_lines)]
#[component]
pub fn TaskShape(
    capture: CaptureUi,
    item: Item,
    /// A Waiting task ("waiting on someone").
    #[props(default)]
    waiting: bool,
    on_back: EventHandler<()>,
    on_done: EventHandler<Outcome>,
) -> Element {
    let l = use_look();
    let vocab = use_resource(move || {
        let cap = capture.capturer();
        async move { cap.vocabulary(&ActionCaller::app()).await }
    });
    let v = vocab.read().clone().unwrap_or_default();
    let projects = v.projects.clone();
    // What the capture reads as, through quick add's parser.
    let (parsed, first, rest) = use_hook(|| {
        let text = whole_text(&item);
        let first = text.lines().next().unwrap_or_default().to_owned();
        let rest = text.lines().skip(1).collect::<Vec<_>>().join("\n");
        (Capturer::parse(&first, &[]), first, rest)
    });
    let mut title = use_signal(|| {
        if parsed.title.trim().is_empty() {
            first.trim().chars().take(120).collect()
        } else {
            parsed.title.clone()
        }
    });
    let mut details = use_signal(|| rest.trim().to_owned());
    let mut project = use_signal(|| {
        parsed
            .projects
            .first()
            .map(|p| crate::vocabulary::bare_name(p))
    });
    let mut contexts = use_signal(|| parsed.contexts.clone());
    let mut due = use_signal(|| {
        parsed
            .due
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default()
    });
    let mut estimate = use_signal(|| None::<u32>);
    let mut who = use_signal(String::new);
    let mut said = use_signal(|| None::<String>);

    let chips = {
        let mut all = context_suggestions(&v.contexts);
        for c in contexts.read().iter() {
            let c = format!("@{}", c.trim_start_matches('@'));
            if !all.contains(&c) {
                all.insert(0, c);
            }
        }
        all
    };
    let today = actions::today();
    let to_saturday = (5 - i64::from(today.weekday().num_days_from_monday())).rem_euclid(7);
    let dates = [
        ("Today", today.to_string()),
        ("Tomorrow", (today + Duration::days(1)).to_string()),
        ("Weekend", (today + Duration::days(to_saturday)).to_string()),
        ("Next week", (today + Duration::days(7)).to_string()),
    ];

    let save = {
        let path = item.path;
        let tags = parsed.tags.clone();
        let priority = parsed.priority.clone();
        let repeat = parsed.repeat;
        move |_| {
            let t = title.read().trim().to_owned();
            if t.is_empty() {
                return;
            }
            let mut body = details.read().trim().to_owned();
            let mut chips = Map::new();
            if waiting {
                chips.insert("status".to_owned(), json!("waiting"));
                let w = who.read().trim().to_owned();
                if !w.is_empty() {
                    body = if body.is_empty() {
                        format!("Waiting on {w}.")
                    } else {
                        format!("Waiting on {w}.\n\n{body}")
                    };
                }
            }
            chips.insert(
                crate::parse::keys::CONTEXTS.to_owned(),
                json!(contexts.read().clone()),
            );
            let d = due.read().clone();
            chips.insert(
                crate::parse::keys::DUE.to_owned(),
                if d.is_empty() { Value::Null } else { json!(d) },
            );
            chips.insert(
                crate::parse::keys::PROJECTS.to_owned(),
                project
                    .read()
                    .as_ref()
                    .map_or(Value::Null, |p| json!([format!("[[{p}]]")])),
            );
            if !tags.is_empty() {
                chips.insert(crate::parse::keys::TAGS.to_owned(), json!(tags));
            }
            if let Some(p) = &priority {
                chips.insert(crate::parse::keys::PRIORITY.to_owned(), json!(p));
            }
            if let Some(r) = &repeat {
                chips.insert(crate::parse::keys::RECURRENCE.to_owned(), json!(r.rule));
                chips.insert(
                    crate::parse::keys::RECURRENCE_ANCHOR.to_owned(),
                    json!(r.anchor),
                );
            }
            if let Some(m) = *estimate.read() {
                chips.insert("time_estimate".to_owned(), json!(m));
            }
            let cap = capture.capturer();
            let path = path.clone();
            spawn(async move {
                let out = cap
                    .process_with(
                        &ActionCaller::app(),
                        &path,
                        "task",
                        Some(&t),
                        Some(&body),
                        &chips,
                    )
                    .await;
                match out {
                    Ok(_) => {
                        capture.changed();
                        on_done.call(Outcome::Filed(if waiting {
                            format!("Waiting: {t}")
                        } else {
                            format!("Task added: {t}")
                        }));
                    }
                    Err(e) => said.set(Some(e)),
                }
            });
        }
    };

    let chip_on = format!(
        "{RESET} border-radius: 999px; border: 1px solid {}; background: {}; padding: 6px 12px; font-size: 14px; color: {};",
        wash(&l.brand, 60),
        wash(&l.brand, 15),
        l.ink
    );
    let chip_off = format!(
        "{RESET} border-radius: 999px; border: 1px solid {}; padding: 6px 12px; font-size: 14px; color: {}; --cap-hc: {}; --cap-h: transparent;",
        wash(&l.rule, 70),
        l.ink_2,
        l.ink
    );
    let field = format!(
        "width: 100%; box-sizing: border-box; border-radius: 8px; border: 1px solid {}; background: {}; padding: 8px 12px; \
         font-size: 14px; color: {}; font-family: inherit; outline: none; --cap-ph: {};",
        l.rule, l.page, l.ink, l.ink_3
    );
    let heading = if waiting {
        "Waiting on someone"
    } else {
        "Make it a task"
    };
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 16px;",
            Question { text: heading, on_back: move |()| on_back.call(()) }
            input {
                r#type: "text",
                "aria-label": "Title",
                class: "cap-in",
                style: "{FIELD_RESET} width: 100%; border-bottom: 1px solid {l.rule}; padding: 4px 0; font-size: 18px; \
                        font-weight: 500; line-height: 1.375; color: {l.ink};",
                value: "{title}",
                oninput: move |e| title.set(e.value()),
            }
            if waiting {
                input {
                    r#type: "text",
                    class: "cap-in",
                    style: "{field}",
                    placeholder: "Who or what are you waiting on?",
                    value: "{who}",
                    oninput: move |e| who.set(e.value()),
                }
            }
            // Task's select, as chips: Blitz draws no select.
            if !projects.is_empty() {
                Field { label: "Project",
                    div { style: "display: flex; flex-wrap: wrap; gap: 6px;",
                        for name in std::iter::once(String::new()).chain(projects.iter().cloned()) {
                            {
                                let on = project().unwrap_or_default() == name;
                                let pick = name.clone();
                                rsx! {
                                    button {
                                        key: "{name}",
                                        r#type: "button",
                                        class: "cap-h",
                                        style: if on { "{chip_on}" } else { "{chip_off}" },
                                        onclick: move |_| project.set(Some(pick.clone()).filter(|p| !p.is_empty())),
                                        if name.is_empty() { "No project" } else { "{name}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Field { label: "Context",
                div { style: "display: flex; flex-wrap: wrap; gap: 6px;",
                    for c in chips.into_iter() {
                        {
                            let on = contexts.read().iter().any(|x| x.trim_start_matches('@').eq_ignore_ascii_case(c.trim_start_matches('@')));
                            let c2 = c.clone();
                            rsx! {
                                button {
                                    key: "{c}",
                                    r#type: "button",
                                    class: "cap-h",
                                    style: if on { "{chip_on}" } else { "{chip_off}" },
                                    "aria-pressed": "{on}",
                                    onclick: move |_| {
                                        let name = c2.trim_start_matches('@').to_ascii_lowercase();
                                        let mut list = contexts();
                                        if let Some(i) = list.iter().position(|x| x.trim_start_matches('@').eq_ignore_ascii_case(&name)) {
                                            list.remove(i);
                                        } else {
                                            list.push(format!("@{name}"));
                                        }
                                        contexts.set(list);
                                    },
                                    "{c}"
                                }
                            }
                        }
                    }
                }
            }
            Field { label: "Due",
                div { style: "display: flex; flex-wrap: wrap; gap: 6px;",
                    for (label , value) in dates.into_iter() {
                        {
                            let on = due() == value;
                            rsx! {
                                button {
                                    key: "{label}",
                                    r#type: "button",
                                    class: "cap-h",
                                    style: if on { "{chip_on}" } else { "{chip_off}" },
                                    onclick: move |_| due.set(if on { String::new() } else { value.clone() }),
                                    "{label}"
                                }
                            }
                        }
                    }
                }
            }
            Field { label: "How long",
                div { style: "display: flex; flex-wrap: wrap; gap: 6px;",
                    for (label , mins) in [("5m", 5u32), ("15m", 15), ("30m", 30), ("1h", 60), ("2h", 120)] {
                        {
                            let on = estimate() == Some(mins);
                            rsx! {
                                button {
                                    key: "{label}",
                                    r#type: "button",
                                    class: "cap-h",
                                    style: if on { "{chip_on}" } else { "{chip_off}" },
                                    onclick: move |_| estimate.set(if on { None } else { Some(mins) }),
                                    "{label}"
                                }
                            }
                        }
                    }
                }
            }
            Field { label: "Notes",
                // Blitz seeds a textarea from its text, not its `value`.
                textarea {
                    class: "cap-in",
                    style: "{field} min-height: 96px; resize: vertical;",
                    value: "{details}",
                    oninput: move |e| details.set(e.value()),
                    {details.peek().clone()}
                }
            }
            if let Some(e) = said() {
                div { style: "font-size: 13px; color: {l.danger};", "{e}" }
            }
            button {
                r#type: "button",
                class: "cap-btn",
                style: "{RESET} display: flex; min-height: 48px; align-items: center; justify-content: center; gap: 8px; border-radius: 12px; \
                        background: {l.brand}; color: {l.brand_ink}; padding: 0 16px; font-size: 14px; font-weight: 600;",
                disabled: title.read().trim().is_empty(),
                onclick: save,
                if waiting { "Save as waiting" } else { "Add task" }
            }
        }
    }
}

#[component]
fn Field(label: &'static str, children: Element) -> Element {
    let l = use_look();
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 6px;",
            span { style: "font-size: 12px; font-weight: 500; color: {l.ink_2};", "{label}" }
            {children}
        }
    }
}

// ── the deck ────────────────────────────────────────────────────────

/// Fast or Clarify: the person's choice, remembered on this device.
#[component]
pub fn ModeSwitch(mode: Signal<Mode>) -> Element {
    let l = use_look();
    let ground = wash(&l.ink, 6);
    let seg = move |m: Mode, label: &'static str| {
        let on = mode() == m;
        let look = if on {
            format!(
                "background: {}; color: {}; font-weight: 500; box-shadow: 0 1px 2px rgba(0,0,0,0.12);",
                l.page, l.ink
            )
        } else {
            format!(
                "background: transparent; color: {}; --cap-hc: {}; --cap-h: transparent;",
                l.ink_2, l.ink
            )
        };
        rsx! {
            button {
                r#type: "button",
                class: "cap-h",
                id: "process-mode-{m.as_str()}",
                style: "{RESET} border-radius: 6px; padding: 6px 10px; font-size: 12px; {look}",
                "aria-pressed": "{on}",
                title: match m {
                    Mode::Fast => "Every outcome at once",
                    Mode::Clarify => "Ask: actionable? next step?",
                },
                onclick: move |_| {
                    let mut mode = mode;
                    mode.set(m);
                    m.remember();
                },
                "{label}"
            }
        }
    };
    rsx! {
        div { style: "display: inline-flex; flex-shrink: 0; align-items: center; gap: 2px; border-radius: 8px; background: {ground}; padding: 2px;",
            {seg(Mode::Fast, "Fast")}
            {seg(Mode::Clarify, "Clarify")}
        }
    }
}

/// The deck: the captures due, frozen into a queue when it opens, then
/// one [`ProcessCard`] at a time. Every outcome advances.
#[allow(clippy::too_many_lines)]
#[component]
pub fn ProcessDeck(
    capture: CaptureUi,
    on_exit: EventHandler<()>,
    /// The capture it opens on (the one tapped in a list).
    #[props(default)]
    start: Option<String>,
) -> Element {
    let l = use_look();
    let phone = style::phone();
    let mut cursor = use_signal(|| 0usize);
    let mode = use_signal(Mode::stored);
    let mut said = use_signal(|| None::<String>);
    // Read once: the queue holds still while it is worked through.
    let items = use_resource(move || {
        let cap = capture.capturer();
        async move { cap.inbox(&ActionCaller::app(), false).await }
    });
    let mut placed = use_signal(|| false);
    let loaded = items.read().clone();
    let (list, failed): (Vec<Item>, Option<String>) = match loaded {
        Some(Ok(v)) => (v, None),
        Some(Err(e)) => (Vec::new(), Some(e)),
        None => {
            return rsx! {
                div { style: "padding: 24px; font-size: 13px; color: {l.ink_2};", "Reading the inbox…" }
            };
        }
    };
    if !*placed.peek() {
        placed.set(true);
        if let Some(at) = start
            .as_ref()
            .and_then(|p| list.iter().position(|i| &i.path == p))
        {
            cursor.set(at);
        }
    }
    let total = list.len();
    let idx = cursor();
    let safe = style::insets();
    let header_pad = if phone {
        format!("padding: {}px 16px 12px;", safe.top.max(12.0))
    } else {
        "padding: 12px 32px;".to_owned()
    };
    let body_pad = if phone {
        format!("padding: 0 16px {}px;", 24.0 + safe.bottom)
    } else {
        "padding: 0 32px 24px;".to_owned()
    };
    let shown = (idx + 1).min(total);
    let width = pct(idx, total);
    rsx! {
        CaptureCss {}
        div {
            style: "display: flex; height: 100%; min-height: 0; flex: 1; flex-direction: column; color: {l.ink}; font-family: {l.font};",
            tabindex: "-1",
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Named(NamedKey::Escape) {
                    on_exit.call(());
                }
            },
            // ── leave · progress · mode ──
            header { style: "display: flex; align-items: center; gap: 12px; {header_pad}",
                button {
                    r#type: "button",
                    class: "cap-h",
                    style: "{RESET} display: flex; width: 40px; height: 40px; flex-shrink: 0; align-items: center; justify-content: center; \
                            border-radius: 8px; color: {l.ink_2}; --cap-h: {l.hover}; --cap-hc: {l.ink};",
                    title: "Leave (Esc)",
                    "aria-label": "Leave processing",
                    onclick: move |_| on_exit.call(()),
                    lc::X { size: 18 }
                }
                div { style: "display: flex; min-width: 0; flex: 1; flex-direction: column; gap: 4px;",
                    if total > 0 {
                        div { style: "height: 4px; overflow: hidden; border-radius: 999px; background: {wash(&l.ink, 10)};",
                            div { style: "height: 100%; border-radius: 999px; background: {l.brand}; width: {width}%;" }
                        }
                        span { style: "font-family: monospace; font-size: 11px; color: {l.ink_2};", "{shown} of {total}" }
                    } else {
                        span { style: "font-size: 13px; font-weight: 600; color: {l.ink};", "Inbox" }
                    }
                }
                ModeSwitch { mode }
            }
            if let Some(e) = failed {
                div { style: "padding: 0 16px; font-size: 13px; color: {l.danger};", "{e}" }
            }
            if idx >= total {
                // ── done ──
                div { style: "display: flex; flex: 1; flex-direction: column; align-items: center; justify-content: center; gap: 16px; \
                              padding: 24px; text-align: center;",
                    div { style: "display: flex; width: 56px; height: 56px; align-items: center; justify-content: center; border-radius: 16px; \
                                  background: {wash(&l.brand, 15)}; color: {l.brand};",
                        lc::Check { size: 28 }
                    }
                    h2 { style: "margin: 0; font-size: 20px; font-weight: 600;", if total == 0 { "Inbox zero" } else { "Inbox processed" } }
                    p { style: "margin: 0; max-width: 28rem; font-size: 14px; color: {l.ink_2};",
                        "Everything you captured has a place. What you put off comes back when it's due."
                    }
                    if let Some(s) = said() {
                        p { style: "margin: 0; font-size: 12px; color: {l.ink_2};", "{s}" }
                    }
                    button {
                        r#type: "button",
                        style: "{RESET} border-radius: 8px; background: {l.brand}; color: {l.brand_ink}; padding: 8px 16px; font-size: 14px; font-weight: 600;",
                        onclick: move |_| on_exit.call(()),
                        "Done"
                    }
                }
            } else if let Some(item) = list.get(idx).cloned() {
                // ── the card ──
                div { style: "display: flex; min-height: 0; flex: 1; justify-content: center; overflow-y: auto; {body_pad}",
                    div { style: "display: flex; width: 100%; max-width: 36rem; flex-direction: column; gap: 8px; padding: 8px 0;",
                        ProcessCard {
                            key: "{item.path}",
                            capture,
                            item,
                            mode,
                            skippable: true,
                            on_done: move |o: Outcome| {
                                said.set(o.said());
                                cursor += 1;
                            },
                        }
                        if let Some(s) = said() {
                            div { style: "font-size: 12px; color: {l.ink_2};", "{s}" }
                        }
                    }
                }
            }
        }
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn pct(idx: usize, total: usize) -> i64 {
    ((idx as f64) / (total.max(1) as f64) * 100.0).round() as i64
}

/// Processing one fresh capture on the spot ("Process now"): a sheet
/// filling a phone, a centred card on a desk. Closing undecided leaves it
/// in the inbox, as it was written.
#[component]
pub fn ProcessNowSheet(capture: CaptureUi, item: Item, on_close: EventHandler<()>) -> Element {
    let l = use_look();
    let phone = style::phone();
    let mode = use_signal(Mode::stored);
    let frame = if phone {
        format!(
            "position: absolute; top: 0; left: 0; right: 0; bottom: 0; z-index: {}; display: flex; background: {};",
            architect_shell::layer::SHEET,
            l.page
        )
    } else {
        format!(
            "position: absolute; top: 0; left: 0; right: 0; bottom: 0; z-index: {}; display: flex; align-items: flex-start; \
             justify-content: center; background: {}; padding: 8vh 24px 24px; box-sizing: border-box;",
            architect_shell::layer::SHEET,
            l.scrim
        )
    };
    let card = if phone {
        format!(
            "display: flex; flex: 1; flex-direction: column; background: {};",
            l.page
        )
    } else {
        format!(
            "display: flex; width: 576px; max-width: 100%; max-height: 84vh; flex-direction: column; overflow: hidden; \
             border-radius: 16px; border: 1px solid {}; background: {}; box-shadow: {};",
            l.rule, l.page, l.float
        )
    };
    let safe = style::insets();
    let top = if phone {
        format!("padding: {}px 16px 8px;", safe.top.max(12.0))
    } else {
        "padding: 12px 16px 8px;".to_owned()
    };
    rsx! {
        CaptureCss {}
        div { style: "{frame}", onclick: move |_| on_close.call(()),
            div {
                "data-testid": "process-now",
                role: "dialog",
                "aria-label": "Process this capture",
                style: "{card}",
                onclick: move |e| e.stop_propagation(),
                header { style: "display: flex; align-items: center; gap: 12px; {top}",
                    button {
                        r#type: "button",
                        class: "cap-h",
                        style: "{RESET} display: flex; width: 40px; height: 40px; flex-shrink: 0; align-items: center; justify-content: center; \
                                border-radius: 8px; color: {l.ink_2}; --cap-h: {l.hover}; --cap-hc: {l.ink};",
                        "aria-label": "Keep in inbox",
                        title: "Keep in inbox",
                        onclick: move |_| on_close.call(()),
                        lc::X { size: 18 }
                    }
                    span { style: "flex: 1; font-size: 14px; font-weight: 600; color: {l.ink};", "Process" }
                    ModeSwitch { mode }
                }
                div { style: "min-height: 0; flex: 1; overflow-y: auto; padding: 0 16px 24px;",
                    ProcessCard {
                        key: "{item.path}",
                        capture,
                        item,
                        mode,
                        on_done: move |_| on_close.call(()),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn intervals_read_like_speech() {
        assert_eq!(fmt_interval(0), "today");
        assert_eq!(fmt_interval(1), "tomorrow");
        assert_eq!(fmt_interval(3), "3 days");
        assert_eq!(fmt_interval(14), "2 weeks");
        assert_eq!(fmt_interval(60), "2 months");
    }

    #[test]
    fn context_chips_lead_with_the_persons_own_and_never_repeat() {
        let chips = context_suggestions(&["@Studio".into(), "@errands".into()]);
        assert_eq!(chips[0], "@studio");
        assert_eq!(chips.iter().filter(|c| *c == "@errands").count(), 1);
        assert!(chips.contains(&"@chores".to_owned()));
    }

    #[test]
    fn fast_and_clarify_answer_different_keys() {
        let k = |step, mode, c| key_pick(step, mode, c, true).is_some();
        assert!(k(Step::Decide, Mode::Fast, "d"));
        assert!(!k(Step::Decide, Mode::Clarify, "d"), "Clarify asks first");
        assert!(k(Step::Decide, Mode::Clarify, "y"));
        assert!(k(Step::Actionable, Mode::Clarify, "w"));
        assert!(k(Step::Decide, Mode::Fast, " "));
        assert!(key_pick(Step::Decide, Mode::Fast, " ", false).is_none());
    }

    #[test]
    fn a_rewritten_note_keeps_the_capture_beneath() {
        assert_eq!(
            rewritten_note("Mine.", "a line\nanother"),
            "Mine.\n\n> a line\n> another"
        );
        assert_eq!(rewritten_note("", "a line"), "> a line");
    }

    #[test]
    fn where_it_came_from() {
        let item = |s: &str| Item {
            source: s.into(),
            ..Item::default()
        };
        assert_eq!(Origin::of(&item("mcp")), Origin::Agent);
        assert_eq!(Origin::of(&item("ui")), Origin::Thought);
        assert_eq!(Origin::of(&item("window")).label(), "Window");
    }

    #[test]
    fn ages_read_like_a_list() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-07T12:00:00+00:00").unwrap();
        assert_eq!(ago("2026-11-07T11:59:30+00:00", now), "now");
        assert_eq!(ago("2026-11-07T11:00:00+00:00", now), "1h");
        assert_eq!(ago("2026-11-06T10:00:00+00:00", now), "Yesterday");
        assert_eq!(ago("2026-10-01T10:00:00+00:00", now), "Oct 1");
    }

    #[test]
    fn a_capture_s_whole_text_is_its_title_and_body() {
        let i = Item {
            title: "Call the venue".into(),
            body: "about the date".into(),
            ..Item::default()
        };
        assert_eq!(whole_text(&i), "Call the venue\nabout the date");
        let j = Item {
            body: "just a body".into(),
            ..Item::default()
        };
        assert_eq!(whole_text(&j), "just a body");
    }
}
