//! The composer: capturing, Linear's way, with a title read as it is
//! typed (#445).
//!
//! Ported from Task (`task:features/task/task-ui/src/views/composer.rs`),
//! its look and behaviour kept, onto architect's contracts: what it makes
//! goes through `CAPTURE_ADD` (the inbox, or a `[[capture_targets]]`
//! target), and what it offers to pick comes from the vault
//! ([`crate::vocabulary`]).
//!
//! A title, a description, and the property chips under them (Status,
//! Priority, Assignee, Project, Context, and Labels, a date and a repeat
//! once set: [`super::chips`]), a More menu for the rest (labels, a due
//! date, a repeat, sub-tasks made right there, a link), and Create with
//! a "Create more" switch.
//!
//! What Linear doesn't do and this does: the title is read as you type.
//!
//! - `@na…` opens the people list under the title, filtered by what
//!   follows the `@`; ↑ / ↓ / Enter (or Tab) pick, Esc dismisses. Picking
//!   removes the `@…` from the title and sets the Assignee chip. The
//!   same list offers contexts (`@errands`, `@home`), which file the
//!   capture under where it gets done instead of who.
//! - `#la…` does the same for labels (the vault's tags; and offers to
//!   create one).
//! - `[[Project]]`, `tomorrow`, `fri`, `every week`, `!high` are read by
//!   the quick-add parser ([`crate::parse`]) and shown as chips the
//!   moment they're typed; a chip set by hand wins over the text.
//!
//! A capture is a note into the inbox until it is given something a task
//! has (a person, a date, a priority, a project, a label, a repeat, a
//! sub-task), and then it is a task (the `task` target, when the Tasks
//! extension is there). `project: …` makes a project. The kind at the
//! top is also a menu: any target can be picked by hand (an event, time
//! logged, a plain note).
//!
//! On a phone it is a full-screen sheet whose text gets the whole page:
//! the properties ride the keyboard as one scrolling bar, with the
//! create button beside it. On a desk, a card with the chips in a row.
//! ⌘↩ creates.

// Days, a cursor over a short list, a line height: small counts.
#![allow(clippy::arithmetic_side_effects)]

use architect::action::ActionCaller;
use architect_shell::lucide::LucideIcon;
use dioxus::prelude::*;
use lucide_dioxus as lc;
use serde_json::{Map, Value, json};

use super::chips::{
    AssigneeChip, Hotkey, LabelsChip, PersonDot, PriorityChip, ProjectChip, StatusChip, StatusIcon,
    label_color,
};
use super::picker::{PickOption, PropertyPicker, filtered};
use super::style::{self, CaptureCss, FIELD_RESET, RESET, TRUNCATE, use_look, wash};
use super::{CaptureUi, facet_of, repeat_label};
use crate::actions::{self, AddArgs, Capturer};
use crate::parse::keys as k;
use crate::target::Target;

/// The `@` / `#` word being typed at the end of the title, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trigger {
    Person(String),
    Label(String),
}

/// What the end of `title` is asking for: the last word, when it
/// starts with `@` or `#` and the title doesn't end in a space.
#[must_use]
#[allow(clippy::option_if_let_else)] // Task's, as it was
pub fn trigger_at_end(title: &str) -> Option<Trigger> {
    if title.ends_with(char::is_whitespace) {
        return None;
    }
    let word = title.rsplit(char::is_whitespace).next()?;
    match word.strip_prefix('@') {
        Some(q) => Some(Trigger::Person(q.to_owned())),
        None => word.strip_prefix('#').map(|q| Trigger::Label(q.to_owned())),
    }
}

/// `title` without its last word (the trigger just resolved), trimmed
/// of the space before it.
#[must_use]
#[allow(clippy::option_if_let_else)] // Task's, as it was
pub fn drop_last_word(title: &str) -> String {
    match title.rfind(char::is_whitespace) {
        Some(i) => format!("{} ", title.get(..i).unwrap_or_default().trim_end()),
        None => String::new(),
    }
}

/// What a capture becomes. Nobody picks Note or Task: a capture is a
/// note until it is given something a task has, and then it is a task.
/// A project is the one deliberate choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaptureKind {
    /// A fleeting thought into the inbox, processed later.
    #[default]
    Note,
    /// Something to do: it has a who, a when, a where or a priority.
    Task,
    Project,
}

/// What the text says it is: `project: …` (or `new project …`) is a
/// project; a line that files or schedules itself (`[[Project]]`,
/// `@context`, a date, a repeat, `!priority`) is a task; anything else
/// is a note.
#[must_use]
pub fn kind_from_text(text: &str) -> CaptureKind {
    let first = text.trim().lines().next().unwrap_or("").trim();
    let lower = first.to_lowercase();
    if lower.starts_with("project:") || lower.starts_with("new project ") {
        return CaptureKind::Project;
    }
    let props = crate::parse::parse(first, actions::today()).properties();
    if crate::inbox::suggests_task(&props) {
        CaptureKind::Task
    } else {
        CaptureKind::Note
    }
}

/// What the surface is making right now.
///
/// Opened as a capture (and `switchable`), it stays a capture until it is `given` something a task
/// has by hand, or its text says so ([`kind_from_text`]); opened as a new
/// task it is one.
#[must_use]
pub fn resolved_kind(
    opened_as: CaptureKind,
    switchable: bool,
    given: bool,
    text: &str,
) -> CaptureKind {
    if !switchable {
        return opened_as;
    }
    if opened_as == CaptureKind::Task || given || kind_from_text(text) == CaptureKind::Task {
        CaptureKind::Task
    } else {
        CaptureKind::Note
    }
}

/// Whether the surface offers "Process now": only while what is typed
/// is still a plain capture (a task or a project is already decided).
#[must_use]
pub fn offers_process_now(kind: CaptureKind) -> bool {
    kind == CaptureKind::Note
}

/// A project's name from what was typed: the first line, without a
/// leading `project:` / `new project`.
#[must_use]
pub fn project_title(text: &str) -> String {
    let first = text.trim().lines().next().unwrap_or("").trim();
    let lower = first.to_lowercase();
    let rest = if lower.starts_with("project:") {
        first.get("project:".len()..).unwrap_or_default()
    } else if lower.starts_with("new project ") {
        first.get("new project ".len()..).unwrap_or_default()
    } else {
        first
    };
    rest.trim().to_owned()
}

