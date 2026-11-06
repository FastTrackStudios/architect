//! A task, as the note it is: Task's `TaskInfo` (`task:features/task/task-proto/src/model.rs`),
//! ported to the vault. The frontmatter keeps TaskNotes' names, so a
//! TaskNotes vault reads as it is: `status`, `priority`, `due`,
//! `scheduled`, `tags`, `contexts`, `projects`, `timeEstimate`,
//! `recurrence`, `recurrence_anchor`, `complete_instances`,
//! `completedDate`, `dateCreated`, `dateModified`, `blockedBy`. Past
//! TaskNotes: `parent` (a sub-task's task), `relates`, `assignees` and
//! `milestone`.
//!
//! Where Task points at a project or another task by UUID, a note points
//! by link: `projects: ["[[Projects/Site/project|Site]]"]`,
//! `blockedBy: ["[[Tasks/Design]]"]`. A link is what Obsidian, the
//! vault's index and a person reading the file all follow.
//!
//! Everything here is pure: no host, no files.

use chrono::{Datelike, Duration, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// The frontmatter keys a task writes, TaskNotes' names first.
pub mod keys {
    pub const TYPE: &str = "type";
    pub const TITLE: &str = "title";
    pub const STATUS: &str = "status";
    pub const PRIORITY: &str = "priority";
    pub const DUE: &str = "due";
    pub const SCHEDULED: &str = "scheduled";
    pub const TAGS: &str = "tags";
    pub const CONTEXTS: &str = "contexts";
    pub const PROJECTS: &str = "projects";
    pub const TIME_ESTIMATE: &str = "timeEstimate";
    pub const RECURRENCE: &str = "recurrence";
    pub const RECURRENCE_ANCHOR: &str = "recurrence_anchor";
    pub const COMPLETE_INSTANCES: &str = "complete_instances";
    pub const COMPLETED_DATE: &str = "completedDate";
    pub const DATE_CREATED: &str = "dateCreated";
    pub const DATE_MODIFIED: &str = "dateModified";
    pub const BLOCKED_BY: &str = "blockedBy";
    pub const PARENT: &str = "parent";
    pub const RELATES: &str = "relates";
    pub const ASSIGNEES: &str = "assignees";
    pub const MILESTONE: &str = "milestone";

    /// Every key a task reads, for the base that lists tasks.
    pub const ALL: &[&str] = &[
        TITLE,
        STATUS,
        PRIORITY,
        DUE,
        SCHEDULED,
        TAGS,
        CONTEXTS,
        PROJECTS,
        TIME_ESTIMATE,
        RECURRENCE,
        RECURRENCE_ANCHOR,
        COMPLETE_INSTANCES,
        COMPLETED_DATE,
        DATE_CREATED,
        DATE_MODIFIED,
        BLOCKED_BY,
        PARENT,
        RELATES,
        ASSIGNEES,
        MILESTONE,
    ];
}

/// What a note is a task by: `type: task`, or the `task` tag (TaskNotes').
pub const KIND: &str = "task";

/// One task note.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Task {
    /// Its vault path (`Tasks/Buy milk.md`).
    pub path: String,
    pub title: String,
    /// Free text, so custom statuses round-trip; [`Status`] reads the
    /// ones it knows.
    pub status: String,
    pub priority: String,
    pub due: Option<String>,
    pub scheduled: Option<String>,
    pub tags: Vec<String>,
    pub contexts: Vec<String>,
    /// Links to the projects it belongs to.
    pub projects: Vec<String>,
    pub milestone: Option<String>,
    /// The task it is a sub-task of.
    pub parent: Option<String>,
    /// Tasks that must close before this one can start.
    pub blocked_by: Vec<String>,
    pub relates: Vec<String>,
    pub assignees: Vec<String>,
    /// Minutes.
    pub time_estimate: Option<u32>,
    /// RFC 5545 RRULE (`FREQ=WEEKLY;BYDAY=MO`), TaskNotes' `recurrence`.
    pub recurrence: Option<String>,
    /// `scheduled` (the next one from the date it was due) or
    /// `completion` (from the day it was done).
    pub recurrence_anchor: Option<String>,
    pub complete_instances: Vec<String>,
    pub completed_date: Option<String>,
    pub date_created: Option<String>,
    pub date_modified: Option<String>,
}

