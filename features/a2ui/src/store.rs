//! Surfaces on a Loro document: the only place an A2UI surface lives.
//!
//! Each surface is a map under the store's root map (`a2ui` unless the
//! app says otherwise): its catalog, its properties, who wrote it, its
//! components (id → the component's JSON) and its data model, a tree of
//! Loro maps. An `updateDataModel` upsert at a JSON Pointer is a write
//! into that tree, so a surface persists with its document, syncs with it
//! and can be co-edited; a field the person types into writes to the same
//! place ([`Surfaces::set_data`]). Each apply is one step of the
//! document's history when the store has one ([`Surfaces::with_history`]).
//!
//! The messages and their binding rules are `a2ui-base`'s: this store
//! keeps what they say in Loro instead of in its `MessageProcessor`'s
//! memory, and hands a renderer an `a2ui_base` [`SurfaceModel`] read back
//! from it ([`Surface::model`]).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use a2ui_base::model::component_model::ComponentModel;
use a2ui_base::model::data_model::DataModel;
use a2ui_base::model::surface_model::SurfaceModel;
use a2ui_base::protocol::server_to_client::{A2uiMessage, A2uiPayload};
use architect::action::Principal;
use crdt::CrdtDoc;
use crdt::loro::{Container, LoroDoc, LoroMap, LoroValue, ValueOrContainer};
use serde_json::{Map, Value};

use crate::catalog::{self, CATALOG_ID};

/// The root map surfaces live under when the app names none.
pub const ROOT: &str = "a2ui";

/// Why a message was not applied.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SurfaceError {
    /// It is not A2UI (or not JSON).
    #[error("not an A2UI message: {0}")]
    Parse(String),
    #[error("no surface “{0}”")]
    NotFound(String),
    #[error("surface “{0}” already exists (render it again to replace it)")]
    Exists(String),
    /// It names a catalog this app does not draw.
    #[error("catalog “{0}” is not this app's; use “{CATALOG_ID}”")]
    Catalog(String),
    /// A component without an `id` or a `component`.
    #[error("{0}")]
    Component(String),
    /// A data write the model cannot hold (a root that is not an object).
    #[error("{0}")]
    Data(String),
    /// A message a surface does not take from an agent here
    /// (`callFunction`, `actionResponse`).
    #[error("{0} is not taken here")]
    Unsupported(&'static str),
    #[error("the document refused the write: {0}")]
    Loro(String),
}

/// One surface, read back from the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Surface {
    pub id: String,
    pub catalog_id: String,
    pub properties: Option<Value>,
    /// Who rendered it: its events run as this caller, asking before every
    /// write. `None` for the app itself.
    pub author: Option<Principal>,
    /// Its components by id.
    pub components: Vec<Value>,
    /// Its data model.
    pub data: Value,
}

impl Surface {
    /// Its components outside the catalog, `(id, component)`: each shows
    /// as a placeholder, never drawn.
    #[must_use]
    pub fn outside_catalog(&self) -> Vec<(String, String)> {
        self.components
            .iter()
            .filter_map(|c| {
                let name = c.get("component")?.as_str()?;
                let id = c.get("id")?.as_str()?;
                catalog::component(name)
                    .is_none()
                    .then(|| (id.to_owned(), name.to_owned()))
            })
            .collect()
    }

    /// `a2ui-base`'s model of it, for a renderer to resolve bindings
    /// against.
    #[must_use]
    pub fn model(&self) -> SurfaceModel {
        let model = SurfaceModel::new(
            self.id.clone(),
            self.catalog_id.clone(),
            self.properties.clone(),
            false,
        )
        .with_data_model(self.data.clone());
        // Each was checked on the way in.
        let _ = model
            .components
            .borrow_mut()
            .add_from_json(&self.components);
        model
    }

