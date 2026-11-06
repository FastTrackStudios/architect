//! The notification centre (`[[views]] id = "centre"`, an overlay under
//! the bell): your notifications newest first, unread marked, each with
//! "Go to" and mark read; mark all read and clear; the session's toasts
//! below.

use std::sync::atomic::Ordering;
use std::time::Duration;

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::model::{Kind, Notification};
use crate::{REVISION, cache};

fn use_notifications() -> u64 {
    let mut seen = use_signal(|| REVISION.load(Ordering::Relaxed));
    use_future(move || async move {
        let _ = crate::refresh().await;
        let mut ticks = 0_u32;
        loop {
            futures_timer::Delay::new(Duration::from_millis(300)).await;
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

fn ago(at: &str) -> String {
    let Ok(t) = DateTime::parse_from_rfc3339(at) else {
        return at.to_owned();
    };
    let secs = (Utc::now() - t.with_timezone(&Utc)).num_seconds().max(0);
    match secs {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86_400 => format!("{}h ago", secs / 3600),
        86_400..604_800 => format!("{}d ago", secs / 86_400),
        _ => t.format("%b %-d").to_string(),
    }
}

/// A kind's mark colour: a theme token.
const fn tint(kind: Kind) -> &'static str {
    match kind {
        Kind::Mention | Kind::ThreadReply => "var(--primary)",
        Kind::TaskAssigned | Kind::TaskDue | Kind::TaskCompleted => {
            "var(--chart-2, var(--primary))"
        }
        Kind::ShareOpened | Kind::DeviceEnrolled => "var(--chart-4, var(--muted-foreground))",
        Kind::Toast | Kind::Other => "var(--muted-foreground)",
    }
}

const MUTED: &str = "color: var(--muted-foreground);";

fn chip(active: bool) -> String {
    format!(
        "padding: 2px 9px; border-radius: 999px; font-size: 12px; cursor: pointer; border: 1px solid var(--border); \
         background: {}; color: {};",
        if active {
            "var(--primary)"
        } else {
            "transparent"
        },
        if active {
            "var(--primary-foreground)"
        } else {
            "var(--foreground)"
        }
    )
}

const LINK: &str = "border: none; background: transparent; padding: 2px 4px; font-size: 12px; cursor: pointer; \
                    color: var(--primary);";

fn act(
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

/// The centre.
#[component]
pub fn Centre() -> Element {
    let _ = use_notifications();
    let mut unread_only = use_signal(|| false);
    let problem = use_signal(|| None::<String>);
    let (me, items, toasts, error) = {
        let c = cache();
        (
            c.me.clone(),
            c.items.clone(),
            c.toasts.clone(),
            c.error.clone(),
        )
    };
    let unread = crate::model::unread(&items);
    let shown: Vec<Notification> = items
        .into_iter()
        .filter(|n| !unread_only() || n.unread())
        .collect();
    rsx! {
        div { class: "notify-centre", "data-unread": "{unread}",
            style: "display: flex; flex-direction: column; height: 100%; box-sizing: border-box; font-family: var(--default-font-family, var(--font-sans)); \
                    color: var(--foreground); font-size: 13px;",
            div { style: "display: flex; align-items: center; gap: 8px; padding: 10px 12px; border-bottom: 1px solid var(--border);",
                span { style: "font-weight: 700; font-size: 14px;", "Notifications" }
                if unread > 0 {
                    span { style: "padding: 0 7px; border-radius: 999px; font-size: 11px; font-weight: 700; \
                                   background: var(--primary); color: var(--primary-foreground);", "{unread}" }
                }
                span { style: "flex: 1;" }
                button { style: chip(!unread_only()), onclick: move |_| unread_only.set(false), "All" }
                button { style: chip(unread_only()), onclick: move |_| unread_only.set(true), "Unread" }
            }
            div { style: "display: flex; gap: 4px; padding: 4px 8px; border-bottom: 1px solid var(--border); {MUTED} font-size: 12px; align-items: center;",
                span { style: "flex: 1; padding-left: 4px;", "For {me}" }
                button { style: LINK, onclick: move |_| act(crate::read_all(json!({})), problem), "Mark all read" }
                button { style: LINK, onclick: move |_| act(crate::clear(json!({})), problem), "Clear read" }
            }
            if let Some(e) = problem().or(error) {
                div { style: "color: var(--destructive); font-size: 12px; padding: 6px 12px;", "{e}" }
            }
            div { style: "flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column;",
                if shown.is_empty() {
                    div { style: "padding: 28px 12px; text-align: center; {MUTED}",
                        if unread_only() { "No unread notifications." } else { "You're all caught up." }
                    }
                }
                for n in shown {
                    Row { key: "{n.id}", n, problem }
                }
                if !toasts.is_empty() {
                    div { style: "padding: 10px 12px 4px; font-size: 11px; font-weight: 700; letter-spacing: 0.06em; \
                                  text-transform: uppercase; {MUTED}", "This session" }
                    for (i, t) in toasts.into_iter().enumerate() {
                        div { key: "toast-{i}", class: "notify-toast-row",
                            style: "display: flex; flex-direction: column; gap: 1px; padding: 6px 12px;",
                            div { style: "display: flex; gap: 6px; align-items: baseline;",
                                span { style: "font-weight: 600;", if t.title.is_empty() { "{t.from}" } else { "{t.title}" } }
                                span { style: "flex: 1;" }
                                span { style: "font-size: 11px; {MUTED}", "{ago(&t.at)}" }
                            }
                            span { style: "{MUTED}", "{t.text}" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Row(n: Notification, problem: Signal<Option<String>>) -> Element {
    let kind = n.kind();
    let unread = n.unread();
    let weight = if unread { "600" } else { "400" };
    let bg = if unread {
        "color-mix(in srgb, var(--primary) 7%, transparent)"
    } else {
        "transparent"
    };
    let dot = if unread { tint(kind) } else { "transparent" };
    let can_go = !n.source.href.is_empty() || !n.source.service.is_empty();
    let id = n.id.clone();
    let go = id.clone();
    let mark = id.clone();
    let byline = match (n.actor.is_empty(), ago(&n.created)) {
        (true, when) => when,
        (false, when) => format!("{} · {when}", n.actor),
    };
    rsx! {
        div { class: "notify-row", "data-id": "{id}", "data-kind": "{n.kind}", "data-unread": "{unread}",
            style: "display: flex; gap: 10px; padding: 9px 12px; border-bottom: 1px solid var(--border); \
                    background: {bg}; cursor: pointer;",
            onclick: move |_| if can_go { act(crate::open(json!({ "id": go })), problem) },
            div { style: "flex: none; width: 8px; height: 8px; margin-top: 5px; border-radius: 999px; background: {dot};" }
            div { style: "display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1;",
                div { style: "display: flex; gap: 6px; align-items: baseline;",
                    span { style: "flex: none; font-size: 10px; font-weight: 700; letter-spacing: 0.04em; text-transform: uppercase; color: {tint(kind)};",
                        "{kind.label()}"
                    }
                    span { style: "font-weight: {weight}; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;", "{n.title}" }
                }
                if !n.body.is_empty() {
                    span { style: "{MUTED} overflow: hidden; text-overflow: ellipsis; white-space: nowrap;", "{n.body}" }
                }
                div { style: "display: flex; gap: 8px; align-items: center; font-size: 11px; {MUTED}",
                    span { style: "flex: 1;", "{byline}" }
                    if can_go {
                        span { style: "color: var(--primary); font-weight: 600;", "Go to →" }
                    }
                    if unread {
                        button { style: LINK, title: "Mark read",
                            onclick: move |e| {
                                e.stop_propagation();
                                act(crate::mark_read(json!({ "id": mark })), problem);
                            },
                            "Mark read"
                        }
                    }
                }
            }
        }
    }
}
