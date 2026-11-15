//! The query engine: a base's view run over the vault index, the same
//! code on every client (a desk, a phone, a browser), and the one
//! implementation of `vault_proto::query::VaultQuery`.
//!
//! It reads an index ([`IndexFeed`]): a desk's or a phone's kept from its
//! own vault, a browser's synced from the node. Never a note's body.
//!
//! **Live.** Every answer carries the index's `revision` and a `hash` of
//! what it says. A view redraws on the document events and asks again
//! with `if_changed: <hash>`: the same answer is `{unchanged: true}`.
//!
//! **Memory.** It keeps no rows: a query makes one per note from the
//! index's entries (sharing their properties), runs its view, and lets
//! them go. What a client holds between queries is its index alone. A
//! query that asks with `if_changed` on an index that has not moved makes
//! none.

use std::collections::HashMap;
use std::hash::Hasher;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use vault_proto::query::{QueryError, QueryRequest, VaultQuery};

use crate::base_json;
use crate::bases::{BaseRow, ParsedBase};
use crate::index::{Built, Entry, VaultIndex};

/// Where the engine's index comes from: a desk's builder over its vault, a
/// browser's replica.
pub trait IndexFeed: architect::MaybeSendSync + 'static {
    /// The index now.
    fn index(&self) -> Arc<VaultIndex>;
}

/// A note's entry as a base's row: its file's facts, its properties, its
/// tags (the frontmatter's and inline) and its links (the frontmatter's
/// and the body's).
#[must_use]
pub fn row_of(entry: &Entry) -> BaseRow {
    let (folder, basename) = crate::rows::split_rel(&entry.path);
    let ext = entry
        .path
        .rsplit_once('.')
        .map(|(_, e)| e.to_owned())
        .unwrap_or_default();
    let tags: Vec<String> = entry.tags.iter().map(ToString::to_string).collect();
    BaseRow::from_shared(
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, entry.path.as_bytes()),
        basename,
        &*entry.path,
        folder,
        ext,
        Arc::clone(&entry.props),
        &tags,
    )
    .with_links(entry.links.iter().map(|l| &**l))
    .at(entry.mtime)
}

/// The query engine over an [`IndexFeed`]. Cheap to clone; clones share
/// their rows.
#[derive(Clone)]
pub struct QueryEngine {
    feed: Arc<dyn IndexFeed>,
    /// Each query's last answer's hash, and the revision it was at: asked
    /// again with that hash on an index that has not moved, the answer is
    /// `unchanged` with nothing run.
    answered: Arc<Mutex<HashMap<String, (u64, String)>>>,
}

impl QueryEngine {
    #[must_use]
    pub fn new(feed: Arc<dyn IndexFeed>) -> Self {
        Self {
            feed,
            answered: Arc::default(),
        }
    }

    /// A row for each note of `index`, made now (they share its entries'
    /// properties) and let go after the query.
    fn rows(index: &VaultIndex) -> Vec<BaseRow> {
        index
            .entries
            .values()
            .filter(|e| !e.is_base())
            .map(|e| row_of(e))
            .collect()
    }

