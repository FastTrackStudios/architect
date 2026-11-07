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
//! MIT OR Apache-2.0 like the rest of architect. [`DayPlans`] is the
//! extension (`extensions/dayplan/extension.toml`): its `DAYPLAN_*`
//! actions ([`plans`], Task's `get_day_plan` and `upsert_day_plan` and
//! the rest), the service `architect.dayplan` the Calendar draws plans
//! with, its views. It reads the vault and, when they are there, the
//! Calendar's events (for a rule's `#tag`) and the Time Tracker's entries
//! (planned against tracked) only through the host, so its process copy
//! (`src/main.rs`, `EXT_CLONE {"id":"dayplan"}`) works as the linked one
//! does.

// Minutes of a day stay far inside u16 and i64.
#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

pub mod book;
pub mod model;
pub mod notes;
pub mod plans;
pub mod resolve;
pub mod rules;
pub mod ui;
pub mod vault;

pub use book::{Book, Day, assign, no_tags, planned_vs_tracked, set_span, sort};
pub use model::{
    AssignmentKind, BlockAssignment, BlockCategory, DayPlan, DayTemplate, PlannedBlock, Revision,
    TimeBlock, TimeBlockId, TimeOfDay,
};
pub use rules::{Override, Reason, Resolution, Rule, Rules, RulesRevision, Span, When};

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use architect_extensions::linked::{Builder, Extension};
use chrono::{Local, NaiveDate};
use serde_json::json;

/// Its id: its actions are `DAYPLAN_*`.
pub const ID: &str = "dayplan";

/// The service it provides (`[provides]`): method `m` is `DAYPLAN_<M>`.
pub const SERVICE: &str = "architect.dayplan";

/// The services it reads when they are there.
pub(crate) const CALENDAR: &str = "architect.calendar";
pub(crate) const TIME: &str = "architect.time";

/// Where its notes go unless `dayplan.folder` says otherwise.
pub const FOLDER: &str = "Calendar";

/// What it knows of the vault, as last read.
struct Cache {
    book: Book,
    folder: String,
    read: bool,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache {
    book: Book {
        templates: Vec::new(),
        rules: Rules {
            path: String::new(),
            rules: Vec::new(),
            fallback: None,
            history: Vec::new(),
            overrides: Vec::new(),
        },
        plans: Vec::new(),
    },
    folder: String::new(),
    read: false,
});

/// Moves on with every read and write, for the views.
static REVISION: AtomicU64 = AtomicU64::new(0);

fn cache() -> std::sync::MutexGuard<'static, Cache> {
    CACHE.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn bump() {
    REVISION.fetch_add(1, Ordering::Relaxed);
}

/// What it knows changed since `revision()` last said.
#[must_use]
pub fn revision() -> u64 {
    REVISION.load(Ordering::Relaxed)
}

#[must_use]
pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// The templates, the rules and the plans, as last read.
#[must_use]
pub fn book() -> Book {
    cache().book.clone()
}

pub(crate) fn with_book<T>(f: impl FnOnce(&mut Book) -> T) -> T {
    let out = f(&mut cache().book);
    bump();
    out
}

/// Whether it has read the vault yet.
pub(crate) fn cache_read() -> bool {
    cache().read
}

/// The folder its notes are under (`Calendar`).
#[must_use]
pub fn folder() -> String {
    let c = cache();
    if c.read {
        c.folder.clone()
    } else {
        FOLDER.to_owned()
    }
}

/// Reads its notes and settings again.
///
/// # Errors
///
/// When the vault cannot be read.
pub async fn refresh() -> Result<Book, String> {
    let folder = vault::setting("dayplan.folder")
        .await
        .as_str()
        .map(|f| f.trim().trim_matches('/').to_owned())
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| FOLDER.to_owned());
    let book = plans::read().await?;
    let changed = {
        let mut c = cache();
        let changed = !c.read || c.book != book || c.folder != folder;
        c.book = book.clone();
        c.folder = folder;
        c.read = true;
        changed
    };
    // Only a change moves the revision on: a view that reads again when it
    // moves would otherwise read for ever.
    if changed {
        bump();
    }
    Ok(book)
}

/// The extension: link it in (`Extensions::with_linked`) or run its
/// program.
#[derive(Debug, Clone, Copy, Default)]
pub struct DayPlans;

impl Extension for DayPlans {
    fn manifest(&self) -> &'static str {
        include_str!("../extension.toml")
    }

    fn build(&self, ext: &mut Builder) {
        vault::connect(ext.handle());
        ext.view("templates", ui::TemplatesView)
            .view("rules", ui::RulesView)
            .action("DAYPLAN_OPEN", plans::open)
            .action("DAYPLAN_GET", plans::get)
            .action("DAYPLAN_DAYS", plans::days_action)
            .action("DAYPLAN_UPSERT", plans::upsert)
            .action("DAYPLAN_ASSIGN", plans::assign)
            .action("DAYPLAN_SET_BLOCK", plans::set_block)
            .action("DAYPLAN_RESET", plans::reset)
            .action("DAYPLAN_TEMPLATES", plans::templates)
            .action("DAYPLAN_TEMPLATE", plans::template)
            .action("DAYPLAN_TEMPLATE_RULES", plans::template_rules)
            .action("DAYPLAN_SET_WEEKDAY", plans::set_weekday)
            .action("DAYPLAN_OVERRIDE", plans::set_override)
            .action("DAYPLAN_RESOLVE", plans::resolve)
            .action("DAYPLAN_STARTED", |_| async {
                let _ = refresh().await;
                Ok(json!({ "ready": true }))
            });
    }
}
