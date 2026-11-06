//! Threads as vault data: one note per thread under [`FOLDER`].
//!
//! Its conversation is in the frontmatter (Task's `threads-proto` `Thread` and
//! `Message`, `task:features/threads/threads-proto/src/thread.rs`, as
//! properties).
//!
//! A thread is anchored to any entity by `(entityType, entityId)`, as in
//! Task: a vault note is `("note", <its path>)`, and a project, a task, an
//! event or a shared page is whatever its owner calls it. `anchorPath` is
//! a place inside the entity (a heading slug, `entries/<id>`), opaque here.
//!
//! The note is the store: it syncs as every note does (live between peers,
//! the sweep as fallback, the headless node holds it), and two devices
//! that post at once each append a row to `comments`, which the note's
//! document merges. Nothing here does I/O; `crate::store` reads and writes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Where threads are kept.
pub const FOLDER: &str = "Threads";

/// The `type` a thread's note has.
pub const TYPE: &str = "thread";

/// The entity type of a vault note: its id is the note's path.
pub const NOTE: &str = "note";

/// Kinds a thread can take (Task's `THREAD_KINDS`); unknown ones are kept.
pub const KINDS: &[&str] = &["discussion", "question", "decision", "action", "praise"];

/// Its frontmatter keys.
pub mod keys {
    pub const TYPE: &str = "type";
    pub const THREAD: &str = "thread";
    pub const ENTITY_TYPE: &str = "entityType";
    pub const ENTITY_ID: &str = "entityId";
    pub const ANCHOR: &str = "anchorPath";
    pub const TITLE: &str = "title";
    pub const KIND: &str = "kind";
    pub const RESOLVED: &str = "resolved";
    pub const RESOLVED_BY: &str = "resolvedBy";
    pub const RESOLVED_AT: &str = "resolvedAt";
    pub const CREATED_BY: &str = "createdBy";
    pub const CREATED: &str = "created";
    pub const COMMENTS: &str = "comments";

    /// Every key, in the order a base lists them.
    pub const ALL: &[&str] = &[
        THREAD,
        ENTITY_TYPE,
        ENTITY_ID,
        ANCHOR,
        TITLE,
        KIND,
        RESOLVED,
        RESOLVED_BY,
        RESOLVED_AT,
        CREATED_BY,
        CREATED,
        COMMENTS,
    ];
}

/// What a thread is about: an entity, and a place inside it.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Anchor {
    pub entity_type: String,
    pub entity_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_path: Option<String>,
}

impl Anchor {
    /// A vault note, as a whole.
    #[must_use]
    pub fn note(path: &str) -> Self {
        Self {
            entity_type: NOTE.to_owned(),
            entity_id: path.to_owned(),
            anchor_path: None,
        }
    }

    /// The entity's key in counts: `<type>:<id>`.
    #[must_use]
    pub fn key(&self) -> String {
        key(&self.entity_type, &self.entity_id)
    }
}

/// An entity's key in [`counts`]: `<type>:<id>`.
#[must_use]
pub fn key(entity_type: &str, entity_id: &str) -> String {
    format!("{entity_type}:{entity_id}")
}

/// An anchor path as stored: surrounding slashes and whitespace dropped,
/// `None` when nothing is left (Task's `anchor_path`).
#[must_use]
pub fn anchor_path(path: Option<&str>) -> Option<String> {
    path.map(|p| p.trim().trim_matches('/'))
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
}

/// Whether a thread at `held` sits at `place` or under it; an empty
/// `place` holds everything (Task's `anchor_within`).
#[must_use]
pub fn anchor_within(held: Option<&str>, place: &str) -> bool {
    let place = place.trim().trim_matches('/');
    place.is_empty()
        || held.is_some_and(|h| {
            h == place || (h.starts_with(place) && h.as_bytes().get(place.len()) == Some(&b'/'))
        })
}

/// One comment in a thread.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    /// Who wrote it, as people mention them (`cody`).
    pub author: String,
    /// Markdown.
    pub body: String,
    /// The comment it answers, in this thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    /// Who it mentions (`@name`), as written.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<String>,
    /// RFC 3339.
    pub posted: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited: Option<String>,
}

