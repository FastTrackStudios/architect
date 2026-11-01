//! A story's scenarios (feature `story`), played by the runner.
//!
//! What `#[story(scenario = "x.styx")]` gives a story (`architect-story`)
//! is played as the lab's scenarios are: filmed
//! ([`crate::cli::main_stories`]) or tested ([`test_stories`]).
//!
//! The app a story's scenario runs over is the story alone ([`app`]): its
//! render with its knobs' defaults, an Action List, and its scenarios as
//! tours. A story is made-up data, so its tours are always safe to play.

use std::sync::Arc;

use architect::action::ActionList;
use architect_story_runtime::{NoKnobs, STORIES, Story, render_fn};
use dioxus::prelude::*;

use crate::run::{App, run};
use crate::tour::{TourLayer, TourSet, use_tours};
use crate::{LoadError, Scenario};

/// `story`'s scenarios, read.
///
/// # Errors
///
/// The first that does not load, named with the story.
pub fn scenarios(story: &Story) -> Result<Vec<Scenario>, LoadError> {
    story
        .scenarios
        .iter()
        .map(|source| {
            crate::load(source).map_err(|e| LoadError::Parse(format!("story {}: {e}", story.name)))
        })
        .collect()
}

/// Every story linked into the binary that carries scenarios.
pub fn with_scenarios() -> impl Iterator<Item = &'static Story> {
    STORIES.iter().copied().filter(|s| !s.scenarios.is_empty())
}

/// The story found in context: which one [`StoryRoot`] draws.
#[derive(Clone, Copy)]
struct Drawn(&'static Story);

/// `story` as a scenario's app: its render, a fresh Action List, its
/// scenarios as tours.
#[must_use]
pub fn app(story: &'static Story) -> App {
    App::new(StoryRoot, Arc::new(ActionList::new())).context(Drawn(story))
}

/// The story, with its tours over it.
#[component]
fn StoryRoot() -> Element {
    let Drawn(story) = use_context::<Drawn>();
    let list = use_context::<Arc<ActionList>>();
    let set =
        use_hook(|| TourSet::new(scenarios(story).unwrap_or_default()).playground(|_| Ok(())));
    let _ = use_tours(list, set);
    // SAFETY: `story.render` is a `RenderFn`, made by `#[story]` or
    // `const_story`.
    let render = unsafe { render_fn(story) };
    rsx! {
        div { style: "position: absolute; left: 0; top: 0; right: 0; bottom: 0; overflow: hidden; \
                      background: #09090b; color: #e4e4e7;",
            div { style: "padding: 24px;", {render(&NoKnobs)} }
            TourLayer {}
        }
    }
}

/// Plays every scenario of `stories` as a test: no frames, each step's
/// `done` asserted, every failure collected.
///
/// # Errors
///
/// Each scenario that did not load or did not run, one per line.
pub fn test_stories(stories: impl IntoIterator<Item = &'static Story>) -> Result<usize, String> {
    let mut failed = Vec::new();
    let mut ran = 0_usize;
    for story in stories {
        let list = match scenarios(story) {
            Ok(list) => list,
            Err(e) => {
                failed.push(e.to_string());
                continue;
            }
        };
        for scenario in &list {
            ran = ran.saturating_add(1);
            if let Err(e) = run(&app(story), scenario, None) {
                failed.push(format!("{} (story {}): {e}", scenario.id, story.name));
            }
        }
    }
    if failed.is_empty() {
        Ok(ran)
    } else {
        Err(failed.join("\n"))
    }
}
