//! Cursors in the Architect app (#509): every pane of the workspace
//! publishes this person's pointer and draws the others', and extensions
//! reach the same through the `CURSORS_*` actions
//! (`architect_workspace::pane_space`).
//!
//! The room is over a channel of this window's own until the app joins a
//! vault's presence channel (vault sync in the app, #383): the panes, the
//! tabs and the actions are all wired, and nobody else is on it yet. When
//! the app syncs a vault, its `crdt::hooks::Presence` replaces
//! [`Alone`] here and nothing else changes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use architect::action::ActionList;
use architect_workspace::pane_space::{self, PresenceBoard};
use architect_workspace::{PaneRoom, use_pane_room};
use crdt::presence::{Here, PeerState, PresenceSink, Roster};
use dioxus::prelude::*;
use loro::LoroValue;

/// A presence channel with nobody else on it.
#[derive(Clone, Default)]
struct Alone(Arc<Mutex<HashMap<String, LoroValue>>>);

impl PresenceSink for Alone {
    fn set(&self, key: &str, value: LoroValue) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }

    fn delete(&self, key: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
    }
}

/// Milliseconds since the app's first frame.
fn clock_ms() -> f64 {
    static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    BASE.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// The app's cursors: the `CURSORS_*` actions on `list`, and the room
/// every pane under the caller is in.
pub fn use_app_cursors(list: &Arc<ActionList>) -> PaneRoom {
    let roster = use_signal(Roster::default);
    let board = use_hook(|| {
        let board = PresenceBoard::default();
        // Already there when the page is mounted a second time.
        let _ = pane_space::register(list, &board);
        board
    });
    use_pane_room(|| {
        // One per window: two windows of one person are two pointers.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let me = format!("local-{}-{nanos}", std::process::id());
        let state = PeerState::new(&me, "Me");
        PaneRoom::new(
            Here::new(Alone::default(), me, state),
            roster,
            board,
            clock_ms,
        )
    })
}