impl Task {
    /// Read from a note's properties (a base row's `values`, or its
    /// frontmatter as JSON); `path` and `name` are the note's.
    #[must_use]
    pub fn from_values(path: &str, name: &str, values: &Map<String, Value>) -> Self {
        let text = |k: &str| values.get(k).and_then(scalar).filter(|s| !s.is_empty());
        let list = |k: &str| values.get(k).map(list_of).unwrap_or_default();
        Self {
            path: path.to_owned(),
            title: text(keys::TITLE).unwrap_or_else(|| name.to_owned()),
            status: text(keys::STATUS).unwrap_or_else(|| Status::Open.as_str().to_owned()),
            priority: text(keys::PRIORITY).unwrap_or_else(|| Priority::Normal.as_str().to_owned()),
            due: text(keys::DUE),
            scheduled: text(keys::SCHEDULED),
            tags: list(keys::TAGS),
            contexts: list(keys::CONTEXTS),
            projects: list(keys::PROJECTS),
            milestone: text(keys::MILESTONE),
            parent: text(keys::PARENT),
            blocked_by: list(keys::BLOCKED_BY),
            relates: list(keys::RELATES),
            assignees: list(keys::ASSIGNEES),
            time_estimate: text(keys::TIME_ESTIMATE).and_then(|s| s.parse().ok()),
            recurrence: text(keys::RECURRENCE),
            recurrence_anchor: text(keys::RECURRENCE_ANCHOR),
            complete_instances: list(keys::COMPLETE_INSTANCES),
            completed_date: text(keys::COMPLETED_DATE),
            date_created: text(keys::DATE_CREATED),
            date_modified: text(keys::DATE_MODIFIED),
        }
    }

    /// Its frontmatter, as a new note's (`VAULT_CREATE`'s `properties`):
    /// `type: task` and the `task` tag, then what it has.
    #[must_use]
    pub fn frontmatter(&self) -> Map<String, Value> {
        let mut m = Map::new();
        let mut put = |k: &str, v: Value| {
            m.insert(k.to_owned(), v);
        };
        put(keys::TYPE, json!(KIND));
        put(keys::TITLE, json!(self.title));
        put(keys::STATUS, json!(self.status));
        put(keys::PRIORITY, json!(self.priority));
        let mut tags = self.tags.clone();
        if !tags.iter().any(|t| t == KIND) {
            tags.insert(0, KIND.to_owned());
        }
        put(keys::TAGS, json!(tags));
        let opt = |v: &Option<String>| v.as_ref().map(|s| json!(s));
        let many = |v: &[String]| (!v.is_empty()).then(|| json!(v));
        for (k, v) in [
            (keys::DUE, opt(&self.due)),
            (keys::SCHEDULED, opt(&self.scheduled)),
            (keys::CONTEXTS, many(&self.contexts)),
            (keys::PROJECTS, many(&self.projects)),
            (keys::MILESTONE, opt(&self.milestone)),
            (keys::PARENT, opt(&self.parent)),
            (keys::BLOCKED_BY, many(&self.blocked_by)),
            (keys::RELATES, many(&self.relates)),
            (keys::ASSIGNEES, many(&self.assignees)),
            (keys::TIME_ESTIMATE, self.time_estimate.map(|n| json!(n))),
            (keys::RECURRENCE, opt(&self.recurrence)),
            (keys::RECURRENCE_ANCHOR, opt(&self.recurrence_anchor)),
            (keys::COMPLETE_INSTANCES, many(&self.complete_instances)),
            (keys::COMPLETED_DATE, opt(&self.completed_date)),
            (keys::DATE_CREATED, opt(&self.date_created)),
            (keys::DATE_MODIFIED, opt(&self.date_modified)),
        ] {
            if let Some(v) = v {
                put(k, v);
            }
        }
        m
    }

    /// Whether it is still to do ([`status_is_open`]).
    #[must_use]
    pub fn is_open(&self) -> bool {
        status_is_open(&self.status)
    }

    /// Its column on a board: [`Status::group`] of its status.
    #[must_use]
    pub fn group(&self) -> Group {
        Status::group_of(&self.status)
    }

