//! Notifications as vault data: each person's in one note.
//!
//! That note is `Notifications/<name>.md`, the rows in its frontmatter's `items`
//! (Task's `notify-proto` `Notification`,
//! `task:features/notify/notify-proto/src/model.rs`, as properties).
//!
//! The note is the delivery: a mention written on one device lands in
//! the person's note, which syncs to their other devices (live between
//! peers, the sweep as fallback, through the headless node) like any
//! note. Two devices that add a row at once each append one, and the
//! note's document merges them. Nothing here does I/O.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Where each person's notifications are kept.
pub const FOLDER: &str = "Notifications";

/// The `type` a person's notifications note has.
pub const TYPE: &str = "notifications";

/// The most rows a note keeps: past it, the oldest read ones go first.
pub const KEEP: usize = 200;

/// Its frontmatter keys.
pub mod keys {
    pub const TYPE: &str = "type";
    pub const FOR: &str = "for";
    pub const ITEMS: &str = "items";
}

/// What kind of news a notification carries: its source's type. The
/// centre keys its mark off it; a kind it does not know shows as
/// [`Kind::Other`] and keeps its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// Someone @mentioned you in a thread.
    Mention,
    /// Someone replied in a thread you are in.
    ThreadReply,
    /// A task was assigned to you.
    TaskAssigned,
    /// A task of yours is due.
    TaskDue,
    /// A task was done.
    TaskCompleted,
    /// Someone opened a link you shared.
    ShareOpened,
    /// A device was enrolled in the org.
    DeviceEnrolled,
    /// A toast an extension showed (kept for the session, not stored).
    Toast,
    Other,
}

impl Kind {
    /// Every kind, in the order the centre's filter lists them.
    pub const ALL: &[Self] = &[
        Self::Mention,
        Self::ThreadReply,
        Self::TaskAssigned,
        Self::TaskDue,
        Self::TaskCompleted,
        Self::ShareOpened,
        Self::DeviceEnrolled,
        Self::Toast,
        Self::Other,
    ];

    /// Its kebab-case name: the stored value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mention => "mention",
            Self::ThreadReply => "thread-reply",
            Self::TaskAssigned => "task-assigned",
            Self::TaskDue => "task-due",
            Self::TaskCompleted => "task-completed",
            Self::ShareOpened => "share-opened",
            Self::DeviceEnrolled => "device-enrolled",
            Self::Toast => "toast",
            Self::Other => "other",
        }
    }

    /// The kind named `s`; an unknown name is [`Kind::Other`].
    #[must_use]
    pub fn parse(s: &str) -> Self {
        Self::ALL
            .iter()
            .copied()
            .find(|k| k.as_str() == s)
            .unwrap_or(Self::Other)
    }

    /// What the centre shows for it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Mention => "Mention",
            Self::ThreadReply => "Reply",
            Self::TaskAssigned => "Assigned",
            Self::TaskDue => "Due",
            Self::TaskCompleted => "Done",
            Self::ShareOpened => "Share",
            Self::DeviceEnrolled => "Device",
            Self::Toast => "Toast",
            Self::Other => "Notice",
        }
    }
}

/// Where a notification came from: enough to go to it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Source {
    /// The producing service (`architect.threads`, `architect.tasks`): the
    /// centre asks it to `reveal` the entity.
    pub service: String,
    /// The entity in that service's terms (a thread's id, a task's path).
    pub entity: String,
    /// What to open when the service cannot reveal it: a note's path, a
    /// link. Empty: nowhere to go.
    pub href: String,
    /// A place inside the entity (a comment's id).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub comment: String,
}

/// One notification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    /// Its [`Kind`]'s name; kept as written, so a newer kind survives.
    pub kind: String,
    /// One line: "alex mentioned you on Album".
    pub title: String,
    /// A detail line; empty: the title alone.
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub source: Source,
    /// Who caused it; empty: unknown.
    #[serde(default)]
    pub actor: String,
    /// RFC 3339.
    pub created: String,
    /// When it was read (RFC 3339); `None` while unread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<String>,
}

impl Notification {
    #[must_use]
    pub fn kind(&self) -> Kind {
        Kind::parse(&self.kind)
    }

    #[must_use]
    pub const fn unread(&self) -> bool {
        self.read.is_none()
    }
}

/// A name as a note's file name: lower case, letters, digits and `-`.
#[must_use]
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().trim_start_matches('@').chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

/// Person `who`'s note: `Notifications/<slug>.md`; `None` for no name.
#[must_use]
pub fn path_for(who: &str) -> Option<String> {
    let s = slug(who);
    (!s.is_empty()).then(|| format!("{FOLDER}/{s}.md"))
}