    /// Runs `query`: the base's views, the view run (`{name, kind,
    /// columns, groups: [{label, rows: [{path, name, values, formulas}]}]}`),
    /// `total`, every column it could show, its formulas and filters, the
    /// index's `revision` and the answer's `hash`.
    ///
    /// # Errors
    ///
    /// Neither `path` nor `source`, no such base or view, YAML that does
    /// not parse.
    pub fn run(&self, query: &QueryRequest) -> Result<Value, String> {
        let text = self.run_text(query)?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    /// [`Self::run`] as JSON text: the answer serialized once, its hash
    /// that text's, `revision` and `hash` put in front of it. What the
    /// service answers.
    ///
    /// # Errors
    ///
    /// As [`Self::run`].
    pub fn run_text(&self, query: &QueryRequest) -> Result<String, String> {
        let index = self.feed.index();
        let key = format!(
            "{:?}|{:?}|{:?}|{:?}",
            query.path, query.source, query.view, query.limit
        );
        let unchanged = |revision: u64, hash: &str| {
            json!({ "unchanged": true, "revision": revision, "hash": hash }).to_string()
        };
        if let Some(asked) = query.if_changed.as_deref() {
            let answered = self.answered.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((rev, hash)) = answered.get(&key) {
                if *rev == index.revision && hash == asked {
                    return Ok(unchanged(*rev, hash));
                }
            }
        }
        let path = query.path.as_deref().unwrap_or_default();
        let yaml = match query.source.as_deref() {
            Some(source) => source,
            None if path.is_empty() => {
                return Err("give a base's `path` or its `source`".to_owned());
            }
            None => index
                .base(path)
                .ok_or_else(|| format!("{path}: no such base"))?,
        };
        let base: ParsedBase =
            base_json::with_a_view(crate::bases::parse(yaml).map_err(|e| format!("{path}: {e}"))?);
        let i = base_json::view_index(&base, query.view.as_deref())?;
        let spec = base.views.get(i).ok_or("no such view")?;
        let revision = index.revision;
        let rows = Self::rows(&index);
        let keys = base_json::property_keys(&rows);
        let limit = query
            .limit
            .map_or(usize::MAX, |n| usize::try_from(n).unwrap_or(usize::MAX));
        let (table, total) = crate::bases::view_table_at_most(&base, spec, &rows, limit);
        drop(rows);
        let (join, clauses) = crate::bases::filter_clauses(&base.global_filter);
        let answer = json!({
            "base": path,
            "views": base.views.iter().map(|v| json!({ "name": v.name, "type": v.kind.as_str() })).collect::<Vec<_>>(),
            "view": base_json::view_json(spec),
            "rows": table,
            "total": total,
            "columns": base_json::columns(&base, &keys),
            "formulas": base.formulas.iter().map(|f| json!({ "name": f.name, "expression": f.expression })).collect::<Vec<_>>(),
            "filters": { "join": join, "clauses": clauses },
        })
        .to_string();
        let hash = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            h.write(answer.as_bytes());
            format!("{:016x}", h.finish())
        };
        {
            let mut answered = self.answered.lock().unwrap_or_else(PoisonError::into_inner);
            // Bounded: the queries a session's views ask.
            if answered.len() > 1024 {
                answered.clear();
            }
            answered.insert(key, (revision, hash.clone()));
        }
        if query.if_changed.as_deref() == Some(hash.as_str()) {
            return Ok(unchanged(revision, &hash));
        }
        // `{"revision":…,"hash":"…",` and the answer's own fields after.
        let rest = answer.strip_prefix('{').unwrap_or(&answer);
        Ok(format!(
            "{{\"revision\":{revision},\"hash\":\"{hash}\",{rest}"
        ))
    }
}

impl VaultQuery for QueryEngine {
    async fn query(&self, request: QueryRequest) -> Result<String, QueryError> {
        self.run_text(&request).map_err(QueryError::Refused)
    }
}

impl architect::Services for QueryEngine {
    fn layers() -> impl architect::Layer<Self> {
        architect::layers![vault_proto::query::VaultQueryService]
    }
}

/// An index kept from a snapshot and the deltas after it: a browser's,
/// synced from the node. An [`IndexFeed`].
#[derive(Default)]
pub struct Replica(Mutex<Arc<VaultIndex>>);

