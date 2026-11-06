//! Threads (`features/threads/extension.toml`, #348): conversations
//! anchored to any entity — a note, a project, a task, an event, a shared
//! page — with comments, replies, mentions and resolve/reopen. Ported
//! from Task's `task:features/threads`.
//!
//! One [`Threads`] value runs it two ways, as every first-party
//! extension: the app links it in (`Extensions::with_linked`), and its
//! program (`src/main.rs`) serves the same value as a process, so
//! `EXT_CLONE {"id":"threads"}` gives a development copy.
//!
//! - **Data:** a thread is a note in `Threads/` ([`model`]): its anchor,
//!   title, kind, resolved state and comments in the frontmatter. It syncs
//!   as every note does, so a comment reaches the org's other devices and
//!   the headless node without a store of its own. It is read with one
//!   `BASES_ROWS` query and written with `VAULT_CREATE` and
//!   `VAULT_SET_PROPERTY`, reading the note right before each write.
//! - **Mentions:** `@name` in a comment notifies that person through
//!   `architect.notify` (`[uses]`), and a reply notifies the thread's
//!   other participants. Without Notify, comments still work.
//! - **Views:** a comments pane in the right dock that follows the note
//!   being read (`THREADS_PANE`); a badge with the open threads on each
//!   commented note's tab and file row.
//! - **Service:** `architect.threads`, each method its `THREADS_<METHOD>`
//!   action, for extensions (Projects, Tasks, Media Review) and share
//!   links that let a guest comment.

// Its actions read JSON with `value["key"]`: serde_json's index answers
// `Null` for what is missing, never a panic.
#![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

pub mod host;
pub mod model;
mod views;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use architect_extensions::linked::{Builder, Extension};
use chrono::{SecondsFormat, Utc};
use serde_json::{Map, Value, json};

use crate::model::{Anchor, Comment, Thread};

/// Its id: its actions are `THREADS_*`.
pub const ID: &str = "threads";

/// The service it provides (`[provides]`): method `m` is `THREADS_<M>`.
pub const SERVICE: &str = "architect.threads";

/// The service it notifies people through, when it is there.
pub const NOTIFY: &str = "architect.notify";

/// Its comments pane.
pub const PANE: &str = "comments";

/// The base that finds every thread.
#[must_use]
pub fn threads_base() -> String {
    let mut out = String::from(
        "filters:\n  and:\n    - type == \"thread\"\nviews:\n  - type: table\n    name: Threads\n    order:\n",
    );
    for k in model::keys::ALL {
        out.push_str("      - ");
        out.push_str(k);
        out.push('\n');
    }
    out
}

/// What it knows of the vault's threads, as last read.
#[derive(Debug, Default)]
struct Cache {
    threads: Vec<Thread>,
    /// The badges it has set, by note path.
    badges: BTreeMap<String, String>,
    /// The note the pane follows.
    current: Option<String>,
    /// The thread the pane shows first (a notification's "go to").
    focus: Option<String>,
    error: Option<String>,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache {
    threads: Vec::new(),
    badges: BTreeMap::new(),
    current: None,
    focus: None,
    error: None,
});

/// Moves on with every read and write, for the views.
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

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Who is writing: `author` when given, else the name Notify knows the
/// person by, else the account's user name.
async fn author(args: &Value) -> String {
    if let Some(a) = arg(args, "author") {
        return a.to_owned();
    }
    if let Some(Ok(v)) = host::service(NOTIFY, "me", json!({})).await
        && let Some(me) = v
            .get("me")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    {
        return me.to_owned();
    }
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "me".to_owned())
}

/// Reads every thread again, and puts the badges right.
// The cache is held while the badges are worked out, then let go.
#[allow(clippy::significant_drop_tightening)]
async fn refresh() -> Result<Vec<Thread>, String> {
    let rows = host::rows(&threads_base()).await;
    let read: Result<Vec<Thread>, String> = rows.map(|rows| {
        rows.iter()
            .filter_map(|r| Thread::from_values(&r.path, &r.values))
            .collect()
    });
    let (set, unset) = {
        let mut c = cache();
        match &read {
            Ok(t) => {
                c.threads.clone_from(t);
                c.error = None;
            }
            Err(e) => c.error = Some(e.clone()),
        }
        let want: BTreeMap<String, String> = model::counts(&c.threads)
            .into_iter()
            .filter(|(_, n)| n.open > 0)
            .filter_map(|(k, n)| {
                k.strip_prefix("note:")
                    .map(|p| (p.to_owned(), n.open.to_string()))
            })
            .collect();
        let set: Vec<(String, String)> = want
            .iter()
            .filter(|(p, b)| c.badges.get(*p) != Some(*b))
            .map(|(p, b)| (p.clone(), b.clone()))
            .collect();
        let unset: Vec<String> = c
            .badges
            .keys()
            .filter(|p| !want.contains_key(*p))
            .cloned()
            .collect();
        c.badges = want;
        (set, unset)
    };
    for (path, b) in set {
        host::badge(&path, Some(b)).await;
    }
    for path in unset {
        host::badge(&path, None).await;
    }
    bump();
    read
}

