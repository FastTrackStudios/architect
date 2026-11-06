//! Notify (`features/notify/extension.toml`, #348): persistent
//! notifications with a typed source — a mention in a thread, a task
//! assigned or due, a share opened, a device enrolled — and the
//! notification centre in the shell. Ported from Task's
//! `task:features/notify`.
//!
//! One [`Notify`] value runs it two ways, as every first-party
//! extension: linked in, or served as a process by `src/main.rs`
//! (`EXT_CLONE {"id":"notify"}`).
//!
//! - **Data:** each person's notifications are one note,
//!   `Notifications/<name>.md` ([`model`]). A notification for someone
//!   else is a row added to their note, which reaches their devices as
//!   every note does: that is delivery across devices. A push relay
//!   (#200) is the seam [`PUSH`]: when an extension provides it, each
//!   new row is handed to its `send` as well.
//! - **The centre:** a bell in the status bar with the unread count; it
//!   opens the centre over the window (`NOTIFY_CENTRE`): unread and read
//!   rows, mark read, mark all read, clear, and "go to" (`NOTIFY_OPEN`:
//!   the source service's `reveal`, else its `href` opened). A row that
//!   arrives while the app runs shows as a toast too; the toasts other
//!   extensions show are kept in the centre for the session (the host
//!   hands them to `NOTIFY_TOASTED`).
//! - **Service:** `architect.notify` (each method its `NOTIFY_<METHOD>`)
//!   for extensions: Threads' mentions, Tasks' assignments, share links,
//!   devices.

#![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

pub mod host;
pub mod model;
mod views;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use architect_extensions::linked::{Builder, Extension};
use chrono::{SecondsFormat, Utc};
use serde_json::{Value, json};

use crate::model::{Kind, Notification, Source};

/// Its id: its actions are `NOTIFY_*`.
pub const ID: &str = "notify";

/// The service it provides (`[provides]`): method `m` is `NOTIFY_<M>`.
pub const SERVICE: &str = "architect.notify";

/// The push relay's service (#200), when an extension provides it: each
/// new notification is handed to its `send` (`{to, notification}`).
pub const PUSH: &str = "architect.push";

/// Its status item: the bell.
pub const BELL: &str = "bell";

/// The centre's view.
pub const CENTRE: &str = "centre";

/// Its settings.
pub mod settings {
    /// The name people mention you by, and whose notifications you see.
    pub const ME: &str = "notify.me";
    /// Whether a notification that arrives shows a toast.
    pub const TOASTS: &str = "notify.toasts";
}

/// A toast an extension showed, kept for the session.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Toasted {
    pub from: String,
    pub title: String,
    pub text: String,
    pub kind: String,
    pub at: String,
}

#[derive(Debug, Default)]
struct Cache {
    me: String,
    items: Vec<Notification>,
    toasts: Vec<Toasted>,
    /// Ids it has seen, so only a new row toasts.
    seen: BTreeSet<String>,
    /// It has read once (rows there at start do not toast).
    primed: bool,
    error: Option<String>,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache {
    me: String::new(),
    items: Vec::new(),
    toasts: Vec::new(),
    seen: BTreeSet::new(),
    primed: false,
    error: None,
});

static REVISION: AtomicU64 = AtomicU64::new(0);

fn cache() -> std::sync::MutexGuard<'static, Cache> {
    CACHE.lock().unwrap_or_else(PoisonError::into_inner)
}

fn bump() {
    REVISION.fetch_add(1, Ordering::Relaxed);
}

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// The name you go by: Settings → Notifications, else the account's user
/// name.
async fn me() -> String {
    if let Some(m) = host::setting(settings::ME)
        .await
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return m.to_owned();
    }
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "me".to_owned())
}

/// `args.who`, else you.
async fn who(args: &Value) -> String {
    match arg(args, "who") {
        Some(w) => w.to_owned(),
        None => me().await,
    }
}

/// The rows of `who`'s note (none when it has no note yet).
async fn read_for(who: &str) -> Result<(String, Vec<Notification>, bool), String> {
    let path = model::path_for(who).ok_or("give a name")?;
    match host::read(&path).await {
        Ok(values) => Ok((path, model::items(&values), true)),
        Err(_) => Ok((path, Vec::new(), false)),
    }
}

