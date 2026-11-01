//! The person's values on every device (feature `synced`): kept in a
//! CRDT document's root map, one entry per key, so two devices that each
//! changed a setting offline both keep theirs when they meet (a key
//! changed on both: the later write wins).
//!
//! The document is any `loro::LoroDoc` the app syncs — a
//! `crdt::CrdtDoc`'s (`CrdtDoc::loro`), opened under [`synced_doc_id`]
//! with `crdt::docs`' `Lineage::Merge`, since every device makes it
//! offline under the same id. The `Project` scope can live the same way
//! in a project's own document.

use std::sync::Arc;

use loro::{EventTriggerKind, LoroDoc, LoroMap, LoroValue, Subscription, ValueOrContainer};
use uuid::Uuid;

use crate::model::Value;
use crate::values::Values;

/// The root map the values live in.
pub const ROOT: &str = "settings";

/// The id of `person`'s synced-settings document for `app`: the same on
/// every device, so each makes it offline and they merge.
#[must_use]
pub fn synced_doc_id(app: &str, person: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("architect-settings:{app}:{person}").as_bytes(),
    )
}

/// Values kept in a document's [`ROOT`] map.
#[derive(Debug, Clone)]
pub struct DocValues {
    doc: LoroDoc,
}

impl DocValues {
    /// Over `doc` (a handle to it: the doc is shared, not copied).
    #[must_use]
    pub fn new(doc: &LoroDoc) -> Self {
        Self { doc: doc.clone() }
    }

    fn map(&self) -> LoroMap {
        self.doc.get_map(ROOT)
    }

    /// What the document holds now; a value of a kind settings do not
    /// keep is left out.
    #[must_use]
    pub fn load(&self) -> Values {
        let mut out = Values::new();
        self.map().for_each(|key, v| {
            if let ValueOrContainer::Value(v) = v {
                if let Some(v) = from_loro(&v) {
                    out.set.insert(key.to_owned(), v);
                }
            }
        });
        out
    }

    /// Writes `keys` as `values` has them (a key it does not hold is
    /// removed: back to its default), and commits. Only these keys: what
    /// another device changed meanwhile is left alone.
    ///
    /// # Errors
    ///
    /// Loro's message.
    pub fn save_keys<S: AsRef<str>>(&self, values: &Values, keys: &[S]) -> Result<(), String> {
        let map = self.map();
        for key in keys {
            let key = key.as_ref();
            let done = match values.set.get(key) {
                Some(v) => map.insert(key, to_loro(v)),
                None if map.get(key).is_some() => map.delete(key),
                None => Ok(()),
            };
            done.map_err(|e| e.to_string())?;
        }
        self.doc.commit();
        Ok(())
    }

    /// Writes all of `values`: each key it holds, and removes the rest.
    ///
    /// # Errors
    ///
    /// Loro's message.
    pub fn save(&self, values: &Values) -> Result<(), String> {
        let mut keys: Vec<String> = self.map().keys().map(|k| k.to_string()).collect();
        keys.extend(values.set.keys().cloned());
        keys.sort();
        keys.dedup();
        self.save_keys(values, &keys)
    }

    /// Calls `changed` when the values change from elsewhere (an import:
    /// another device's edit arriving), not for this device's own saves.
    /// Read them with [`load`](Self::load). Kept while the returned
    /// subscription is.
    ///
    /// `changed` must not hold the document (a doc kept alive by its own
    /// subscription is never dropped): signal a task that holds a
    /// `DocValues`, or a UI that does.
    #[must_use = "the subscription ends when dropped"]
    pub fn on_remote_change(&self, changed: impl Fn() + Send + Sync + 'static) -> Subscription {
        self.doc.subscribe_root(Arc::new(move |e| {
            if e.triggered_by == EventTriggerKind::Import {
                changed();
            }
        }))
    }
}

fn to_loro(v: &Value) -> LoroValue {
    match v {
        Value::Bool(b) => LoroValue::Bool(*b),
        Value::Int(i) => LoroValue::I64(*i),
        Value::Float(f) => LoroValue::Double(*f),
        Value::Text(t) => LoroValue::from(t.as_str()),
    }
}

fn from_loro(v: &LoroValue) -> Option<Value> {
    match v {
        LoroValue::Bool(b) => Some(Value::Bool(*b)),
        LoroValue::I64(i) => Some(Value::Int(*i)),
        LoroValue::Double(f) => Some(Value::Float(*f)),
        LoroValue::String(s) => Some(Value::Text(s.to_string())),
        _ => None,
    }
}