/// The days the Due picker offers, one tap each: `(YYYY-MM-DD, label)`,
/// and "No date" to clear it.
fn due_options() -> Vec<PickOption> {
    use chrono::Datelike as _;
    let today = actions::today();
    let days_to_monday = (7 - today.weekday().num_days_from_monday()) % 7;
    let next_monday = actions::day_after(i64::from(if days_to_monday == 0 {
        7
    } else {
        days_to_monday
    }));
    let days_to_sat = (5 + 7 - today.weekday().num_days_from_monday()) % 7;
    let weekend = actions::day_after(i64::from(days_to_sat));
    let opt = |d: chrono::NaiveDate, label: &str| {
        PickOption::new(
            d.to_string(),
            format!("{label} · {}", d.format("%a %-d %b")),
        )
    };
    vec![
        opt(today, "Today"),
        opt(actions::day_after(1), "Tomorrow"),
        opt(weekend, "This weekend"),
        opt(next_monday, "Next week"),
        opt(actions::day_after(14), "In two weeks"),
        PickOption::new("", "No date"),
    ]
}

/// The repeats the More menu offers, as RRULE bodies.
const REPEATS: &[(&str, &str)] = &[
    ("Every day", "FREQ=DAILY"),
    ("Every weekday", "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"),
    ("Every week", "FREQ=WEEKLY"),
    ("Every month", "FREQ=MONTHLY"),
];

/// A context in the `@` list: its option id is this prefix and the name.
const CONTEXT_ID: &str = "ctx:";
/// A context the `@` list offers to make.
const CONTEXT_NEW: &str = "ctx+:";

/// The context an `@` suggestion names, if it is one (not a person).
fn context_of(id: &str) -> Option<&str> {
    id.strip_prefix(CONTEXT_NEW)
        .or_else(|| id.strip_prefix(CONTEXT_ID))
}

/// What the composer has been given by hand: the chips that beat the
/// text, and what they say about the kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hand {
    pub status: String,
    pub priority: Option<String>,
    pub assignee: Option<String>,
    pub project: Option<String>,
    pub labels: Vec<String>,
    pub contexts: Vec<String>,
    pub due: Option<String>,
    pub recurrence: Option<String>,
}

impl Hand {
    /// Whether anything a task has was given.
    #[must_use]
    pub fn given(&self, subs: bool) -> bool {
        (!self.status.is_empty() && self.status != "open")
            || self.priority.is_some()
            || self.assignee.is_some()
            || self.project.is_some()
            || !self.labels.is_empty()
            || !self.contexts.is_empty()
            || self.due.is_some()
            || self.recurrence.is_some()
            || subs
    }

    /// The properties set by hand, as a capture's: they beat the text's
    /// (`crate::parse::merge`). `labels` and `contexts` are all that
    /// show (the text's and the hand's together), so the text's are kept.
    #[must_use]
    pub fn properties(&self, labels: &[String], contexts: &[String]) -> Map<String, Value> {
        let mut m = Map::new();
        if !self.status.is_empty() && self.status != "open" {
            m.insert("status".to_owned(), json!(self.status));
        }
        if let Some(p) = &self.priority {
            m.insert(k::PRIORITY.to_owned(), json!(p));
        }
        if let Some(a) = &self.assignee {
            m.insert("assignees".to_owned(), json!([format!("[[{a}]]")]));
        }
        if let Some(p) = &self.project {
            m.insert(k::PROJECTS.to_owned(), json!([format!("[[{p}]]")]));
        }
        if !labels.is_empty() {
            m.insert(k::TAGS.to_owned(), json!(labels));
        }
        if !contexts.is_empty() {
            let at: Vec<String> = contexts.iter().map(|c| format!("@{c}")).collect();
            m.insert(k::CONTEXTS.to_owned(), json!(at));
        }
        if let Some(d) = &self.due {
            m.insert(k::DUE.to_owned(), json!(d));
        }
        if let Some(r) = &self.recurrence {
            m.insert(k::RECURRENCE.to_owned(), json!(r));
            m.insert(k::RECURRENCE_ANCHOR.to_owned(), json!("scheduled"));
        }
        m
    }
}

/// The capture's words: the title, then the details after a blank line.
fn words(title: &str, details: &str) -> String {
    let (t, d) = (title.trim(), details.trim());
    if d.is_empty() {
        t.to_owned()
    } else {
        format!("{t}\n\n{d}")
    }
}

fn toggle(list: &mut Vec<String>, name: &str) {
    if let Some(i) = list.iter().position(|x| x.eq_ignore_ascii_case(name)) {
        list.remove(i);
    } else {
        list.push(name.to_owned());
    }
}

fn add_once(list: &mut Vec<String>, name: &str) {
    if !list.iter().any(|x| x.eq_ignore_ascii_case(name)) {
        list.push(name.to_owned());
    }
}