    /// Whether it belongs to the project at `project` (a `project.md`'s
    /// path): one of its `projects` links names it, or the task's note is
    /// inside the project's folder.
    #[must_use]
    pub fn in_project(&self, project: &str) -> bool {
        let folder = project
            .rsplit_once('/')
            .map_or("", |(dir, _)| dir)
            .to_owned();
        (!folder.is_empty() && self.path.starts_with(&format!("{folder}/")))
            || self.projects.iter().any(|l| link_names(l, project))
    }

    /// Whether it is on someone's list: `me` among its assignees (names
    /// or links, any case).
    #[must_use]
    pub fn assigned_to(&self, me: &str) -> bool {
        let me = me.trim().to_lowercase();
        !me.is_empty()
            && self
                .assignees
                .iter()
                .any(|a| link_target(a).to_lowercase().ends_with(&me) || a.to_lowercase() == me)
    }
}

/// A frontmatter value as text: a string, a number, a bool.
#[must_use]
pub fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A frontmatter value as a list: a sequence's strings, or one string.
#[must_use]
pub fn list_of(v: &Value) -> Vec<String> {
    match v {
        Value::Array(items) => items.iter().filter_map(scalar).collect(),
        Value::String(s) if !s.is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// What a link points at: `[[Projects/Site/project|Site]]` →
/// `Projects/Site/project`; anything else as it is.
#[must_use]
pub fn link_target(link: &str) -> &str {
    let inner = link
        .trim()
        .strip_prefix("[[")
        .and_then(|l| l.strip_suffix("]]"))
        .unwrap_or_else(|| link.trim());
    let inner = inner.split_once('|').map_or(inner, |(t, _)| t);
    inner.split_once('#').map_or(inner, |(t, _)| t).trim()
}

/// A link to the note at `path` (`Tasks/Design.md` → `[[Tasks/Design]]`),
/// shown as `alias` when given.
#[must_use]
pub fn link_to(path: &str, alias: Option<&str>) -> String {
    let target = path.strip_suffix(".md").unwrap_or(path);
    match alias {
        Some(a) if !a.is_empty() => format!("[[{target}|{a}]]"),
        _ => format!("[[{target}]]"),
    }
}

/// Whether `link` names the note at `path`: by its path (with or without
/// `.md`), or by its file name when the link gives no folder.
#[must_use]
pub fn link_names(link: &str, path: &str) -> bool {
    let target = link_target(link);
    let target = target.strip_suffix(".md").unwrap_or(target);
    let path = path.strip_suffix(".md").unwrap_or(path);
    if target.contains('/') {
        return target.eq_ignore_ascii_case(path);
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    target.eq_ignore_ascii_case(name)
}

/// The statuses it knows; any other text is kept as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    Open,
    InProgress,
    Waiting,
    Done,
    Cancelled,
}

/// Where a status puts a task: its board column, its list's section.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    Todo,
    Doing,
    Waiting,
    Done,
    Cancelled,
}

impl Group {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Todo => "To do",
            Self::Doing => "In progress",
            Self::Waiting => "Waiting",
            Self::Done => "Done",
            Self::Cancelled => "Cancelled",
        }
    }

    /// The status a task moved into this group is given.
    #[must_use]
    pub fn status(self) -> Status {
        match self {
            Self::Todo => Status::Open,
            Self::Doing => Status::InProgress,
            Self::Waiting => Status::Waiting,
            Self::Done => Status::Done,
            Self::Cancelled => Status::Cancelled,
        }
    }

    pub const ALL: [Self; 5] = [
        Self::Todo,
        Self::Doing,
        Self::Waiting,
        Self::Done,
        Self::Cancelled,
    ];
}

