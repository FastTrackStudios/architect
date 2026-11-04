//! The lab on its Action List, for the guide scenarios
//! (`apps/lab/scenarios`): where it is, read back by a check, and each
//! move heard as a `lab` change.

use std::sync::{Arc, Mutex, PoisonError};

use architect::action::{ActionList, ActionMeta, Effect};
use dioxus::prelude::*;
use facet::Facet;

use crate::{Lab, Route};

/// What `LAB_STATE` answers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet)]
pub struct LabState {
    /// The tab on show (`inbox`, `tasks`, `schedule`, `vault`).
    pub tab: String,
    /// `list`, `item`, `capture` or `help`.
    pub page: String,
    /// A phone's bar at the foot: `actions`, or `editing` while a field
    /// is typed in. Read when asked.
    pub bar: String,
}

impl LabState {
    fn of(route: Route) -> Self {
        let page = match route {
            Route::Tab(_) => "list",
            Route::Item(..) => "item",
            Route::Capture(_) => "capture",
            Route::Help(_) => "help",
        };
        Self {
            tab: route.tab().to_owned(),
            page: page.to_owned(),
            bar: String::new(),
        }
    }
}

static STATE: ActionMeta = ActionMeta {
    id: "LAB_STATE",
    trait_name: "",
    method_name: "",
    display_name: "Where the lab is",
    description: "The tab on show, whether it is the list, an item or a capture, and whether a phone's bar is the action bar or the editing bar.",
    category: "Lab",
    group: "Go",
    shortcut: "",
    toggleable: false,
    undo: false,
};

static GO: ActionMeta = ActionMeta {
    id: "LAB_GO",
    trait_name: "",
    method_name: "",
    display_name: "Go to a tab",
    description: "Shows tab `tab` (inbox, tasks, schedule or vault), as ⌃⌘1–4 do. The Vault's workspace actions (WORKSPACE_*) are on the list while it shows.",
    category: "Lab",
    group: "Go",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// What `LAB_GO` takes.
#[derive(Debug, Clone, Facet)]
pub struct GoArgs {
    /// `inbox`, `tasks`, `schedule` or `vault`.
    pub tab: String,
}

/// Puts `LAB_STATE` and `LAB_GO` on `list`, and tells it (`lab`) when
/// the lab moves.
pub fn use_lab_actions(list: &Arc<ActionList>, lab: &Lab) {
    let lab = *lab;
    // `LAB_GO` runs on the window's thread, where the route lives.
    let goes = use_coroutine(
        move |mut asks: UnboundedReceiver<&'static str>| async move {
            use futures_util::StreamExt as _;
            while let Some(tab) = asks.next().await {
                lab.jump(Route::Tab(tab));
            }
        },
    );
    use_hook({
        let list = list.clone();
        move || {
            let tx = goes.tx();
            let _ = list.register(&GO, Effect::Mutating, move |a: GoArgs| {
                let tab = crate::TABS
                    .iter()
                    .map(|(_, _, t)| *t)
                    .find(|t| *t == a.tab)
                    .ok_or_else(|| {
                        format!("no tab {:?}: inbox, tasks, schedule or vault", a.tab)
                    })?;
                tx.unbounded_send(tab)
                    .map_err(|_| "the lab is gone".to_owned())?;
                Ok::<_, String>(LabState {
                    tab: tab.to_owned(),
                    page: "list".to_owned(),
                    bar: String::new(),
                })
            });
        }
    });
    let kept = use_hook(|| Arc::new(Mutex::new(LabState::of(*lab.route.peek()))));
    use_hook({
        let list = list.clone();
        let kept = kept.clone();
        move || {
            let _ = list.register(&STATE, Effect::ReadOnly, move |(): ()| {
                let mut now = kept.lock().unwrap_or_else(PoisonError::into_inner).clone();
                let bar = if architect_shell::keyboard().editing() {
                    "editing"
                } else {
                    "actions"
                };
                bar.clone_into(&mut now.bar);
                Ok(now)
            });
        }
    });
    let told = list.clone();
    use_effect(move || {
        let now = LabState::of((lab.route)());
        let mut state = kept.lock().unwrap_or_else(PoisonError::into_inner);
        if *state != now {
            let tab = now.tab.clone();
            *state = now;
            drop(state);
            told.changed("lab", &tab);
        }
    });
}

/// The lab's guided tours: two of its scenarios (`apps/lab/scenarios`),
/// played live. The lab's data is all made up, so it is its own
/// playground: every tour may play.
#[must_use]
pub fn tours() -> architect_guide::tour::TourSet {
    architect_guide::tour::TourSet::from_sources([
        include_str!("../../scenarios/palette-run.styx"),
        include_str!("../../scenarios/stacked-tabs.styx"),
    ])
    .unwrap_or_default()
    .resolver(architect_workspace::target_selector)
    .resolver(architect_commands::target_selector)
    .playground(|_| Ok(()))
}