/// The thread `args.thread` names (its id, or its note's path), read from
/// its note now.
async fn find(args: &Value) -> Result<Thread, String> {
    let which = arg(args, "thread").ok_or("give `thread` (its id or its note's path)")?;
    let path = if std::path::Path::new(which)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
    {
        which.to_owned()
    } else {
        let known = cache()
            .threads
            .iter()
            .find(|t| t.id == which)
            .map(|t| t.path.clone());
        match known {
            Some(p) => p,
            None => refresh()
                .await?
                .into_iter()
                .find(|t| t.id == which)
                .map(|t| t.path)
                .ok_or_else(|| format!("no thread {which}"))?,
        }
    };
    let values = host::read(&path).await?;
    Thread::from_values(&path, &values).ok_or_else(|| format!("{path} is not a thread"))
}

/// The entity `args` names: `entity_type` + `entity_id`, or a note's
/// `path`, or the note the pane follows.
fn anchor_of(args: &Value) -> Result<Anchor, String> {
    let place = model::anchor_path(arg(args, "anchor_path"));
    let mut anchor = match (
        arg(args, "entity_type"),
        arg(args, "entity_id"),
        arg(args, "path"),
    ) {
        (Some(t), Some(id), _) => Anchor {
            entity_type: t.to_owned(),
            entity_id: id.to_owned(),
            anchor_path: None,
        },
        (None, Some(id), _) | (_, None, Some(id)) => Anchor::note(id),
        _ => Anchor::note(
            &cache()
                .current
                .clone()
                .ok_or("give `path`, or `entity_type` and `entity_id`")?,
        ),
    };
    anchor.anchor_path = place;
    Ok(anchor)
}

fn thread_json(t: &Thread) -> Value {
    serde_json::to_value(t).unwrap_or(Value::Null)
}

/// `THREADS_LIST {path | entity_type + entity_id, anchor_path?, resolved?}`.
async fn list(args: Value) -> Result<Value, String> {
    let anchor = anchor_of(&args)?;
    let all = refresh().await?;
    let resolved = args
        .get("resolved")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let shown = model::on(
        &all,
        &anchor.entity_type,
        &anchor.entity_id,
        anchor.anchor_path.as_deref(),
        resolved,
    );
    Ok(json!({
        "entity": anchor,
        "open": shown.iter().filter(|t| !t.resolved).count(),
        "threads": shown.iter().map(|t| thread_json(t)).collect::<Vec<_>>(),
    }))
}

/// Tells the people a comment reaches: whoever it mentions, and on a
/// reply the thread's other participants. Their names, as notified.
async fn notify(thread: &Thread, comment: &Comment, first: bool) -> Vec<String> {
    let place = if thread.anchor.entity_type == model::NOTE {
        title_of(&thread.anchor.entity_id)
    } else {
        format!("{} {}", thread.anchor.entity_type, thread.anchor.entity_id)
    };
    let href = if thread.anchor.entity_type == model::NOTE {
        thread.anchor.entity_id.clone()
    } else {
        thread.path.clone()
    };
    let mut told: Vec<String> = Vec::new();
    let mut tell = |who: &str, kind: &str, title: String| {
        if model::same_person(who, &comment.author)
            || told.iter().any(|t| model::same_person(t, who))
        {
            return None;
        }
        told.push(who.to_owned());
        Some(json!({
            "to": who,
            "kind": kind,
            "title": title,
            "body": model::excerpt(&comment.body, 140),
            "actor": comment.author,
            "source": {
                "service": SERVICE,
                "entity": thread.id,
                "href": href,
                "comment": comment.id,
            },
        }))
    };
    let mut posts = Vec::new();
    for m in &comment.mentions {
        posts.extend(tell(
            m,
            "mention",
            format!("{} mentioned you on {place}", comment.author),
        ));
    }
    if !first {
        for p in thread.participants() {
            posts.extend(tell(
                &p,
                "thread-reply",
                format!("{} replied on {place}", comment.author),
            ));
        }
    }
    let mut out = Vec::new();
    for p in posts {
        let to = p["to"].as_str().unwrap_or_default().to_owned();
        if let Some(Ok(_)) = host::service(NOTIFY, "post", p).await {
            out.push(to);
        }
    }
    out
}

fn title_of(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name).to_owned()
}