impl Status {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InProgress => "in-progress",
            Self::Waiting => "waiting",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }

    /// The statuses it knows, and their synonyms (`todo`, `doing`,
    /// `blocked`, `completed`, `canceled`). `None` for any other.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "open" | "todo" | "to-do" | "none" | "" => Self::Open,
            "in-progress" | "in_progress" | "doing" | "started" => Self::InProgress,
            "waiting" | "blocked" | "on-hold" | "on_hold" => Self::Waiting,
            "done" | "completed" | "complete" | "shipped" | "archived" => Self::Done,
            "cancelled" | "canceled" | "abandoned" | "wontfix" => Self::Cancelled,
            _ => return None,
        })
    }

    /// The group any status text falls in: an unknown one is to do,
    /// unless it reads as finished ([`status_is_terminal`]).
    #[must_use]
    pub fn group_of(status: &str) -> Group {
        match Self::parse(status) {
            Some(Self::Open) => Group::Todo,
            Some(Self::InProgress) => Group::Doing,
            Some(Self::Waiting) => Group::Waiting,
            Some(Self::Done) => Group::Done,
            Some(Self::Cancelled) => Group::Cancelled,
            None if status_is_terminal(status) => Group::Done,
            None => Group::Todo,
        }
    }
}

/// Status text that means a task is finished: Task's one terminal set,
/// raw-string aware, so `shipped` and `archived` close a task as `done`
/// does, on every surface.
#[must_use]
pub fn status_is_terminal(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "done"
            | "complete"
            | "completed"
            | "shipped"
            | "cancelled"
            | "canceled"
            | "abandoned"
            | "archived"
            | "wontfix"
    )
}

/// A task is open while its status is not [`status_is_terminal`].
#[must_use]
pub fn status_is_open(status: &str) -> bool {
    !status_is_terminal(status)
}

/// Its priorities, lowest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    None,
    Low,
    Normal,
    High,
    Critical,
}

impl Priority {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "none" | "" => Self::None,
            "low" | "4" => Self::Low,
            "normal" | "medium" | "med" | "3" => Self::Normal,
            "high" | "2" => Self::High,
            "critical" | "urgent" | "1" => Self::Critical,
            _ => return None,
        })
    }

    /// How a list sorts it: unknown text as normal.
    #[must_use]
    pub fn rank(s: &str) -> Self {
        Self::parse(s).unwrap_or(Self::Normal)
    }
}

/// A date's `YYYY-MM-DD` part.
#[must_use]
pub fn day(s: &str) -> &str {
    s.get(..10).unwrap_or(s)
}

/// Whether `due` or `scheduled` falls on or before `date`: Task's "what is
/// due" rule, by the dates' day.
#[must_use]
pub fn is_due_on_or_before(due: Option<&str>, scheduled: Option<&str>, date: &str) -> bool {
    let target = day(date);
    due.map(day).is_some_and(|d| d <= target) || scheduled.map(day).is_some_and(|d| d <= target)
}

/// Whether it is on Today: open, and in progress or due (or planned) by
/// `today`.
#[must_use]
pub fn is_today(task: &Task, today: &str) -> bool {
    task.is_open()
        && (task.group() == Group::Doing
            || is_due_on_or_before(task.due.as_deref(), task.scheduled.as_deref(), today))
}

/// The order a list shows open work in: in progress first, then the
/// earliest due, then the highest priority, then by title.
#[must_use]
pub fn order(a: &Task, b: &Task) -> std::cmp::Ordering {
    let doing = |t: &Task| t.group() != Group::Doing;
    let due = |t: &Task| {
        t.due
            .as_deref()
            .or(t.scheduled.as_deref())
            .map_or_else(|| "9999".to_owned(), |d| day(d).to_owned())
    };
    doing(a)
        .cmp(&doing(b))
        .then_with(|| due(a).cmp(&due(b)))
        .then_with(|| Priority::rank(&b.priority).cmp(&Priority::rank(&a.priority)))
        .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
}

