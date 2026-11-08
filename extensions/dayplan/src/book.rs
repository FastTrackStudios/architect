//! A day as it is planned ([`Book::day`]): its saved plan, else its
//! template's blocks as they were that day; and the edits on it.

use chrono::NaiveDate;
use serde::Serialize;

use crate::model::{BlockAssignment, DayPlan, DayTemplate, PlannedBlock, TimeOfDay};
use crate::resolve;
use crate::rules::{Reason, Rules};

/// Everything the day plans read: the templates, the rules and the saved
/// plans.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Book {
    pub templates: Vec<DayTemplate>,
    pub rules: Rules,
    pub plans: Vec<DayPlan>,
}

/// A day as it is planned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Day {
    pub date: NaiveDate,
    /// The template it uses (none: no template).
    pub template: Option<String>,
    /// Why, in words: `Rule 1: Mon–Fri`, `Override for this day`.
    pub reason: String,
    /// Chosen by hand (an override).
    pub chosen: bool,
    /// It has a plan of its own: edited, kept when the template or the
    /// rules change.
    pub edited: bool,
    /// Before today: what the rules or a template now say never reaches
    /// it.
    pub past: bool,
    /// Its template is named but not in the vault.
    pub missing: bool,
    /// Its blocks, by start.
    pub blocks: Vec<PlannedBlock>,
}

impl Day {
    /// Its plan, to save after an edit: the blocks as they are, with the
    /// template and why.
    #[must_use]
    pub fn plan(&self, path: String) -> DayPlan {
        DayPlan {
            path,
            date: self.date,
            template: self.template.clone(),
            reason: Some(self.reason.clone()),
            blocks: self.blocks.clone(),
        }
    }

    /// Its block `id`.
    #[must_use]
    pub fn block(&self, id: &str) -> Option<&PlannedBlock> {
        self.blocks.iter().find(|b| b.id.0 == id)
    }
}

/// Never a tag: for callers with no events.
#[must_use]
pub const fn no_tags(_: &str, _: NaiveDate) -> bool {
    false
}

impl Book {
    /// The template called `name`.
    #[must_use]
    pub fn template(&self, name: &str) -> Option<&DayTemplate> {
        self.templates.iter().find(|t| t.is(name))
    }

    /// `date`'s saved plan.
    #[must_use]
    pub fn plan(&self, date: NaiveDate) -> Option<&DayPlan> {
        self.plans.iter().find(|p| p.date == date)
    }

    /// `date` as it is planned, `today` being today: its saved plan, else
    /// its template's blocks as they were that day.
    #[must_use]
    pub fn day(
        &self,
        date: NaiveDate,
        today: NaiveDate,
        has_tag: &dyn Fn(&str, NaiveDate) -> bool,
    ) -> Day {
        let past = date < today;
        if let Some(p) = self.plan(date) {
            let mut blocks = p.blocks.clone();
            sort(&mut blocks);
            return Day {
                date,
                template: p.template.clone(),
                reason: p.reason.clone().unwrap_or_else(|| Reason::Saved.words()),
                chosen: self
                    .rules
                    .override_on(date)
                    .is_some_and(|o| o.template == p.template),
                edited: true,
                past,
                missing: p
                    .template
                    .as_deref()
                    .is_some_and(|t| self.template(t).is_none()),
                blocks,
            };
        }
        let r = self.rules.resolve(date, has_tag);
        let tpl = r.template.as_deref().and_then(|t| self.template(t));
        let mut blocks: Vec<PlannedBlock> = tpl
            .map(|t| t.blocks_on(date).iter().map(PlannedBlock::from).collect())
            .unwrap_or_default();
        sort(&mut blocks);
        Day {
            date,
            missing: r.template.is_some() && tpl.is_none(),
            template: tpl.map(|t| t.name.clone()).or(r.template),
            chosen: r.reason.is_override(),
            reason: r.reason.words(),
            edited: false,
            past,
            blocks,
        }
    }
}

/// By start, a wrap past midnight last.
pub fn sort(blocks: &mut [PlannedBlock]) {
    blocks.sort_by_key(|b| (resolve::day_span(b.start, b.end).0, b.id.0.clone()));
}

/// Gives block `id` of `day` to `assignment` (none: takes it away).
///
/// # Errors
///
/// When the day has no block `id`.
pub fn assign(day: &mut Day, id: &str, assignment: Option<BlockAssignment>) -> Result<(), String> {
    let b = day
        .blocks
        .iter_mut()
        .find(|b| b.id.0 == id)
        .ok_or_else(|| format!("{} has no block {id:?}", day.date))?;
    b.assignment = assignment;
    Ok(())
}

/// Moves or resizes block `id` of `day` to `start`..`end`.
///
/// # Errors
///
/// When the day has no block `id`.
pub fn set_span(day: &mut Day, id: &str, start: TimeOfDay, end: TimeOfDay) -> Result<(), String> {
    let b = day
        .blocks
        .iter_mut()
        .find(|b| b.id.0 == id)
        .ok_or_else(|| format!("{} has no block {id:?}", day.date))?;
    b.start = start;
    b.end = end;
    sort(&mut day.blocks);
    Ok(())
}

/// What tracked time covers of each block: `(block id, planned minutes,
/// tracked minutes)`, the tracked spans given as minutes of the day.
#[must_use]
pub fn planned_vs_tracked(
    blocks: &[PlannedBlock],
    tracked: &[(u16, u16)],
) -> Vec<(String, u16, u16)> {
    blocks
        .iter()
        .map(|b| {
            let (s, e) = resolve::day_span(b.start, b.end);
            let got = tracked
                .iter()
                .map(|&(ts, te)| te.min(e).saturating_sub(ts.max(s)))
                .fold(0_u16, u16::saturating_add);
            (
                b.id.0.clone(),
                e.saturating_sub(s),
                got.min(e.saturating_sub(s)),
            )
        })
        .collect()
}
