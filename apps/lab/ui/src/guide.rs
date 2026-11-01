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
        }
    }
}

static STATE: ActionMeta = ActionMeta {
    id: "LAB_STATE",
    trait_name: "",
    method_name: "",
    display_name: "Where the lab is",
    description: "The tab on show and whether it is the list, an item or a capture.",
    category: "Lab",
    group: "Go",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// Puts `LAB_STATE` on `list`, and tells it (`lab`) when the lab moves.
pub fn use_lab_actions(list: &Arc<ActionList>, lab: &Lab) {
    let lab = *lab;
    let kept = use_hook(|| Arc::new(Mutex::new(LabState::of(*lab.route.peek()))));
    use_hook({
        let list = list.clone();
        let kept = kept.clone();
        move || {
            let _ = list.register(&STATE, Effect::ReadOnly, move |(): ()| {
                Ok(kept.lock().unwrap_or_else(PoisonError::into_inner).clone())
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