    /// Its data model as JSON, with the components: what `A2UI_SURFACE`
    /// answers.
    #[must_use]
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "surfaceId": self.id,
            "catalogId": self.catalog_id,
            "author": self.author.as_ref().map(Principal::describe),
            "components": self.components,
            "dataModel": self.data,
            "outsideCatalog": self.outside_catalog().into_iter().map(|(id, c)| serde_json::json!({"id": id, "component": c})).collect::<Vec<_>>(),
        })
    }
}

/// What an apply did, per message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// The surfaces it touched, in order.
    pub surfaces: Vec<String>,
    /// Components it put in place.
    pub components: usize,
    /// Data writes.
    pub writes: usize,
    pub deleted: Vec<String>,
}

/// The surfaces of one document. Cheap to clone.
#[derive(Clone)]
pub struct Surfaces {
    doc: CrdtDoc,
    root: Arc<str>,
    history: Option<Arc<Mutex<crdt::History>>>,
}

impl std::fmt::Debug for Surfaces {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Surfaces")
            .field("root", &self.root)
            .field("history", &self.history.is_some())
            .finish_non_exhaustive()
    }
}

impl Surfaces {
    /// The surfaces under `root` in `doc`.
    pub fn new(doc: CrdtDoc, root: impl Into<String>) -> Self {
        Self {
            doc,
            root: Arc::from(root.into()),
            history: None,
        }
    }

    /// Each apply one named step of `history` (the document's undo).
    #[must_use]
    pub fn with_history(mut self, history: Arc<Mutex<crdt::History>>) -> Self {
        self.history = Some(history);
        self
    }

    /// The document they live in.
    #[must_use]
    pub const fn doc(&self) -> &CrdtDoc {
        &self.doc
    }

    fn root(&self) -> LoroMap {
        self.doc.loro().get_map(&*self.root)
    }

    /// Runs `change` as one step named `name`.
    fn step<R>(&self, name: &str, change: impl FnOnce(&LoroDoc) -> R) -> R {
        let doc = self.doc.loro();
        if let Some(history) = &self.history {
            return history
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .transact(doc, name, change);
        }
        let out = change(doc);
        doc.commit();
        out
    }

