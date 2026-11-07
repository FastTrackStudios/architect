//! A day's shape: blocks of time in categories, Task's
//! `task:features/scheduling/scheduling-proto/src/time_block.rs` and
//! `day_plan.rs`, with the same names so [`crate::resolve`] ports as it
//! is.
//!
//! - A [`DayTemplate`] is a reusable day ("Workday", "Studio day"): its
//!   [`TimeBlock`]s, and the blocks it had before each change
//!   ([`Revision`]), so a past day keeps the shape it had then.
//! - A [`DayPlan`] is one date's own copy, saved when it is edited: its
//!   [`PlannedBlock`]s, each with what it is given to
//!   ([`BlockAssignment`]: a task, a project, a label, a recipe).

use std::fmt;

use chrono::NaiveDate;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A block's id, unique in its template or plan (`block-1`).
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TimeBlockId(pub String);

impl From<&str> for TimeBlockId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

/// A time of day, minutes since midnight (0 to 1440); written `HH:MM`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimeOfDay {
    pub minutes_since_midnight: u16,
}

impl TimeOfDay {
    /// The end of the day, `24:00`.
    pub const END: Self = Self {
        minutes_since_midnight: 24 * 60,
    };

    #[must_use]
    pub fn new(hours: u8, minutes: u8) -> Self {
        Self {
            minutes_since_midnight: u16::from(hours) * 60 + u16::from(minutes),
        }
    }

    /// `minutes` since midnight, held to the day.
    #[must_use]
    pub fn at(minutes: i64) -> Self {
        Self {
            minutes_since_midnight: u16::try_from(minutes.clamp(0, 24 * 60)).unwrap_or(0),
        }
    }

    #[must_use]
    pub const fn hours(self) -> u16 {
        self.minutes_since_midnight / 60
    }

    #[must_use]
    pub const fn minutes(self) -> u16 {
        self.minutes_since_midnight % 60
    }

    /// `07:30`, `7:30`, `24:00`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (h, m) = text.trim().split_once(':')?;
        let (h, m): (u16, u16) = (h.trim().parse().ok()?, m.trim().parse().ok()?);
        (m < 60 && (h < 24 || (h == 24 && m == 0))).then_some(Self {
            minutes_since_midnight: h * 60 + m,
        })
    }

    /// As a chrono time (`24:00` as the last minute's end is not one:
    /// `None`).
    #[must_use]
    pub fn time(self) -> Option<chrono::NaiveTime> {
        chrono::NaiveTime::from_hms_opt(u32::from(self.hours()), u32::from(self.minutes()), 0)
    }
}

impl fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hours(), self.minutes())
    }
}

impl Serialize for TimeOfDay {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for TimeOfDay {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        match &v {
            serde_json::Value::String(s) => Self::parse(s),
            // A YAML `7:30` can be read as sexagesimal minutes: a number.
            serde_json::Value::Number(n) => n.as_i64().map(Self::at),
            serde_json::Value::Object(o) => o
                .get("minutes_since_midnight")
                .and_then(serde_json::Value::as_i64)
                .map(Self::at),
            _ => None,
        }
        .ok_or_else(|| serde::de::Error::custom(format!("not a time of day: {v}")))
    }
}

/// What kind of time a block is: its colour and how the resolver treats
/// it (Task's set).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockCategory {
    /// Wake-up routine, morning reset.
    Reset,
    /// Spiritual time, journaling, meditation.
    Spiritual,
    /// Cooking, eating, cleaning up; meal prep.
    Meal,
    /// Movement, the gym, walks.
    Exercise,
    /// Getting ready.
    Hygiene,
    /// One of the three Work / Event / Free-time blocks ("Block 1/2/3"),
    /// given to a task, a project or a label day by day.
    Allocatable,
    /// Errands, chores, admin.
    Maintenance,
    /// The evening's wind-down.
    WindDown,
    Sleep,
    #[default]
    Other,
}

/// Read in any case, `wind-down` too; what it does not know is
/// [`BlockCategory::Other`].
impl<'de> Deserialize<'de> for BlockCategory {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        Ok(v.as_str().map(Self::parse).unwrap_or_default())
    }
}

impl BlockCategory {
    /// Every category, in the order a picker shows them.
    pub const ALL: [Self; 10] = [
        Self::Reset,
        Self::Spiritual,
        Self::Meal,
        Self::Exercise,
        Self::Hygiene,
        Self::Allocatable,
        Self::Maintenance,
        Self::WindDown,
        Self::Sleep,
        Self::Other,
    ];

