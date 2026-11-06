//! The comments pane (`[[views]] id = "comments"`, the right dock): the
//! threads on the note being read, a field to start one, each thread's
//! comments with replies under what they answer, a reply field, and
//! Resolve or Reopen. It follows the focused note (the app's selection,
//! `reads = ["selection"]`) and stays on it while the pane itself has
//! focus.

use std::sync::atomic::Ordering;
use std::time::Duration;

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::model::{self, Comment, Thread};
use crate::{REVISION, bump, cache, host};

/// The vault note view's kind in the app (`apps/ui/src/vault.rs`).
const NOTE_VIEW: &str = "vault.note";

/// Re-renders when what it knows changes; follows the focused note, and
/// reads again every few seconds (a comment from another device).
fn use_threads() -> u64 {
    let mut seen = use_signal(|| REVISION.load(Ordering::Relaxed));
    use_future(move || async move {
        let _ = crate::refresh().await;
        let mut ticks = 0_u32;
        loop {
            follow_focus().await;
            futures_timer::Delay::new(Duration::from_millis(400)).await;
            ticks = ticks.wrapping_add(1);
            if ticks.is_multiple_of(10) {
                let _ = crate::refresh().await;
            }
            let now = REVISION.load(Ordering::Relaxed);
            if *seen.peek() != now {
                seen.set(now);
            }
        }
    });
    seen()
}

/// The note the app has focused, when it is a note.
pub async fn follow_focus() {
    let sel = host::selection().await;
    if sel["focused_view"].as_str() != Some(NOTE_VIEW) {
        return;
    }
    let Some(path) = sel["focused_state"].as_str().filter(|p| !p.is_empty()) else {
        return;
    };
    let changed = {
        let mut c = cache();
        if c.current.as_deref() == Some(path) {
            false
        } else {
            c.current = Some(path.to_owned());
            c.focus = None;
            true
        }
    };
    if changed {
        bump();
    }
}

fn ago(at: &str) -> String {
    let Ok(t) = DateTime::parse_from_rfc3339(at) else {
        return at.to_owned();
    };
    let secs = (Utc::now() - t.with_timezone(&Utc)).num_seconds().max(0);
    match secs {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{}m", secs / 60),
        3600..86_400 => format!("{}h", secs / 3600),
        86_400..604_800 => format!("{}d", secs / 86_400),
        _ => t.format("%b %-d").to_string(),
    }
}

const MUTED: &str = "color: var(--muted-foreground);";
const FIELD: &str = "flex: 1; min-width: 0; padding: 6px 8px; border-radius: 6px; border: 1px solid var(--input); \
                     background: var(--background); color: var(--foreground); font-size: 13px;";

fn button(primary: bool) -> String {
    format!(
        "padding: 4px 10px; border-radius: 6px; font-size: 12px; cursor: pointer; border: 1px solid var(--border); \
         background: {}; color: {};",
        if primary {
            "var(--primary)"
        } else {
            "transparent"
        },
        if primary {
            "var(--primary-foreground)"
        } else {
            "var(--foreground)"
        }
    )
}

fn run(
    work: impl std::future::Future<Output = Result<Value, String>> + 'static,
    mut problem: Signal<Option<String>>,
) {
    spawn(async move {
        match work.await {
            Ok(_) => problem.set(None),
            Err(e) => problem.set(Some(e)),
        }
    });
}