    /// Every surface's id, sorted.
    #[must_use]
    pub fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.root().keys().map(|k| k.to_string()).collect();
        ids.sort();
        ids
    }

    /// Surface `id`, read back.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<Surface> {
        let map = child_map(&self.root(), id)?;
        let text = |key: &str| match map.get(key) {
            Some(ValueOrContainer::Value(LoroValue::String(s))) => s.to_string(),
            _ => String::new(),
        };
        let value = |key: &str| match map.get(key) {
            Some(ValueOrContainer::Value(v)) => Some(to_json(&v)),
            _ => None,
        };
        let mut components: Vec<Value> = child_map(&map, "components")
            .map(|m| match to_json(&m.get_deep_value()) {
                Value::Object(o) => o.into_iter().map(|(_, v)| v).collect(),
                _ => Vec::new(),
            })
            .unwrap_or_default();
        components.sort_by(|a, b| {
            let id = |v: &Value| v.get("id").and_then(Value::as_str).map(str::to_owned);
            id(a).cmp(&id(b))
        });
        Some(Surface {
            id: id.to_owned(),
            catalog_id: text("catalogId"),
            properties: value("properties").filter(|v| !v.is_null()),
            author: value("author").as_ref().and_then(principal_from_json),
            components,
            data: child_map(&map, "data").map_or_else(
                || Value::Object(Map::new()),
                |m| to_json(&m.get_deep_value()),
            ),
        })
    }

    /// Applies `messages` as one step: all of them, or — at the first
    /// that is refused — none.
    ///
    /// # Errors
    ///
    /// The first message refused, and why.
    pub fn apply(
        &self,
        messages: &[A2uiMessage],
        author: Option<&Principal>,
    ) -> Result<Applied, SurfaceError> {
        let span = tracing::info_span!(
            "a2ui.apply",
            a2ui.messages = messages.len(),
            a2ui.author = author.map(Principal::describe).unwrap_or_default(),
            a2ui.surfaces = tracing::field::Empty,
            a2ui.outcome = tracing::field::Empty,
        );
        let _entered = span.enter();
        // Checked before anything is written.
        self.check(messages)?;
        let name = format!(
            "A2UI: {}",
            messages
                .first()
                .map(|m| surface_of(&m.payload))
                .unwrap_or_default()
        );
        let result = self.step(&name, |_| {
            let mut applied = Applied::default();
            for m in messages {
                self.apply_one(m, author, &mut applied)?;
            }
            Ok(applied)
        });
        span.record(
            "a2ui.outcome",
            if result.is_ok() { "applied" } else { "refused" },
        );
        if let Ok(a) = &result {
            span.record("a2ui.surfaces", a.surfaces.join(",").as_str());
        }
        result
    }

    /// Refuses what [`Self::apply`] would refuse, without writing.
    fn check(&self, messages: &[A2uiMessage]) -> Result<(), SurfaceError> {
        let mut exists: Vec<String> = self.ids();
        for m in messages {
            match &m.payload {
                A2uiPayload::CreateSurface(p) => {
                    let s = &p.create_surface;
                    if s.catalog_id != CATALOG_ID {
                        return Err(SurfaceError::Catalog(s.catalog_id.clone()));
                    }
                    if exists.contains(&s.surface_id) {
                        return Err(SurfaceError::Exists(s.surface_id.clone()));
                    }
                    for c in s.components.iter().flatten() {
                        component_ok(c)?;
                    }
                    if let Some(d) = &s.data_model
                        && !d.is_object()
                    {
                        return Err(SurfaceError::Data(
                            "a data model is an object at its root".to_owned(),
                        ));
                    }
                    exists.push(s.surface_id.clone());
                }
                A2uiPayload::UpdateComponents(p) => {
                    let s = &p.update_components;
                    if !exists.contains(&s.surface_id) {
                        return Err(SurfaceError::NotFound(s.surface_id.clone()));
                    }
                    for c in &s.components {
                        component_ok(c)?;
                    }
                }
                A2uiPayload::UpdateDataModel(p) => {
                    let s = &p.update_data_model;
                    if !exists.contains(&s.surface_id) {
                        return Err(SurfaceError::NotFound(s.surface_id.clone()));
                    }
                    let root = s.path.as_deref().is_none_or(|p| p.is_empty() || p == "/");
                    if root
                        && s.value
                            .as_ref()
                            .is_some_and(|v| !v.is_object() && !v.is_null())
                    {
                        return Err(SurfaceError::Data(
                            "a data model is an object at its root".to_owned(),
                        ));
                    }
                }
                A2uiPayload::DeleteSurface(p) => {
                    let id = &p.delete_surface.surface_id;
                    if !exists.contains(id) {
                        return Err(SurfaceError::NotFound(id.clone()));
                    }
                    exists.retain(|e| e != id);
                }
                A2uiPayload::CallFunction(_) => {
                    return Err(SurfaceError::Unsupported("callFunction"));
                }
                A2uiPayload::ActionResponse(_) => {
                    return Err(SurfaceError::Unsupported("actionResponse"));
                }
            }
        }
        Ok(())
    }

    fn apply_one(
        &self,
        message: &A2uiMessage,
        author: Option<&Principal>,
        applied: &mut Applied,
    ) -> Result<(), SurfaceError> {
        let root = self.root();
        match &message.payload {
            A2uiPayload::CreateSurface(p) => {
                let s = &p.create_surface;
                let map = root
                    .insert_container(&s.surface_id, LoroMap::new())
                    .map_err(loro)?;
                map.insert("catalogId", s.catalog_id.as_str())
                    .map_err(loro)?;
                if let Some(props) = &s.surface_properties {
                    map.insert("properties", to_loro(props)).map_err(loro)?;
                }
                if let Some(who) = author {
                    map.insert("author", to_loro(&principal_to_json(who)))
                        .map_err(loro)?;
                }
                let components = map
                    .insert_container("components", LoroMap::new())
                    .map_err(loro)?;
                let data = map.insert_container("data", LoroMap::new()).map_err(loro)?;
                for c in s.components.iter().flatten() {
                    put_component(&components, c)?;
                    applied.components = applied.components.saturating_add(1);
                }
                if let Some(Value::Object(d)) = &s.data_model {
                    for (k, v) in d {
                        write(&data, k, v)?;
                    }
                    applied.writes = applied.writes.saturating_add(1);
                }
                applied.surfaces.push(s.surface_id.clone());
            }
            A2uiPayload::UpdateComponents(p) => {
                let s = &p.update_components;
                let components = surface_part(&root, &s.surface_id, "components")?;
                for c in &s.components {
                    put_component(&components, c)?;
                    applied.components = applied.components.saturating_add(1);
                }
                applied.surfaces.push(s.surface_id.clone());
            }
            A2uiPayload::UpdateDataModel(p) => {
                let s = &p.update_data_model;
                let data = surface_part(&root, &s.surface_id, "data")?;
                set_pointer(&data, s.path.as_deref().unwrap_or("/"), s.value.as_ref())?;
                applied.writes = applied.writes.saturating_add(1);
                applied.surfaces.push(s.surface_id.clone());
            }
            A2uiPayload::DeleteSurface(p) => {
                let id = &p.delete_surface.surface_id;
                root.delete(id).map_err(loro)?;
                applied.deleted.push(id.clone());
            }
            A2uiPayload::CallFunction(_) => return Err(SurfaceError::Unsupported("callFunction")),
            A2uiPayload::ActionResponse(_) => {
                return Err(SurfaceError::Unsupported("actionResponse"));
            }
        }
        Ok(())
    }

    /// Writes `value` at `pointer` in surface `id`'s data model (a field the
    /// person typed into; an action's answer at its `responsePath`): one
    /// step, the same write an `updateDataModel` makes. `None` deletes.
    ///
    /// # Errors
    ///
    /// No such surface, or the document refused it.
    pub fn set_data(
        &self,
        id: &str,
        pointer: &str,
        value: Option<&Value>,
    ) -> Result<(), SurfaceError> {
        let root = self.root();
        let data = surface_part(&root, id, "data")?;
        self.step(&format!("A2UI: {id}"), |_| {
            set_pointer(&data, pointer, value)
        })
    }

    /// Deletes every surface.
    pub fn clear(&self) {
        let root = self.root();
        self.step("A2UI: clear", |_| {
            let _ = root.clear();
        });
    }

    /// Calls `changed` after every change to the document (one of its own,
    /// or one that arrived). Keep the subscription; dropping it stops.
    ///
    /// `changed` must not own the document: a channel's sender, a flag.
    pub fn subscribe(
        &self,
        changed: impl Fn() + Send + Sync + 'static,
    ) -> crdt::loro::Subscription {
        self.doc.loro().subscribe_root(Arc::new(move |_| changed()))
    }
}

