//! The sheet a person approves a caller's call in (feature `ui`): the
//! human side of the Action List's confirmation tickets
//! (`ActionList::require_confirmation`).
//!
//! When an agent (a model over MCP, a peer) runs a destructive action, the
//! list refuses it with a ticket and the call waits in
//! `ActionList::pending_confirmations`. [`ConfirmSheet`] shows the oldest:
//! the action by name and id, the arguments it would run with, who asked,
//! and how long the ticket has left. Allow once approves it (the caller's
//! retry runs it, once); Deny, or Esc, refuses it. Neither is an action on
//! the list: a caller must never be able to approve its own ticket.

use std::sync::Arc;
use std::time::Duration;

use architect::action::{ActionList, Confirmation, Effect};
use dioxus::prelude::*;
use lucide_dioxus::{Bot, ShieldAlert};

use crate::kit::theme;

/// How often the sheet looks for calls waiting: also its countdown's tick.
const POLL: Duration = Duration::from_millis(250);

/// The stacking layer the sheet draws on: over the palette (90), since a
/// waiting agent must not be hidden behind it.
const LAYER: u8 = 95;

/// Mounted once inside the app, over everything (absolute against the
/// app's root), as [`crate::CommandLayer`] is. Reads the app's Action List
/// from context (`Arc<ActionList>`); draws nothing while no call waits.
#[component]
pub fn ConfirmSheet() -> Element {
    let list = use_hook(try_consume_context::<Arc<ActionList>>);
    let mut pending = use_signal(Vec::<Confirmation>::new);
    let watched = list.clone();
    use_future(move || {
        let list = watched.clone();
        async move {
            let Some(list) = list else {
                return;
            };
            loop {
                let now = list.pending_confirmations();
                if *pending.peek() != now {
                    pending.set(now);
                }
                futures_timer::Delay::new(POLL).await;
            }
        }
    });
    let (Some(list), Some(first)) = (list, pending.read().first().cloned()) else {
        return rsx! {};
    };
    let waiting = pending.read().len();
    let t = theme();
    let answer = move |approve: bool, ticket: &str| {
        // False when it expired meanwhile: the sheet moves on either way.
        let _ = if approve {
            list.approve(ticket)
        } else {
            list.deny(ticket)
        };
        pending.set(list.pending_confirmations());
    };
    let (allow, deny, esc) = {
        let (mut a, mut d, mut e) = (answer.clone(), answer.clone(), answer);
        let ticket = first.ticket.clone();
        let (t1, t2, t3) = (ticket.clone(), ticket.clone(), ticket);
        (
            move |_| a(true, &t1),
            move |_| d(false, &t2),
            move |ev: KeyboardEvent| {
                if ev.key() == Key::Named(NamedKey::Escape) {
                    e(false, &t3);
                }
            },
        )
    };
    // A command run through `COMMANDS_RUN` is named as itself.
    let (name, id) = named(&first);
    let destructive = first.effect == Effect::Destructive || ran_destructive(&first);
    let go = if destructive { &*t.danger } else { &*t.accent };
    let warn = if destructive {
        "It may not be undoable: allow it only if you meant it to happen."
    } else {
        "Allow it only if you meant it to happen."
    };
    let args = pretty(&first.args);
    let left = countdown(first.expires_in);
    let caller = first.caller.clone();
    rsx! {
        div {
            "data-confirm": "{first.ticket}",
            style: "position: absolute; top: 0; left: 0; right: 0; bottom: 0; z-index: {LAYER}; background: {t.scrim}; \
                    display: flex; justify-content: center; align-items: center; font-family: {t.font};",
            onkeydown: esc,
            div {
                role: "alertdialog",
                "aria-label": "{caller} wants to run {name}",
                style: "width: 460px; max-width: 92%; display: flex; flex-direction: column; gap: 16px; padding: 20px; \
                        border-radius: 14px; background: {t.pill}; border: 1px solid {t.rule}; box-shadow: {t.shadow_float}; \
                        color: {t.ink};",
                div { style: "display: flex; align-items: flex-start; gap: 12px;",
                    div { style: "flex-shrink: 0; width: 36px; height: 36px; border-radius: 10px; display: flex; \
                                  align-items: center; justify-content: center; background: {t.active};",
                        ShieldAlert { size: 20, color: go }
                    }
                    div { style: "display: flex; flex-direction: column; gap: 4px; min-width: 0;",
                        div { style: "font-size: 15px; font-weight: 600; line-height: 1.35;",
                            "An agent wants to run “{name}”"
                        }
                        div { style: "font-size: 13px; color: {t.ink_2}; line-height: 1.4;",
                            "{warn}"
                        }
                    }
                }
                div { style: "display: flex; flex-direction: column; gap: 10px; font-size: 13px;",
                    Field { label: "Action",
                        span { "{name} " }
                        span { style: "color: {t.ink_3}; font-family: ui-monospace, Menlo, monospace; font-size: 12px;", "{id}" }
                    }
                    Field { label: "Caller",
                        span { style: "display: inline-flex; align-items: center; gap: 6px;",
                            Bot { size: 14, color: t.ink_2.to_string() }
                            span { style: "font-family: ui-monospace, Menlo, monospace; font-size: 12px;", "{caller}" }
                        }
                    }
                    Field { label: "Arguments",
                        div { style: "white-space: pre-wrap; word-break: break-all; padding: 8px 10px; border-radius: 8px; \
                                      background: {t.sheet}; border: 1px solid {t.rule}; max-height: 160px; overflow-y: scroll; \
                                      font-family: ui-monospace, Menlo, monospace; font-size: 12px; color: {t.ink};",
                            "{args}"
                        }
                    }
                }
                div { style: "display: flex; align-items: center; gap: 8px;",
                    div { style: "flex: 1; font-size: 12px; color: {t.ink_3};",
                        if waiting > 1 {
                            "1 of {waiting} waiting · expires in {left}"
                        } else {
                            "Expires in {left}"
                        }
                    }
                    button {
                        "data-confirm-answer": "deny",
                        style: "height: 32px; padding: 0 14px; border-radius: 8px; border: 1px solid {t.rule}; \
                                background: transparent; color: {t.ink}; font-family: {t.font}; font-size: 13px; font-weight: 500;",
                        onclick: deny,
                        // Deny has the focus: Return never approves by accident.
                        onmounted: move |e| {
                            spawn(async move {
                                let _ = e.data().set_focus(true).await;
                            });
                        },
                        "Deny"
                    }
                    button {
                        "data-confirm-answer": "allow",
                        style: "height: 32px; padding: 0 14px; border-radius: 8px; border: none; \
                                background: {go}; color: {t.accent_ink}; font-family: {t.font}; font-size: 13px; font-weight: 600;",
                        onclick: allow,
                        "Allow once"
                    }
                }
            }
        }
    }
}

