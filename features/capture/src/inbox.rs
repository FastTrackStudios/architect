//! The inbox: captures as notes in the vault's `Inbox/` folder, each one
//! back once to be processed — filed as a note, turned into a target's
//! thing (a task, …), put off until later, or discarded.
//!
//! A capture's note keeps Task's inbox frontmatter
//! (`task:features/inbox/inbox/src/lib.rs` `serialize_inbox_item`), so a
//! vault Task wrote reads the same:
//!
//! ```markdown
//! ---
//! type: inbox-item
//! status: open
//! source: window
//! created: 2026-11-05T10:12:00-08:00
//! projects: ["[[Lab]]"]
//! due: 2026-11-06
//! ---
//! The rest of what was typed, after the first line.
//! ```
//!
//! A vault's own inbox convention reads too: `type: capture`, `captured`
//! (when) and `processed: false` (the demo vault's,
//! `apps/ui/demo-vault/Inbox`). Only the notes directly in the folder are
//! captures (`Inbox/Processed/` is not).
//!
//! Its file name is its title; the properties the quick-add tokens gave
//! (`crate::parse`) ride along until it is processed. Put off, it has a
//! `resurface_on` date and stays out of the inbox until then.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::parse::{self, Draft};

/// The folder captures land in when the app says no other.
pub const FOLDER: &str = "Inbox";

/// What an inbox note's `type` says.
pub const KIND: &str = "inbox-item";

/// What a vault's own convention calls one (`type: capture`).
pub const KIND_ALT: &str = "capture";

/// The frontmatter keys the inbox owns (a capture's own properties are
/// the parser's, `crate::parse::keys`).
pub mod keys {
    pub const TYPE: &str = "type";
    pub const STATUS: &str = "status";
    pub const SOURCE: &str = "source";
    pub const CREATED: &str = "created";
    pub const RESURFACE_ON: &str = "resurface_on";
    /// The other convention's `created` and `status`.
    pub const CAPTURED: &str = "captured";
    pub const PROCESSED: &str = "processed";
    /// Every key that only means something in the inbox: taken off a
    /// capture as it leaves.
    pub const ALL: &[&str] = &[
        TYPE,
        STATUS,
        SOURCE,
        CREATED,
        RESURFACE_ON,
        CAPTURED,
        PROCESSED,
    ];
}

/// A capture's note, before it is written: where, its frontmatter, its
/// body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewNote {
    /// In the inbox folder; the vault adds ` 2` when it is taken.
    pub path: String,
    pub title: String,
    pub properties: Map<String, Value>,
    pub body: String,
}

/// One capture in the inbox, as read back.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub path: String,
    pub title: String,
    pub body: String,
    /// Where it came from: `ui`, `window`, `mcp`, `cli`.
    pub source: String,
    /// When, RFC 3339.
    pub created: String,
    /// Put off until then.
    pub resurface_on: Option<NaiveDate>,
    /// What it carries to whatever it becomes (the inbox's own keys out).
    pub properties: Map<String, Value>,
}

impl Item {
    /// Reads a `VAULT_READ` answer (`{path, title, properties, body}`):
    /// `None` for a note that is not an open capture (another note in the
    /// folder, one processed already).
    #[must_use]
    pub fn from_note(note: &Value) -> Option<Self> {
        let props = note.get("properties")?.as_object()?;
        let text = |m: &Map<String, Value>, k: &str| {
            m.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        // A note dropped in the folder by hand is a capture too; one that
        // says it is something else, or done, is not.
        let kind = text(props, keys::TYPE);
        if !kind.is_empty() && kind != KIND && kind != KIND_ALT {
            return None;
        }
        if props.get(keys::PROCESSED).and_then(Value::as_bool) == Some(true) {
            return None;
        }
        let status = text(props, keys::STATUS);
        if !status.is_empty() && status != "open" && status != "suggested" {
            return None;
        }
        Some(Self {
            path: note.get("path")?.as_str()?.to_owned(),
            title: note
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            body: note
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned(),
            source: text(props, keys::SOURCE),
            created: Some(text(props, keys::CREATED))
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| text(props, keys::CAPTURED)),
            resurface_on: props
                .get(keys::RESURFACE_ON)
                .and_then(Value::as_str)
                .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()),
            properties: carried(props),
        })
    }

    /// Whether it is in the inbox `today`: not put off past it.
    #[must_use]
    pub fn is_due(&self, today: NaiveDate) -> bool {
        self.resurface_on.is_none_or(|d| d <= today)
    }

    /// What it is, at a glance, for a deck or a list: its title, else its
    /// body's first line.
    #[must_use]
    pub fn label(&self) -> &str {
        if self.title.is_empty() {
            self.body.lines().next().unwrap_or_default()
        } else {
            &self.title
        }
    }
}