fn surface_of(payload: &A2uiPayload) -> String {
    match payload {
        A2uiPayload::CreateSurface(p) => p.create_surface.surface_id.clone(),
        A2uiPayload::UpdateComponents(p) => p.update_components.surface_id.clone(),
        A2uiPayload::UpdateDataModel(p) => p.update_data_model.surface_id.clone(),
        A2uiPayload::DeleteSurface(p) => p.delete_surface.surface_id.clone(),
        A2uiPayload::CallFunction(_) | A2uiPayload::ActionResponse(_) => String::new(),
    }
}

fn loro(e: impl std::fmt::Display) -> SurfaceError {
    SurfaceError::Loro(e.to_string())
}

fn component_ok(c: &Value) -> Result<(), SurfaceError> {
    ComponentModel::from_json(c)
        .map(|_| ())
        .map_err(|e| SurfaceError::Component(e.to_string()))
}

fn put_component(components: &LoroMap, c: &Value) -> Result<(), SurfaceError> {
    let model = ComponentModel::from_json(c).map_err(|e| SurfaceError::Component(e.to_string()))?;
    components.insert(&model.id, to_loro(c)).map_err(loro)
}

fn child_map(map: &LoroMap, key: &str) -> Option<LoroMap> {
    match map.get(key)? {
        ValueOrContainer::Container(Container::Map(m)) => Some(m),
        _ => None,
    }
}

