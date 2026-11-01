//! A story's guide scenarios as tours (feature `tours`): every story's
//! scenarios (`#[story(scenario = "…")]`) offered at the Lookbook's root,
//! a button per scenario on its story, the tour drawn over the shell
//! (`architect-guide`'s `TourLayer`). A story is made-up data, so every
//! tour may play.

use std::sync::Arc;

use architect::action::ActionList;
use architect_guide::tour::{use_tours, TourSet};
use architect_story_runtime::{Story, STORIES};
use dioxus::prelude::*;

pub use architect_guide::tour::TourLayer;

/// Offers every story's scenarios as tours, on the app's Action List if
/// it gave one in context, else on the shell's own.
pub fn use_story_tours() {
    let list = use_hook(|| {
        try_consume_context::<Arc<ActionList>>().unwrap_or_else(|| Arc::new(ActionList::new()))
    });
    let set = use_hook(|| {
        let scenarios = STORIES
            .iter()
            .flat_map(|s| s.scenarios.iter())
            .filter_map(|source| architect_guide::load(source).ok())
            .collect();
        TourSet::new(scenarios).playground(|_| Ok(()))
    });
    let _ = use_tours(list, set);
}

/// A button per scenario of `story`: plays it as a tour.
#[component]
pub fn StoryTours(story: &'static Story) -> Element {
    let tours = try_use_context::<architect_guide::tour::Tours>();
    let offered: Vec<(String, String)> = story
        .scenarios
        .iter()
        .filter_map(|source| architect_guide::load(source).ok())
        .map(|s| (s.id, s.title))
        .collect();
    let Some(tours) = tours.filter(|_| !offered.is_empty()) else {
        return rsx! {};
    };
    rsx! {
        div { class: "flex flex-wrap gap-2",
            for (id, title) in offered {
                button {
                    key: "{id}",
                    class: "inline-flex h-7 items-center rounded-md border border-border px-2 text-xs text-foreground hover:bg-accent",
                    "data-tour": "{id}",
                    onclick: move |_| {
                        let _ = tours.start(&id);
                    },
                    "Tour: {title}"
                }
            }
        }
    }
}