/// The composer: a title read as it is typed, its chips, and Create.
#[allow(clippy::too_many_lines)]
#[component]
pub fn Composer(
    capture: CaptureUi,
    text: String,
    /// A target picked before it opened (`CAPTURE_OPEN {target}`): `task`
    /// opens it as a new task.
    target: Option<String>,
    /// `ui`, `window`: written as the capture's `source`.
    source: &'static str,
    on_close: EventHandler<()>,
    /// On a plain capture: decide it now instead of filing it. Gets the
    /// whole text.
    #[props(default)]
    on_process: Option<EventHandler<String>>,
) -> Element {
    let l = use_look();
    let phone = style::phone();
    let cap: Capturer = capture.capturer();
    let mut title = use_signal(|| text.clone());
    let mut description = use_signal(String::new);
    let mut hand = use_signal(|| Hand {
        status: "open".to_owned(),
        ..Hand::default()
    });
    let mut context_open = use_signal(|| false);
    let mut subs = use_signal(Vec::<String>::new);
    let mut sub_draft = use_signal(String::new);
    let mut adding_sub = use_signal(|| false);
    let mut repeat_open = use_signal(|| false);
    let mut details_open = use_signal(|| false);
    let details_focus = use_signal(|| false);
    let mut due_open = use_signal(|| false);
    let mut more_open = use_signal(|| false);
    let mut kind_open = use_signal(|| false);
    let mut show_labels = use_signal(|| false);
    let mut link_draft = use_signal(|| None::<String>);
    let mut create_more = use_signal(|| false);
    let mut cursor = use_signal(|| 0usize);
    let mut busy = use_signal(|| false);
    let mut said = use_signal(|| None::<String>);
    // A target picked by hand at the top, over what the text says.
    let mut into = use_signal(|| target.clone().filter(|t| t != "task"));
    // A project typed into the Project picker that does not exist yet:
    // made along with the task.
    let mut new_project = use_signal(|| None::<String>);
    let mut bar_key = use_signal(|| None::<Hotkey>);

    let targets = use_resource(move || {
        let cap = capture.capturer();
        async move { cap.targets(&ActionCaller::app()).await }
    });
    let vocab = use_resource(move || {
        let cap = capture.capturer();
        async move { cap.vocabulary(&ActionCaller::app()).await }
    });
    let all: Vec<Target> = targets
        .read()
        .clone()
        .unwrap_or_else(|| vec![Target::note()]);
    let has_task = all.iter().any(|t| t.id == "task");
    let v = vocab.read().clone().unwrap_or_default();
    let people = v.people.clone();
    let projects = v.projects.clone();
    let known_labels = v.labels.clone();
    let known_contexts = v.context_choices();
    let can_project = cap.has("PROJECTS_CREATE");
    let can_timer = cap.has("TIME_START");

    let opened_as = if target.as_deref() == Some("task") {
        CaptureKind::Task
    } else {
        CaptureKind::Note
    };
    let switchable = opened_as == CaptureKind::Note;
    let h = hand.read().clone();
    let given = h.given(!subs.read().is_empty()) || new_project.read().is_some();
    let picked = into.read().clone();
    let kind = resolved_kind(opened_as, switchable, given, &title.read());
    let picked_target = picked
        .as_ref()
        .and_then(|id| all.iter().find(|t| &t.id == id).cloned());

    // What the title reads as right now (for the parsed chips).
    let parsed = Capturer::parse(&title.read(), &all);
    let trigger = trigger_at_end(&title.read());
    let people_opts: Vec<PickOption> = people
        .iter()
        .map(|n| {
            PickOption::new(n.clone(), n.clone())
                .icon(rsx! { PersonDot { name: n.clone(), size: 16 } })
        })
        .collect();
    let label_opts: Vec<PickOption> = known_labels
        .iter()
        .map(|t| PickOption::new(t.clone(), t.clone()).dot(label_color(t)))
        .collect();
    let context_opts: Vec<PickOption> = known_contexts
        .iter()
        .map(|c| {
            PickOption::new(format!("{CONTEXT_ID}{c}"), format!("@{c}"))
                .icon(rsx! { lc::DoorOpen { size: 15 } })
        })
        .collect();
    let suggestions: Vec<PickOption> = match &trigger {
        Some(Trigger::Person(q)) => {
            let mut hits: Vec<PickOption> = filtered(&people_opts, q)
                .into_iter()
                .map(|(_, o)| o)
                .take(4)
                .collect();
            hits.extend(
                filtered(&context_opts, q)
                    .into_iter()
                    .map(|(_, o)| o)
                    .take(4),
            );
            let known = known_contexts.iter().any(|c| c.eq_ignore_ascii_case(q))
                || people.iter().any(|n| n.eq_ignore_ascii_case(q));
            if !q.is_empty() && !known {
                hits.push(
                    PickOption::new(format!("{CONTEXT_NEW}{q}"), format!("New context “@{q}”"))
                        .icon(rsx! { lc::DoorOpen { size: 15 } }),
                );
            }
            hits
        }
        Some(Trigger::Label(q)) => {
            let mut hits: Vec<PickOption> = filtered(&label_opts, q)
                .into_iter()
                .map(|(_, o)| o)
                .take(6)
                .collect();
            if !q.is_empty() && !known_labels.iter().any(|t| t.eq_ignore_ascii_case(q)) {
                hits.push(
                    PickOption::new(format!("+{q}"), format!("Create “{q}”")).dot(label_color(q)),
                );
            }
            hits
        }
        None => Vec::new(),
    };
    let at = (*cursor.read()).min(suggestions.len().saturating_sub(1));

    let resolve = {
        let people = people.clone();
        move |opt: PickOption| {
            match trigger_at_end(&title.peek()) {
                Some(Trigger::Person(_)) => {
                    if let Some(name) = context_of(&opt.id) {
                        add_once(&mut hand.write().contexts, name);
                    } else if let Some(who) = people.iter().find(|n| **n == opt.id) {
                        hand.write().assignee = Some(who.clone());
                    }
                }
                Some(Trigger::Label(_)) => {
                    let name = opt.id.strip_prefix('+').unwrap_or(&opt.id).to_owned();
                    add_once(&mut hand.write().labels, &name);
                }
                None => {}
            }
            let next = drop_last_word(&title.peek());
            title.set(next);
            cursor.set(0);
        }
    };

    // The chips the title's text produced, shown until a hand-set chip
    // replaces them.
    let parsed_due = h
        .due
        .is_none()
        .then(|| parsed.due.map(|d| d.format("%Y-%m-%d").to_string()))
        .flatten();
    let parsed_project = h
        .project
        .is_none()
        .then(|| {
            parsed
                .projects
                .first()
                .map(|p| crate::vocabulary::bare_name(p))
        })
        .flatten();
    let parsed_repeat = h
        .recurrence
        .is_none()
        .then(|| parsed.repeat.as_ref().map(|r| r.rule.clone()))
        .flatten();
    // Labels chosen by hand, then any the title's `#tags` add.
    let chosen_labels: Vec<String> = {
        let mut out = h.labels.clone();
        for t in &parsed.tags {
            add_once(&mut out, t);
        }
        out
    };
    // Contexts chosen by hand, then any the title's `@contexts` add: an
    // `@word` that is not a person, and not the word still being typed.
    let chosen_contexts: Vec<String> = {
        let mut out = h.contexts.clone();
        let typing = match &trigger {
            Some(Trigger::Person(q)) => Some(q.as_str()),
            _ => None,
        };
        for t in &parsed.contexts {
            let t = t.trim_start_matches('@');
            if typing == Some(t) {
                continue;
            }
            let person = people.iter().any(|n| {
                n.split_whitespace()
                    .next()
                    .is_some_and(|f| f.eq_ignore_ascii_case(t))
            });
            if !person {
                add_once(&mut out, t);
            }
        }
        out
    };
    let mut context_choices = known_contexts;
    for c in &chosen_contexts {
        add_once(&mut context_choices, c);
    }
    let priority_now = h
        .priority
        .clone()
        .or_else(|| parsed.priority.clone().filter(|p| p != "normal"))
        .unwrap_or_else(|| "none".to_owned());
    let project_now = h.project.clone().or_else(|| {
        parsed_project.as_ref().map(|n| {
            projects
                .iter()
                .find(|t| t.eq_ignore_ascii_case(n))
                .cloned()
                .unwrap_or_else(|| n.clone())
        })
    });

    let create = {
        let cap = cap.clone();
        let chosen_labels = chosen_labels.clone();
        let chosen_contexts = chosen_contexts.clone();
        let picked_target = picked_target.clone();
        move || {
            if *busy.peek() {
                return;
            }
            let t = title.peek().trim().to_owned();
            if t.is_empty() {
                return;
            }
            let d = description.peek().trim().to_owned();
            let cap = cap.clone();
            let app = ActionCaller::app();
            let more = *create_more.peek();
            let hand_now = hand.peek().clone();
            let subs_now = subs.peek().clone();
            let pending = new_project.peek().clone();
            let props = hand_now.properties(&chosen_labels, &chosen_contexts);
            let picked_target = picked_target.clone();
            busy.set(true);
            spawn(async move {
                let out: Result<(), String> = async {
                    // A target picked by hand: the capture goes there.
                    if let Some(tg) = picked_target {
                        let args = AddArgs {
                            text: words(&t, &d),
                            properties: (!props.is_empty()).then(|| facet_of(&props)).flatten(),
                            target: Some(tg.id),
                            source: Some(source.to_owned()),
                        };
                        return cap.capture(&app, args).await.map(|_| ());
                    }
                    match kind {
                        CaptureKind::Note => {
                            let args = AddArgs {
                                text: words(&t, &d),
                                properties: None,
                                target: None,
                                source: Some(source.to_owned()),
                            };
                            cap.capture(&app, args).await.map(|_| ())
                        }
                        CaptureKind::Project => {
                            let made = cap
                                .call(
                                    &app,
                                    "PROJECTS_CREATE",
                                    json!({ "title": project_title(&t) }),
                                )
                                .await?;
                            if let (false, Some(path)) =
                                (d.is_empty(), made.get("path").and_then(Value::as_str))
                            {
                                let _ = cap
                                    .call(&app, "VAULT_APPEND", json!({ "path": path, "text": d }))
                                    .await;
                            }
                            Ok(())
                        }
                        CaptureKind::Task => {
                            let mut props = props;
                            // A project named in the picker that does not
                            // exist yet is made now, and the task filed
                            // under it by name.
                            if let (None, Some(name)) = (&hand_now.project, &pending) {
                                if can_project {
                                    let _ = cap
                                        .call(&app, "PROJECTS_CREATE", json!({ "title": name }))
                                        .await;
                                }
                                props
                                    .insert(k::PROJECTS.to_owned(), json!([format!("[[{name}]]")]));
                            }
                            // No Tasks extension: sub-tasks ride in the note.
                            let body = if has_task || subs_now.is_empty() {
                                d.clone()
                            } else {
                                let list: Vec<String> =
                                    subs_now.iter().map(|s| format!("- [ ] {s}")).collect();
                                words(&d, &list.join("\n"))
                            };
                            let args = AddArgs {
                                text: words(&t, &body),
                                properties: (!props.is_empty()).then(|| facet_of(&props)).flatten(),
                                target: has_task.then(|| "task".to_owned()),
                                source: Some(source.to_owned()),
                            };
                            let made = cap.capture(&app, args).await?;
                            if has_task && cap.has("TASKS_CREATE") {
                                let parent =
                                    made.get("path").and_then(Value::as_str).unwrap_or_default();
                                for s in &subs_now {
                                    cap.call(
                                        &app,
                                        "TASKS_CREATE",
                                        json!({ "title": s, "parent": parent }),
                                    )
                                    .await?;
                                }
                            }
                            Ok(())
                        }
                    }
                }
                .await;
                busy.set(false);
                match out {
                    Ok(()) => {
                        capture.changed();
                        if more {
                            title.set(String::new());
                            description.set(String::new());
                            subs.set(Vec::new());
                            said.set(None);
                        } else {
                            on_close.call(());
                        }
                    }
                    Err(e) => said.set(Some(e)),
                }
            });
        }
    };
    let mut create_key = create.clone();
    let mut create_btn = create.clone();
    let mut create_round = create.clone();
    let mut create_enter = create.clone();
    let (title_hint, detail_hint, action_label) = match (&picked_target, kind) {
        (Some(tg), _) => (
            "Get it out of your head…",
            "More, if there is more…",
            if tg.id == crate::target::NOTE {
                "Capture".to_owned()
            } else {
                format!("Create {}", tg.name.to_lowercase())
            },
        ),
        (None, CaptureKind::Note) => (
            "Get it out of your head…",
            "More, if there is more…",
            "Capture".to_owned(),
        ),
        (None, CaptureKind::Task) => ("Task title", "Add description…", "Create task".to_owned()),
        (None, CaptureKind::Project) => (
            "Project name",
            "What it's for…",
            "Create project".to_owned(),
        ),
    };
    let mut resolve_key = resolve.clone();
    let suggestions_key = suggestions.clone();
    let note_like = picked_target.is_some() || kind == CaptureKind::Note;

    let on_status = EventHandler::new(move |s: String| hand.write().status = s);
    let on_priority = EventHandler::new(move |p: String| hand.write().priority = Some(p));
    let on_assignee = EventHandler::new(move |id: Option<String>| hand.write().assignee = id);
    let on_project_pick = EventHandler::new(move |p: Option<String>| {
        new_project.set(None);
        hand.write().project = p;
    });
    // Type a name no project has and press Enter: it is made with the task.
    let on_project_create = can_project.then(|| {
        EventHandler::new(move |name: String| {
            hand.write().project = None;
            new_project.set(Some(name));
        })
    });
    let on_label = EventHandler::new(move |name: String| toggle(&mut hand.write().labels, &name));

    // ── how it looks ──
    let safe = style::insets();
    let header_pad = if phone {
        format!("padding: {}px 16px 4px;", safe.top.max(12.0))
    } else {
        "padding: 12px 16px 4px;".to_owned()
    };
    let (kind_label, kind_task) = match (&picked_target, kind) {
        (Some(tg), _) => (tg.name.clone(), true),
        (None, CaptureKind::Task) if switchable => ("Task".to_owned(), true),
        (None, CaptureKind::Task) => ("New task".to_owned(), true),
        (None, CaptureKind::Project) => ("New project".to_owned(), true),
        (None, CaptureKind::Note) => ("Capture".to_owned(), false),
    };
    let kind_style = if kind_task {
        format!("background: {}; color: {};", wash(&l.brand, 15), l.brand)
    } else {
        format!(
            "background: {}; color: {};",
            wash(&l.ink, 7),
            wash(&l.ink, 80)
        )
    };
    let close_size = if phone { 36 } else { 28 };
    let scroll_overflow = if !phone && !suggestions.is_empty() {
        "visible"
    } else {
        "auto"
    };
    let (title_size, title_lh) = if phone { (20, 28) } else { (18, 28) };
    let bar_outer = if phone {
        format!(
            "display: flex; align-items: center; gap: 8px; padding: 4px 8px {}px;",
            safe.bottom.max(8.0)
        )
    } else {
        format!(
            "display: block; border-top: 1px solid {}; padding: 10px 16px;",
            wash(&l.rule, 60)
        )
    };
    let bar_inner = match (
        phone,
        kind == CaptureKind::Project && picked_target.is_none(),
    ) {
        (true, false) => format!(
            "display: flex; height: 48px; min-width: 0; flex: 1; align-items: center; gap: 2px; overflow-x: auto; \
             border-radius: 999px; background: {}; padding: 0 4px;",
            wash(&l.ink, 7)
        ),
        (true, true) => "min-width: 0; flex: 1;".to_owned(),
        (false, false) => {
            "display: flex; flex-wrap: wrap; align-items: center; gap: 6px;".to_owned()
        }
        (false, true) => "display: none;".to_owned(),
    };
    let quiet = style::chip(&l, phone, true);
    let loud = style::chip(&l, phone, false);
    let empty = title.read().trim().is_empty();
    let field_ph = format!("--cap-ph: {};", l.ink_2);
    let row_h = if phone { 40 } else { 32 };
    let row_text = if phone { 14 } else { 13 };
    let narrow = if phone { "" } else { "max-width: 20rem;" };
    let hover_bg = l.hover.clone();
    let row_bg = move |on: bool| {
        if on {
            hover_bg.clone()
        } else {
            "transparent".to_owned()
        }
    };
    let shows_props = !(kind == CaptureKind::Project && picked_target.is_none());
    let sub_count = subs.read().len();

    rsx! {
        CaptureCss {}
        div {
            style: "position: relative; display: flex; flex-direction: column; flex: 1; min-height: 0; min-width: 0; width: 100%; \
                    color: {l.ink}; font-family: {l.font};",
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Named(NamedKey::Enter) && (e.modifiers().meta() || e.modifiers().ctrl()) {
                    e.prevent_default();
                    create_key();
                } else if e.key() == Key::Named(NamedKey::Escape) {
                    e.prevent_default();
                    on_close.call(());
                }
            },
            // ── what it is ──
            header { style: "display: flex; align-items: center; gap: 8px; {header_pad} font-size: 13px; color: {l.ink_2};",
                // What it is right now: a capture, until it is given
                // something a task has, then a task. A tap picks any
                // target by hand.
                button {
                    r#type: "button",
                    "data-testid": "composer-kind",
                    id: "composer-kind",
                    style: "{RESET} border-radius: 999px; padding: 2px 10px; font-size: 13px; font-weight: 600; {kind_style}",
                    title: "What it becomes",
                    onclick: move |_| kind_open.set(true),
                    "{kind_label}"
                }
                button {
                    r#type: "button",
                    class: "cap-h",
                    style: "{RESET} margin-left: auto; display: flex; width: {close_size}px; height: {close_size}px; align-items: center; \
                            justify-content: center; border-radius: 6px; color: {l.ink_2}; --cap-h: {l.hover}; --cap-hc: {l.ink};",
                    "aria-label": "Close",
                    onclick: move |_| on_close.call(()),
                    lc::X { size: 16 }
                }
            }
            div { style: "display: flex; min-height: 0; flex: 1; flex-direction: column; overflow-y: {scroll_overflow}; padding: 0 16px 8px;",
                // ── the title, read as you type ──
                div { style: "position: relative;",
                    // One line (Task's textarea never takes a newline): an
                    // input, which Blitz fills from `value`.
                    input {
                        r#type: "text",
                        "aria-label": "{title_hint}",
                        id: "composer-title",
                        class: "cap-in",
                        style: "{FIELD_RESET} {field_ph} width: 100%; padding: 8px 0 0; font-size: {title_size}px; font-weight: 600; \
                                line-height: {title_lh}px; letter-spacing: -0.1px; color: {l.ink}; height: {title_lh + 8}px;",
                        placeholder: "{title_hint}",
                        value: "{title}",
                        autofocus: true,
                        onmounted: move |e| async move { let _ = e.set_focus(true).await; },
                        oninput: move |e| {
                            title.set(e.value());
                            cursor.set(0);
                        },
                        onkeydown: move |e: KeyboardEvent| {
                            if suggestions_key.is_empty() {
                                if e.key() == Key::Named(NamedKey::Enter) && !e.modifiers().meta() && !e.modifiers().ctrl() {
                                    // The title is one line. On a quick
                                    // note, Enter is done: write it down.
                                    e.prevent_default();
                                    if note_like && !e.modifiers().shift() {
                                        create_enter();
                                    }
                                }
                                return;
                            }
                            let n = suggestions_key.len();
                            match e.key() {
                                Key::Named(NamedKey::ArrowDown) => { e.prevent_default(); cursor.set((at + 1) % n); }
                                Key::Named(NamedKey::ArrowUp) => { e.prevent_default(); cursor.set((at + n - 1) % n); }
                                Key::Named(NamedKey::Enter | NamedKey::Tab) => {
                                    e.prevent_default();
                                    e.stop_propagation();
                                    if let Some(o) = suggestions_key.get(at).cloned() {
                                        resolve_key(o);
                                    }
                                }
                                Key::Named(NamedKey::Escape) => {
                                    e.prevent_default();
                                    e.stop_propagation();
                                    let next = format!("{} ", title.peek());
                                    title.set(next);
                                }
                                _ => {}
                            }
                        },
                    }
                    if !suggestions.is_empty() {
                        div { style: "position: absolute; left: 0; right: 0; top: 100%; z-index: 20; margin-top: 4px; display: flex; flex-direction: column; \
                                      border-radius: 8px; border: 1px solid {l.rule}; background: {l.sheet}; padding: 4px; box-shadow: {l.float}; {narrow}",
                            for (i , o) in suggestions.iter().cloned().enumerate() {
                                {
                                    let mut resolve = resolve.clone();
                                    let pick = o.clone();
                                    rsx! {
                                        button {
                                            key: "{o.id}",
                                            r#type: "button",
                                            style: "{RESET} display: flex; min-height: {row_h}px; align-items: center; gap: 10px; border-radius: 6px; padding: 0 10px; \
                                                    text-align: left; font-size: {row_text}px; color: {l.ink}; background: {row_bg(i == at)};",
                                            onmouseenter: move |_| cursor.set(i),
                                            onclick: move |_| resolve(pick.clone()),
                                            if let Some(icon) = o.icon.clone() { {icon} }
                                            if let Some(c) = o.dot.as_ref() { span { style: "width: 10px; height: 10px; border-radius: 999px; background: {c};" } }
                                            span { style: "{TRUNCATE}", "{o.label}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if details_open() || !description.read().is_empty() {
                    textarea {
                        "aria-label": "Description",
                        class: "cap-in",
                        onmounted: move |e| async move { if *details_focus.peek() { let _ = e.set_focus(true).await; } },
                        style: "{FIELD_RESET} {field_ph} margin-top: 4px; min-height: 80px; width: 100%; resize: none; font-size: 15px; font-weight: 450; \
                                line-height: 24px; letter-spacing: -0.1px; color: {wash(&l.ink, 90)};",
                        placeholder: "{detail_hint}",
                        value: "{description}",
                        oninput: move |e| description.set(e.value()),
                        // Blitz seeds a textarea from its text, not its `value`.
                        {description.peek().clone()}
                    }
                } else {
                    button {
                        r#type: "button",
                        "data-testid": "composer-details",
                        style: "{RESET} margin-top: 4px; display: flex; align-items: center; gap: 6px; align-self: flex-start; border-radius: 999px; \
                                padding: 4px 2px; font-size: 15px; color: {l.ink_2};",
                        onclick: move |_| {
                            let mut f = details_focus;
                            f.set(true);
                            details_open.set(true);
                        },
                        lc::Plus { size: 15 }
                        "Add details"
                    }
                }

                // ── sub-tasks ──
                if sub_count > 0 || adding_sub() {
                    div { style: "margin-top: 8px; display: flex; flex-direction: column; border-radius: 8px; border: 1px solid {wash(&l.rule, 70)}; padding: 8px;",
                        span { style: "padding: 0 4px 4px; font-size: 12px; font-weight: 500; color: {l.ink_2};", "Subtasks {sub_count}" }
                        for (i , s) in subs.read().iter().cloned().enumerate() {
                            div { key: "{i}", style: "display: flex; min-height: 36px; align-items: center; gap: 8px; padding: 0 4px; font-size: 14px;",
                                StatusIcon { status: "open".to_owned() }
                                span { style: "min-width: 0; flex: 1; {TRUNCATE}", "{s}" }
                                button {
                                    r#type: "button",
                                    class: "cap-h",
                                    style: "{RESET} display: flex; width: 28px; height: 28px; align-items: center; justify-content: center; border-radius: 6px; \
                                            color: {l.ink_2}; --cap-h: {l.hover};",
                                    "aria-label": "Remove subtask",
                                    onclick: move |_| { subs.write().remove(i); },
                                    lc::X { size: 13 }
                                }
                            }
                        }
                        if adding_sub() {
                            input {
                                r#type: "text",
                                class: "cap-in",
                                style: "{FIELD_RESET} {field_ph} min-height: 36px; padding: 0 4px; font-size: 14px; color: {l.ink};",
                                placeholder: "Subtask — Enter to add",
                                value: "{sub_draft}",
                                onmounted: move |e| async move { let _ = e.set_focus(true).await; },
                                oninput: move |e| sub_draft.set(e.value()),
                                onkeydown: move |e: KeyboardEvent| match e.key() {
                                    Key::Named(NamedKey::Enter) if !e.modifiers().meta() => {
                                        e.prevent_default();
                                        let t = sub_draft.peek().trim().to_owned();
                                        if !t.is_empty() {
                                            subs.write().push(t);
                                            sub_draft.set(String::new());
                                        }
                                    }
                                    Key::Named(NamedKey::Escape) => { e.prevent_default(); e.stop_propagation(); adding_sub.set(false); }
                                    _ => {}
                                },
                            }
                        }
                    }
                }
                if let Some(link) = link_draft() {
                    input {
                        r#type: "url",
                        class: "cap-in",
                        style: "{FIELD_RESET} {field_ph} margin-top: 8px; min-height: 36px; border-radius: 6px; border: 1px solid {l.rule}; \
                                padding: 0 8px; font-size: 14px; color: {l.ink};",
                        placeholder: "Paste a link — Enter to add",
                        value: "{link}",
                        onmounted: move |e| async move { let _ = e.set_focus(true).await; },
                        oninput: move |e| link_draft.set(Some(e.value())),
                        onkeydown: move |e: KeyboardEvent| {
                            if e.key() == Key::Named(NamedKey::Enter) {
                                e.prevent_default();
                                let url = link_draft.peek().clone().unwrap_or_default();
                                if !url.trim().is_empty() {
                                    let d = description.peek().clone();
                                    description.set(if d.trim().is_empty() { url.trim().to_owned() } else { format!("{}\n\n{}", d.trim_end(), url.trim()) });
                                }
                                link_draft.set(None);
                            } else if e.key() == Key::Named(NamedKey::Escape) {
                                e.prevent_default();
                                e.stop_propagation();
                                link_draft.set(None);
                            }
                        },
                    }
                }
                if let Some(s) = said() {
                    div { style: "margin-top: 8px; font-size: 12px; color: {l.danger};", "{s}" }
                }
            }

            // ── the bar ──
            // A phone: one row riding the keyboard, the properties in a pill
            // that scrolls sideways, the create button beside it, so the text
            // above gets the rest of the screen. Each chip opens its picker
            // as a sheet over the keyboard. A desk: the chips wrap in a row
            // over the footer.
            div { style: "{bar_outer}",
                div {
                    "data-testid": "composer-bar",
                    class: "cap-noscroll",
                    style: "{bar_inner}",
                    if shows_props {
                        StatusChip { status: h.status.clone(), on_change: on_status, opens: Some(bar_key) }
                        PriorityChip { priority: priority_now.clone(), on_change: on_priority, opens: Some(bar_key) }
                        AssigneeChip {
                            assignee: h.assignee.clone(),
                            people: people.clone(),
                            on_change: on_assignee,
                            opens: Some(bar_key),
                        }
                        ProjectChip {
                            project: project_now.clone(),
                            projects: projects.clone(),
                            on_change: on_project_pick,
                            on_create: on_project_create,
                            pending: new_project(),
                            opens: Some(bar_key),
                        }
                        // Where it gets done: what makes a chore a chore.
                        button {
                            r#type: "button",
                            "data-testid": "composer-context",
                            class: "cap-h cap-a",
                            style: if chosen_contexts.is_empty() { "{quiet}" } else { "{loud}" },
                            title: "Context",
                            onclick: move |_| context_open.set(true),
                            if chosen_contexts.is_empty() {
                                lc::DoorOpen { size: 14 }
                                "Context"
                            } else {
                                span { style: "display: flex; color: {l.ink_2};", lc::DoorOpen { size: 14 } }
                                span { style: "{TRUNCATE}", {chosen_contexts.iter().map(|c| format!("@{c}")).collect::<Vec<_>>().join(", ")} }
                            }
                        }
                        // The rest only once set: chosen from More.
                        if !chosen_labels.is_empty() || show_labels() {
                            LabelsChip {
                                chosen: chosen_labels.clone(),
                                known: known_labels.clone(),
                                on_toggle: on_label,
                                opens: Some(bar_key),
                            }
                        }
                        if let Some(d) = h.due.clone().or(parsed_due) {
                            button {
                                r#type: "button",
                                class: "cap-h cap-a",
                                style: "{loud}",
                                title: "Due date",
                                onclick: move |_| due_open.set(true),
                                span { style: "display: flex; color: {l.ink_2};", lc::CalendarDays { size: 14 } }
                                {d.get(..10).and_then(|s| s.parse::<chrono::NaiveDate>().ok()).map_or_else(|| d.clone(), |x| x.format("%b %-d").to_string())}
                            }
                        }
                        if let Some(r) = h.recurrence.clone().or(parsed_repeat) {
                            button {
                                r#type: "button",
                                class: "cap-h cap-a",
                                style: "{loud}",
                                title: "Repeat",
                                onclick: move |_| repeat_open.set(true),
                                span { style: "display: flex; color: {l.ink_2};", lc::Repeat { size: 14 } }
                                {capitalised(&repeat_label(&r))}
                            }
                        }
                        // More: labels, a date, a repeat, a subtask, a link.
                        button {
                            r#type: "button",
                            "data-testid": "composer-more",
                            class: "cap-h cap-a",
                            style: "{quiet}",
                            "aria-label": "More properties",
                            onclick: move |_| more_open.set(true),
                            lc::Plus { size: 14 }
                            "More"
                        }
                        // Process it now: decided in the same gesture rather
                        // than filed for the deck.
                        if let (true, true, Some(on_process)) = (offers_process_now(kind), picked_target.is_none(), on_process) {
                            button {
                                r#type: "button",
                                "data-testid": "composer-process",
                                class: "cap-h cap-a cap-btn",
                                style: "{quiet}",
                                title: "Decide what this is now",
                                disabled: empty,
                                onclick: move |_| {
                                    let t = title.peek().trim().to_owned();
                                    if t.is_empty() {
                                        return;
                                    }
                                    on_process.call(words(&t, &description.peek()));
                                    on_close.call(());
                                },
                                lc::ArrowRight { size: 15 }
                                "Process now"
                            }
                        }
                        // Start a timer instead: the swipe-up on the capture
                        // button, for whoever has not found it yet.
                        if kind == CaptureKind::Note && picked_target.is_none() && can_timer {
                            button {
                                r#type: "button",
                                "data-testid": "composer-timer",
                                class: "cap-h cap-a",
                                style: "{quiet}",
                                title: "Start a timer with this as its description",
                                onclick: {
                                    let cap = cap;
                                    move |_| {
                                        let what = title.peek().trim().to_owned();
                                        let cap = cap.clone();
                                        spawn(async move {
                                            let _ = cap.call(&ActionCaller::app(), "TIME_START", json!({ "description": what })).await;
                                        });
                                        on_close.call(());
                                    }
                                },
                                lc::Clock { size: 15 }
                                "Start a timer"
                            }
                        }
                    }
                }
                // Create, on a phone: the round button at the bar's end.
                if phone {
                    button {
                        r#type: "button",
                        "data-testid": "composer-create",
                        class: "cap-btn",
                        style: "{RESET} display: flex; width: 48px; height: 48px; flex-shrink: 0; align-items: center; justify-content: center; \
                                border-radius: 999px; background: {l.brand}; color: {l.brand_ink};",
                        "aria-label": "{action_label}",
                        title: "{action_label}",
                        disabled: empty || busy(),
                        onclick: move |_| create_round(),
                        lc::ArrowUp { size: 22 }
                    }
                }
            }
            // ── the pickers ──
            // Outside the bar, so a phone's scrolling bar does not clip their
            // sheets; the bar's chips open them (`bar_key`). On a desk they
            // drop down from here, under the chips.
            div { style: if phone { "" } else { "position: relative; margin: 0 16px;" },
                StatusChip { status: h.status.clone(), on_change: on_status, hotkey: Some(bar_key), picker_only: true }
                PriorityChip { priority: priority_now, on_change: on_priority, hotkey: Some(bar_key), picker_only: true }
                AssigneeChip {
                    assignee: h.assignee.clone(),
                    people: people,
                    on_change: on_assignee,
                    hotkey: Some(bar_key),
                    picker_only: true,
                }
                ProjectChip {
                    project: project_now,
                    projects: projects,
                    on_change: on_project_pick,
                    on_create: on_project_create,
                    pending: new_project(),
                    hotkey: Some(bar_key),
                    picker_only: true,
                }
                LabelsChip {
                    chosen: chosen_labels,
                    known: known_labels,
                    on_toggle: on_label,
                    hotkey: Some(bar_key),
                    picker_only: true,
                }
                PropertyPicker {
                    open: context_open(),
                    title: "Context…",
                    options: context_choices
                        .iter()
                        .map(|c| PickOption::new(c.clone(), format!("@{c}")).icon(rsx! { lc::DoorOpen { size: 15 } }))
                        .collect::<Vec<_>>(),
                    selected: chosen_contexts.clone(),
                    multi: true,
                    number_from: None,
                    on_create: Some(EventHandler::new(move |name: String| {
                        let name = name.trim().trim_start_matches('@').to_owned();
                        if !name.is_empty() {
                            add_once(&mut hand.write().contexts, &name);
                        }
                    })),
                    on_pick: move |name: String| toggle(&mut hand.write().contexts, &name),
                    on_close: move |()| context_open.set(false),
                }
                PropertyPicker {
                    open: more_open(),
                    title: "Add…",
                    options: vec![
                        PickOption::new("labels", "Labels").icon(rsx! { lc::Tag { size: 15 } }),
                        PickOption::new("due", "Due date").icon(rsx! { lc::CalendarDays { size: 15 } }),
                        PickOption::new("repeat", "Repeat").icon(rsx! { lc::Repeat { size: 15 } }),
                        PickOption::new("subtask", "Subtask").icon(rsx! { lc::ListTree { size: 15 } }),
                        PickOption::new("link", "Link").icon(rsx! { lc::Link2 { size: 15 } }),
                    ],
                    number_from: None,
                    on_pick: move |what: String| match what.as_str() {
                        "labels" => {
                            show_labels.set(true);
                            bar_key.set(Some(Hotkey::Labels));
                        }
                        "due" => due_open.set(true),
                        "repeat" => repeat_open.set(true),
                        "subtask" => adding_sub.set(true),
                        "link" => link_draft.set(Some(String::new())),
                        _ => {}
                    },
                    on_close: move |()| more_open.set(false),
                }
                // When: the common days, one tap each.
                PropertyPicker {
                    open: due_open(),
                    title: "Due…",
                    options: due_options(),
                    selected: vec![h.due.clone().unwrap_or_default()],
                    number_from: None,
                    on_pick: move |d: String| hand.write().due = Some(d).filter(|d| !d.is_empty()),
                    on_close: move |()| due_open.set(false),
                }
                PropertyPicker {
                    open: repeat_open(),
                    title: "Repeat…",
                    options: std::iter::once(PickOption::new("", "Doesn't repeat"))
                        .chain(REPEATS.iter().map(|(label, rule)| PickOption::new(*rule, *label)))
                        .collect::<Vec<_>>(),
                    selected: vec![h.recurrence.unwrap_or_default()],
                    number_from: None,
                    on_pick: move |rule: String| hand.write().recurrence = Some(rule).filter(|r| !r.is_empty()),
                    on_close: move |()| repeat_open.set(false),
                }
                // What it becomes, by hand: the inbox, or any target.
                PropertyPicker {
                    open: kind_open(),
                    title: "Make it…",
                    options: std::iter::once(
                        PickOption::new("", "Capture — into the inbox").icon(rsx! { lc::Inbox { size: 15 } }),
                    )
                    .chain(all.iter().map(|tg| {
                        let icon = if tg.icon.is_empty() { "file-text".to_owned() } else { tg.icon.clone() };
                        PickOption::new(tg.id.clone(), tg.name.clone()).icon(target_icon(&icon))
                    }))
                    .collect::<Vec<_>>(),
                    selected: vec![picked.unwrap_or_default()],
                    number_from: None,
                    on_pick: move |id: String| into.set(Some(id).filter(|i| !i.is_empty())),
                    on_close: move |()| kind_open.set(false),
                }
            }
            // ── create, on a desk ──
            if !phone {
                footer { style: "display: flex; align-items: center; justify-content: flex-end; gap: 12px; border-top: 1px solid {wash(&l.rule, 60)}; padding: 10px 16px;",
                    if kind == CaptureKind::Task && picked_target.is_none() {
                        label { style: "margin-left: auto; display: flex; align-items: center; gap: 8px; font-size: 13px; color: {l.ink_2}; cursor: pointer;",
                            input {
                                r#type: "checkbox",
                                checked: create_more(),
                                onchange: move |e| create_more.set(e.checked()),
                            }
                            "Create more"
                        }
                    }
                    button {
                        r#type: "button",
                        id: "capture-go",
                        class: "cap-btn",
                        style: "{RESET} display: flex; height: 32px; align-items: center; justify-content: center; border-radius: 8px; \
                                background: {l.brand}; color: {l.brand_ink}; padding: 0 16px; font-size: 13px; font-weight: 600;",
                        disabled: empty || busy(),
                        onclick: move |_| create_btn(),
                        "{action_label}"
                    }
                }
            }
        }
    }
}

/// A target's icon by its Lucide name (its manifest's `icon`): the ones
/// architect's targets use as components, any other by name.
fn target_icon(name: &str) -> Element {
    match name {
        "check" => rsx! { lc::Check { size: 15 } },
        "calendar-plus" => rsx! { lc::CalendarPlus { size: 15 } },
        "timer" => rsx! { lc::Timer { size: 15 } },
        "file-text" => rsx! { lc::FileText { size: 15 } },
        other => rsx! { LucideIcon { name: other.to_owned(), size: 15 } },
    }
}

/// `every week` as `Every week`.
fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| {
        f.to_uppercase().collect::<String>() + c.as_str()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "Process now" is offered exactly while what is typed is a plain
    /// capture: a capture given a chip or written with task syntax is a
    /// task already, and New task is never a capture.
    #[test]
    fn process_now_is_offered_while_it_is_still_a_capture() {
        let door = |opened, switchable, given, text| {
            offers_process_now(resolved_kind(opened, switchable, given, text))
        };
        assert!(door(
            CaptureKind::Note,
            true,
            false,
            "Ask the tuner what the soundboard costs"
        ));
        assert!(
            !door(CaptureKind::Note, true, true, "Ask the tuner"),
            "a chip makes it a task"
        );
        assert!(
            !door(CaptureKind::Note, true, false, "Ask the tuner !high"),
            "so does task syntax"
        );
        assert!(!door(
            CaptureKind::Note,
            true,
            false,
            "Ask the tuner @calls"
        ));
        assert!(
            !door(CaptureKind::Task, true, false, "plain words"),
            "New task is a task"
        );
        assert!(door(
            CaptureKind::Note,
            false,
            true,
            "a capture-only surface"
        ));
    }

    #[test]
    fn an_at_suggestion_knows_a_context_from_a_person() {
        assert_eq!(context_of("ctx:errands"), Some("errands"));
        assert_eq!(context_of("ctx+:garage"), Some("garage"));
        assert_eq!(context_of("Sam Okafor"), None);
    }

    #[test]
    fn the_last_word_says_what_it_wants() {
        assert_eq!(
            trigger_at_end("Bounce stems @co"),
            Some(Trigger::Person("co".into()))
        );
        assert_eq!(
            trigger_at_end("Bounce stems #mix"),
            Some(Trigger::Label("mix".into()))
        );
        assert_eq!(
            trigger_at_end("Bounce stems @"),
            Some(Trigger::Person(String::new()))
        );
        assert_eq!(
            trigger_at_end("Bounce stems @cody "),
            None,
            "a space closes it"
        );
        assert_eq!(
            trigger_at_end("email@example.com"),
            None,
            "only at a word's start"
        );
        assert_eq!(drop_last_word("Bounce stems @co"), "Bounce stems ");
        assert_eq!(drop_last_word("@co"), "");
    }

    #[test]
    fn a_capture_reads_as_a_note_a_task_or_a_project() {
        assert_eq!(
            kind_from_text("Idea: a stems-only bonus edition"),
            CaptureKind::Note
        );
        assert_eq!(
            kind_from_text("Pick up gaff tape @errands"),
            CaptureKind::Task
        );
        assert_eq!(
            kind_from_text("Bounce stems [[Crescendum Album]]"),
            CaptureKind::Task
        );
        assert_eq!(kind_from_text("Call the venue tomorrow"), CaptureKind::Task);
        assert_eq!(
            kind_from_text("project: Live session EP"),
            CaptureKind::Project
        );
        assert_eq!(
            kind_from_text("New project Remix pack"),
            CaptureKind::Project
        );
        assert_eq!(
            project_title("project:  Live session EP\nfour songs"),
            "Live session EP"
        );
        assert_eq!(project_title("Remix pack"), "Remix pack");
    }

    #[test]
    fn chips_set_by_hand_beat_the_text_and_the_text_fills_the_rest() {
        let text = Capturer::parse(
            "Bounce stems for the client [[Crescendum Album]] tomorrow !high #mix",
            &[],
        )
        .properties();
        let hand = Hand {
            status: "open".into(),
            priority: Some("critical".into()),
            assignee: Some("Sam Okafor".into()),
            labels: vec!["mixing".into()],
            ..Hand::default()
        };
        let out = crate::parse::merge(
            text,
            &hand.properties(&["mixing".into(), "mix".into()], &[]),
        );
        assert_eq!(out["priority"], json!("critical"), "the hand-set chip");
        assert_eq!(
            out["projects"],
            json!(["[[Crescendum Album]]"]),
            "from [[…]]"
        );
        assert!(out.contains_key("due"), "from 'tomorrow'");
        assert_eq!(
            out["tags"],
            json!(["mixing", "mix"]),
            "the text's labels kept"
        );
        assert_eq!(out["assignees"], json!(["[[Sam Okafor]]"]));
        assert!(!out.contains_key("status"), "open is nothing to say");
        assert!(hand.given(false));
        assert!(!Hand::default().given(false));
    }
}
