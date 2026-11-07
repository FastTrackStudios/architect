//! Quick add and capture: a line, or a capture from architect's inbox,
//! made into a task.
//!
//! The words are read by architect's one quick-add parser
//! (`features/capture/src/parse.rs` `parse`, ported from Task's capture):
//! `[[Project]]`, `#tag`, `@context`, `!high`, a date, a repeat; the rest
//! is the title. "New task", `TASKS_ADD` and the `task` capture target
//! (`TASKS_CAPTURE`, `[[capture_targets]]` in `extension.toml`) all come
//! here.

use architect_capture::parse::keys as k;
use architect_capture::{Capture, Draft};
use chrono::NaiveDate;
use serde_json::{Map, Value};

use crate::model::{Priority, Status, Task};

/// Reads `input` into a new task, dates counted from `today`.
#[must_use]
pub fn parse(input: &str, today: NaiveDate) -> Task {
    from_draft(&architect_capture::parse(input, today))
}

/// A parsed line as a new task: open, normal unless it says otherwise.
#[must_use]
pub fn from_draft(draft: &Draft) -> Task {
    let mut task = from_properties(&draft.title, &draft.properties());
    if task.title.is_empty() {
        "Untitled task".clone_into(&mut task.title);
    }
    task
}

/// A capture (`TASKS_CAPTURE`) as a new task: its title, and the
/// properties a task has from the ones it carries (`TaskNotes`' keys, as
/// the parser writes them). Its body is the task's details.
#[must_use]
pub fn from_capture(capture: &Capture) -> Task {
    let mut task = from_properties(capture.title.trim(), &capture.properties);
    if task.title.is_empty() {
        "Untitled task".clone_into(&mut task.title);
    }
    task
}

fn from_properties(title: &str, props: &Map<String, Value>) -> Task {
    let text = |key: &str| props.get(key).and_then(Value::as_str).map(str::to_owned);
    let list = |key: &str| match props.get(key) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
        _ => Vec::new(),
    };
    Task {
        title: title.to_owned(),
        status: text(crate::model::keys::STATUS)
            .unwrap_or_else(|| Status::Open.as_str().to_owned()),
        priority: text(k::PRIORITY)
            .and_then(|p| Priority::parse(&p))
            .unwrap_or(Priority::Normal)
            .as_str()
            .to_owned(),
        due: text(k::DUE),
        scheduled: text(crate::model::keys::SCHEDULED),
        tags: list(k::TAGS),
        contexts: list(k::CONTEXTS),
        projects: list(k::PROJECTS),
        recurrence: text(k::RECURRENCE),
        recurrence_anchor: text(k::RECURRENCE_ANCHOR),
        // The composer's Assignee chip and processing's estimate (#445).
        assignees: list("assignees"),
        time_estimate: props
            .get("time_estimate")
            .and_then(Value::as_u64)
            .and_then(|m| u32::try_from(m).ok()),
        ..Task::default()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A Thursday.
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 5).unwrap()
    }

    #[test]
    fn a_capture_s_assignee_and_estimate_reach_the_task() {
        let capture: architect_capture::Capture = serde_json::from_value(json!({
            "title": "Call the venue",
            "properties": { "assignees": ["[[Sam Okafor]]"], "time_estimate": 15, "status": "waiting" }
        }))
        .unwrap();
        let t = from_capture(&capture);
        assert_eq!(t.assignees, ["[[Sam Okafor]]"]);
        assert_eq!(t.time_estimate, Some(15));
        assert_eq!(t.status, "waiting");
    }

    #[test]
    fn a_line_is_a_task_through_the_shared_parser() {
        let t = parse(
            "Call the venue [[Album]] #gigs @phone !high tomorrow",
            today(),
        );
        assert_eq!(t.title, "Call the venue");
        assert_eq!(t.projects, ["[[Album]]"]);
        assert_eq!(t.tags, ["gigs"]);
        assert_eq!(t.contexts, ["@phone"]);
        assert_eq!(t.priority, "high");
        assert_eq!(t.due.as_deref(), Some("2026-11-06"));
        assert_eq!(t.status, "open");
        let t = parse("Standup every monday", today());
        assert_eq!(t.recurrence.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO"));
        assert_eq!(t.recurrence_anchor.as_deref(), Some("scheduled"));
        assert!(crate::model::Repeat::parse(t.recurrence.as_deref().unwrap()).is_some());
        assert_eq!(parse("", today()).title, "Untitled task");
        assert_eq!(parse("", today()).priority, "normal");
    }

    #[test]
    fn a_capture_is_a_task_its_properties_kept() {
        let c = Capture {
            title: "Sketch the toolbar".to_owned(),
            body: "Three ideas".to_owned(),
            properties: json!({
                "projects": ["[[Lab]]"], "due": "2026-11-06", "priority": "high",
                "tags": ["design"], "status": "waiting",
            })
            .as_object()
            .cloned()
            .unwrap(),
            ..Capture::default()
        };
        let t = from_capture(&c);
        assert_eq!(t.title, "Sketch the toolbar");
        assert_eq!(t.projects, ["[[Lab]]"]);
        assert_eq!(t.due.as_deref(), Some("2026-11-06"));
        assert_eq!(t.priority, "high");
        assert_eq!(t.tags, ["design"]);
        assert_eq!(t.status, "waiting", "a status set by hand is kept");
    }
}