async fn write_for(
    path: &str,
    exists: bool,
    who: &str,
    rows: &[Notification],
) -> Result<(), String> {
    let items = model::items_value(rows);
    if !exists {
        let mut props = model::new_note(who);
        props.insert(model::keys::ITEMS.to_owned(), items);
        let body = format!(
            "Notifications for {who}. Architect keeps this note; read them in the notification centre.\n"
        );
        if host::create(path, &props, &body).await.is_ok() {
            return Ok(());
        }
        // Made meanwhile (another device's row synced in): add to it.
        let values = host::read(path).await?;
        let mut merged = model::items(&values);
        for r in rows {
            if !merged.iter().any(|m| m.id == r.id) {
                merged.push(r.clone());
            }
        }
        merged.sort_by(|a, b| b.created.cmp(&a.created));
        return host::set(path, model::keys::ITEMS, &model::items_value(&merged)).await;
    }
    host::set(path, model::keys::ITEMS, &items).await
}

/// Reads your notes again: the bell, and a toast for each row that
/// arrived since.
// The cache is held while what arrived is worked out, then let go.
#[allow(clippy::significant_drop_tightening)]
async fn refresh() -> Result<Vec<Notification>, String> {
    let me = me().await;
    let read = read_for(&me).await.map(|(_, rows, _)| rows);
    let fresh: Vec<Notification> = {
        let mut c = cache();
        if c.me != me {
            c.me.clone_from(&me);
            c.primed = false;
            c.seen.clear();
        }
        match &read {
            Ok(rows) => {
                let fresh: Vec<Notification> = if c.primed {
                    model::arrived(rows, &c.seen).into_iter().cloned().collect()
                } else {
                    Vec::new()
                };
                c.seen.extend(rows.iter().map(|n| n.id.clone()));
                c.items.clone_from(rows);
                c.primed = true;
                c.error = None;
                fresh
            }
            Err(e) => {
                c.error = Some(e.clone());
                Vec::new()
            }
        }
    };
    if let Ok(rows) = &read {
        if let Some(h) = host::handle() {
            h.status(BELL, model::bell_text(model::unread(rows)));
        }
        let toasts = host::setting(settings::TOASTS)
            .await
            .as_bool()
            .unwrap_or(true);
        if toasts {
            for n in fresh.iter().take(3) {
                let text = if n.body.is_empty() {
                    n.title.clone()
                } else {
                    format!("{} — {}", n.title, n.body)
                };
                host::toast(n.kind().label(), &text).await;
            }
        }
    }
    bump();
    read
}

/// The people `args.to` names: a name, or a list of them.
fn recipients(args: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |s: &str| {
        let s = s.trim();
        if !s.is_empty() && !out.iter().any(|o| model::same_person(o, s)) {
            out.push(s.trim_start_matches('@').to_owned());
        }
    };
    match args.get("to") {
        Some(Value::String(s)) => s.split(',').for_each(&mut add),
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).for_each(&mut add),
        _ => {}
    }
    out
}

/// `NOTIFY_POST {to, kind, title, body?, actor?, source?, self?}`: a
/// notification for each person `to` names (not the actor, unless
/// `self`), added to their note; the push relay told when there is one.
async fn post(args: Value) -> Result<Value, String> {
    let to = recipients(&args);
    if to.is_empty() {
        return Err("give `to`: a name or a list of names".to_owned());
    }
    let title = arg(&args, "title").ok_or("give `title`")?.to_owned();
    let kind = arg(&args, "kind").unwrap_or("other").to_owned();
    let actor = arg(&args, "actor").unwrap_or_default().to_owned();
    let to_self = args.get("self").and_then(Value::as_bool).unwrap_or(false);
    let source: Source = args
        .get("source")
        .and_then(|s| serde_json::from_value(s.clone()).ok())
        .unwrap_or_default();
    let mut sent = Vec::new();
    for who in to {
        if !to_self && model::same_person(&who, &actor) {
            continue;
        }
        let n = Notification {
            id: uuid::Uuid::new_v4().to_string(),
            kind: kind.clone(),
            title: title.clone(),
            body: arg(&args, "body").unwrap_or_default().to_owned(),
            source: source.clone(),
            actor: actor.clone(),
            created: now(),
            read: None,
        };
        let (path, mut rows, exists) = read_for(&who).await?;
        rows.insert(0, n.clone());
        write_for(&path, exists, &who, &rows).await?;
        // The seam for the push relay (#200): nothing provides it yet.
        let pushed = matches!(
            host::service(PUSH, "send", json!({ "to": who, "notification": n })).await,
            Some(Ok(_))
        );
        sent.push(json!({ "to": who, "id": n.id, "path": path, "pushed": pushed }));
    }
    let _ = refresh().await;
    Ok(json!({ "sent": sent }))
}

