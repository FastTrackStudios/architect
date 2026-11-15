//! The app, through the host service.
//!
//! The only way Review reaches notes, Threads and the playhead, so its process copy works as the linked one
//! does (`[capabilities]`, `[requires]` and `[uses]` in `extension.toml`).

use std::sync::{Mutex, PoisonError};

use architect_extensions::hostcall::{HostError, HostRequest};
use architect_extensions::linked::Handle;
use serde_json::{Map, Value, json};

/// The handle of the copy built last.
static HANDLE: Mutex<Option<Handle>> = Mutex::new(None);

pub fn connect(handle: Handle) {
    *HANDLE.lock().unwrap_or_else(PoisonError::into_inner) = Some(handle);
}

fn handle() -> Result<Handle, String> {
    HANDLE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .ok_or_else(|| "the review extension is not running".to_owned())
}

/// Runs app action `action` with `args`: its answer.
pub async fn run(action: &str, args: Value) -> Result<Value, String> {
    handle()?
        .host(HostRequest::Run {
            action: action.to_owned(),
            args,
            ticket: None,
        })
        .await
        .map_err(|e| e.to_string())
}

/// Calls `method` of service `service`: `None` when no loaded extension
/// provides it.
pub async fn service(service: &str, method: &str, args: Value) -> Option<Result<Value, String>> {
    let h = handle().ok()?;
    match h.service(service, method, args).await {
        Ok(v) => Some(Ok(v)),
        Err(HostError::NotFound { what }) if what.starts_with("provider of") => None,
        Err(e) => Some(Err(e.to_string())),
    }
}

/// Threads' method `method` (it is required: its absence is an error).
pub async fn threads(method: &str, args: Value) -> Result<Value, String> {
    service(crate::THREADS, method, args)
        .await
        .unwrap_or_else(|| Err("Threads is not loaded: Review keeps its comments there".to_owned()))
}

/// The playhead's method `method`, when there is one.
pub async fn playhead(method: &str, args: Value) -> Option<Result<Value, String>> {
    service(crate::PLAYHEAD, method, args).await
}

/// The note at `path` (`VAULT_READ`): its properties.
pub async fn read(path: &str) -> Result<Map<String, Value>, String> {
    let note = run("VAULT_READ", json!({ "path": path })).await?;
    Ok(note
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default())
}

/// Sets property `key` of the note at `path`.
pub async fn set(path: &str, key: &str, value: &Value) -> Result<(), String> {
    run(
        "VAULT_SET_PROPERTY",
        json!({ "path": path, "key": key, "value": value.to_string() }),
    )
    .await
    .map(|_| ())
}

/// What is focused in the app (`HostRequest::Selection`), or `null`.
pub async fn selection() -> Value {
    let Ok(h) = handle() else {
        return Value::Null;
    };
    h.host(HostRequest::Selection).await.unwrap_or(Value::Null)
}