/// How often a task comes back: a subset of RFC 5545's RRULE, the shape
/// TaskNotes writes (`FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,TH`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repeat {
    pub freq: Freq,
    pub interval: u32,
    /// For a weekly one: the days it falls on (none: the day it was).
    pub by_day: Vec<Weekday>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl Repeat {
    /// Reads an RRULE (`RRULE:` optional); `None` when it is not one.
    #[must_use]
    pub fn parse(rule: &str) -> Option<Self> {
        let rule = rule.trim();
        let rule = rule.strip_prefix("RRULE:").unwrap_or(rule);
        let mut freq = None;
        let mut interval = 1;
        let mut by_day = Vec::new();
        for part in rule.split(';').filter(|p| !p.is_empty()) {
            let (k, v) = part.split_once('=')?;
            match k.trim().to_ascii_uppercase().as_str() {
                "FREQ" => {
                    freq = Some(match v.trim().to_ascii_uppercase().as_str() {
                        "DAILY" => Freq::Daily,
                        "WEEKLY" => Freq::Weekly,
                        "MONTHLY" => Freq::Monthly,
                        "YEARLY" => Freq::Yearly,
                        _ => return None,
                    });
                }
                "INTERVAL" => interval = v.trim().parse().ok().filter(|n| *n > 0)?,
                "BYDAY" => {
                    for d in v.split(',') {
                        by_day.push(weekday(d.trim())?);
                    }
                }
                // UNTIL, COUNT, … are kept in the note, not read here.
                _ => {}
            }
        }
        Some(Self {
            freq: freq?,
            interval,
            by_day,
        })
    }

    /// The rule as TaskNotes writes it.
    #[must_use]
    pub fn rule(&self) -> String {
        let freq = match self.freq {
            Freq::Daily => "DAILY",
            Freq::Weekly => "WEEKLY",
            Freq::Monthly => "MONTHLY",
            Freq::Yearly => "YEARLY",
        };
        let mut out = format!("FREQ={freq}");
        if self.interval > 1 {
            out.push_str(&format!(";INTERVAL={}", self.interval));
        }
        if !self.by_day.is_empty() {
            let days: Vec<&str> = self.by_day.iter().map(|d| rrule_day(*d)).collect();
            out.push_str(&format!(";BYDAY={}", days.join(",")));
        }
        out
    }

    /// The first time it falls after `from`.
    #[must_use]
    pub fn next_after(&self, from: NaiveDate) -> NaiveDate {
        let n = i64::from(self.interval);
        match self.freq {
            Freq::Daily => from + Duration::days(n),
            Freq::Weekly if self.by_day.is_empty() => from + Duration::weeks(n),
            Freq::Weekly => {
                // The next listed day this week, else the first listed one
                // `interval` weeks on.
                let mut d = from + Duration::days(1);
                let week_end =
                    from + Duration::days(7 - i64::from(from.weekday().num_days_from_monday()));
                while d < week_end {
                    if self.by_day.contains(&d.weekday()) {
                        return d;
                    }
                    d += Duration::days(1);
                }
                let start = week_end + Duration::weeks(n - 1);
                (0..7)
                    .map(|i| start + Duration::days(i))
                    .find(|d| self.by_day.contains(&d.weekday()))
                    .unwrap_or(start)
            }
            Freq::Monthly => add_months(from, self.interval),
            Freq::Yearly => add_months(from, self.interval.saturating_mul(12)),
        }
    }
}

fn add_months(from: NaiveDate, months: u32) -> NaiveDate {
    let total = from.month0() + months;
    let year = from.year() + i32::try_from(total / 12).unwrap_or(0);
    let month = total % 12 + 1;
    // The same day, or the month's last when it has fewer.
    (0..4)
        .find_map(|back| NaiveDate::from_ymd_opt(year, month, from.day().saturating_sub(back)))
        .unwrap_or(from)
}

fn weekday(s: &str) -> Option<Weekday> {
    Some(
        match s.to_ascii_uppercase().get(s.len().saturating_sub(2)..)? {
            "MO" => Weekday::Mon,
            "TU" => Weekday::Tue,
            "WE" => Weekday::Wed,
            "TH" => Weekday::Thu,
            "FR" => Weekday::Fri,
            "SA" => Weekday::Sat,
            "SU" => Weekday::Sun,
            _ => return None,
        },
    )
}

fn rrule_day(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "MO",
        Weekday::Tue => "TU",
        Weekday::Wed => "WE",
        Weekday::Thu => "TH",
        Weekday::Fri => "FR",
        Weekday::Sat => "SA",
        Weekday::Sun => "SU",
    }
}

