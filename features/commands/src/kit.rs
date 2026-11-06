//! The palette kit (feature `ui`): the pieces a palette extension draws
//! with — the overlay's [`Frame`], a [`CommandRow`], [`Lit`] text, a
//! [`Kbd`] chip, the [`AskPalette`] that asks an action's arguments a
//! step at a time, and a row's [`Pick`]. The Command Palette
//! (`features/palette/extension`) is drawn with them; a replacement may
//! use them or draw its own.

use architect_shell::{ShellTheme, try_use_shell};
use dioxus::prelude::*;
use lucide_dioxus::Search;

use crate::ui::try_use_commands;
use architect_palette::Ranked;

pub use crate::keys_ui::{AskPalette, Pick};

/// A row's fill: lit when picked.
pub fn fill(on: bool, t: &ShellTheme) -> &str {
    if on { &t.active } else { "transparent" }
}

/// A row's quiet ink: a step up on the picked row, where `ink_3` falls
/// under 4.5:1 on `fill`.
pub fn quiet(on: bool, t: &ShellTheme) -> &str {
    if on { &t.ink_2 } else { &t.ink_3 }
}

pub fn theme() -> ShellTheme {
    try_use_shell().map(|s| s.theme).unwrap_or_default()
}

/// The overlay's frame: a scrim, and a panel near the top with a field.
#[component]
pub fn Frame(
    placeholder: String,
    query: Signal<String>,
    on_key: EventHandler<KeyboardEvent>,
    children: Element,
) -> Element {
    let commands = try_use_commands();
    let t = theme();
    let mut query = query;
    rsx! {
        div {
            style: "position: absolute; top: 0; left: 0; right: 0; bottom: 0; z-index: 90; background: rgba(0,0,0,0.45); \
                    display: flex; justify-content: center; align-items: flex-start; padding-top: 12vh;",
            onclick: move |_| {
                if let Some(c) = commands {
                    c.close();
                }
            },
            div {
                style: "width: 620px; max-width: 92%; max-height: 70vh; display: flex; flex-direction: column; \
                        border-radius: 12px; background: {t.pill}; border: 1px solid {t.rule}; \
                        box-shadow: 0 24px 64px rgba(0,0,0,0.6); overflow: hidden; font-family: {t.font};",
                onclick: move |e| e.stop_propagation(),
                div { style: "flex-shrink: 0; height: 48px; display: flex; align-items: center; gap: 10px; padding: 0 14px; \
                              border-bottom: 1px solid {t.rule};",
                    Search { size: 16, color: t.ink_3.to_string() }
                    input {
                        r#type: "text",
                        value: "{query}",
                        style: "flex: 1; min-width: 0; height: 32px; border: none; outline: none; background: transparent; \
                                color: {t.ink}; font-family: {t.font}; font-size: 15px;",
                        "aria-label": placeholder,
                        oninput: move |e| query.set(e.value()),
                        onkeydown: move |e| on_key.call(e),
                        onmounted: move |e| {
                            spawn(async move {
                                let _ = e.data().set_focus(true).await;
                            });
                        },
                    }
                    if query.read().is_empty() {
                        span { style: "position: absolute; margin-left: 26px; color: {t.ink_3}; font-size: 15px; pointer-events: none;",
                            "{placeholder}"
                        }
                    }
                    Kbd { text: "esc" }
                }
                div { style: "flex: 1; min-height: 0; overflow-y: scroll; padding: 6px;",
                    {children}
                }
            }
        }
    }
}

/// A key, as a chip; `on` on a picked row (the quiet ink a step up, for
/// 4.5:1 on its fill).
#[component]
pub fn Kbd(text: String, #[props(default)] on: bool) -> Element {
    let t = theme();
    let ink = if on { &*t.ink_2 } else { &*t.ink_3 };
    rsx! {
        span { style: "flex-shrink: 0; padding: 1px 6px; border-radius: 4px; border: 1px solid {t.rule}; \
                       color: {ink}; font-size: 11px; font-family: {t.font};",
            "{text}"
        }
    }
}

/// A space's width, in ems.
const WORD_SPACE: f32 = 0.27;

/// `n` as a count of word spaces.
fn count(n: usize) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
}

/// `text` with the characters at `positions` lit.
#[component]
pub fn Lit(text: String, positions: Vec<usize>, ink: String, lit: String) -> Element {
    // Runs of lit and unlit characters, so a row is a few spans, not one
    // per character.
    let mut runs: Vec<(bool, String)> = Vec::new();
    for (i, c) in text.chars().enumerate() {
        let on = positions.contains(&i);
        match runs.last_mut() {
            Some((was, s)) if *was == on => s.push(c),
            _ => runs.push((on, c.to_string())),
        }
    }
    // Blitz drops a space where two spans meet: a run's edge spaces become
    // margin, a word space each.
    let runs: Vec<(bool, String, f32, f32)> = runs
        .into_iter()
        .map(|(on, s)| {
            let lead = s.chars().take_while(|c| *c == ' ').count();
            let trail = s.chars().rev().take_while(|c| *c == ' ').count();
            let word = WORD_SPACE;
            (
                on,
                s.trim().to_owned(),
                word * count(lead),
                word * count(trail),
            )
        })
        .collect();
    rsx! {
        span { style: "white-space: nowrap; overflow: hidden;",
            for (i, (on, s, before, after)) in runs.into_iter().enumerate() {
                span {
                    key: "{i}",
                    style: if on {
                        format!("display: inline-block; color: {lit}; font-weight: 650; margin-left: {before}em; margin-right: {after}em;")
                    } else {
                        format!("display: inline-block; color: {ink}; margin-left: {before}em; margin-right: {after}em;")
                    },
                    "{s}"
                }
            }
        }
    }
}

#[component]
pub fn CommandRow(row: Ranked, on: bool, onpress: EventHandler<()>) -> Element {
    let t = theme();
    let c = row.item;
    let fill = if on { &*t.active } else { "transparent" };
    let quiet = quiet(on, &t);
    let ink = if c.enabled { &*t.ink } else { quiet };
    rsx! {
        div {
            class: "lp",
            "data-command": "{c.id}",
            style: "height: 36px; display: flex; align-items: center; gap: 10px; padding: 0 10px; border-radius: 8px; \
                    background: {fill}; font-size: 13px; cursor: pointer;",
            onclick: move |_| onpress.call(()),
            span { style: "flex: 1; min-width: 0; display: flex; align-items: baseline; gap: 8px; overflow: hidden;",
                if !c.group.is_empty() {
                    span { style: "flex-shrink: 0; color: {quiet};", "{c.group}:" }
                }
                Lit { text: c.label.clone(), positions: row.positions, ink: ink.to_owned(), lit: t.ink.to_string() }
                if let Some(why) = c.disabled_reason.clone() {
                    span { style: "color: {quiet}; font-size: 12px; white-space: nowrap;", "— {why}" }
                }
            }
            if !c.shortcut.is_empty() {
                Kbd { text: c.shortcut, on }
            }
        }
    }
}
