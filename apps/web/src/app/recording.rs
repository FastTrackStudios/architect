//! A note's recording on the Now Playing bar (#589).
//!
//! A note with a `recording` property (a vault path) shows a play button;
//! its recording is the app's own source on the playhead ([`OWN`]), one
//! item, heard through the app's audio element (`super::audio`). Beside
//! the sources extensions' views set, it is the second source the bar
//! switches between.

use architect_playhead::Caller;
use architect_shell::use_shell;
use dioxus::prelude::*;
use serde_json::{Value, json};

/// The app's own source: a note's recording. Outside the extension-id
/// grammar (`:`), so no extension can hold it (`architect_playhead::deck::reserved`).
pub const OWN: &str = "app:notes";

fn press(owner: &str, action: &str, mut args: Value) {
    if let Some(o) = args.as_object_mut() {
        o.insert("source".to_owned(), Value::from(owner));
    }
    let _ = architect_playhead::ask(&Caller::App, action, &args);
}
/// A note's recording (its `recording` property) on the app's own
/// source: one item, played from the note.
#[component]
pub fn RecordingRow(path: String, title: String, recording: String) -> Element {
    let t = use_shell().theme;
    let mine = architect_playhead::ui::use_playhead();
    let on = mine().active.as_ref().is_some_and(|a| {
        a.source == OWN && a.item.as_ref().is_some_and(|i| i.id == path) && a.playing
    });
    let play = move |_| {
        if on {
            press(OWN, "PLAYHEAD_PAUSE", json!({}));
            return;
        }
        let item =
            json!({ "id": path, "title": title, "subtitle": "Recording", "media": recording });
        let current = architect_playhead::with(|d| {
            d.source(OWN)
                .and_then(|s| s.item().map(|i| i.id == path))
                .unwrap_or(false)
        });
        if !current {
            let _ = architect_playhead::ask(
                &Caller::App,
                "PLAYHEAD_SET",
                &json!({ "source": OWN, "name": "Notes", "queue": [item] }),
            );
        }
        press(OWN, "PLAYHEAD_PLAY", json!({}));
    };
    rsx! {
        div { "data-testid": "note-recording",
            style: "display: flex; align-items: center; gap: 10px; margin: 0 0 16px; font-size: 13px; color: {t.ink_3};",
            button {
                "data-testid": "note-recording-play",
                style: "border: 0; border-radius: 999px; padding: 6px 14px; font: inherit; font-weight: 600; \
                        background: {t.accent}; color: {t.accent_ink}; cursor: pointer;",
                onclick: play,
                if on { "Pause recording" } else { "Play recording" }
            }
            span { "In the Now Playing bar" }
        }
    }
}
