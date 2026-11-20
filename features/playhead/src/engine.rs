//! An engine as the deck's clock (#528, studio#400).
//!
//! The app's player is a [`Clock`] of kind `media`: an element, or the
//! web app's own decoder. A distribution that brings an engine (a DAW
//! that plays a song's tracks itself, in the page) gives it here as an
//! [`EngineClock`]: for an item whose media the engine
//! [`carries`](EngineClock::carries), the app's player loads, plays, seeks
//! and reads the engine (kind `engine`) and holds nothing itself, so there
//! is one clock and one thing heard. An item it does not carry (a song it
//! has not opened, a note that is not a song) plays on the player as
//! before.
//!
//! Whoever brings the engine (a linked extension's browser build, a
//! distribution's start) calls [`provide`]; the app's player reads
//! [`current`] on every load, and says how to load again when the engine
//! carries more or less than it did ([`changed`]). Per thread: the page
//! has one, and a native app gives none.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::clock::Clock;

/// An engine that plays what the deck's items name.
pub trait EngineClock: Clock {
    /// Whether the engine plays the item whose media is at vault path
    /// `media` now. Asked on every load: an engine still opening the song
    /// says no, and the app's own player takes the item.
    fn carries(&self, media: &str) -> bool;
}

/// A shared engine clock, as given.
pub type Shared = Rc<RefCell<dyn EngineClock>>;

thread_local! {
    static ENGINE: RefCell<Option<Shared>> = const { RefCell::new(None) };
    /// What the app's player does when what the engine carries changed.
    static RELOAD: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
    /// Whether the engine is the clock for the item loaded now.
    static IS_CLOCK: Cell<bool> = const { Cell::new(false) };
    /// When the app last told the engine to load, play, pause or seek (ms).
    static TOLD: Cell<f64> = const { Cell::new(f64::NEG_INFINITY) };
}

/// How long after the app tells the engine something the engine's own
/// state may still be the old one (it answers on its next poll), ms.
pub const SETTLE_MS: f64 = 700.0;

/// The engine that is the clock for what it carries. The last given wins.
pub fn provide(engine: Shared) {
    ENGINE.with(|e| *e.borrow_mut() = Some(engine));
}

/// The engine given, if one was.
#[must_use]
pub fn current() -> Option<Shared> {
    ENGINE.with(|e| e.borrow().clone())
}

/// The engine carries more, or less, than it did (it opened, another song
/// arrived): the app's player asks again whose the current item is, and
/// the engine takes it over where the deck is, or gives it back.
pub fn changed() {
    let reload = RELOAD.with(|r| r.borrow().clone());
    if let Some(reload) = reload {
        reload();
    }
}

/// What [`changed`] runs: the app's player's load-again.
pub fn on_changed(reload: impl Fn() + 'static) {
    RELOAD.with(|r| *r.borrow_mut() = Some(Rc::new(reload)));
}

/// Whether the engine is the clock for the item loaded now.
#[must_use]
pub fn is_clock() -> bool {
    IS_CLOCK.with(Cell::get)
}

/// The app's player says whether the engine is its clock now.
pub fn set_is_clock(on: bool) {
    IS_CLOCK.with(|c| c.set(on));
}

/// The app's player told the engine something now.
pub fn told() {
    TOLD.with(|t| t.set(crate::now_ms()));
}

/// Whether the app told the engine something so recently
/// ([`SETTLE_MS`]) that the engine's own state may not show it yet: a
/// watcher that carries the engine's own presses to the deck waits it
/// out.
#[must_use]
pub fn settling() -> bool {
    crate::now_ms() - TOLD.with(Cell::get) < SETTLE_MS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Reading;

    #[derive(Default)]
    struct Engine {
        loaded: Option<String>,
    }

    impl Clock for Engine {
        fn kind(&self) -> &'static str {
            "engine"
        }
        fn load(&mut self, media: Option<&str>) {
            self.loaded = media.map(str::to_owned);
        }
        fn play(&mut self) {}
        fn pause(&mut self) {}
        fn seek(&mut self, _: f64) {}
        fn read(&self) -> Option<Reading> {
            None
        }
    }

    impl EngineClock for Engine {
        fn carries(&self, media: &str) -> bool {
            media.starts_with("Songs/")
        }
    }

    #[test]
    fn the_engine_given_is_asked_what_it_carries_and_a_change_reloads() {
        assert!(current().is_none());
        let engine: Shared = Rc::new(RefCell::new(Engine::default()));
        provide(Rc::clone(&engine));
        let given = current().expect("given");
        assert!(given.borrow().carries("Songs/Hey/hey.ogg"));
        assert!(!given.borrow().carries("Recordings/a.mp3"));
        let reloaded = Rc::new(Cell::new(0));
        let count = Rc::clone(&reloaded);
        on_changed(move || count.set(count.get() + 1));
        changed();
        changed();
        assert_eq!(reloaded.get(), 2);
    }

    #[test]
    fn told_settles_and_the_clock_flag_is_the_players() {
        assert!(!settling());
        told();
        assert!(settling());
        assert!(!is_clock());
        set_is_clock(true);
        assert!(is_clock());
    }
}