/// One labelled row of the sheet.
#[component]
fn Field(label: &'static str, children: Element) -> Element {
    let t = theme();
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 4px;",
            div { style: "font-size: 11px; font-weight: 600; letter-spacing: 0.04em; text-transform: uppercase; color: {t.ink_3};",
                "{label}"
            }
            div { style: "color: {t.ink};", {children} }
        }
    }
}

/// A call's arguments, indented to read; "none" for an action that takes
/// nothing; as they came when they are not JSON.
fn pretty(args: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(args) {
        Ok(serde_json::Value::Null) => "none".to_owned(),
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| args.to_owned()),
        Err(_) => args.to_owned(),
    }
}

/// `1:52`, what a ticket has left.
fn countdown(left: Duration) -> String {
    let secs = left.as_secs();
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// The command a `COMMANDS_RUN` call names (its label, and its id after
/// the runner's); any other call's action as it is.
fn named(c: &Confirmation) -> (String, String) {
    let inner = (c.id == crate::RUN_ACTION)
        .then(|| facet_json::from_str::<crate::RunCommand>(&c.args).ok())
        .flatten();
    let Some(ask) = inner else {
        return (c.display_name.clone(), c.id.clone());
    };
    let label = crate::ui::try_use_commands()
        .and_then(|cmds| cmds.all().into_iter().find(|cmd| cmd.id == ask.id))
        .map_or_else(|| ask.id.clone(), |cmd| cmd.label);
    (label, format!("{} → {}", c.id, ask.id))
}

/// Whether a `COMMANDS_RUN` call runs a destructive command (the runner
/// itself is only mutating).
fn ran_destructive(c: &Confirmation) -> bool {
    c.id == crate::RUN_ACTION
        && facet_json::from_str::<crate::RunCommand>(&c.args)
            .ok()
            .and_then(|ask| {
                crate::ui::try_use_commands()?
                    .all()
                    .into_iter()
                    .find(|cmd| cmd.id == ask.id)
            })
            .is_some_and(|cmd| cmd.effect == crate::Effect::Destructive)
}

#[cfg(test)]
mod tests {
    use super::{countdown, pretty};
    use std::time::Duration;

    #[test]
    fn the_sheet_reads_arguments_and_time_left() {
        assert_eq!(pretty("null"), "none");
        assert_eq!(pretty(r#"{"path":"a"}"#), "{\n  \"path\": \"a\"\n}");
        assert_eq!(pretty("not json"), "not json");
        assert_eq!(countdown(Duration::from_millis(112_400)), "1:52");
        assert_eq!(countdown(Duration::from_secs(5)), "0:05");
    }
}