impl Replica {
    /// From a snapshot's bytes (`VaultIndex::to_bytes`).
    ///
    /// # Errors
    ///
    /// They are not one.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        Ok(Self(Mutex::new(Arc::new(VaultIndex::from_bytes(bytes)?))))
    }

    /// Applies a delta's bytes (`IndexDelta::to_bytes`), in place (a query
    /// running meanwhile keeps the index it had).
    ///
    /// # Errors
    ///
    /// They are not one, or it is from another history or revision (take
    /// a snapshot).
    pub fn apply(&self, bytes: &[u8]) -> Result<(), String> {
        let delta = crate::index::IndexDelta::from_bytes(bytes)?;
        let mut now = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let result = Arc::make_mut(&mut now).apply_interned(delta);
        drop(now);
        result
    }

    /// Takes a new snapshot's bytes in place of what it had: what a
    /// client does when `since` answers "take a snapshot". The engine over
    /// it sees the new index at once.
    ///
    /// # Errors
    ///
    /// They are not one; the index it had stays.
    pub fn reset(&self, bytes: &[u8]) -> Result<(), String> {
        let next = VaultIndex::from_bytes(bytes)?;
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Arc::new(next);
        Ok(())
    }

    /// Its history and revision: what to ask `since` with.
    #[must_use]
    pub fn at(&self) -> (u64, u64) {
        let now = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        (now.epoch, now.revision)
    }

    /// Its revision.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.at().1
    }
}

impl IndexFeed for Replica {
    fn index(&self) -> Arc<VaultIndex> {
        Arc::clone(&self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl<S: crate::index::NoteSource + architect::MaybeSendSync + 'static> IndexFeed for Built<S> {
    fn index(&self) -> Arc<VaultIndex> {
        Self::index(self)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;
    use crate::index::{Builder, Note, NoteSource, Text};

    struct Notes(Vec<(&'static str, &'static str, u64)>);

    impl NoteSource for Notes {
        fn revision(&self) -> u64 {
            0
        }
        fn visit(&self, from: usize, n: usize, each: &mut dyn FnMut(Note<'_>)) -> (u64, usize) {
            for (path, text, mtime) in self.0.iter().skip(from).take(n) {
                each(Note {
                    path,
                    stamp: 1,
                    text: Text::Here(text),
                    parsed: None,
                    mtime: *mtime,
                    is_base: false,
                });
            }
            (0, self.0.len())
        }
    }

    fn engine(notes: Vec<(&'static str, &'static str, u64)>) -> QueryEngine {
        let mut b = Builder::new();
        let _ = b.sync(&Notes(notes));
        let replica = Replica::from_bytes(&b.index().to_bytes()).unwrap();
        QueryEngine::new(Arc::new(replica))
    }

    /// A row keeps its note's time: `file.mtime` sorts and shows as a date.
    #[test]
    fn a_row_has_its_notes_time() {
        let e = engine(vec![
            ("A.md", "# A", 1_700_000_000_000),
            ("B.md", "# B", 1_600_000_000_000),
        ]);
        let q = QueryRequest {
            source: Some("views:\n  - type: table\n    name: T\n    order: [file.name, file.mtime]\n    sort:\n      - property: file.mtime\n        direction: ASC\n".to_owned()),
            ..QueryRequest::default()
        };
        let a = e.run(&q).unwrap();
        let rows = &a["rows"]["groups"][0]["rows"];
        assert_eq!(rows[0]["name"], "B", "{a}");
        assert!(
            rows[0]["values"]["file.mtime"]
                .as_str()
                .unwrap()
                .starts_with("2020-09-13"),
            "{a}"
        );
    }

    /// One serialization: the text answer reads back as the value answer,
    /// with its revision and hash in front, and `unchanged` after.
    #[test]
    fn the_answer_is_written_once() {
        let e = engine(vec![("A.md", "---\nstatus: x\n---\n", 1)]);
        let q = QueryRequest {
            source: Some("views:\n  - type: table\n    name: T\n".to_owned()),
            ..QueryRequest::default()
        };
        let text = e.run_text(&q).unwrap();
        assert!(text.starts_with("{\"revision\":"), "{text}");
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["total"], 1);
        let again = QueryRequest {
            if_changed: v["hash"].as_str().map(str::to_owned),
            ..q
        };
        assert_eq!(e.run(&again).unwrap()["unchanged"], true);
    }
}
