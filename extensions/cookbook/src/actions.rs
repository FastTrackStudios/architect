//! The `COOKBOOK_*` actions: JSON in and out, each one call of the
//! `Recipes` client (its method `m` is `COOKBOOK_<M>`, the
//! `architect.cookbook` service other extensions call).

use architect_extensions::hostcall::HostRequest;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::client::{client, handle};
use crate::service::{CookbookError, Import, Portion, Save, Search};

/// `args` as `T`; `null` as its default.
fn args<T: DeserializeOwned + Default>(v: Value) -> Result<T, String> {
    if v.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(v).map_err(|e| format!("arguments: {e}"))
}

fn out<T: Serialize>(v: Result<T, CookbookError>) -> Result<Value, String> {
    v.map_err(|e| e.to_string())
        .and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string()))
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn number(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|x| {
        x.as_f64()
            .or_else(|| x.as_str().and_then(|s| s.trim().parse().ok()))
    })
}

/// `COOKBOOK_LIST {text?, tag?, course?, ingredient?, limit?}`.
pub async fn list(a: Value) -> Result<Value, String> {
    let search: Search = args(a)?;
    let c = client().await.map_err(|e| e.to_string())?;
    out(c.list(search).await.map_err(architect::vox_into_app))
}

/// `COOKBOOK_GET {path, servings?}`.
pub async fn get(a: Value) -> Result<Value, String> {
    let path = text(&a, "path").ok_or("give the recipe's `path`")?;
    let c = client().await.map_err(|e| e.to_string())?;
    out(c
        .get(path, number(&a, "servings"))
        .await
        .map_err(architect::vox_into_app))
}

/// `COOKBOOK_SAVE {path?, name?, source, create?}`.
pub async fn save(a: Value) -> Result<Value, String> {
    let save: Save = args(a)?;
    let c = client().await.map_err(|e| e.to_string())?;
    out(c.save(save).await.map_err(architect::vox_into_app))
}

/// `COOKBOOK_RENAME {path, to}`.
pub async fn rename(a: Value) -> Result<Value, String> {
    let path = text(&a, "path").ok_or("give the recipe's `path`")?;
    let to = text(&a, "to").ok_or("give where it goes, `to`")?;
    let c = client().await.map_err(|e| e.to_string())?;
    out(c.rename(path, to).await.map_err(architect::vox_into_app))
}

/// `COOKBOOK_DELETE {path}`.
pub async fn delete(a: Value) -> Result<Value, String> {
    let path = text(&a, "path").ok_or("give the recipe's `path`")?;
    let c = client().await.map_err(|e| e.to_string())?;
    out(c
        .delete(path.clone())
        .await
        .map_err(architect::vox_into_app)
        .map(|()| json!({ "deleted": path })))
}

/// `COOKBOOK_INGREDIENTS {portions: [{path, servings?}]}`.
pub async fn ingredients(a: Value) -> Result<Value, String> {
    let portions: Vec<Portion> = match a.get("portions") {
        Some(p) => serde_json::from_value(p.clone()).map_err(|e| format!("portions: {e}"))?,
        None => return Err("give `portions`: [{path, servings?}]".into()),
    };
    let c = client().await.map_err(|e| e.to_string())?;
    out(c
        .ingredients(portions)
        .await
        .map_err(architect::vox_into_app))
}

/// Runs app action `action` with `args` through the host.
async fn run(action: &str, args: Value) -> Result<Value, String> {
    let h = handle().ok_or("the cookbook is not running")?;
    h.host(HostRequest::Run {
        action: action.to_owned(),
        args,
        ticket: None,
    })
    .await
    .map_err(|e| e.to_string())
}