/// A thread: its note's path, its anchor, its comments oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
    /// Its note.
    pub path: String,
    #[serde(flatten)]
    pub anchor: Anchor,
    pub title: String,
    pub kind: String,
    pub resolved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<String>,
    pub created_by: String,
    pub created: String,
    pub comments: Vec<Comment>,
}

fn text(values: &Map<String, Value>, key: &str) -> Option<String> {
    match values.get(key)? {
        Value::String(s) => Some(s.clone()).filter(|s| !s.is_empty()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

impl Thread {
    /// The thread a note's properties hold: `None` when it is not one.
    #[must_use]
    pub fn from_values(path: &str, values: &Map<String, Value>) -> Option<Self> {
        let id = text(values, keys::THREAD)?;
        let entity_id = text(values, keys::ENTITY_ID)?;
        let mut comments: Vec<Comment> = values
            .get(keys::COMMENTS)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|c| serde_json::from_value::<Comment>(stringly(c)).ok())
            .collect();
        comments.sort_by(|a, b| a.posted.cmp(&b.posted));
        // Two devices that posted the same row (a retried write) show it once.
        let mut seen = std::collections::BTreeSet::new();
        comments.retain(|c| seen.insert(c.id.clone()));
        Some(Self {
            id,
            path: path.to_owned(),
            anchor: Anchor {
                entity_type: text(values, keys::ENTITY_TYPE).unwrap_or_else(|| NOTE.to_owned()),
                entity_id,
                anchor_path: anchor_path(text(values, keys::ANCHOR).as_deref()),
            },
            title: text(values, keys::TITLE).unwrap_or_default(),
            kind: text(values, keys::KIND).unwrap_or_else(|| "discussion".to_owned()),
            resolved: matches!(values.get(keys::RESOLVED), Some(Value::Bool(true)))
                || text(values, keys::RESOLVED).as_deref() == Some("true"),
            resolved_by: text(values, keys::RESOLVED_BY),
            resolved_at: text(values, keys::RESOLVED_AT),
            created_by: text(values, keys::CREATED_BY).unwrap_or_default(),
            created: text(values, keys::CREATED).unwrap_or_default(),
            comments,
        })
    }

    /// Its frontmatter, every key (`null` for one it has not).
    #[must_use]
    pub fn to_values(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert(keys::TYPE.to_owned(), json!(TYPE));
        m.insert(keys::THREAD.to_owned(), json!(self.id));
        m.insert(keys::ENTITY_TYPE.to_owned(), json!(self.anchor.entity_type));
        m.insert(keys::ENTITY_ID.to_owned(), json!(self.anchor.entity_id));
        m.insert(keys::ANCHOR.to_owned(), json!(self.anchor.anchor_path));
        m.insert(keys::TITLE.to_owned(), json!(self.title));
        m.insert(keys::KIND.to_owned(), json!(self.kind));
        m.insert(keys::RESOLVED.to_owned(), json!(self.resolved));
        m.insert(keys::RESOLVED_BY.to_owned(), json!(self.resolved_by));
        m.insert(keys::RESOLVED_AT.to_owned(), json!(self.resolved_at));
        m.insert(keys::CREATED_BY.to_owned(), json!(self.created_by));
        m.insert(keys::CREATED.to_owned(), json!(self.created));
        m.insert(keys::COMMENTS.to_owned(), comments_value(&self.comments));
        m
    }

    /// The last comment's time, or when it was opened.
    #[must_use]
    pub fn last_activity(&self) -> &str {
        self.comments
            .last()
            .map_or(self.created.as_str(), |c| c.posted.as_str())
    }

    /// Everyone in it: whoever opened it, wrote in it or was mentioned.
    #[must_use]
    pub fn participants(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut add = |who: &str| {
            if !who.is_empty() && !out.iter().any(|o| same_person(o, who)) {
                out.push(who.to_owned());
            }
        };
        add(&self.created_by);
        for c in &self.comments {
            add(&c.author);
            for m in &c.mentions {
                add(m);
            }
        }
        out
    }
}

/// A `comments` row's values as strings where YAML read a date or a
/// number (`posted: 2026-11-06T…` comes back as text either way).
fn stringly(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| {
                    let v = match v {
                        Value::Number(n) => Value::String(n.to_string()),
                        other => other.clone(),
                    };
                    (k.clone(), v)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

/// `comments` as written to the note.
#[must_use]
pub fn comments_value(comments: &[Comment]) -> Value {
    Value::Array(
        comments
            .iter()
            .map(|c| serde_json::to_value(c).unwrap_or(Value::Null))
            .collect(),
    )
}

/// Whether two names are one person: case aside, and a leading `@`.
#[must_use]
pub fn same_person(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.trim().trim_start_matches('@').to_lowercase();
    !a.trim().is_empty() && norm(a) == norm(b)
}

fn name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.')
}

/// The people a comment mentions: `@name` (letters, digits, `_`, `-`,
/// `.`; not an email's `a@b`) and `@[[Name]]`, each once, as written.
#[must_use]
pub fn mentions(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |name: &str| {
        let name = name.trim().trim_end_matches(['.', '-']);
        if !name.is_empty() && !out.iter().any(|o| same_person(o, name)) {
            out.push(name.to_owned());
        }
    };
    let mut prev: Option<char> = None;
    for (i, c) in body.char_indices() {
        if c == '@' && !prev.is_some_and(name_char) {
            let after = body.get(i.saturating_add(1)..).unwrap_or_default();
            if let Some(link) = after.strip_prefix("[[") {
                if let Some((target, _)) = link.split_once("]]") {
                    let shown = target.rsplit_once('|').map_or(target, |(_, s)| s);
                    push(shown.rsplit('/').next().unwrap_or(shown));
                }
            } else {
                let name = after
                    .split(|c: char| !name_char(c))
                    .next()
                    .unwrap_or_default();
                push(name);
            }
        }
        prev = Some(c);
    }
    out
}

/// A note-name-safe slug of `s`, at most 40 characters.
#[must_use]
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.chars().count() >= 40 {
            break;
        }
    }
    out.trim_end_matches('-').to_owned()
}

/// Where a new thread on `anchor` with id `id` is kept:
/// `Threads/<entity's name>-<first 8 of id>.md`.
#[must_use]
pub fn path_for(anchor: &Anchor, id: &str) -> String {
    let name = anchor
        .entity_id
        .rsplit('/')
        .next()
        .map(|n| n.strip_suffix(".md").unwrap_or(n))
        .unwrap_or_default();
    let name = match slug(name) {
        s if s.is_empty() => slug(&anchor.entity_type),
        s => s,
    };
    let short: String = id.chars().filter(|c| *c != '-').take(8).collect();
    format!("{FOLDER}/{name}-{short}.md")
}

/// The threads on `entity` (at `place` or under it, when given), newest
/// activity first; resolved ones only when asked.
#[must_use]
pub fn on<'a>(
    threads: &'a [Thread],
    entity_type: &str,
    entity_id: &str,
    place: Option<&str>,
    resolved: bool,
) -> Vec<&'a Thread> {
    let mut out: Vec<&Thread> = threads
        .iter()
        .filter(|t| t.anchor.entity_type == entity_type && t.anchor.entity_id == entity_id)
        .filter(|t| place.is_none_or(|p| anchor_within(t.anchor.anchor_path.as_deref(), p)))
        .filter(|t| resolved || !t.resolved)
        .collect();
    out.sort_by(|a, b| b.last_activity().cmp(a.last_activity()));
    out
}