/// A capture's properties without the inbox's own keys: what it carries
/// to a note filed or a target's thing.
#[must_use]
pub fn carried(props: &Map<String, Value>) -> Map<String, Value> {
    props
        .iter()
        .filter(|(k, _)| !keys::ALL.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// The note `text` captures as: its first line read by the quick-add
/// parser (its tokens properties, the rest the title), the lines after it
/// the body, the hand-set `chips` over the parsed properties. `None` when
/// it says nothing.
#[must_use]
pub fn note_for(
    folder: &str,
    text: &str,
    chips: &Map<String, Value>,
    source: &str,
    created: &str,
    draft_of: impl FnOnce(&str) -> Draft,
) -> Option<NewNote> {
    let text = text.trim();
    let (first, rest) = text.split_once('\n').unwrap_or((text, ""));
    let draft = draft_of(first);
    let body = rest.trim().to_owned();
    let title = if draft.title.is_empty() {
        // Only tokens on the first line: the body names it, else the line.
        body.lines()
            .next()
            .map_or_else(|| first.trim().to_owned(), str::to_owned)
    } else {
        draft.title.clone()
    };
    if title.trim().is_empty() {
        return None;
    }
    let mut properties = Map::new();
    properties.insert(keys::TYPE.to_owned(), Value::String(KIND.to_owned()));
    properties.insert(keys::STATUS.to_owned(), Value::String("open".to_owned()));
    properties.insert(keys::SOURCE.to_owned(), Value::String(source.to_owned()));
    properties.insert(keys::CREATED.to_owned(), Value::String(created.to_owned()));
    properties.extend(parse::merge(draft.properties(), chips));
    let folder = folder.trim_matches('/');
    let name = file_name(&title);
    let path = if folder.is_empty() {
        format!("{name}.md")
    } else {
        format!("{folder}/{name}.md")
    };
    Some(NewNote {
        path,
        title,
        properties,
        body,
    })
}

/// A title as a file name: what a vault and every OS take (no `/ \ : * ?
/// " < > |`, no leading dot), at most 80 characters.
#[must_use]
pub fn file_name(title: &str) -> String {
    let clean: String = title
        .chars()
        .map(|c| {
            if "/\\:*?\"<>|#^[]".contains(c) || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let words: Vec<&str> = clean.split_whitespace().collect();
    let joined = words.join(" ");
    let trimmed = joined.trim_start_matches('.');
    let name: String = trimmed.chars().take(80).collect();
    if name.trim().is_empty() {
        "Capture".to_owned()
    } else {
        name.trim().to_owned()
    }
}

/// Where a capture filed into `folder` goes: its name kept.
#[must_use]
pub fn filed_path(path: &str, folder: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    let folder = folder.trim_matches('/');
    if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

/// The capture is a task's when a token only a task has was given: a
/// project, a context, a date, a repeat or a priority. Otherwise a note.
/// What the composer suggests first (Task's `kind_from_text`).
#[must_use]
pub fn suggests_task(props: &Map<String, Value>) -> bool {
    use crate::parse::keys as k;
    [k::PROJECTS, k::CONTEXTS, k::DUE, k::RECURRENCE]
        .iter()
        .any(|key| props.contains_key(*key))
        || props
            .get(k::PRIORITY)
            .and_then(Value::as_str)
            .is_some_and(|p| p != "normal")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 5).unwrap()
    }

    fn note(text: &str, chips: &Value) -> Option<NewNote> {
        note_for(
            FOLDER,
            text,
            chips.as_object().unwrap(),
            "window",
            "2026-11-05T10:00:00-08:00",
            |l| parse::parse(l, today()),
        )
    }

    #[test]
    fn a_capture_is_an_inbox_note_its_tokens_its_properties() {
        let n = note("Call the venue [[Album]] tomorrow\nAsk about load-in", &json!({})).unwrap();
        assert_eq!(n.path, "Inbox/Call the venue.md");
        assert_eq!(n.body, "Ask about load-in");
        assert_eq!(n.properties["type"], json!("inbox-item"));
        assert_eq!(n.properties["status"], json!("open"));
        assert_eq!(n.properties["source"], json!("window"));
        assert_eq!(n.properties["projects"], json!(["[[Album]]"]));
        assert_eq!(n.properties["due"], json!("2026-11-06"));
        assert!(note("   ", &json!({})).is_none());
        assert!(suggests_task(&n.properties));
        assert!(!suggests_task(&note("A thought", &json!({})).unwrap().properties));
    }

    #[test]
    fn hand_set_chips_beat_the_text() {
        let n = note("Mix !low", &json!({ "priority": "high" })).unwrap();
        assert_eq!(n.properties["priority"], json!("high"));
    }

    #[test]
    fn read_back_open_captures_only() {
        let read = |props: Value| {
            Item::from_note(&json!({
                "path": "Inbox/Idea.md", "title": "Idea", "body": "\nmore\n",
                "properties": props,
            }))
        };
        let item = read(json!({
            "type": "inbox-item", "status": "open", "source": "mcp",
            "created": "2026-11-01T09:00:00Z", "tags": ["x"], "resurface_on": "2026-11-09",
        }))
        .unwrap();
        assert_eq!(item.body, "more");
        assert_eq!(item.properties, *json!({ "tags": ["x"] }).as_object().unwrap());
        assert!(!item.is_due(today()), "put off until the 9th");
        assert!(read(json!({})).is_some(), "a note dropped in by hand");
        assert!(read(json!({ "type": "task" })).is_none());
        assert!(read(json!({ "type": "inbox-item", "status": "processed" })).is_none());
        // The demo vault's convention.
        let alt = read(json!({
            "type": "capture", "captured": "2026-11-01T17:30:00", "processed": false,
            "source": "quick-add", "title": "Gift idea",
        }))
        .unwrap();
        assert_eq!(alt.created, "2026-11-01T17:30:00");
        assert_eq!(alt.properties, *json!({ "title": "Gift idea" }).as_object().unwrap());
        assert!(read(json!({ "type": "capture", "processed": true })).is_none());
    }

    #[test]
    fn names_and_filing() {
        assert_eq!(file_name("a/b: c?"), "a b c");
        assert_eq!(file_name("..."), "Capture");
        assert_eq!(filed_path("Inbox/Idea.md", "Notes/"), "Notes/Idea.md");
        assert_eq!(filed_path("Inbox/Idea.md", ""), "Idea.md");
    }
}