fn comment_of(args: &Value, author: String) -> Result<Comment, String> {
    let body = arg(args, "body").ok_or("give `body`")?.to_owned();
    Ok(Comment {
        id: new_id(),
        author,
        mentions: model::mentions(&body),
        body,
        reply_to: arg(args, "reply_to").map(str::to_owned),
        posted: now(),
        edited: None,
    })
}

/// `THREADS_START {path | entity_type + entity_id, anchor_path?, title?,
/// kind?, body, author?}`: a new thread, its first comment `body`.
async fn start(args: Value) -> Result<Value, String> {
    let anchor = anchor_of(&args)?;
    let who = author(&args).await;
    let comment = comment_of(&args, who.clone())?;
    let id = new_id();
    let thread = Thread {
        path: model::path_for(&anchor, &id),
        id,
        title: arg(&args, "title").map_or_else(|| model::excerpt(&comment.body, 60), str::to_owned),
        kind: arg(&args, "kind").unwrap_or("discussion").to_owned(),
        anchor,
        created_by: who,
        created: comment.posted.clone(),
        comments: vec![comment.clone()],
        ..Thread::default()
    };
    let body = if thread.anchor.entity_type == model::NOTE {
        let target = thread
            .anchor
            .entity_id
            .strip_suffix(".md")
            .unwrap_or(&thread.anchor.entity_id);
        format!("Comments on [[{target}]].\n")
    } else {
        String::new()
    };
    host::create(&thread.path, &thread.to_values(), &body).await?;
    let notified = notify(&thread, &comment, true).await;
    let _ = refresh().await;
    Ok(json!({ "thread": thread_json(&thread), "comment": comment, "notified": notified }))
}

/// `THREADS_POST {thread, body, reply_to?, author?}`: a comment in a
/// thread (a reply when `reply_to` names one of its comments). A resolved
/// thread reopens.
async fn post(args: Value) -> Result<Value, String> {
    let mut thread = find(&args).await?;
    let comment = comment_of(&args, author(&args).await)?;
    if let Some(to) = &comment.reply_to
        && !thread.comments.iter().any(|c| &c.id == to)
    {
        return Err(format!("no comment {to} in thread {}", thread.id));
    }
    thread.comments.push(comment.clone());
    host::set(
        &thread.path,
        model::keys::COMMENTS,
        &model::comments_value(&thread.comments),
    )
    .await?;
    if thread.resolved {
        set_resolved(&mut thread, false, None).await?;
    }
    let notified = notify(&thread, &comment, false).await;
    let _ = refresh().await;
    Ok(json!({ "thread": thread_json(&thread), "comment": comment, "notified": notified }))
}

async fn set_resolved(
    thread: &mut Thread,
    resolved: bool,
    by: Option<String>,
) -> Result<(), String> {
    thread.resolved = resolved;
    thread.resolved_at = resolved.then(now);
    thread.resolved_by = by.filter(|_| resolved);
    host::set(&thread.path, model::keys::RESOLVED, &json!(resolved)).await?;
    host::set(
        &thread.path,
        model::keys::RESOLVED_BY,
        &json!(thread.resolved_by),
    )
    .await?;
    host::set(
        &thread.path,
        model::keys::RESOLVED_AT,
        &json!(thread.resolved_at),
    )
    .await
}

/// `THREADS_RESOLVE {thread, by?}` / `THREADS_REOPEN {thread}`.
async fn resolve(args: Value, resolved: bool) -> Result<Value, String> {
    let mut thread = find(&args).await?;
    let by = if resolved {
        Some(author(&json!({ "author": args.get("by") })).await)
    } else {
        None
    };
    set_resolved(&mut thread, resolved, by).await?;
    let _ = refresh().await;
    Ok(thread_json(&thread))
}

/// `THREADS_EDIT {thread, comment, body}`: a comment's new text.
async fn edit(args: Value) -> Result<Value, String> {
    let mut thread = find(&args).await?;
    let id = arg(&args, "comment").ok_or("give `comment`")?;
    let body = arg(&args, "body").ok_or("give `body`")?.to_owned();
    let c = thread
        .comments
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or_else(|| format!("no comment {id}"))?;
    c.mentions = model::mentions(&body);
    c.body = body;
    c.edited = Some(now());
    let after = c.clone();
    host::set(
        &thread.path,
        model::keys::COMMENTS,
        &model::comments_value(&thread.comments),
    )
    .await?;
    let _ = refresh().await;
    Ok(json!({ "thread": thread_json(&thread), "comment": after }))
}

/// `THREADS_DELETE_COMMENT {thread, comment}`.
async fn delete_comment(args: Value) -> Result<Value, String> {
    let mut thread = find(&args).await?;
    let id = arg(&args, "comment").ok_or("give `comment`")?;
    let before = thread.comments.len();
    thread.comments.retain(|c| c.id != id);
    if thread.comments.len() != before {
        host::set(
            &thread.path,
            model::keys::COMMENTS,
            &model::comments_value(&thread.comments),
        )
        .await?;
    }
    let _ = refresh().await;
    Ok(json!({ "deleted": before - thread.comments.len(), "thread": thread_json(&thread) }))
}

