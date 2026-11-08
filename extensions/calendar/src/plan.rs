//! Day plans on the calendar (#444).
//!
//! What the Day Plans extension (`architect.dayplan`,
//! `extensions/dayplan`) says of each day, read
//! through its service when it is there (`[uses]`). The Calendar draws a
//! day's blocks under its events, in their categories' colours, with the
//! time tracked in each; and its navigator section shows each day's plan.

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};
use serde::Deserialize;
use serde_json::{Value, json};

/// The service.
pub const DAYPLAN: &str = "architect.dayplan";

/// What a block is given to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Assignment {
    /// `task`, `project`, `recipe` or `label`.
    pub kind: String,
    pub title: String,
    pub path: Option<String>,
}

/// One block of a day's plan.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct PlanBlock {
    pub id: String,
    /// `HH:MM`; an end at or before the start wraps past midnight.
    pub start: String,
    pub end: String,
    pub label: String,
    pub category: String,
    pub colour: String,
    pub fixed: bool,
    pub assignment: Option<Assignment>,
    /// Minutes planned, and tracked in it (the Time Tracker's).
    pub minutes: u32,
    pub tracked: u32,
}

fn time(text: &str) -> Option<NaiveTime> {
    if text.trim() == "24:00" {
        return None;
    }
    NaiveTime::parse_from_str(text.trim(), "%H:%M").ok()
}

impl PlanBlock {
    /// What it shows: what it is given to, else its label.
    #[must_use]
    pub fn title(&self) -> &str {
        self.assignment.as_ref().map_or(&self.label, |a| &a.title)
    }

    /// Its span on `date`: from its start to its end, cut at midnight
    /// when it wraps (the rest is the next day's morning).
    #[must_use]
    pub fn span(&self, date: NaiveDate) -> Option<(NaiveDateTime, NaiveDateTime)> {
        let midnight = date.and_hms_opt(0, 0, 0)?;
        let start = time(&self.start).map_or(midnight + Duration::days(1), |t| date.and_time(t));
        let end = time(&self.end).map_or(midnight + Duration::days(1), |t| date.and_time(t));
        let end = if end <= start {
            midnight + Duration::days(1)
        } else {
            end
        };
        (end > start).then_some((start, end))
    }

    /// The part of it that runs into the next day's morning, on that day
    /// (a sleep from 22:00 to 06:30: 00:00–06:30).
    #[must_use]
    pub fn tail(&self, date: NaiveDate) -> Option<(NaiveDateTime, NaiveDateTime)> {
        let (s, e) = (time(&self.start)?, time(&self.end)?);
        if e > s || e == NaiveTime::MIN {
            return None;
        }
        let next = date + Duration::days(1);
        Some((next.and_hms_opt(0, 0, 0)?, next.and_time(e)))
    }

    /// Its time in words: `08:00–11:30`.
    #[must_use]
    pub fn when(&self) -> String {
        format!("{}–{}", self.start, self.end)
    }
}

/// One day as planned.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct PlanDay {
    pub date: NaiveDate,
    /// Its template, and why (`Rule 3: Mon–Fri`, `Override for this day`).
    pub template: Option<String>,
    pub reason: String,
    /// Chosen by hand (an override).
    pub chosen: bool,
    /// A plan of its own (kept when the template changes).
    pub edited: bool,
    /// Before today: as it was.
    pub past: bool,
    /// Its template's colour.
    pub color: Option<String>,
    pub blocks: Vec<PlanBlock>,
}

impl PlanDay {
    /// What its allocatable and meal blocks are given to, in a line:
    /// `Block 1: Album · Lunch prep: Lentil soup`; else how many blocks.
    #[must_use]
    pub fn summary(&self) -> String {
        let given: Vec<String> = self
            .blocks
            .iter()
            .filter_map(|b| {
                b.assignment
                    .as_ref()
                    .map(|a| format!("{}: {}", b.label, a.title))
            })
            .collect();
        if given.is_empty() {
            let free = self
                .blocks
                .iter()
                .filter(|b| b.category == "allocatable")
                .count();
            format!("{} blocks, {free} to give", self.blocks.len())
        } else {
            given.join(" · ")
        }
    }

    /// Its template in words, with why: `Workday · Rule 3: Mon–Fri`.
    #[must_use]
    pub fn template_words(&self) -> String {
        self.template.as_ref().map_or_else(
            || format!("No template · {}", self.reason),
            |t| format!("{t} · {}", self.reason),
        )
    }
}

/// The days of `from..to` (at most 62) as Day Plans has them; `None`
/// without Day Plans.
pub async fn days(from: NaiveDate, to: NaiveDate) -> Option<Vec<PlanDay>> {
    let args = json!({ "from": from.to_string(), "to": to.to_string() });
    // Day Plans there but failing: no plans drawn, as without it.
    let out = crate::vault::service(DAYPLAN, "days", args).await?.ok()?;
    Some(days_of(&out))
}

/// [`days`] from the service's answer.
#[must_use]
pub fn days_of(out: &Value) -> Vec<PlanDay> {
    out["days"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| serde_json::from_value(d.clone()).ok())
        .collect()
}