/// Open threads and comments in them per entity ([`key`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Count {
    /// Threads not resolved.
    pub open: usize,
    /// Every thread.
    pub threads: usize,
    /// Comments in the open ones.
    pub comments: usize,
}

/// Each entity's [`Count`], by [`key`].
#[must_use]
pub fn counts(threads: &[Thread]) -> BTreeMap<String, Count> {
    let mut out: BTreeMap<String, Count> = BTreeMap::new();
    for t in threads {
        let c = out.entry(t.anchor.key()).or_default();
        c.threads = c.threads.saturating_add(1);
        if !t.resolved {
            c.open = c.open.saturating_add(1);
            c.comments = c.comments.saturating_add(t.comments.len());
        }
    }
    out
}

/// Where `who` is mentioned: each thread and comment, newest first.
#[must_use]
pub fn mentions_of<'a>(threads: &'a [Thread], who: &str) -> Vec<(&'a Thread, &'a Comment)> {
    let mut out: Vec<(&Thread, &Comment)> = threads
        .iter()
        .flat_map(|t| t.comments.iter().map(move |c| (t, c)))
        .filter(|(_, c)| c.mentions.iter().any(|m| same_person(m, who)))
        .collect();
    out.sort_by(|a, b| b.1.posted.cmp(&a.1.posted));
    out
}