fn surface_part(root: &LoroMap, id: &str, part: &str) -> Result<LoroMap, SurfaceError> {
    let surface = child_map(root, id).ok_or_else(|| SurfaceError::NotFound(id.to_owned()))?;
    child_map(&surface, part).map_or_else(
        || surface.insert_container(part, LoroMap::new()).map_err(loro),
        Ok,
    )
}

/// A JSON Pointer's segments, unescaped (`~1` is `/`, `~0` is `~`).
fn segments(pointer: &str) -> Vec<String> {
    pointer
        .split('/')
        .skip(1)
        .filter(|s| !s.is_empty())
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect()
}

fn escape(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

/// Writes `value` at `pointer` under `data`: objects as Loro maps (so two
/// people's edits to different keys merge), anything else as a value.
/// Under a value that is not a map (an array, a scalar) the write is a
/// read-modify-write of that value, by `a2ui-base`'s pointer rules.
fn set_pointer(data: &LoroMap, pointer: &str, value: Option<&Value>) -> Result<(), SurfaceError> {
    let path = segments(pointer);
    let Some((last, parents)) = path.split_last() else {
        // The root: replaced whole.
        data.clear().map_err(loro)?;
        if let Some(Value::Object(object)) = value {
            for (k, v) in object {
                write(data, k, v)?;
            }
        }
        return Ok(());
    };
    let mut at = data.clone();
    for (i, segment) in parents.iter().enumerate() {
        match at.get(segment) {
            Some(ValueOrContainer::Container(Container::Map(m))) => at = m,
            Some(ValueOrContainer::Value(v)) if !matches!(v, LoroValue::Null) => {
                // A value (an array, say): change it as JSON, write it back.
                // `a2ui-base`'s model is an object at its root, so the
                // value goes under one key.
                let rest = path.iter().skip(i.saturating_add(1)).fold(
                    String::from("/v"),
                    |mut rest, s| {
                        rest.push('/');
                        rest.push_str(&escape(s));
                        rest
                    },
                );
                let mut model = DataModel::from_value(serde_json::json!({ "v": to_json(&v) }));
                model.set(&rest, value.cloned().unwrap_or(Value::Null));
                let changed = model.as_value().get("v").cloned().unwrap_or(Value::Null);
                return at.insert(segment, to_loro(&changed)).map_err(loro);
            }
            _ => {
                at = at.insert_container(segment, LoroMap::new()).map_err(loro)?;
            }
        }
    }
    match value {
        None | Some(Value::Null) => at.delete(last).map_err(loro),
        Some(v) => write(&at, last, v),
    }
}

fn write(map: &LoroMap, key: &str, value: &Value) -> Result<(), SurfaceError> {
    if let Value::Object(object) = value {
        let child = map.insert_container(key, LoroMap::new()).map_err(loro)?;
        for (k, v) in object {
            write(&child, k, v)?;
        }
        Ok(())
    } else {
        map.insert(key, to_loro(value)).map_err(loro)
    }
}

/// JSON as a Loro value.
#[must_use]
pub fn to_loro(value: &Value) -> LoroValue {
    match value {
        Value::Null => LoroValue::Null,
        Value::Bool(b) => LoroValue::Bool(*b),
        Value::Number(n) => n.as_i64().map_or_else(
            || LoroValue::Double(n.as_f64().unwrap_or(0.0)),
            LoroValue::I64,
        ),
        Value::String(s) => LoroValue::from(s.as_str()),
        Value::Array(items) => LoroValue::from(items.iter().map(to_loro).collect::<Vec<_>>()),
        Value::Object(object) => LoroValue::from(
            object
                .iter()
                .map(|(k, v)| (k.clone(), to_loro(v)))
                .collect::<HashMap<String, LoroValue>>(),
        ),
    }
}

/// A Loro value as JSON (a container id, which a deep value never holds,
/// is `null`).
#[must_use]
pub fn to_json(value: &LoroValue) -> Value {
    match value {
        LoroValue::Null | LoroValue::Container(_) => Value::Null,
        LoroValue::Bool(b) => Value::Bool(*b),
        LoroValue::Double(d) => serde_json::Number::from_f64(*d).map_or(Value::Null, Value::Number),
        LoroValue::I64(i) => Value::from(*i),
        LoroValue::String(s) => Value::String(s.to_string()),
        LoroValue::Binary(b) => Value::Array(b.iter().map(|x| Value::from(*x)).collect()),
        LoroValue::List(items) => Value::Array(items.iter().map(to_json).collect()),
        LoroValue::Map(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .filter_map(|k| map.get(k).map(|v| (k.clone(), to_json(v))))
                    .collect(),
            )
        }
    }
}

