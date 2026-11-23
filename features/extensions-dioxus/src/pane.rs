//! The pane a view is drawn in, the same linked in or as a process.
//!
//! Linked in, the shell says (`architect_views::PaneHostRef`, in context
//! around each pane); in a process, the host's pane input
//! (`architect_extensions::linked::PANE_CLAIM`) says.

use architect_extensions::linked::{EmbedContext, PANE_CLAIM};
use architect_views::PaneHostRef;
use dioxus::prelude::{ReadableExt as _, try_consume_context, try_use_context};

/// Where a view is in what it shows (zoom, scroll, selection, a mode),
/// kept by its pane: [`use_view_state`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewState {
    /// The pane it is kept for.
    pub pane: String,
    /// What was kept: "" when nothing was.
    pub value: String,
}

impl ViewState {
    /// Keeps `value` for this view's pane: given back when the view
    /// starts again — its program restarted (a development copy rebuilt),
    /// its pane moved or popped out, the app opened again. Write it when a
    /// gesture ends, not every frame: each write is a workspace change.
    /// Call it from the view (a handler, an effect).
    pub fn set(&self, value: impl Into<String>) {
        let value = value.into();
        if value == self.value {
            return;
        }
        #[cfg(feature = "guest")]
        if architect_extensions::linked::in_process() {
            let request = architect_extensions::hostcall::HostRequest::SetViewState {
                pane: self.pane.clone(),
                state: value,
            };
            // On the view's own runtime: call it from the view (a
            // handler, an effect).
            dioxus::prelude::spawn(async move {
                // Not kept (a host before this call): the view starts
                // where it is told, as before.
                let _ = architect_extensions::guest::host(request).await;
            });
            return;
        }
        if let Some(pane) = try_consume_context::<PaneHostRef>()
            && pane.0.id().0 == self.pane
        {
            pane.0.set_view_state(value);
        }
    }
}

/// A value of the host's pane input, when this view is in a process pane.
fn from_input(key_a: &str, key_b: &str) -> Option<(String, String)> {
    let input = try_use_context::<EmbedContext>()?.0;
    let now = input.read();
    if now.claim != PANE_CLAIM {
        return None;
    }
    let text = |key: &str| {
        now.props
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Some((text(key_a), text(key_b)))
}

/// Where this view is in what it shows, as its pane kept it.
///
/// The same linked in or as a process: read in a view, it draws again
/// when it changes; [`ViewState::set`] keeps a new one. On start (a process
/// restarted, the pane moved or popped out) it is what was kept last, so
/// the view picks up where it was. `None` outside a pane, and in a
/// process until its pane's input arrives (just after the first render):
/// apply it when it first is `Some`.
#[must_use]
pub fn use_view_state() -> Option<ViewState> {
    if let Some(pane) = try_use_context::<PaneHostRef>() {
        return Some(ViewState {
            pane: pane.0.id().0,
            value: pane.0.view_state(),
        });
    }
    from_input("pane", "view_state").map(|(pane, value)| ViewState { pane, value })
}

/// The pane this view is drawn in, as its id and its state (an opener's
/// path), linked in or as a process: read in a view, it draws again when
/// either changes. `None` outside a pane (a slot, an embed).
#[must_use]
pub fn use_pane_state() -> Option<(String, String)> {
    if let Some(pane) = try_use_context::<PaneHostRef>() {
        return Some((pane.0.id().0, pane.0.state()));
    }
    from_input("pane", "state")
}
