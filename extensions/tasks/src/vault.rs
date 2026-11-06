//! The vault, through the host service: the only way this extension
//! reaches notes, so its process copy works as the linked one does
//! (`[capabilities] actions` in `extension.toml`).

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
    let h = handle().ok_or("the tasks extension is not running")?;
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

/// A setting of its own (`tasks.<key>`), or `null`.
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

/// A new note at `path` with `properties` and `body`; a taken path gets
/// ` 2`, ` 3`, … before its `.md`. The path written.
pub async fn create(
    path: &str,
    properties: &Map<String, Value>,
    body: &str,
) -> Result<String, String> {
    let stem = path.strip_suffix(".md").unwrap_or(path);
    let props = Value::Object(properties.clone()).to_string();
    let mut last = String::new();
    for n in 1..=50 {
        let at = if n == 1 {
            format!("{stem}.md")
        } else {
            format!("{stem} {n}.md")
        };
        match run(
            "VAULT_CREATE",
            json!({ "path": at, "body": body, "properties": props }),
        )
        .await
        {
            Ok(_) => return Ok(at),
            Err(e) if e.contains("exists") || e.contains("taken") => last = e,
            Err(e) => return Err(e),
        }
    }
    Err(last)
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