/// Who rendered a surface, as it is kept in the document.
fn principal_to_json(who: &Principal) -> Value {
    match who {
        Principal::User { user_id } => serde_json::json!({ "user": user_id }),
        Principal::Service { name } => serde_json::json!({ "service": name }),
        Principal::Guest { link_id, .. } => serde_json::json!({ "guest": link_id }),
        Principal::Host { endpoint } => serde_json::json!({ "host": endpoint }),
        Principal::Extension { id, on_behalf_of } => serde_json::json!({
            "extension": id,
            "for": on_behalf_of.as_deref().map(principal_to_json),
        }),
        Principal::Anonymous => serde_json::json!({ "anonymous": true }),
    }
}

fn principal_from_json(value: &Value) -> Option<Principal> {
    let text = |k: &str| value.get(k).and_then(Value::as_str).map(str::to_owned);
    text("user")
        .map(|user_id| Principal::User { user_id })
        .or_else(|| text("service").map(|name| Principal::Service { name }))
        .or_else(|| {
            text("guest").map(|link_id| Principal::Guest {
                link_id,
                display: None,
            })
        })
        .or_else(|| text("host").map(|endpoint| Principal::Host { endpoint }))
        .or_else(|| {
            text("extension").map(|id| Principal::Extension {
                id,
                on_behalf_of: value.get("for").and_then(principal_from_json).map(Box::new),
            })
        })
        .or_else(|| {
            (value.get("anonymous") == Some(&Value::Bool(true))).then_some(Principal::Anonymous)
        })
}

/// Parses A2UI messages: a JSON array of them, one object, or JSONL (one
/// per line). A model's smart quotes and trailing commas are fixed
/// by `a2ui-base`'s payload fixer.
///
/// # Errors
///
/// Something in it is not an A2UI message.
pub fn parse(text: &str) -> Result<Vec<A2uiMessage>, SurfaceError> {
    let fix = |t: &str| {
        a2ui_base::validate::payload_fixer::parse_and_fix(t)
            .map_err(|e| SurfaceError::Parse(e.to_string()))
    };
    let values = match fix(text) {
        Ok(values) => values,
        // JSONL: one message a line.
        Err(whole) => {
            let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
            if lines.len() < 2 {
                return Err(whole);
            }
            let mut values = Vec::new();
            for line in lines {
                values.extend(fix(line)?);
            }
            values
        }
    };
    values.into_iter().map(message).collect()
}