    /// Its id as written (`wind_down`).
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Reset => "reset",
            Self::Spiritual => "spiritual",
            Self::Meal => "meal",
            Self::Exercise => "exercise",
            Self::Hygiene => "hygiene",
            Self::Allocatable => "allocatable",
            Self::Maintenance => "maintenance",
            Self::WindDown => "wind_down",
            Self::Sleep => "sleep",
            Self::Other => "other",
        }
    }

    /// Its id read back; anything else is [`Self::Other`].
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let t = text.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        Self::ALL
            .into_iter()
            .find(|c| c.id() == t || (t == "winddown" && *c == Self::WindDown))
            .unwrap_or_default()
    }

    /// Its name for people.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Reset => "Reset",
            Self::Spiritual => "Spiritual",
            Self::Meal => "Meal",
            Self::Exercise => "Exercise",
            Self::Hygiene => "Hygiene",
            Self::Allocatable => "Allocatable",
            Self::Maintenance => "Maintenance",
            Self::WindDown => "Wind-down",
            Self::Sleep => "Sleep",
            Self::Other => "Other",
        }
    }

    /// The colour it draws in (Task's `day_template_view.rs` palette).
    #[must_use]
    pub const fn colour(self) -> &'static str {
        match self {
            Self::Reset => "#8b5cf6",
            Self::Spiritual => "#a855f7",
            Self::Meal => "#f59e0b",
            Self::Exercise => "#10b981",
            Self::Hygiene => "#06b6d4",
            Self::Allocatable => "#3b82f6",
            Self::Maintenance => "#64748b",
            Self::WindDown => "#6366f1",
            Self::Sleep => "#334155",
            Self::Other => "#94a3b8",
        }
    }
}

/// One block of a template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeBlock {
    pub id: TimeBlockId,
    pub start: TimeOfDay,
    /// Exclusive; at or before `start` it wraps past midnight (sleep).
    pub end: TimeOfDay,
    pub label: String,
    #[serde(default)]
    pub category: BlockCategory,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Never moved by the resolver ("at work 10:00").
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fixed: bool,
}

/// What a block is given to on a day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentKind {
    /// Free text.
    Label,
    /// A task's note.
    Task,
    /// A project's note.
    Project,
    /// A recipe or meal note, for a meal block.
    Recipe,
}

impl AssignmentKind {
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "label" => Some(Self::Label),
            "task" => Some(Self::Task),
            "project" => Some(Self::Project),
            "recipe" | "meal" | "note" => Some(Self::Recipe),
            _ => None,
        }
    }
}

/// What a block is given to that day (Task's `BlockAssignment`): a task,
/// a project, a label, or for a meal a recipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockAssignment {
    pub kind: AssignmentKind,
    /// What it shows: the label, or the note's title.
    pub title: String,
    /// The note's vault path (a task, a project, a recipe); none for a
    /// label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// One block of a day's plan: a template's block and what it is given
/// to that day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedBlock {
    pub id: TimeBlockId,
    pub start: TimeOfDay,
    pub end: TimeOfDay,
    pub label: String,
    #[serde(default)]
    pub category: BlockCategory,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignment: Option<BlockAssignment>,
    /// Immovable: the resolver reflows the others around it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fixed: bool,
}

impl From<&TimeBlock> for PlannedBlock {
    fn from(b: &TimeBlock) -> Self {
        Self {
            id: b.id.clone(),
            start: b.start,
            end: b.end,
            label: b.label.clone(),
            category: b.category,
            note: b.note.clone(),
            assignment: None,
            fixed: b.fixed,
        }
    }
}

impl PlannedBlock {
    /// Its length in minutes on its own day (a wrap is cut at midnight).
    #[must_use]
    pub fn minutes(&self) -> u16 {
        let (s, e) = crate::resolve::day_span(self.start, self.end);
        e.saturating_sub(s)
    }

    /// What it shows: what it is given to, else its label.
    #[must_use]
    pub fn title(&self) -> &str {
        self.assignment.as_ref().map_or(&self.label, |a| &a.title)
    }
}

/// The blocks a template had up to and including `until`: what a day on
/// or before it resolves to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision {
    pub until: NaiveDate,
    pub blocks: Vec<TimeBlock>,
}

/// A reusable day, a note (`Calendar/Day templates/Workday.md`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayTemplate {
    /// Its note.
    pub path: String,
    /// Its name, what rules and overrides name it by.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// A CSS colour for its badge on a day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub blocks: Vec<TimeBlock>,
    /// Its blocks before each change, oldest first: a past day keeps the
    /// shape it had then.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Revision>,
}

impl DayTemplate {
    /// Its blocks as they were on `date`: the first revision kept until
    /// that day or later, else the current ones.
    #[must_use]
    pub fn blocks_on(&self, date: NaiveDate) -> &[TimeBlock] {
        self.history
            .iter()
            .filter(|r| r.until >= date)
            .min_by_key(|r| r.until)
            .map_or(&self.blocks, |r| &r.blocks)
    }

    /// Changes its blocks to `blocks` from `today` on: the ones it had are
    /// kept for the days before (a revision until yesterday), unless they
    /// already are (a second change today).
    pub fn change_blocks(&mut self, blocks: Vec<TimeBlock>, today: NaiveDate) {
        if blocks == self.blocks {
            return;
        }
        if let Some(yesterday) = today.pred_opt()
            && !self.history.iter().any(|r| r.until >= yesterday)
        {
            self.history.push(Revision {
                until: yesterday,
                blocks: std::mem::take(&mut self.blocks),
            });
        }
        self.blocks = blocks;
    }

    /// Whether it is called `name` (any case).
    #[must_use]
    pub fn is(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name.trim())
    }
}

/// One date's own plan, saved when it is edited (or a template is chosen
/// for it): `Calendar/Day plans/2026-11-12.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayPlan {
    pub path: String,
    pub date: NaiveDate,
    /// The template it was made from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// Why that template, when it was made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub blocks: Vec<PlannedBlock>,
}
