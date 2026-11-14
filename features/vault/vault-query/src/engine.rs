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
//! **Cost.** The rows are kept between queries, by entry: when the index
//! moves only the entries that changed are made rows again, and a query
//! on an index that has not moved makes none.

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
    BaseRow::from_shared(
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, entry.path.as_bytes()),
        basename,
        entry.path.as_str(),
        folder,
        ext,
        Arc::clone(&entry.props),
        &entry.tags,
    )
    .with_links(&entry.links)
}

/// The rows at one revision of the index.
struct Kept {
    revision: u64,
    made_rows: Arc<Vec<BaseRow>>,
    keys: Arc<Vec<String>>,
    /// Each row's entry, to keep it when the entry is the same.
    made: HashMap<Arc<str>, (Arc<Entry>, usize)>,
}

/// The query engine over an [`IndexFeed`]. Cheap to clone; clones share
/// their rows.
#[derive(Clone)]
pub struct QueryEngine {
    feed: Arc<dyn IndexFeed>,
    rows: Arc<Mutex<Option<Kept>>>,
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
            rows: Arc::default(),
            answered: Arc::default(),
        }
    }

    /// The rows of `index`: those kept when it has not moved; else, when
    /// it has the same notes, the changed entries' rows replaced in place;
    /// else made again, keeping every unchanged entry's row.
    fn rows(&self, index: &VaultIndex) -> (u64, Arc<Vec<BaseRow>>, Arc<Vec<String>>) {
        let mut kept = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(r) = kept.as_ref().filter(|r| r.revision == index.revision) {
            return (r.revision, Arc::clone(&r.made_rows), Arc::clone(&r.keys));
        }
        let notes = || index.entries.iter().filter(|(_, e)| !e.is_base());
        let fresh = match kept.take() {
            // The same notes: only the changed ones' rows are made again.
            Some(mut before)
                if before.made.len() == notes().count()
                    && notes().all(|(p, _)| before.made.contains_key(p)) =>
            {
                let rows = Arc::make_mut(&mut before.made_rows);
                for (path, entry) in notes() {
                    if let Some((had, i)) = before.made.get_mut(path) {
                        if !Arc::ptr_eq(had, entry) {
                            if let Some(row) = rows.get_mut(*i) {
                                *row = row_of(entry);
                            }
                            *had = Arc::clone(entry);
                        }
                    }
                }
                before.keys = Arc::new(base_json::property_keys(rows));
                before.revision = index.revision;
                before
            }
            before => {
                let mut rows = Vec::with_capacity(index.entries.len());
                let mut made = HashMap::with_capacity(index.entries.len());
                for (path, entry) in notes() {
                    let same = before
                        .as_ref()
                        .and_then(|b| b.made.get(path).filter(|(e, _)| Arc::ptr_eq(e, entry)).and_then(|(_, i)| b.made_rows.get(*i)));
                    made.insert(Arc::clone(path), (Arc::clone(entry), rows.len()));
                    rows.push(same.cloned().unwrap_or_else(|| row_of(entry)));
                }
                Kept {
                    revision: index.revision,
                    keys: Arc::new(base_json::property_keys(&rows)),
                    made_rows: Arc::new(rows),
                    made,
                }
            }
        };
        let out = (fresh.revision, Arc::clone(&fresh.made_rows), Arc::clone(&fresh.keys));
        *kept = Some(fresh);
        out
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
        let index = self.feed.index();
        let key = format!("{:?}|{:?}|{:?}|{:?}", query.path, query.source, query.view, query.limit);
        if let Some(asked) = query.if_changed.as_deref() {
            let answered = self.answered.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((rev, hash)) = answered.get(&key) {
                if *rev == index.revision && hash == asked {
                    return Ok(json!({ "unchanged": true, "revision": rev, "hash": hash }));
                }
            }
        }
        let path = query.path.as_deref().unwrap_or_default();
        let yaml = match query.source.as_deref() {
            Some(source) => source,
            None if path.is_empty() => return Err("give a base's `path` or its `source`".to_owned()),
            None => index.base(path).ok_or_else(|| format!("{path}: no such base"))?,
        };
        let base: ParsedBase = base_json::with_a_view(
            crate::bases::parse(yaml).map_err(|e| format!("{path}: {e}"))?,
        );
        let i = base_json::view_index(&base, query.view.as_deref())?;
        let spec = base.views.get(i).ok_or("no such view")?;
        let (revision, rows, keys) = self.rows(&index);
        let limit = query
            .limit
            .map_or(usize::MAX, |n| usize::try_from(n).unwrap_or(usize::MAX));
        let (table, total) = crate::bases::view_table_at_most(&base, spec, &rows, limit);
        let (join, clauses) = crate::bases::filter_clauses(&base.global_filter);
        let mut answer = json!({
            "base": path,
            "views": base.views.iter().map(|v| json!({ "name": v.name, "type": v.kind.as_str() })).collect::<Vec<_>>(),
            "view": base_json::view_json(spec),
            "rows": table,
            "total": total,
            "columns": base_json::columns(&base, &keys),
            "formulas": base.formulas.iter().map(|f| json!({ "name": f.name, "expression": f.expression })).collect::<Vec<_>>(),
            "filters": { "join": join, "clauses": clauses },
        });
        let hash = {
            // Serialized into the hasher, not into a string first.
            let mut h = HashWriter(std::collections::hash_map::DefaultHasher::new());
            let _ = serde_json::to_writer(&mut h, &answer);
            format!("{:016x}", h.0.finish())
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
            return Ok(json!({ "unchanged": true, "revision": revision, "hash": hash }));
        }
        if let Some(map) = answer.as_object_mut() {
            map.insert("revision".to_owned(), json!(revision));
            map.insert("hash".to_owned(), json!(hash));
        }
        Ok(answer)
    }
}

/// Bytes written into a hasher.
struct HashWriter(std::collections::hash_map::DefaultHasher);

impl std::io::Write for HashWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl VaultQuery for QueryEngine {
    async fn query(&self, request: QueryRequest) -> Result<String, QueryError> {
        self.run(&request)
            .map(|v| v.to_string())
            .map_err(QueryError::Refused)
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

    /// Applies a delta's bytes (`IndexDelta::to_bytes`).
    ///
    /// # Errors
    ///
    /// They are not one, or it is from another revision (take a snapshot).
    pub fn apply(&self, bytes: &[u8]) -> Result<(), String> {
        let delta = crate::index::IndexDelta::from_bytes(bytes)?;
        // One sync loop applies the deltas: the lock is held only to read
        // the index and to put the next one in its place.
        let mut next = (**self.0.lock().unwrap_or_else(PoisonError::into_inner)).clone();
        next.apply(&delta)?;
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Arc::new(next);
        Ok(())
    }

    /// Its revision.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).revision
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