/// The pane.
#[component]
pub fn CommentsPane() -> Element {
    let _ = use_threads();
    let mut draft = use_signal(String::new);
    let mut show_resolved = use_signal(|| false);
    let problem = use_signal(|| None::<String>);
    let (current, focus, threads, error) = {
        let c = cache();
        (
            c.current.clone(),
            c.focus.clone(),
            c.threads.clone(),
            c.error.clone(),
        )
    };
    let Some(path) = current else {
        return rsx! {
            div { class: "threads-pane", style: "padding: 20px; font-size: 13px; {MUTED}",
                "Open a note to see its comments."
            }
        };
    };
    let all: Vec<Thread> = model::on(&threads, model::NOTE, &path, None, true)
        .into_iter()
        .cloned()
        .collect();
    let resolved = all.iter().filter(|t| t.resolved).count();
    let mut shown: Vec<Thread> = all
        .into_iter()
        .filter(|t| show_resolved() || !t.resolved || focus.as_deref() == Some(t.id.as_str()))
        .collect();
    // The thread a notification went to comes first.
    if let Some(f) = &focus {
        shown.sort_by_key(|t| &t.id != f);
    }
    let title = crate::title_of(&path);
    rsx! {
        div { class: "threads-pane", "data-note": "{path}",
            style: "display: flex; flex-direction: column; gap: 12px; padding: 14px 16px; height: 100%; font-family: var(--default-font-family, var(--font-sans)); \
                    box-sizing: border-box; overflow-y: auto; color: var(--foreground); font-size: 13px;",
            div { style: "display: flex; align-items: baseline; gap: 8px;",
                span { style: "font-size: 15px; font-weight: 700;", "Comments" }
                span { style: "flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; {MUTED}", "{title}" }
                if resolved > 0 {
                    button { style: button(show_resolved()), onclick: move |_| show_resolved.toggle(),
                        "Resolved ({resolved})"
                    }
                }
            }
            div { style: "display: flex; gap: 6px;",
                input { class: "threads-new", style: FIELD, value: "{draft}",
                    placeholder: "Comment on this note… @name to mention",
                    oninput: move |e: FormEvent| draft.set(e.value()),
                    onkeydown: {
                        let p = path.clone();
                        move |e: KeyboardEvent| if e.key() == Key::Named(NamedKey::Enter) { start_thread(draft, problem, &p) }
                    },
                }
                button { style: button(true), onclick: {
                        let p = path;
                        move |_| start_thread(draft, problem, &p)
                    }, "Comment" }
            }
            if let Some(e) = problem().or(error) {
                div { style: "color: var(--destructive); font-size: 12px;", "{e}" }
            }
            if shown.is_empty() {
                div { style: "{MUTED} padding: 8px 0;", "No comments yet." }
            }
            for t in shown {
                ThreadCard { key: "{t.id}", focused: focus.as_deref() == Some(t.id.as_str()), thread: t }
            }
        }
    }
}

/// Starts a thread on `path` with what the field holds.
fn start_thread(mut draft: Signal<String>, problem: Signal<Option<String>>, path: &str) {
    let body = draft.peek().trim().to_owned();
    if body.is_empty() {
        return;
    }
    draft.set(String::new());
    run(crate::start(json!({ "path": path, "body": body })), problem);
}

/// Posts what a thread's reply field holds.
fn send_reply(
    mut reply: Signal<String>,
    mut reply_to: Signal<Option<String>>,
    problem: Signal<Option<String>>,
    thread: &str,
) {
    let body = reply.peek().trim().to_owned();
    if body.is_empty() {
        return;
    }
    reply.set(String::new());
    let to = reply_to.peek().clone();
    reply_to.set(None);
    run(
        crate::post(json!({ "thread": thread, "body": body, "reply_to": to })),
        problem,
    );
}

/// Comments with replies under what they answer, depth first.
fn ordered(comments: &[Comment]) -> Vec<(usize, Comment)> {
    fn walk(all: &[Comment], parent: Option<&str>, depth: usize, out: &mut Vec<(usize, Comment)>) {
        for c in all.iter().filter(|c| c.reply_to.as_deref() == parent) {
            out.push((depth, c.clone()));
            walk(all, Some(&c.id), depth.saturating_add(1).min(4), out);
        }
    }
    let mut out = Vec::new();
    walk(comments, None, 0, &mut out);
    // A reply whose comment is gone still shows, at the top level.
    for c in comments {
        if !out.iter().any(|(_, o)| o.id == c.id) {
            out.push((0, c.clone()));
        }
    }
    out
}

