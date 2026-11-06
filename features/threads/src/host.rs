//! The app, through the host service.
//!
//! It is the only way this extension reaches notes, so its process copy
//! works as the linked one does
//! (`[capabilities]` in `extension.toml`). As the Time Tracker's
//! (`extensions/time-tracker/src/vault.rs`).

use std::sync::{Mutex, PoisonError};

use architect_extensions::hostcall::HostRequest;
use architect_extensions::linked::Handle;
use serde_json::{Map, Value, json};

/// The handle of the copy built last.
static HANDLE: Mutex<Option<Handle>> = Mutex::new(None);

pub fn connect(handle: Handle) {
    *HANDLE.lock().unwrap_or_else(PoisonError::into_inner) = Some(handle);
}

pub fn handle() -> Option<Handle> {
    HANDLE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Runs app action `action` with `args`: its answer.
pub async fn run(action: &str, args: Value) -> Result<Value, String> {
    let h = handle().ok_or("the threads extension is not running")?;
    h.host(HostRequest::Run {
        action: action.to_owned(),
        args,
        ticket: None,
    })
    .await
    .map_err(|e| e.to_string())
}

/// Calls `method` of service `service`, one it `[uses]`: `None` when no
/// loaded extension provides it.
pub async fn service(service: &str, method: &str, args: Value) -> Option<Result<Value, String>> {
    let h = handle()?;
    match h.service(service, method, args).await {
        Ok(v) => Some(Ok(v)),
        Err(architect_extensions::hostcall::HostError::NotFound { what })
            if what.starts_with("provider of") =>
        {
            None
        }
        Err(e) => Some(Err(e.to_string())),
    }
}

/// A setting of its own (`time.<key>`), or `null`.
pub async fn setting(key: &str) -> Value {
    let Some(h) = handle() else {
        return Value::Null;
    };
    h.host(HostRequest::Setting {
        key: key.to_owned(),
    })
    .await
    .unwrap_or(Value::Null)
}

/// One note a base found: its path, name and properties.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub path: String,
    pub name: String,
    pub values: Map<String, Value>,
}

/// The rows of a base written as YAML (`BASES_ROWS {source}`), every
/// group's, in order.
pub async fn rows(source: &str) -> Result<Vec<Row>, String> {
    let out = run("BASES_ROWS", json!({ "source": source })).await?;
    Ok(rows_of(&out))
}

/// The rows of a `BASES_ROWS` answer (`{rows: {groups: [{rows}]}}`); a
/// value named `note.<key>` read as `<key>`.
#[must_use]
pub fn rows_of(out: &Value) -> Vec<Row> {
    let table = out.get("rows").unwrap_or(out);
    table
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|g| {
            g.get("rows")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .map(|r| {
            let text = |k: &str| {
                r.get(k)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            let mut values = Map::new();
            if let Some(m) = r.get("values").and_then(Value::as_object) {
                for (k, v) in m {
                    let k = k.strip_prefix("note.").unwrap_or(k);
                    if !v.is_null() {
                        values.insert(k.to_owned(), v.clone());
                    }
                }
            }
            Row {
                path: text("path"),
                name: text("name"),
                values,
            }
        })
        .collect()
}

/// The note at `path` (`VAULT_READ`): its properties; `None` when there
/// is none.
pub async fn properties(path: &str) -> Option<Map<String, Value>> {
    let note = run("VAULT_READ", json!({ "path": path })).await.ok()?;
    Some(
        note.get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default(),
    )
}

/// A new note at `path` with `properties` (refused when it is taken).
pub async fn create(path: &str, properties: &Map<String, Value>, body: &str) -> Result<(), String> {
    run(
        "VAULT_CREATE",
        json!({ "path": path, "body": body, "properties": Value::Object(properties.clone()).to_string() }),
    )
    .await
    .map(|_| ())
}

/// Sets property `key` of the note at `path` (`null` removes it).
pub async fn set(path: &str, key: &str, value: &Value) -> Result<(), String> {
    if value.is_null() {
        // Removing one it does not have is no failure.
        let _ = run("VAULT_REMOVE_PROPERTY", json!({ "path": path, "key": key })).await;
        return Ok(());
    }
    run(
        "VAULT_SET_PROPERTY",
        json!({ "path": path, "key": key, "value": value.to_string() }),
    )
    .await
    .map(|_| ())
}

/// The note at `path` (`VAULT_READ`): its properties, or why not.
pub async fn read(path: &str) -> Result<Map<String, Value>, String> {
    let note = run("VAULT_READ", json!({ "path": path })).await?;
    Ok(note
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default())
}

/// What is focused in the app (`HostRequest::Selection`), or `null`.
pub async fn selection() -> Value {
    let Some(h) = handle() else {
        return Value::Null;
    };
    h.host(HostRequest::Selection).await.unwrap_or(Value::Null)
}

/// Opens `path` in the app.
pub async fn open(path: &str) -> Result<Value, String> {
    let h = handle().ok_or("the threads extension is not running")?;
    h.host(HostRequest::Open {
        path: path.to_owned(),
    })
    .await
    .map_err(|e| e.to_string())
}

/// A badge on the file-tree row and the tab of `path` (`None` takes it off).
pub async fn badge(path: &str, badge: Option<String>) {
    let Some(h) = handle() else {
        return;
    };
    for target in [
        architect_extensions::hostcall::Target::file(path),
        architect_extensions::hostcall::Target::tab(path),
    ] {
        let decoration = badge
            .clone()
            .map(|b| architect_extensions::hostcall::Decoration {
                icon: Some("message-square".to_owned()),
                badge: Some(b),
                ..architect_extensions::hostcall::Decoration::default()
            });
        let _ = h.host(HostRequest::Decorate { target, decoration }).await;
    }
}
