//! Screensets in the host: ⌘1–9 recall the set in that slot for this
//! screen's class, the live layout is kept per class as it changes, and
//! all of it is read from and written to the app's [`Store`] — read once
//! when the host mounts, written when the workspace settles (a drag's
//! frames are not each a write).

use std::rc::Rc;

use architect_commands::{Command, use_command};
use dioxus::prelude::*;

use super::{Host, use_host};
use crate::control::{self, RecallArgs};
use crate::store::{self, Store};

/// The app's [`Store`], as a prop (compared by identity).
#[derive(Clone)]
pub struct WorkspaceStore(pub Rc<dyn Store>);

impl WorkspaceStore {
    pub fn new(store: impl Store + 'static) -> Self {
        Self(Rc::new(store))
    }
}

impl PartialEq for WorkspaceStore {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// Reads what `store` kept when the host mounts (the live layout for this
/// class comes back), then keeps the live layout in the screensets and
/// the screensets in the store whenever the workspace settles.
pub(super) fn use_kept(host: Host, store: Option<WorkspaceStore>) {
    let mut sets = host.screensets;
    let mut workspace = host.workspace;
    let kept = store.clone();
    use_hook(move || {
        let Some(kept) = kept else {
            return;
        };
        let builtins: Vec<_> = sets
            .peek()
            .sets
            .iter()
            .filter(|s| s.builtin)
            .cloned()
            .collect();
        let mut loaded = store::load(kept.0.as_ref(), builtins);
        // A set the app made active (or recalled before mounting) stays so.
        if loaded.active.is_none() {
            loaded.active.clone_from(&sets.peek().active);
        }
        let class = *host.form.peek();
        if let Some(live) = loaded.working(class).cloned() {
            workspace.set(live);
        }
        sets.set(loaded);
    });
    use_effect(move || {
        let live = workspace.read().clone();
        // Mid-drag the workspace changes every frame: wait for the end.
        if host.drag.read().is_some() {
            return;
        }
        let class = *host.form.read();
        let same = sets.peek().working(class) == Some(&live);
        if !same {
            sets.write().set_working(class, live);
        }
        if let Some(store) = &store {
            let _ = store::save(store.0.as_ref(), &sets.peek());
        }
    });
}

/// ⌘1–9, Save and Reset, in the palette, while the host has screensets.
#[component]
pub(super) fn ScreensetCommands() -> Element {
    let host = use_host();
    let sets = host.screensets.read();
    let active = sets
        .active
        .as_deref()
        .and_then(|id| sets.get(id))
        .map(|s| s.name.clone());
    let slots: Vec<(u8, Option<String>)> = (1..=9)
        .map(|n| (n, sets.in_slot(n).map(|s| s.name.clone())))
        .collect();
    let dirty = sets.is_dirty(*host.form.read());
    drop(sets);
    for (n, name) in slots {
        let id = format!("SCREENSET_RECALL_{n}");
        let command = name.map_or_else(
            || {
                Command::new(id.clone(), format!("Recall screenset {n}"))
                    .disabled(format!("no screenset is in slot {n}"))
            },
            |name| Command::new(id.clone(), format!("Recall {name}")),
        );
        use_command(
            command.group("Screensets").shortcut(format!("⌘{n}")),
            move || {
                host.write(
                    control::recall,
                    RecallArgs {
                        if_revision: None,
                        id: None,
                        slot: Some(n),
                    },
                );
            },
        );
    }
    let label = active.as_ref().map_or_else(
        || "Save the screenset".to_owned(),
        |name| format!("Save {name}"),
    );
    let save = Command::new("SCREENSET_SAVE", label).group("Screensets");
    use_command(
        if active.is_none() {
            save.disabled("no screenset is active")
        } else if dirty {
            save
        } else {
            save.disabled("nothing has changed")
        },
        move || host.write(control::save, control::RevisionArgs::default()),
    );
    let reset = Command::new("SCREENSET_RESET", "Reset to the saved layout").group("Screensets");
    use_command(
        if active.is_some() && dirty {
            reset
        } else {
            reset.disabled("the layout is the saved one")
        },
        move || host.write(control::reset, control::RevisionArgs::default()),
    );
    rsx! {}
}