/// What finishing a task writes: the property changes, by key (`null`
/// removes one).
///
/// A one-off closes: `status` and `completedDate`. A repeating one stays
/// open (TaskNotes'): `today` joins `complete_instances`, and its `due`
/// (and `scheduled`) moves to the next time, counted from the date it was
/// due, or from `today` when its anchor is `completion`.
#[must_use]
pub fn complete(task: &Task, status: &str, today: NaiveDate) -> Vec<(&'static str, Value)> {
    let today_s = today.format("%Y-%m-%d").to_string();
    let finishing = status_is_terminal(status) && Status::parse(status) != Some(Status::Cancelled);
    let repeat = task.recurrence.as_deref().and_then(Repeat::parse);
    match (finishing, repeat) {
        (true, Some(rule)) => {
            let mut done = task.complete_instances.clone();
            if !done.contains(&today_s) {
                done.push(today_s.clone());
            }
            let from_completion = task.recurrence_anchor.as_deref() == Some("completion");
            let next = |date: &Option<String>| {
                let base = date
                    .as_deref()
                    .and_then(|d| NaiveDate::parse_from_str(day(d), "%Y-%m-%d").ok())
                    .filter(|_| !from_completion)
                    .unwrap_or(today);
                // Past today, so an overdue one is not due again at once.
                let mut n = rule.next_after(base);
                while n <= today && !from_completion {
                    n = rule.next_after(n);
                }
                json!(n.format("%Y-%m-%d").to_string())
            };
            let mut out = vec![
                (keys::COMPLETE_INSTANCES, json!(done)),
                (keys::STATUS, json!(Status::Open.as_str())),
            ];
            if task.due.is_some() || task.scheduled.is_none() {
                out.push((keys::DUE, next(&task.due)));
            }
            if task.scheduled.is_some() {
                out.push((keys::SCHEDULED, next(&task.scheduled)));
            }
            out
        }
        (true, None) => vec![
            (keys::STATUS, json!(status)),
            (keys::COMPLETED_DATE, json!(today_s)),
        ],
        (false, _) if status_is_terminal(&task.status) => vec![
            (keys::STATUS, json!(status)),
            (keys::COMPLETED_DATE, Value::Null),
        ],
        (false, _) => vec![(keys::STATUS, json!(status))],
    }
}

/// One edge between two tasks, by their paths: `from` `kind`s `to`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Edge {
    pub from: String,
    pub kind: Relation,
    pub to: String,
}

/// Task's relation kinds, in the vault: `blocks` (from another's
/// `blockedBy`), `subtask` (from a `parent`) and `relates`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    Blocks,
    Subtask,
    Relates,
}

/// Every edge among `tasks` whose links resolve to one of them (Task's
/// `relations::edges`, over links): `blockedBy: [b]` on `t` is `b blocks
/// t`, `parent: p` on `t` is `t subtask p`, `relates: [r]` is `t relates
/// r`. Self-edges are dropped.
#[must_use]
pub fn edges(tasks: &[Task]) -> Vec<Edge> {
    let find = |link: &str| {
        tasks
            .iter()
            .find(|t| link_names(link, &t.path))
            .map(|t| t.path.clone())
    };
    let mut out = Vec::new();
    for t in tasks {
        for b in &t.blocked_by {
            if let Some(from) = find(b) {
                out.push(Edge {
                    from,
                    kind: Relation::Blocks,
                    to: t.path.clone(),
                });
            }
        }
        if let Some(to) = t.parent.as_deref().and_then(find) {
            out.push(Edge {
                from: t.path.clone(),
                kind: Relation::Subtask,
                to,
            });
        }
        for r in &t.relates {
            if let Some(to) = find(r) {
                out.push(Edge {
                    from: t.path.clone(),
                    kind: Relation::Relates,
                    to,
                });
            }
        }
    }
    out.retain(|e| e.from != e.to);
    out
}

/// What one task is to the rest: what blocks it (and whether any is still
/// open), what it blocks, its sub-tasks (and how many are done), its
/// parent, and what it relates to.
#[must_use]
pub fn relations_of(path: &str, tasks: &[Task]) -> Value {
    let all = edges(tasks);
    let open = |p: &str| tasks.iter().any(|t| t.path == p && t.is_open());
    let blockers: Vec<&str> = all
        .iter()
        .filter(|e| e.kind == Relation::Blocks && e.to == path)
        .map(|e| e.from.as_str())
        .collect();
    let subtasks: Vec<&str> = all
        .iter()
        .filter(|e| e.kind == Relation::Subtask && e.to == path)
        .map(|e| e.from.as_str())
        .collect();
    json!({
        "blocked_by": blockers,
        "blocked": blockers.iter().any(|b| open(b)),
        "blocks": all.iter().filter(|e| e.kind == Relation::Blocks && e.from == path).map(|e| &e.to).collect::<Vec<_>>(),
        "parent": all.iter().find(|e| e.kind == Relation::Subtask && e.from == path).map(|e| &e.to),
        "subtasks": subtasks,
        "subtasks_done": subtasks.iter().filter(|s| !open(s)).count(),
        "relates": all.iter().filter(|e| e.kind == Relation::Relates && (e.from == path || e.to == path))
            .map(|e| if e.from == path { &e.to } else { &e.from }).collect::<Vec<_>>(),
    })
}