#[component]
fn ThreadCard(thread: Thread, focused: bool) -> Element {
    let mut reply = use_signal(String::new);
    let mut reply_to = use_signal(|| None::<String>);
    let problem = use_signal(|| None::<String>);
    let id = thread.id.clone();
    let border = if focused {
        "var(--primary)"
    } else {
        "var(--border)"
    };
    let opacity = if thread.resolved { "0.7" } else { "1" };
    let toggle_id = id.clone();
    let resolved = thread.resolved;
    let replying = reply_to().and_then(|r| {
        thread
            .comments
            .iter()
            .find(|c| c.id == r)
            .map(|c| c.author.clone())
    });
    rsx! {
        div { class: "thread-card", "data-thread": "{id}", "data-resolved": "{resolved}",
            style: "display: flex; flex-direction: column; gap: 8px; padding: 10px 12px; border-radius: 8px; \
                    border: 1px solid {border}; background: var(--card); opacity: {opacity};",
            div { style: "display: flex; align-items: center; gap: 8px;",
                span { style: "font-weight: 600; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;", "{thread.title}" }
                if resolved {
                    span { style: "font-size: 11px; {MUTED}",
                        "Resolved"
                        if let Some(by) = &thread.resolved_by { " by {by}" }
                    }
                }
                button { style: button(false),
                    onclick: move |_| run(crate::resolve(json!({ "thread": toggle_id }), !resolved), problem),
                    if resolved { "Reopen" } else { "Resolve" }
                }
            }
            for (depth, c) in ordered(&thread.comments) {
                CommentRow { key: "{c.id}", depth, comment: c,
                    on_reply: move |cid: String| reply_to.set(Some(cid)) }
            }
            if let Some(who) = replying {
                div { style: "font-size: 11px; {MUTED}", "Replying to {who} · "
                    span { style: "cursor: pointer; text-decoration: underline;", onclick: move |_| reply_to.set(None), "cancel" }
                }
            }
            div { style: "display: flex; gap: 6px;",
                input { class: "thread-reply", style: FIELD, value: "{reply}", placeholder: "Reply…",
                    oninput: move |e: FormEvent| reply.set(e.value()),
                    onkeydown: {
                        let t = id.clone();
                        move |e: KeyboardEvent| if e.key() == Key::Named(NamedKey::Enter) { send_reply(reply, reply_to, problem, &t) }
                    },
                }
                button { style: button(false), onclick: {
                        let t = id;
                        move |_| send_reply(reply, reply_to, problem, &t)
                    }, "Reply" }
            }
            if let Some(e) = problem() {
                div { style: "color: var(--destructive); font-size: 12px;", "{e}" }
            }
        }
    }
}

/// A comment's words, a mention drawn as one.
fn words(body: &str) -> Vec<(bool, String)> {
    let mut out: Vec<(bool, String)> = Vec::new();
    for (i, w) in body.split(' ').enumerate() {
        let sep = if i == 0 { "" } else { " " };
        let mention = w.starts_with('@') && w.len() > 1;
        match out.last_mut() {
            Some((false, text)) if !mention => {
                text.push_str(sep);
                text.push_str(w);
            }
            _ => {
                if !sep.is_empty() {
                    out.push((false, sep.to_owned()));
                }
                out.push((mention, w.to_owned()));
            }
        }
    }
    out
}

#[component]
fn CommentRow(comment: Comment, depth: usize, on_reply: EventHandler<String>) -> Element {
    let indent = depth.saturating_mul(14);
    let when = ago(&comment.posted);
    let cid = comment.id.clone();
    let initial: String = comment
        .author
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    rsx! {
        div { class: "thread-comment", "data-comment": "{comment.id}",
            style: "display: flex; gap: 8px; margin-left: {indent}px;",
            div { style: "flex: none; width: 22px; height: 22px; border-radius: 999px; background: var(--muted); \
                          display: flex; align-items: center; justify-content: center; font-size: 11px; font-weight: 700;",
                "{initial}"
            }
            div { style: "display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1;",
                div { style: "display: flex; gap: 6px; align-items: baseline;",
                    span { style: "font-weight: 600;", "{comment.author}" }
                    span { style: "font-size: 11px; {MUTED}", "{when}" }
                    if comment.edited.is_some() { span { style: "font-size: 11px; {MUTED}", "(edited)" } }
                    span { style: "flex: 1;" }
                    span { style: "font-size: 11px; cursor: pointer; {MUTED}", onclick: move |_| on_reply.call(cid.clone()), "Reply" }
                }
                div { style: "white-space: pre-wrap; overflow-wrap: anywhere;",
                    for (mention, w) in words(&comment.body) {
                        if mention {
                            span { style: "color: var(--primary); font-weight: 600;", "{w}" }
                        } else {
                            span { "{w}" }
                        }
                    }
                }
            }
        }
    }
}
