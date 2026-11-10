//! Capture in the Architect app (#352): `CAPTURE_*` and `INBOX_*` on the
//! app's list over the open vault (`architect_capture`), the composer on
//! ⇧⌘N (`LAB_CAPTURE`), the sidebar's feather and File › Capture, the
//! inbox deck on File › Process inbox (`INBOX_OPEN`), and the capture
//! window — a small window over every app — on a system-wide shortcut,
//! the menu-bar item's Capture window and `CAPTURE_OPEN {window: true}`.
//!
//! The shortcut is ⌃⌥N unless `ARCHITECT_CAPTURE_SHORTCUT` names another
//! (`"shift+super+Space"`: the modifiers and a key code). The sandbox
//! keeps its own made-up Inbox and has none of this.

use std::sync::Arc;

use architect::action::ActionList;
use architect_capture::actions::Config;
use architect_capture::ui::{CaptureUi, use_capture};
use dioxus::prelude::*;

/// The command that opens the capture window (the menu-bar item's).
pub const WINDOW_COMMAND: &str = "LAB_CAPTURE_WINDOW";

/// The capture window's system-wide shortcut.
#[must_use]
pub fn shortcut() -> String {
    std::env::var("ARCHITECT_CAPTURE_SHORTCUT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "control+alt+KeyN".to_owned())
}

/// The shortcut as a menu shows it (`⌃⌥N`).
fn shown(spec: &str) -> String {
    spec.split('+')
        .map(|part| match part.trim().to_ascii_lowercase().as_str() {
            "control" | "ctrl" => "⌃".to_owned(),
            "alt" | "option" => "⌥".to_owned(),
            "shift" => "⇧".to_owned(),
            "super" | "cmd" | "command" | "meta" => "⌘".to_owned(),
            _ => part
                .trim()
                .trim_start_matches("Key")
                .trim_start_matches("Digit")
                .to_owned(),
        })
        .collect()
}

/// Capture in the app's window: `None` in the sandbox or a run that keeps
/// nothing (`files` false). `windowed`: a desk's window, where the
/// system-wide shortcut is registered.
pub fn use_app_capture(list: &Arc<ActionList>, files: bool, windowed: bool) -> Option<CaptureUi> {
    // Fixed for the process: the hooks below run on every render or never.
    if crate::distribution().is_sandbox() || !files {
        return None;
    }
    let capture = use_capture(list, Config::default());
    let spec = use_hook(shortcut);
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    if windowed {
        let failed = architect_capture::shortcut::use_global_shortcut(
            &spec,
            Callback::new(move |()| capture.window("")),
        );
        use_hook(move || {
            if let Some(why) = failed {
                tracing::warn!("{why}");
            }
        });
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let _ = windowed;
    architect_commands::use_command(
        architect_commands::Command::new(WINDOW_COMMAND, "Capture window")
            .group("Inbox")
            .shortcut(shown(&spec)),
        move || capture.window(""),
    );
    Some(capture)
}

/// What the navigator's Inbox rows ask for: triage one capture, by its
/// vault path (#443). The Queue's capture rows open it
/// (`architect_tasks::queue::PROCESS_CARD`, `{"path": …}`).
pub const PROCESS_CARD: &str = architect_tasks::queue::PROCESS_CARD;

/// Triages capture `path`: `ProcessCard`'s shape (one capture, its targets,
/// Later, Discard; #445). Until #445's card lands, the inbox deck
/// (`INBOX_OPEN`), which starts at the oldest capture.
pub fn process_card(list: Arc<ActionList>, path: &str) {
    tracing::info!(path, "app.inbox.process_card");
    spawn(async move {
        if let Err(e) = list
            .run_json_async(architect_capture::actions::ids::INBOX_OPEN, b"null")
            .await
        {
            tracing::warn!(error = %e, "app.inbox.process_card_failed");
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_shortcut_as_a_menu_shows_it() {
        assert_eq!(super::shown("control+alt+KeyN"), "⌃⌥N");
        assert_eq!(super::shown("shift+super+Space"), "⇧⌘Space");
    }
}
