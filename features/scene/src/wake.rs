//! Waking a host that rests: when a clock jumps, and at a layer's next
//! change.
//!
//! A host draws every frame only while a layer moves every frame
//! ([`crate::anim::Next::Every`]). Otherwise its window rests, and two
//! things must wake it:
//!
//! - A clock that jumps while nothing moves: a seek while paused, a play
//!   pressed. Whoever moves the clock calls [`clocks_moved`]; every host
//!   that asked ([`on_clocks_moved`]) is woken, in whichever window it
//!   is, so a pop-out sees the seek the main window's bar made.
//! - A layer's next step ([`crate::anim::Next::In`]): the host asks to be
//!   woken then ([`after`]), by one timer thread for the whole app.
//!
//! A waker is the host's own (its window's redraw request); it is held
//! weakly here, so a host that closes is forgotten with no word.

use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

/// What wakes a host: its window's redraw request.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

static MOVED: Mutex<Vec<Weak<dyn Fn() + Send + Sync>>> = Mutex::new(Vec::new());

/// Wakes `waker` whenever [`clocks_moved`] is called, for as long as it
/// is held elsewhere.
pub fn on_clocks_moved(waker: &Waker) {
    let mut all = MOVED.lock().unwrap_or_else(PoisonError::into_inner);
    all.retain(|w| w.strong_count() > 0);
    all.push(Arc::downgrade(waker));
}

/// A clock jumped (a seek, a play, a pause, a new rate): every host that
/// asked is woken to draw its layers at the new reading.
pub fn clocks_moved() {
    let wakers: Vec<Waker> = {
        let mut all = MOVED.lock().unwrap_or_else(PoisonError::into_inner);
        all.retain(|w| w.strong_count() > 0);
        all.iter().filter_map(Weak::upgrade).collect()
    };
    for wake in wakers {
        wake();
    }
}

/// Wakes `waker` in `delay`, if it is still held then: whether a timer
/// will (`false` where there is no thread to run one, in a browser; its
/// host then draws every frame instead).
#[must_use]
pub fn after(delay: Duration, waker: &Waker) -> bool {
    timer::schedule(delay, Arc::downgrade(waker))
}

#[cfg(not(target_family = "wasm"))]
mod timer {
    use std::collections::BinaryHeap;
    use std::sync::{Condvar, Mutex, OnceLock, PoisonError, Weak};
    use std::time::{Duration, Instant};

    struct Due {
        at: Instant,
        waker: Weak<dyn Fn() + Send + Sync>,
    }

    impl PartialEq for Due {
        fn eq(&self, other: &Self) -> bool {
            self.at == other.at
        }
    }

    impl Eq for Due {}

    impl PartialOrd for Due {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for Due {
        // The soonest first out of a max-heap.
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            other.at.cmp(&self.at)
        }
    }

    struct Timer {
        due: Mutex<BinaryHeap<Due>>,
        changed: Condvar,
    }

    fn timer() -> Option<&'static Timer> {
        static TIMER: OnceLock<Option<&'static Timer>> = OnceLock::new();
        *TIMER.get_or_init(|| {
            let timer: &'static Timer = Box::leak(Box::new(Timer {
                due: Mutex::new(BinaryHeap::new()),
                changed: Condvar::new(),
            }));
            std::thread::Builder::new()
                .name("architect-scene-timer".to_owned())
                .spawn(move || run(timer))
                .ok()
                .map(|_| timer)
        })
    }

    fn run(timer: &Timer) {
        let mut due = timer.due.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            let now = Instant::now();
            match due.peek().map(|d| d.at) {
                Some(at) if at <= now => {
                    let Some(d) = due.pop() else { continue };
                    drop(due);
                    if let Some(wake) = d.waker.upgrade() {
                        wake();
                    }
                    due = timer.due.lock().unwrap_or_else(PoisonError::into_inner);
                }
                Some(at) => {
                    due = timer
                        .changed
                        .wait_timeout(due, at.saturating_duration_since(now))
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                }
                None => {
                    due = timer
                        .changed
                        .wait(due)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            }
        }
    }

    pub fn schedule(delay: Duration, waker: Weak<dyn Fn() + Send + Sync>) -> bool {
        let Some(timer) = timer() else {
            return false;
        };
        let Some(at) = Instant::now().checked_add(delay) else {
            return false;
        };
        timer
            .due
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Due { at, waker });
        timer.changed.notify_one();
        true
    }
}

#[cfg(target_family = "wasm")]
mod timer {
    use std::sync::Weak;
    use std::time::Duration;

    pub fn schedule(_: Duration, _: Weak<dyn Fn() + Send + Sync>) -> bool {
        false
    }
}

/// How a host that rests wakes for its layers: one per host, kept with
/// it. It asks the timer for its next change only when that is sooner
/// than what it asked for already.
pub struct Alarm {
    waker: Waker,
    /// When it asked to be woken, on the app's wall clock.
    set: Mutex<Option<f64>>,
}

impl std::fmt::Debug for Alarm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Alarm").finish_non_exhaustive()
    }
}

impl Alarm {
    /// An alarm that calls `waker`, which also hears [`clocks_moved`].
    #[must_use]
    pub fn new(waker: Waker) -> Self {
        on_clocks_moved(&waker);
        Self {
            waker,
            set: Mutex::new(None),
        }
    }

    /// What it calls: to watch an engine's table with
    /// ([`crate::engine::watch`]).
    #[must_use]
    pub const fn waker(&self) -> &Waker {
        &self.waker
    }

    /// The host drew; what it drew next changes at `next`. Whether it
    /// must draw every frame (it moves, or there is no timer to wake it).
    pub fn drawn(&self, next: crate::anim::Next) -> bool {
        match next {
            crate::anim::Next::Every => true,
            crate::anim::Next::Never => false,
            crate::anim::Next::In(seconds) => {
                let now = crate::anim::wall_seconds();
                let at = now + seconds.max(0.0);
                let mut set = self.set.lock().unwrap_or_else(PoisonError::into_inner);
                if set.is_some_and(|s| s > now && s <= at) {
                    return false;
                }
                // A frame late rather than early: the step is drawn once
                // its time has come.
                let delay = Duration::from_secs_f64(seconds.clamp(0.0, 1.0e6))
                    .saturating_add(Duration::from_millis(1));
                if after(delay, &self.waker) {
                    *set = Some(at);
                    false
                } else {
                    true
                }
            }
        }
    }
}
