//! Screensets in the host: ⌘1–9 recall the set in that slot for this
//! screen's class, the live layout is kept per class as it changes, and
//! all of it is read from and written to the app's [`Store`] — read once
//! when the host mounts, written when the workspace settles (a drag's
//! frames are not each a write).

use std::rc::Rc;

use architect_commands::{Command, use_command};
use dioxus::prelude::*;

use super::{Host, use_host};
use crate::control::{self, LoadArgs, SaveArgs};
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
/// the screensets in the store whenever the workspace settles. With a
/// `device` store too, the saved sets go to `store` and the working copy
/// to `device` ([`store::load_split`]).
pub(super) fn use_kept(host: Host, store: Option<WorkspaceStore>, device: Option<WorkspaceStore>) {
    let mut sets = host.screensets;
    let mut workspace = host.workspace;
    let kept = store.clone();
    let here = device.clone();
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
        let mut loaded = match &here {
            Some(device) => store::load_split(kept.0.as_ref(), device.0.as_ref(), builtins),
            None => store::load(kept.0.as_ref(), builtins),
        };
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
        match (&store, &device) {
            (Some(store), Some(device)) => {
                let _ = store::save_split(store.0.as_ref(), device.0.as_ref(), &sets.peek());
            }
            (Some(store), None) => {
                let _ = store::save(store.0.as_ref(), &sets.peek());
            }
            _ => {}
        }
    });
}

/// The saved layouts in the palette: ⌘1–9 (when the option is on), Load
/// for each set, Save, Reset and Manage, named with the app's noun
/// ("Workspace"). Each is its `WORKSPACES_*` action.
#[component]
pub(super) fn ScreensetCommands() -> Element {
    let host = use_host();
    let noun = host.sets_noun.read().clone();
    let lower = noun.to_lowercase();
    let sets = host.screensets.read();
    let active = sets
        .active
        .as_deref()
        .and_then(|id| sets.get(id))
        .map(|s| s.name.clone());
    let slots: Vec<(u8, Option<String>)> = (1..=9)
        .map(|n| (n, sets.in_slot(n).map(|s| s.name.clone())))
        .collect();
    let keys = sets.options.slot_keys;
    let each: Vec<(String, String)> = sets
        .sets
        .iter()
        .map(|s| (s.id.clone(), s.name.clone()))
        .collect();
    let dirty = sets.is_dirty(*host.form.read());
    drop(sets);
    for (n, name) in slots {
        let id = format!("SCREENSET_RECALL_{n}");
        let command = name.map_or_else(
            || {
                Command::new(id.clone(), format!("Load {lower} {n}"))
                    .disabled(format!("no {lower} is in slot {n}"))
            },
            |name| Command::new(id.clone(), format!("Load {name}")),
        );
        let command = command.group(noun.clone()).shortcut(if keys {
            format!("⌘{n}")
        } else {
            String::new()
        });
        use_command(command, move || {
            host.write(
                control::load,
                LoadArgs {
                    if_revision: None,
                    id: None,
                    name: None,
                    slot: Some(n),
                },
            );
        });
    }
    let label = active.as_ref().map_or_else(
        || format!("Save the {lower} layout"),
        |name| format!("Save {name}"),
    );
    let save = Command::new("SCREENSET_SAVE", label).group(noun.clone());
    use_command(
        if active.is_none() {
            save.disabled(format!("no {lower} is active"))
        } else if dirty {
            save
        } else {
            save.disabled("nothing has changed")
        },
        move || {
            host.write(
                control::save,
                SaveArgs {
                    if_revision: None,
                    name: None,
                },
            );
        },
    );
    let reset = Command::new("SCREENSET_RESET", "Reset to the saved layout").group(noun.clone());
    use_command(
        if active.is_some() && dirty {
            reset
        } else {
            reset.disabled("the layout is the saved one")
        },
        move || host.write(control::reset, control::RevisionArgs::default()),
    );
    use_command(
        Command::new(
            control::ids::WORKSPACES_MANAGE,
            format!("Manage {lower} layouts"),
        )
        .group(noun.clone()),
        move || {
            let mut managing = host.managing;
            managing.set(true);
        },
    );
    rsx! {
        for (id, name) in each {
            LoadCommand { key: "{id}", id, name, noun: noun.clone() }
        }
    }
}

/// "Load workspace: Mix": one palette row per saved layout.
#[component]
fn LoadCommand(id: String, name: String, noun: String) -> Element {
    let host = use_host();
    let command_id = format!(
        "WORKSPACES_LOAD_{}",
        id.to_uppercase()
            .replace(|c: char| !c.is_ascii_alphanumeric(), "_")
    );
    let lower = noun.to_lowercase();
    use_command(
        Command::new(command_id, format!("Load {lower}: {name}")).group(noun),
        move || {
            host.write(
                control::load,
                LoadArgs {
                    if_revision: None,
                    id: Some(id.clone()),
                    name: None,
                    slot: None,
                },
            );
        },
    );
    rsx! {}
}