/// `NOTIFY_LIST {who?, unread_only?, limit?}`: newest first.
async fn list(args: Value) -> Result<Value, String> {
    let who = who(&args).await;
    let rows = if model::same_person(&who, &me().await) {
        refresh().await?
    } else {
        read_for(&who).await?.1
    };
    let unread_only = args
        .get("unread_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(100);
    let shown: Vec<&Notification> = rows
        .iter()
        .filter(|n| !unread_only || n.unread())
        .take(limit)
        .collect();
    Ok(json!({
        "who": who,
        "unread": model::unread(&rows),
        "notifications": shown,
        "toasts": cache().toasts.clone(),
    }))
}

/// Changes `who`'s rows with `change` and writes them back: the rows
/// after.
async fn change(
    who: &str,
    change: impl FnOnce(&mut Vec<Notification>) -> Result<(), String>,
) -> Result<Vec<Notification>, String> {
    let (path, mut rows, exists) = read_for(who).await?;
    let before = rows.clone();
    change(&mut rows)?;
    if rows != before {
        write_for(&path, exists, who, &rows).await?;
    }
    let _ = refresh().await;
    Ok(rows)
}

/// `NOTIFY_READ {id, who?}` (`read: false` marks it unread again).
async fn mark_read(args: Value) -> Result<Value, String> {
    let id = arg(&args, "id").ok_or("give `id`")?.to_owned();
    let read = args.get("read").and_then(Value::as_bool).unwrap_or(true);
    let rows = change(&who(&args).await, |rows| {
        let n = rows
            .iter_mut()
            .find(|n| n.id == id)
            .ok_or_else(|| format!("no notification {id}"))?;
        if read && n.read.is_none() {
            n.read = Some(now());
        } else if !read {
            n.read = None;
        }
        Ok(())
    })
    .await?;
    Ok(json!(rows.iter().find(|n| n.id == id)))
}

/// `NOTIFY_READ_ALL {who?}`: how many it marked.
async fn read_all(args: Value) -> Result<Value, String> {
    let mut marked = 0_usize;
    let at = now();
    change(&who(&args).await, |rows| {
        for n in rows.iter_mut().filter(|n| n.read.is_none()) {
            n.read = Some(at.clone());
            marked = marked.saturating_add(1);
        }
        Ok(())
    })
    .await?;
    Ok(json!({ "marked": marked }))
}

/// `NOTIFY_DELETE {id, who?}`.
async fn delete(args: Value) -> Result<Value, String> {
    let id = arg(&args, "id").ok_or("give `id`")?.to_owned();
    let rows = change(&who(&args).await, |rows| {
        rows.retain(|n| n.id != id);
        Ok(())
    })
    .await?;
    Ok(json!({ "left": rows.len() }))
}

/// `NOTIFY_CLEAR {who?}`: the read ones, gone; and the session's toasts.
async fn clear(args: Value) -> Result<Value, String> {
    cache().toasts.clear();
    let rows = change(&who(&args).await, |rows| {
        rows.retain(Notification::unread);
        Ok(())
    })
    .await?;
    Ok(json!({ "left": rows.len() }))
}

/// `NOTIFY_OPEN {id}`: "go to". Marks it read, asks its source's service
/// to `reveal` it, else opens its `href`; the centre closes.
async fn open(args: Value) -> Result<Value, String> {
    let id = arg(&args, "id").ok_or("give `id`")?.to_owned();
    let me = me().await;
    let rows = change(&me, |rows| {
        if let Some(n) = rows.iter_mut().find(|n| n.id == id)
            && n.read.is_none()
        {
            n.read = Some(now());
        }
        Ok(())
    })
    .await?;
    let n = rows
        .into_iter()
        .find(|n| n.id == id)
        .ok_or_else(|| format!("no notification {id}"))?;
    let s = &n.source;
    let mut went = Value::Null;
    let mut why = None;
    if !s.service.is_empty() && s.service != SERVICE {
        let ask = json!({ "entity": s.entity, "comment": s.comment, "href": s.href });
        match host::service(&s.service, "reveal", ask).await {
            Some(Ok(v)) => went = json!({ "revealed_by": s.service, "answer": v }),
            Some(Err(e)) => why = Some(e),
            None => {}
        }
    }
    if went.is_null() && !s.href.is_empty() {
        went = match host::open(&s.href).await {
            Ok(v) => json!({ "opened": v }),
            Err(e) => json!({ "href": s.href, "error": e, "reveal_error": why }),
        };
    }
    let _ = host::run("EXT_VIEW_CLOSE", json!({ "extension": ID, "view": CENTRE })).await;
    Ok(json!({ "notification": n, "went": went }))
}

/// `NOTIFY_ME {name?}`: the name you go by; with `name`, set first.
async fn me_action(args: Value) -> Result<Value, String> {
    if let Some(name) = arg(&args, "name") {
        host::set_setting(settings::ME, json!(name)).await?;
        let _ = refresh().await;
    }
    Ok(json!({ "me": me().await }))
}

/// `NOTIFY_TOASTED {from, text, kind?, title?}`: a toast another
/// extension showed, kept in the centre for the session.
async fn toasted(args: Value) -> Result<Value, String> {
    let t = Toasted {
        from: arg(&args, "from").unwrap_or_default().to_owned(),
        title: arg(&args, "title").unwrap_or_default().to_owned(),
        text: arg(&args, "text").unwrap_or_default().to_owned(),
        kind: arg(&args, "kind").unwrap_or("info").to_owned(),
        at: now(),
    };
    {
        let mut c = cache();
        c.toasts.insert(0, t);
        c.toasts.truncate(50);
    }
    bump();
    Ok(json!({ "kept": true }))
}

/// The extension.
#[derive(Debug, Clone, Copy, Default)]
pub struct Notify;

impl Extension for Notify {
    fn manifest(&self) -> &'static str {
        include_str!("../extension.toml")
    }

    fn build(&self, ext: &mut Builder) {
        host::connect(ext.handle());
        ext.view(CENTRE, views::Centre)
            .action("NOTIFY_POST", post)
            .action("NOTIFY_LIST", list)
            .action("NOTIFY_UNREAD", |args| async move {
                let who = who(&args).await;
                let rows = read_for(&who).await?.1;
                Ok(json!({ "who": who, "unread": model::unread(&rows) }))
            })
            .action("NOTIFY_READ", mark_read)
            .action("NOTIFY_READ_ALL", read_all)
            .action("NOTIFY_DELETE", delete)
            .action("NOTIFY_CLEAR", clear)
            .action("NOTIFY_OPEN", open)
            .action("NOTIFY_CENTRE", |_| async {
                let _ = refresh().await;
                host::run("EXT_VIEW_OPEN", json!({ "extension": ID, "view": CENTRE })).await
            })
            .action("NOTIFY_ME", me_action)
            .action("NOTIFY_KINDS", |_| async {
                Ok(json!(
                    Kind::ALL.iter().map(|k| k.as_str()).collect::<Vec<_>>()
                ))
            })
            .action("NOTIFY_TOASTED", toasted)
            .action("NOTIFY_POLL", |_| async {
                let rows = refresh().await?;
                Ok(json!({ "unread": model::unread(&rows) }))
            });
    }
}