/// `THREADS_DELETE {thread}`: the thread's note, gone.
async fn delete(args: Value) -> Result<Value, String> {
    let thread = find(&args).await?;
    host::run("VAULT_DELETE", json!({ "path": thread.path })).await?;
    let _ = refresh().await;
    Ok(json!({ "deleted": thread.id }))
}

/// `THREADS_COUNTS {entity_type?}`: each entity's open threads, threads
/// and comments, by `<type>:<id>`.
async fn counts(args: Value) -> Result<Value, String> {
    let all = refresh().await?;
    let only = arg(&args, "entity_type");
    let out: Map<String, Value> = model::counts(&all)
        .into_iter()
        .filter(|(k, _)| only.is_none_or(|t| k.starts_with(&format!("{t}:"))))
        .map(|(k, c)| (k, json!(c)))
        .collect();
    Ok(Value::Object(out))
}

/// `THREADS_MENTIONS {who?}`: where `who` (you, by default) is mentioned.
async fn mentions(args: Value) -> Result<Value, String> {
    let who = match arg(&args, "who") {
        Some(w) => w.to_owned(),
        None => author(&json!({})).await,
    };
    let all = refresh().await?;
    let found: Vec<Value> = model::mentions_of(&all, &who)
        .into_iter()
        .map(|(t, c)| json!({ "thread": t.id, "title": t.title, "entity": t.anchor, "comment": c }))
        .collect();
    Ok(json!({ "who": who, "mentions": found }))
}

/// `THREADS_PANE {path?}`: the comments pane, on `path` (else the note
/// being read).
async fn pane(args: Value) -> Result<Value, String> {
    if let Some(p) = arg(&args, "path") {
        cache().current = Some(p.to_owned());
        bump();
    } else {
        // The note focused now, before the pane takes the focus.
        views::follow_focus().await;
    }
    host::run("EXT_VIEW_OPEN", json!({ "extension": ID, "view": PANE })).await
}

/// `THREADS_REVEAL {thread | entity, comment?}`: a notification's "go
/// to": the thread's note opened, the pane on it, the thread first.
async fn reveal(args: Value) -> Result<Value, String> {
    let which = arg(&args, "thread").or_else(|| arg(&args, "entity"));
    let thread = find(&json!({ "thread": which })).await?;
    let note = (thread.anchor.entity_type == model::NOTE).then(|| thread.anchor.entity_id.clone());
    {
        let mut c = cache();
        c.current = Some(note.clone().unwrap_or_else(|| thread.path.clone()));
        c.focus = Some(thread.id.clone());
    }
    bump();
    let _ = host::open(note.as_deref().unwrap_or(&thread.path)).await;
    let _ = host::run("EXT_VIEW_OPEN", json!({ "extension": ID, "view": PANE })).await;
    Ok(json!({ "thread": thread.id, "path": note.unwrap_or(thread.path) }))
}

/// The extension.
#[derive(Debug, Clone, Copy, Default)]
pub struct Threads;

impl Extension for Threads {
    fn manifest(&self) -> &'static str {
        include_str!("../extension.toml")
    }

    fn build(&self, ext: &mut Builder) {
        host::connect(ext.handle());
        ext.view(PANE, views::CommentsPane)
            .action("THREADS_LIST", list)
            .action("THREADS_GET", |args| async move {
                find(&args).await.map(|t| thread_json(&t))
            })
            .action("THREADS_START", start)
            .action("THREADS_POST", post)
            .action("THREADS_REPLY", |mut args: Value| async move {
                if let Some(m) = args.as_object_mut()
                    && let Some(c) = m.remove("comment")
                {
                    m.insert("reply_to".to_owned(), c);
                }
                post(args).await
            })
            .action("THREADS_EDIT", edit)
            .action("THREADS_DELETE_COMMENT", delete_comment)
            .action("THREADS_RESOLVE", |args| resolve(args, true))
            .action("THREADS_REOPEN", |args| resolve(args, false))
            .action("THREADS_DELETE", delete)
            .action("THREADS_COUNTS", counts)
            .action("THREADS_MENTIONS", mentions)
            .action("THREADS_PANE", pane)
            .action("THREADS_REVEAL", reveal)
            .action("THREADS_STARTED", |_| async {
                let _ = refresh().await;
                Ok(json!("ready"))
            });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn its_base_parses_and_names_every_key() {
        let base = super::threads_base();
        assert!(base.contains("type == \"thread\""));
        for k in super::model::keys::ALL {
            assert!(base.contains(&format!("      - {k}\n")), "{k}");
        }
    }
}