/// Whether two names are one person (case and a leading `@` aside).
#[must_use]
pub fn same_person(a: &str, b: &str) -> bool {
    !slug(a).is_empty() && slug(a) == slug(b)
}

/// The rows a note's properties hold, newest first (one id once).
#[must_use]
pub fn items(values: &Map<String, Value>) -> Vec<Notification> {
    let mut out: Vec<Notification> = values
        .get(keys::ITEMS)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect();
    // Written oldest first: within one second, the later row is newer.
    out.reverse();
    out.sort_by(|a, b| b.created.cmp(&a.created));
    let mut seen = std::collections::BTreeSet::new();
    out.retain(|n| seen.insert(n.id.clone()));
    out
}

/// `items` as written, oldest first (so a new row is an append), at most
/// [`KEEP`]: past it the oldest read rows go, then the oldest.
#[must_use]
pub fn items_value(newest_first: &[Notification]) -> Value {
    let mut rows: Vec<&Notification> = newest_first.iter().collect();
    while rows.len() > KEEP {
        let drop = rows
            .iter()
            .rposition(|n| !n.unread())
            .unwrap_or_else(|| rows.len().saturating_sub(1));
        rows.remove(drop);
    }
    rows.reverse();
    Value::Array(
        rows.into_iter()
            .map(|n| serde_json::to_value(n).unwrap_or(Value::Null))
            .collect(),
    )
}

/// A new person's note's properties.
#[must_use]
pub fn new_note(who: &str) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert(keys::TYPE.to_owned(), json!(TYPE));
    m.insert(keys::FOR.to_owned(), json!(who));
    m.insert(keys::ITEMS.to_owned(), json!([]));
    m
}

/// How many are unread.
#[must_use]
pub fn unread(items: &[Notification]) -> usize {
    items.iter().filter(|n| n.unread()).count()
}

/// The ones that arrived unread since `seen` (ids already known).
#[must_use]
pub fn arrived<'a>(
    items: &'a [Notification],
    seen: &std::collections::BTreeSet<String>,
) -> Vec<&'a Notification> {
    items
        .iter()
        .filter(|n| n.unread() && !seen.contains(&n.id))
        .collect()
}

/// What the bell says: `3` unread, or nothing.
#[must_use]
pub fn bell_text(unread: usize) -> String {
    match unread {
        0 => String::new(),
        1..=99 => unread.to_string(),
        _ => "99+".to_owned(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn n(id: &str, created: &str, read: bool) -> Notification {
        Notification {
            id: id.to_owned(),
            kind: "mention".to_owned(),
            title: "t".to_owned(),
            body: String::new(),
            source: Source::default(),
            actor: "alex".to_owned(),
            created: created.to_owned(),
            read: read.then(|| "2026-11-06T00:00:00Z".to_owned()),
        }
    }

    #[test]
    fn kinds_round_trip_and_unknown_is_other() {
        for k in Kind::ALL {
            assert_eq!(Kind::parse(k.as_str()), *k);
        }
        assert_eq!(Kind::parse("from-the-future"), Kind::Other);
        assert_eq!(
            serde_json::to_value(Kind::TaskAssigned).unwrap(),
            "task-assigned"
        );
    }

    #[test]
    fn a_person_is_one_note_whatever_the_case() {
        assert_eq!(path_for("@Cody W."), Some("Notifications/cody-w.md".into()));
        assert_eq!(path_for("  "), None);
        assert!(same_person("@Alex", "alex"));
        assert!(!same_person("", ""));
    }

    #[test]
    fn rows_are_written_oldest_first_and_read_newest_first() {
        let rows = vec![n("b", "2026-11-02", false), n("a", "2026-11-01", true)];
        let v = items_value(&rows);
        assert_eq!(v[0]["id"], "a");
        let mut values = new_note("cody");
        values.insert(keys::ITEMS.to_owned(), v);
        let back = items(&values);
        assert_eq!(back, rows);
        assert_eq!(unread(&back), 1);
        let seen = std::iter::once("a".to_owned()).collect();
        assert_eq!(arrived(&back, &seen).len(), 1);
        assert_eq!(bell_text(0), "");
        assert_eq!(bell_text(120), "99+");
    }

    #[test]
    fn past_the_cap_read_rows_go_first() {
        let mut rows: Vec<Notification> = (0..=KEEP)
            .map(|i| n(&format!("{i:04}"), &format!("2026-{i:04}"), false))
            .collect();
        rows.reverse(); // newest first
        // The oldest is unread; one read row in the middle.
        rows[100].read = Some("x".into());
        let kept = items_value(&rows);
        assert_eq!(kept.as_array().unwrap().len(), KEEP);
        assert!(
            kept.as_array()
                .unwrap()
                .iter()
                .all(|r| r.get("read").is_none())
        );
    }
}
