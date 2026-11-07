//! Day Plans (#444), a first-party architect extension: Task's day
//! templates (`task:features/scheduling/scheduling-proto`), the architect
//! way.
//!
//! - **Templates** are notes, reusable days: Cody's "Workday" has a reset,
//!   three allocatable Work / Event / Free-time blocks, meal-prep blocks,
//!   a wind-down and sleep ([`DayTemplate`]).
//! - **Rules** pick a day's template, first match wins; overrides beat
//!   them ([`rules`]).
//! - **A plan** is one date's own copy, saved the first time it is edited
//!   (a block moved, resized or given to a task, a project, a label or a
//!   recipe): `Calendar/Day plans/<date>.md`. A day with none is its
//!   template, so a change to the template or the rules reaches it.
//! - **Only the future changes.** Rules and templates keep what they were
//!   before each change until yesterday; a past day resolves as it did.
//!   Edited days keep their edits ("Reset to template" deletes the plan).
//! - [`resolve`] is Task's reflow, as it is: fixed blocks win, routine
//!   blocks shift or shrink, meals are embedded, the rest shrink.
//!
//! The model, the rules, the resolver and the notes' format are pure, and
//! MIT OR Apache-2.0 like the rest of architect.

// Minutes of a day stay far inside u16 and i64.
#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

pub mod book;
pub mod model;
pub mod notes;
pub mod resolve;
pub mod rules;

pub use book::{Book, Day, assign, no_tags, planned_vs_tracked, set_span, sort};
pub use model::{
    AssignmentKind, BlockAssignment, BlockCategory, DayPlan, DayTemplate, PlannedBlock, Revision,
    TimeBlock, TimeBlockId, TimeOfDay,
};
pub use rules::{Override, Reason, Resolution, Rule, Rules, RulesRevision, Span, When};
