//! What a distribution's web build gives the app besides its extensions:
//! a view of its own compiled in ([`provide_view`]) and an engine that is
//! the clock ([`provide_engine`]).
//!
//! Both are given before [`crate::run`], by the distribution's own start,
//! and are part of its build: code the distribution's maker linked, as its
//! linked extensions are, never something fetched.
//!
//! **A compiled-in view.** A share link's guest runs no linked extension
//! yet (#805), so a bottom tab's view is an island's, in a frame that can
//! reach nothing: no network, no audio of its own. A view that must
//! stream and play (Studio's Arrangement and Mixer, over its engine) is
//! given here instead, under the id the island declares it by
//! (`<extension>`, `<view>`): where the app would mount that island's
//! frame it draws this view in the page itself, for as long as the view
//! says it has something to draw ([`Provided::when`]: an engine with a
//! session open, or opening); otherwise the island's frame is mounted as
//! before. The island's declaration still decides whether the tab exists. This is the narrow form of #805
//! for a distribution's own views, and goes when a guest's host loads the
//! distribution's linked extensions under
//! `share_needs::guest_capabilities`.
//!
//! **An engine as the clock.** The app's player is an element or its own
//! decoder ([`architect_playhead::Clock`], kind `media`). A distribution
//! that brings an engine gives it as a clock of kind `engine`: for an
//! item whose media the engine [`carries`](EngineClock::carries), the
//! engine is loaded, played, sought and read, and the element is left
//! empty, so there is one clock and one thing heard. An item it does not
//! carry (a song its engine has not opened, a note that is not a song)
//! plays on the element as before.

use std::cell::RefCell;
use std::rc::Rc;

use architect_playhead::Clock;
use dioxus::prelude::Element;

/// An engine that plays what the deck's items name.
pub trait EngineClock: Clock {
    /// Whether the engine plays the item whose media is at vault path
    /// `media` now. Asked on every load: an engine still opening a song
    /// says no, and the app's own player takes the item.
    fn carries(&self, media: &str) -> bool;
}

thread_local! {
    static VIEWS: RefCell<Vec<(String, String, Provided)>> = const { RefCell::new(Vec::new()) };
    static ENGINE: RefCell<Option<Rc<RefCell<dyn EngineClock>>>> = const { RefCell::new(None) };
}

/// A view a distribution compiled in.
#[derive(Clone, Copy)]
pub struct Provided {
    /// What draws it, in the page.
    pub root: fn() -> Element,
    /// Whether it has something to draw now. While it says no, the
    /// island's own frame for the same id is mounted instead. Asked a few
    /// times a second while the view is on show.
    pub when: fn() -> bool,
}

/// Draw `view` of `extension` with `provided`, in the page, wherever the
/// app would mount that island view's frame. The last given for an id
/// wins.
pub fn provide_view(extension: &str, view: &str, provided: Provided) {
    VIEWS.with(|v| {
        let mut v = v.borrow_mut();
        v.retain(|(e, n, _)| !(e == extension && n == view));
        v.push((extension.to_owned(), view.to_owned(), provided));
    });
}

/// The view given for `extension`'s `view`, if one was.
#[must_use]
pub fn view(extension: &str, view: &str) -> Option<Provided> {
    VIEWS.with(|v| {
        v.borrow()
            .iter()
            .find(|(e, n, _)| e == extension && n == view)
            .map(|(_, _, provided)| *provided)
    })
}

/// The engine that is the clock for what it carries.
pub fn provide_engine(engine: Rc<RefCell<dyn EngineClock>>) {
    ENGINE.with(|e| *e.borrow_mut() = Some(engine));
}

/// The engine given, if one was.
#[must_use]
pub fn engine() -> Option<Rc<RefCell<dyn EngineClock>>> {
    ENGINE.with(|e| e.borrow().clone())
}

thread_local! {
    /// What the app's player does when the engine's answer to
    /// [`EngineClock::carries`] may have changed.
    static RELOAD: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
    /// Whether the engine is the clock for the item loaded now.
    static IS_CLOCK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// When the app last told the engine to load, play or pause (ms).
    static TOLD: std::cell::Cell<f64> = const { std::cell::Cell::new(f64::NEG_INFINITY) };
}

/// How long after the app tells the engine something its own state may
/// still be the old one (its transport answers on its next poll).
const SETTLE_MS: f64 = 700.0;

/// The engine carries more, or less, than it did (it opened, another song
/// arrived): the app asks again whose the current item is, and the engine
/// takes it over where the deck is, or gives it back.
pub fn engine_changed() {
    let reload = RELOAD.with(|r| r.borrow().clone());
    if let Some(reload) = reload {
        reload();
    }
}

/// Whether the engine is the clock for the item loaded now.
#[must_use]
pub fn engine_is_clock() -> bool {
    IS_CLOCK.with(std::cell::Cell::get)
}

/// Whether the app told the engine something so recently that the
/// engine's own state may not show it yet: a watcher that carries the
/// engine's presses to the deck waits it out.
#[must_use]
pub fn engine_settling() -> bool {
    architect_playhead::now_ms() - TOLD.with(std::cell::Cell::get) < SETTLE_MS
}

/// The app's player says how to load again ([`engine_changed`]).
pub(crate) fn on_engine_changed(reload: impl Fn() + 'static) {
    RELOAD.with(|r| *r.borrow_mut() = Some(Rc::new(reload)));
}

/// The app's player says whether the engine is its clock now.
pub(crate) fn set_engine_is_clock(on: bool) {
    IS_CLOCK.with(|c| c.set(on));
}

/// The app's player told the engine something now.
pub(crate) fn told_engine() {
    TOLD.with(|t| t.set(architect_playhead::now_ms()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one() -> Element {
        dioxus::prelude::VNode::empty()
    }

    fn two() -> Element {
        dioxus::prelude::VNode::empty()
    }

    fn yes() -> bool {
        true
    }

    fn no() -> bool {
        false
    }

    #[test]
    fn a_view_is_found_by_its_extension_and_id_and_the_last_given_wins() {
        assert!(view("studio.transport", "mixer").is_none());
        provide_view(
            "studio.transport",
            "mixer",
            Provided {
                root: one,
                when: no,
            },
        );
        provide_view(
            "studio.transport",
            "arrangement",
            Provided {
                root: one,
                when: yes,
            },
        );
        assert!(view("studio.transport", "mixer").is_some_and(|p| !(p.when)()));
        assert!(view("other", "mixer").is_none());
        provide_view(
            "studio.transport",
            "mixer",
            Provided {
                root: two,
                when: yes,
            },
        );
        let given = view("studio.transport", "mixer").expect("given");
        assert!((given.when)());
        assert!(std::ptr::fn_addr_eq(given.root, two as fn() -> Element));
    }
}
