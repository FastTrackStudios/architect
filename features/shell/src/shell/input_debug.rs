//! The input debug overlay: what the shell hears of touches and the
//! keyboard, drawn over the app, for checking on a phone what a simulator
//! does not show (the keyboard's real height, where a press lands).
//!
//! Off unless asked: `ARCHITECT_INPUT_DEBUG=1` at launch, or
//! [`set_input_debug`] (the Architect app's "Debug: touches and keyboard"
//! command). While on it draws, over everything and taking no press:
//!
//! - a cyan line where the keyboard's top is as `UIKit` reported it, and a
//!   magenta one where the shell lays the foot's bottom edge
//!   ([`foot_bottom`]); on a phone the two should meet while the keyboard
//!   is up;
//! - the keyboard as heard (height, typing), the screen and its insets,
//!   which foot shows, and on iOS whether winit's view has the keyboard;
//! - the last presses and what in the foot heard them (the editing bar,
//!   the Hide button, the action bar, the tabs), and the keyboard's
//!   comings and goings, newest first.
//!
//! On iOS each line goes to the log too (`shell.input`).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use dioxus::prelude::*;

use super::keyboard::{Foot, keyboard, soft_keyboard, use_keyboard_lift};
use crate::screen::Screen;

/// 0: not read yet, 1: off, 2: on.
static ON: AtomicU8 = AtomicU8::new(0);

/// How many lines it keeps.
const KEPT: usize = 12;

struct Ring {
    start: Option<Instant>,
    lines: VecDeque<String>,
}

static RING: Mutex<Ring> = Mutex::new(Ring {
    start: None,
    lines: VecDeque::new(),
});

/// Whether the overlay is on: [`set_input_debug`], else
/// `ARCHITECT_INPUT_DEBUG=1` at launch.
#[must_use]
pub fn input_debug() -> bool {
    match ON.load(Ordering::Relaxed) {
        0 => {
            let on = std::env::var("ARCHITECT_INPUT_DEBUG").is_ok_and(|v| v == "1");
            ON.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
        n => n == 2,
    }
}

/// Turns the overlay on or off.
pub fn set_input_debug(on: bool) {
    ON.store(if on { 2 } else { 1 }, Ordering::Relaxed);
    note(if on { "debug: on" } else { "debug: off" });
}

/// Keeps `line` (with the seconds since the first) while the overlay is
/// on, and logs it.
pub fn note(line: impl Into<String>) {
    if !input_debug() {
        return;
    }
    let line = line.into();
    #[cfg(target_os = "ios")]
    tracing::info!(target: "shell.input", "{line}");
    let mut ring = RING.lock().unwrap_or_else(PoisonError::into_inner);
    let start = *ring.start.get_or_insert_with(Instant::now);
    let at = start.elapsed().as_secs_f64();
    ring.lines.push_front(format!("{at:7.2} {line}"));
    ring.lines.truncate(KEPT);
}

/// A press heard by `what` (a part of the foot) at `(x, y)`.
pub fn hit(what: &str, e: &PointerEvent) {
    if input_debug() {
        let c = e.client_coordinates();
        note(format!("press on {what} at {:.0},{:.0}", c.x, c.y));
    }
}

/// The lines kept, newest first.
fn lines() -> Vec<String> {
    RING.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .lines
        .iter()
        .cloned()
        .collect()
}

/// Where the shell lays a phone's foot's bottom edge, from the screen's
/// top, points: over the keyboard while it is up (`lift`, eased after its
/// height), over the home indicator otherwise (`inset_bottom`).
#[must_use]
pub fn foot_bottom(screen_height: f64, lift: f64, inset_bottom: f64) -> f64 {
    screen_height - lift.max(inset_bottom)
}

/// The overlay, while it is on.
#[component]
pub(super) fn InputDebugOverlay(screen: Screen) -> Element {
    let kb = use_keyboard_lift();
    let mut tick = use_signal(|| 0_u32);
    use_hook(move || {
        spawn(async move {
            loop {
                futures_timer::Delay::new(std::time::Duration::from_millis(250)).await;
                tick.with_mut(|t| *t = t.wrapping_add(1));
            }
        });
    });
    let _ = tick();
    let now = (kb.now)();
    let heard = keyboard();
    let lift = kb.lift();
    let soft = soft_keyboard();
    let foot = match now.foot(soft) {
        Foot::Tabs => "tabs",
        Foot::TabsWithKeyboardButton => "tabs + keyboard button",
        Foot::Editing => "editing bar",
    };
    let ins = screen.insets;
    let key_top = screen.height - heard.height;
    let bottom = foot_bottom(screen.height, lift, ins.bottom);
    #[cfg(target_os = "ios")]
    let responder = match super::keyboard::has_keyboard() {
        Some(true) => "yes",
        Some(false) => "no",
        None => "?",
    };
    #[cfg(not(target_os = "ios"))]
    let responder = "n/a";
    let head = format!(
        "kb {:.0} typing {} lift {:.0} soft {soft} · foot: {foot} · first responder: {responder}",
        heard.height, heard.typing, lift
    );
    let geo = format!(
        "screen {:.0}x{:.0} insets t{:.0} b{:.0} l{:.0} r{:.0} · kb top {key_top:.0} · foot bottom {bottom:.0}",
        screen.width, screen.height, ins.top, ins.bottom, ins.left, ins.right
    );
    let lines = lines();
    rsx! {
        div {
            "data-input-debug": "true",
            style: "position: absolute; top: 0; left: 0; width: 100%; height: 100%; pointer-events: none; z-index: 2147483000;",
            div { style: "position: absolute; left: 0; right: 0; top: {key_top - 1.0}px; height: 2px; background: #22d3ee;" }
            div { style: "position: absolute; left: 0; right: 0; top: {bottom - 1.0}px; height: 2px; background: #e879f9;" }
            div {
                style: "position: absolute; left: 6px; right: 6px; top: {ins.top + 4.0}px; padding: 6px 8px; border-radius: 8px; \
                        background: rgba(0,0,0,0.78); color: #f4f4f5; font-family: ui-monospace, Menlo, monospace; \
                        font-size: 10px; line-height: 13px; white-space: pre-wrap;",
                div { style: "color: #22d3ee;", "{head}" }
                div { style: "color: #e879f9;", "{geo}" }
                for (n, l) in lines.into_iter().enumerate() {
                    div { key: "{n}", "{l}" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::foot_bottom;

    #[test]
    fn the_foot_sits_on_the_keyboard_or_the_home_indicator() {
        // iPhone 17 Pro, 874pt tall, 34pt home indicator.
        assert!((foot_bottom(874.0, 0.0, 34.0) - 840.0).abs() < 1e-9);
        // The keyboard up (336pt, the home indicator under it).
        assert!((foot_bottom(874.0, 336.0, 34.0) - 538.0).abs() < 1e-9);
        // On its way: never lower than the home indicator allows.
        assert!((foot_bottom(874.0, 12.0, 34.0) - 840.0).abs() < 1e-9);
        // No safe area (a desk standing in): the screen's edge.
        assert!((foot_bottom(874.0, 0.0, 0.0) - 874.0).abs() < 1e-9);
    }
}