/// A comment's first line, at most `max` characters: a notification's body.
#[must_use]
pub fn excerpt(body: &str, max: usize) -> String {
    let line = body
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.chars().count() <= max {
        return line.to_owned();
    }
    let mut out: String = line.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn mentions_are_names_after_an_at_once_each() {
        assert_eq!(
            mentions("@alex can you look? cc @Sam. and @alex again, not me@mail.com"),
            vec!["alex", "Sam"]
        );
        assert_eq!(mentions("ask @[[People/Jo Smith|Jo]]"), vec!["Jo"]);
        assert_eq!(mentions("@[[Robin]] and @robin"), vec!["Robin"]);
        assert!(mentions("no one, @ alone").is_empty());
    }

    #[test]
    fn a_path_is_trimmed_and_a_place_holds_what_is_under_it() {
        assert_eq!(anchor_path(Some("/entries/a/")), Some("entries/a".into()));
        assert_eq!(anchor_path(Some(" / ")), None);
        assert!(anchor_within(Some("entries/a/notes"), "/entries/a/"));
        assert!(!anchor_within(Some("entries/ab"), "entries/a"));
        assert!(anchor_within(None, ""));
    }

    fn thread(id: &str, entity: &str, resolved: bool, posted: &[&str]) -> Thread {
        Thread {
            id: id.to_owned(),
            path: format!("Threads/{id}.md"),
            anchor: Anchor::note(entity),
            title: "t".to_owned(),
            kind: "discussion".to_owned(),
            resolved,
            created_by: "cody".to_owned(),
            created: "2026-11-01T00:00:00Z".to_owned(),
            comments: posted
                .iter()
                .enumerate()
                .map(|(i, p)| Comment {
                    id: format!("{id}-{i}"),
                    author: "cody".to_owned(),
                    body: "hi @alex".to_owned(),
                    mentions: vec!["alex".to_owned()],
                    posted: (*p).to_owned(),
                    ..Comment::default()
                })
                .collect(),
            ..Thread::default()
        }
    }

    #[test]
    fn a_thread_round_trips_through_its_properties() {
        let t = thread("a", "Notes/Plan.md", false, &["2026-11-02T10:00:00Z"]);
        let values = t.to_values();
        assert_eq!(values["type"], "thread");
        let back = Thread::from_values(&t.path, &values).unwrap();
        assert_eq!(back, t);
        // Not a thread: no id.
        assert!(Thread::from_values("x.md", &Map::new()).is_none());
    }

    #[test]
    fn counts_and_lists_follow_the_anchor() {
        let threads = vec![
            thread("a", "Notes/Plan.md", false, &["2026-11-02T10:00:00Z"]),
            thread("b", "Notes/Plan.md", true, &["2026-11-03T10:00:00Z"]),
            thread(
                "c",
                "Notes/Other.md",
                false,
                &["2026-11-04T10:00:00Z", "2026-11-05T10:00:00Z"],
            ),
        ];
        let c = counts(&threads);
        assert_eq!(
            c["note:Notes/Plan.md"],
            Count {
                open: 1,
                threads: 2,
                comments: 1
            }
        );
        assert_eq!(c["note:Notes/Other.md"].comments, 2);
        assert_eq!(on(&threads, NOTE, "Notes/Plan.md", None, false).len(), 1);
        let all = on(&threads, NOTE, "Notes/Plan.md", None, true);
        assert_eq!(all[0].id, "b", "newest activity first");
        let m = mentions_of(&threads, "@Alex");
        assert_eq!(m.len(), 4);
        assert_eq!(m[0].1.posted, "2026-11-05T10:00:00Z");
    }

    #[test]
    fn a_new_thread_is_named_after_its_entity() {
        let a = Anchor::note("Projects/Album/Mix Notes.md");
        assert_eq!(path_for(&a, "1234abcd-ef"), "Threads/mix-notes-1234abcd.md");
        let p = Anchor {
            entity_type: "project".into(),
            entity_id: String::new(),
            anchor_path: None,
        };
        assert_eq!(path_for(&p, "ab"), "Threads/project-ab.md");
        assert_eq!(
            excerpt("\n  first line that is long\nsecond", 10),
            "first lin…"
        );
    }
}