/// `COOKBOOK_BASE`: opens `<folder>/Cookbook.base`, writing it first
/// when it is not there.
pub async fn base(_: Value) -> Result<Value, String> {
    let folder = match handle() {
        Some(h) => h
            .host(HostRequest::Setting {
                key: "cookbook.folder".into(),
            })
            .await
            .ok()
            .and_then(|v| v.as_str().map(|s| s.trim().trim_matches('/').to_owned()))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| crate::backend::FOLDER.to_owned()),
        None => crate::backend::FOLDER.to_owned(),
    };
    let path = format!("{folder}/{}", crate::BASE_FILE);
    let exists = run("BASES_READ", json!({ "path": path })).await.is_ok();
    if !exists {
        run("VAULT_CREATE", json!({ "path": path, "body": crate::BASE })).await?;
        // VAULT_CREATE indexes what it writes as a note: read the folder
        // again so Bases knows the new file is a base.
        run("VAULT_RELOAD", Value::Null).await?;
    }
    let opened = run("BASES_OPEN", json!({ "path": path })).await?;
    Ok(json!({ "path": path, "written": !exists, "opened": opened.get("opened") }))
}

/// `COOKBOOK_OPEN {path?, view?, servings?}`: opens the cookbook, or the
/// recipe at `path` in it, in cook mode (`view` cook) or the editor
/// (`view` edit).
pub async fn open(a: Value) -> Result<Value, String> {
    let view = text(&a, "view").unwrap_or_else(|| "cookbook".to_owned());
    if !matches!(view.as_str(), "cookbook" | "cook" | "edit") {
        return Err(format!("view {view:?}: cookbook, cook or edit"));
    }
    if let Some(path) = text(&a, "path") {
        let c = client().await.map_err(|e| e.to_string())?;
        let r = c
            .get(path.clone(), None)
            .await
            .map_err(|e| architect::vox_into_app(e).to_string())?;
        crate::ui::set_open(crate::ui::Open {
            path: Some(path),
            source: r.source,
            servings: number(&a, "servings"),
        });
    } else if view != "cookbook" {
        return Err("give the recipe's `path`".into());
    }
    run(
        "EXT_VIEW_OPEN",
        json!({ "extension": crate::ID, "view": view }),
    )
    .await?;
    Ok(json!({ "opened": view }))
}

/// `COOKBOOK_TIMER {seconds, label?}`: a kitchen timer: the device's
/// notification when it is done (`after_s`), else a toast when it is
/// set, and its id to cancel it with.
pub async fn timer(a: Value) -> Result<Value, String> {
    let seconds = a
        .get("seconds")
        .and_then(Value::as_u64)
        .filter(|s| *s > 0 && *s <= 24 * 3600)
        .ok_or("give `seconds`, 1 to 86400")?;
    let label = text(&a, "label").unwrap_or_else(|| "Timer".to_owned());
    let h = handle().ok_or("the cookbook is not running")?;
    let id: String = format!("timer-{label}-{seconds}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let os = h
        .host(HostRequest::OsNotify(
            architect_extensions::device::OsNotification {
                id: Some(id.clone()),
                title: format!("{label}: done"),
                body: Some(format!(
                    "{} is up.",
                    crate::ui::clock(u32::try_from(seconds).unwrap_or(u32::MAX))
                )),
                at_ms: None,
                after_s: Some(seconds),
            },
        ))
        .await;
    if os.is_ok() {
        Ok(json!({ "id": id, "seconds": seconds, "notify": "os" }))
    } else {
        // No device notifications here (a desk, a browser): say it
        // was set; the view counts it down.
        let _ = h
            .host(HostRequest::Notify {
                text: format!(
                    "{label}: {} timer set",
                    crate::ui::clock(u32::try_from(seconds).unwrap_or(u32::MAX))
                ),
                kind: None,
                title: None,
            })
            .await;
        Ok(json!({ "id": id, "seconds": seconds, "notify": "app" }))
    }
}

/// `COOKBOOK_IMPORT {url? | text?, path?, save?}`.
pub async fn import(a: Value) -> Result<Value, String> {
    let import: Import = args(a)?;
    let c = client().await.map_err(|e| e.to_string())?;
    out(c.import(import).await.map_err(architect::vox_into_app))
}

/// `COOKBOOK_TIDY {folder?}`.
pub async fn tidy(a: Value) -> Result<Value, String> {
    let folder = text(&a, "folder");
    let c = client().await.map_err(|e| e.to_string())?;
    out(c
        .tidy(folder)
        .await
        .map_err(architect::vox_into_app)
        .map(|changed| json!({ "changed": changed })))
}
