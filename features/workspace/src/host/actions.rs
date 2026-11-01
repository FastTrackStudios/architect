//! The workspace on the app's Action List: its [`Arrangement`] read back by a
//! guide scenario's checks, a tour, or a model, and each change heard.

use std::sync::{Arc, Mutex, PoisonError};

use architect::action::{ActionList, ActionMeta, Effect};
use dioxus::prelude::*;

use crate::guide::{Arrangement, LAYOUT_ACTION};
use crate::model::Workspace;

static LAYOUT_META: ActionMeta = ActionMeta {
    id: LAYOUT_ACTION,
    trait_name: "",
    method_name: "",
    display_name: "Workspace layout",
    description: "Every window's tab groups (their tabs in order, the one showing, stacked or not), its splits and its docks' groups, and the active pane.",
    category: "Workspace",
    group: "Panes",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// Puts `workspace` on `list` while the calling component is mounted:
/// [`LAYOUT_ACTION`] (read-only) answers its [`Arrangement`], and each change
/// to it is a `Changed { topic: "workspace" }` event.
pub fn use_workspace_actions(list: Arc<ActionList>, workspace: Signal<Workspace>) {
    let kept = use_hook(|| Arc::new(Mutex::new(Arrangement::of(&workspace.peek()))));
    let registered = use_hook({
        let list = list.clone();
        let kept = kept.clone();
        move || {
            list.register(&LAYOUT_META, Effect::ReadOnly, move |(): ()| {
                Ok(kept.lock().unwrap_or_else(PoisonError::into_inner).clone())
            })
            .is_ok()
        }
    });
    let told = list.clone();
    use_effect(move || {
        let now = Arrangement::of(&workspace.read());
        let mut layout = kept.lock().unwrap_or_else(PoisonError::into_inner);
        if *layout != now {
            let active = now.active.clone();
            *layout = now;
            drop(layout);
            told.changed("workspace", &active);
        }
    });
    use_drop(move || {
        if registered {
            list.unregister(LAYOUT_ACTION);
        }
    });
}