/// A note's file name for a title: what a file system takes, kept
/// readable (`Buy milk / eggs` → `Buy milk - eggs`).
#[must_use]
pub fn file_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if "/\\:*?\"<>|#^[]".contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    if cleaned.is_empty() {
        "Untitled task".to_owned()
    } else {
        cleaned.chars().take(120).collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn terminal_set_covers_synonyms_and_any_case() {
        for s in [
            "done",
            "completed",
            "shipped",
            "cancelled",
            "abandoned",
            "archived",
            "DONE",
        ] {
            assert!(status_is_terminal(s), "{s}");
            assert!(!status_is_open(s));
        }
        for s in ["open", "in-progress", "waiting", "blocked", "", "review"] {
            assert!(status_is_open(s), "{s}");
        }
        assert_eq!(Status::group_of("doing"), Group::Doing);
        assert_eq!(Status::group_of("shipped"), Group::Done);
        assert_eq!(
            Status::group_of("review"),
            Group::Todo,
            "unknown and open: to do"
        );
    }

    #[test]
    fn due_counts_due_or_scheduled_by_day() {
        assert!(is_due_on_or_before(Some("2026-06-10"), None, "2026-06-14"));
        assert!(is_due_on_or_before(
            None,
            Some("2026-06-14T09:00:00Z"),
            "2026-06-14"
        ));
        assert!(!is_due_on_or_before(
            Some("2026-06-20"),
            Some("2026-06-21"),
            "2026-06-14"
        ));
        assert!(!is_due_on_or_before(None, None, "2026-06-14"));
    }

    #[test]
    fn a_tasknotes_note_reads_and_writes_back() {
        let values: Map<String, Value> = serde_json::from_value(json!({
            "status": "in-progress", "priority": "high", "due": "2026-11-06",
            "tags": ["task", "web"], "projects": ["[[Projects/Site/project|Site]]"],
            "blockedBy": "[[Tasks/Design]]", "timeEstimate": 90,
            "recurrence": "FREQ=WEEKLY;BYDAY=MO",
        }))
        .unwrap();
        let t = Task::from_values("Tasks/Ship it.md", "Ship it", &values);
        assert_eq!(t.title, "Ship it", "the file's name when no title");
        assert_eq!(
            t.blocked_by,
            ["[[Tasks/Design]]"],
            "one string is a list of one"
        );
        assert_eq!(t.time_estimate, Some(90));
        assert!(t.in_project("Projects/Site/project.md"));
        assert!(!t.in_project("Projects/Other/project.md"));
        let fm = t.frontmatter();
        assert_eq!(fm["type"], "task");
        assert_eq!(fm["tags"], json!(["task", "web"]), "the tag kept once");
        assert_eq!(fm["blockedBy"], json!(["[[Tasks/Design]]"]));
        assert_eq!(Task::from_values(&t.path, "x", &fm).title, "Ship it");
    }

    #[test]
    fn a_task_inside_a_project_folder_belongs_to_it() {
        let t = Task {
            path: "Projects/Site/Tasks/Copy.md".into(),
            ..Task::default()
        };
        assert!(t.in_project("Projects/Site/project.md"));
        assert!(!t.in_project("Projects/Sitemap/project.md"));
    }

    #[test]
    fn links_name_notes_by_path_or_name() {
        assert_eq!(
            link_target("[[Projects/Site/project|Site]]"),
            "Projects/Site/project"
        );
        assert!(link_names(
            "[[Projects/Site/project|Site]]",
            "Projects/Site/project.md"
        ));
        assert!(link_names("[[Design]]", "Tasks/Design.md"));
        assert!(!link_names("[[Other/Design]]", "Tasks/Design.md"));
        assert_eq!(link_to("Tasks/Design.md", None), "[[Tasks/Design]]");
    }

    #[test]
    fn repeats_read_and_fall_where_they_should() {
        let weekly = Repeat::parse("FREQ=WEEKLY;BYDAY=MO,TH").unwrap();
        assert_eq!(weekly.rule(), "FREQ=WEEKLY;BYDAY=MO,TH");
        // 2026-11-05 is a Thursday: the next is Monday.
        assert_eq!(weekly.next_after(date("2026-11-05")), date("2026-11-09"));
        assert_eq!(weekly.next_after(date("2026-11-09")), date("2026-11-12"));
        let fortnightly = Repeat::parse("RRULE:FREQ=WEEKLY;INTERVAL=2").unwrap();
        assert_eq!(
            fortnightly.next_after(date("2026-11-05")),
            date("2026-11-19")
        );
        let monthly = Repeat::parse("FREQ=MONTHLY").unwrap();
        assert_eq!(
            monthly.next_after(date("2026-01-31")),
            date("2026-02-28"),
            "the month's last"
        );
        assert!(Repeat::parse("FREQ=HOURLY").is_none());
        assert!(Repeat::parse("nonsense").is_none());
    }

    #[test]
    fn finishing_a_repeating_task_moves_it_on_and_keeps_it_open() {
        let t = Task {
            status: "open".into(),
            due: Some("2026-11-02".into()),
            recurrence: Some("FREQ=WEEKLY".into()),
            ..Task::default()
        };
        let out = complete(&t, "done", date("2026-11-05"));
        let get = |k: &str| {
            out.iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(get("status"), Some(json!("open")));
        assert_eq!(
            get("due"),
            Some(json!("2026-11-09")),
            "a week from the date it was due, past today"
        );
        assert_eq!(get("complete_instances"), Some(json!(["2026-11-05"])));
        let once = Task::default();
        let out = complete(&once, "done", date("2026-11-05"));
        assert!(out.contains(&("completedDate", json!("2026-11-05"))));
        let reopen = Task {
            status: "done".into(),
            ..Task::default()
        };
        assert!(
            complete(&reopen, "open", date("2026-11-05")).contains(&("completedDate", Value::Null))
        );
    }

    #[test]
    fn relations_merge_blockers_parents_and_links() {
        let task = |path: &str| Task {
            path: path.into(),
            status: "open".into(),
            ..Task::default()
        };
        let mut design = task("Tasks/Design.md");
        design.status = "done".into();
        let mut build = task("Tasks/Build.md");
        build.blocked_by = vec!["[[Design]]".into(), "[[Tasks/Review]]".into()];
        build.parent = Some("[[Tasks/Launch]]".into());
        let review = task("Tasks/Review.md");
        let launch = task("Tasks/Launch.md");
        let all = [design, build, review, launch];
        let r = relations_of("Tasks/Build.md", &all);
        assert_eq!(
            r["blocked_by"],
            json!(["Tasks/Design.md", "Tasks/Review.md"])
        );
        assert_eq!(r["blocked"], true, "Review is still open");
        assert_eq!(r["parent"], "Tasks/Launch.md");
        let l = relations_of("Tasks/Launch.md", &all);
        assert_eq!(l["subtasks"], json!(["Tasks/Build.md"]));
        assert_eq!(l["subtasks_done"], 0);
        assert_eq!(
            relations_of("Tasks/Design.md", &all)["blocks"],
            json!(["Tasks/Build.md"])
        );
    }

    #[test]
    fn open_work_sorts_doing_then_due_then_priority() {
        let t = |title: &str, status: &str, due: Option<&str>, p: &str| Task {
            title: title.into(),
            status: status.into(),
            due: due.map(Into::into),
            priority: p.into(),
            ..Task::default()
        };
        let mut v = [
            t("c", "open", None, "high"),
            t("b", "open", Some("2026-11-06"), "low"),
            t("a", "in-progress", None, "low"),
            t("d", "open", Some("2026-11-06"), "high"),
        ];
        v.sort_by(order);
        let titles: Vec<&str> = v.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["a", "d", "b", "c"]);
    }
}