/// One message from its JSON (`version` defaults to v1.0).
///
/// # Errors
///
/// It is not an A2UI message.
pub fn message(mut value: Value) -> Result<A2uiMessage, SurfaceError> {
    if let Some(o) = value.as_object_mut() {
        o.entry("version")
            .or_insert_with(|| Value::String(catalog::VERSION.to_owned()));
    }
    serde_json::from_value(value).map_err(|e| SurfaceError::Parse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn store() -> Surfaces {
        Surfaces::new(CrdtDoc::ephemeral(), ROOT)
    }

    fn create(id: &str) -> Value {
        json!({ "createSurface": { "surfaceId": id, "catalogId": CATALOG_ID,
            "components": [
                { "id": "root", "component": "Column", "children": ["t", "x"] },
                { "id": "t", "component": "Text", "text": { "path": "/user/name" } },
                { "id": "x", "component": "Script", "src": "evil.js" },
            ],
            "dataModel": { "user": { "name": "Ada" }, "tags": ["a", "b"] } } })
    }

    #[test]
    fn a_surface_round_trips_through_loro() {
        let s = store();
        let author = Principal::Service {
            name: "agent".to_owned(),
        };
        s.apply(&[message(create("s1")).unwrap()], Some(&author))
            .unwrap();
        let got = s.get("s1").unwrap();
        assert_eq!(got.author, Some(author));
        assert_eq!(got.components.len(), 3);
        assert_eq!(got.data["user"]["name"], "Ada");
        assert_eq!(
            got.outside_catalog(),
            [("x".to_owned(), "Script".to_owned())]
        );

        // Upserts: into a map, into an array value, and a delete.
        let patch = [
            json!({ "updateDataModel": { "surfaceId": "s1", "path": "/user/name", "value": "Grace" } }),
            json!({ "updateDataModel": { "surfaceId": "s1", "path": "/tags/1", "value": "c" } }),
            json!({ "updateDataModel": { "surfaceId": "s1", "path": "/user/age", "value": 36 } }),
            json!({ "updateDataModel": { "surfaceId": "s1", "path": "/user/age" } }),
        ];
        let patch: Vec<_> = patch.into_iter().map(|m| message(m).unwrap()).collect();
        s.apply(&patch, None).unwrap();
        let got = s.get("s1").unwrap();
        assert_eq!(
            got.data,
            json!({ "user": { "name": "Grace" }, "tags": ["a", "c"] })
        );

        // Another document that imports this one's bytes has it all.
        let bytes = s
            .doc()
            .loro()
            .export(crdt::loro::ExportMode::Snapshot)
            .unwrap();
        let other = LoroDoc::new();
        other.import(&bytes).unwrap();
        let reloaded = Surfaces::new(CrdtDoc::from_loro(other), ROOT);
        assert_eq!(reloaded.get("s1"), Some(got));
    }

    #[test]
    fn a_batch_applies_whole_or_not_at_all() {
        let s = store();
        let bad = [
            message(create("s1")).unwrap(),
            message(json!({ "updateComponents": { "surfaceId": "nope", "components": [] } }))
                .unwrap(),
        ];
        assert_eq!(
            s.apply(&bad, None),
            Err(SurfaceError::NotFound("nope".to_owned()))
        );
        assert!(s.ids().is_empty());
        let other =
            json!({ "createSurface": { "surfaceId": "s", "catalogId": "https://evil/catalog" } });
        assert!(matches!(
            s.apply(&[message(other).unwrap()], None),
            Err(SurfaceError::Catalog(_))
        ));
    }

    #[test]
    fn messages_parse_as_an_array_or_jsonl() {
        let one = create("s1").to_string();
        assert_eq!(parse(&format!("[{one}]")).unwrap().len(), 1);
        assert_eq!(parse(&format!("{one}\n{one}")).unwrap().len(), 2);
        assert!(parse("{\"nope\": 1}").is_err());
    }
}
