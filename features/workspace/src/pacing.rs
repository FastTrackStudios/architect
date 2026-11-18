//! A runtime view's frames, paced to the display (#840).
//!
//! A runtime view's batch (a frame from its process) arrives whenever its
//! process answered, at a moment that has nothing to do with the
//! display. Asking the window for a frame then starts one at once: its
//! commands take a fraction of a millisecond, and then the renderer waits,
//! most of a refresh, for a swapchain image, which comes free only when
//! the frame before it is on screen. Measured on a 120 Hz display
//! (the sandbox's arrangement as a process): 0.6 ms of work
//! and 7.6 ms waiting a frame, the window's thread blocked meanwhile, and
//! what the frame shows taken a refresh before it needed to be.
//!
//! [`Pacer::request`] asks instead, when the window's last frame waited
//! for its image (its render took at least half a refresh, so it ended
//! at a refresh), for a frame one refresh after that one ended
//! ([`frame_ended`]), when the next image is about to come free: the
//! batches that arrive until then share it, the window's thread is free
//! meanwhile, and the frame shows the newest batch. After a frame that
//! did not wait (an idle window, or one slower than a refresh, whose
//! next image may be free already) it asks at once: no wait is added
//! where none was saved.
//!
//! The launcher tells it when each frame ended and the display's refresh
//! ([`set_refresh`]). `ARCHITECT_FRAME_PACING=0` turns it off (each batch
//! asks for a frame at once, as before), for a before-and-after.

// Times in microseconds and frame counts: far from any bound, and the
// percentiles index sorted, non-empty slices.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::suboptimal_flops
)]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// When the last frame of any window ended (µs since [`epoch`], 0: none).
static LAST_END: AtomicU64 = AtomicU64::new(0);
/// Whether that frame waited for its image (its render took at least
/// half a refresh): then it ended at a refresh.
static LAST_WAITED: AtomicBool = AtomicBool::new(false);
/// The display's refresh interval, µs (0: not known; 60 Hz is assumed).
static REFRESH: AtomicU64 = AtomicU64::new(0);

/// Started this much before a refresh after the last frame: a frame's
/// own work, so it is ready when its image is.
pub const LEAD: Duration = Duration::from_millis(1);

fn epoch() -> Instant {
    static E: OnceLock<Instant> = OnceLock::new();
    *E.get_or_init(Instant::now)
}

fn micros(at: Instant) -> u64 {
    u64::try_from(at.saturating_duration_since(epoch()).as_micros())
        .unwrap_or(u64::MAX)
        .max(1)
}

/// Whether frames are paced (`ARCHITECT_FRAME_PACING=0`: not).
#[must_use]
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("ARCHITECT_FRAME_PACING").map_or(true, |v| v != "0" && v != "off")
    })
}

/// A window ended a frame at `at`, its renderer's call (rasterise,
/// composite, present: waiting for an image included) having taken
/// `render` (the launcher, after every frame).
pub fn frame_ended(at: Instant, render: Duration) {
    LAST_WAITED.store(render >= refresh() / 2, Ordering::Relaxed);
    LAST_END.store(micros(at), Ordering::Relaxed);
}

/// The display refreshes every `interval` (the launcher, when it learns
/// it or the window moves to another display).
pub fn set_refresh(interval: Duration) {
    let us = u64::try_from(interval.as_micros()).unwrap_or(0);
    if us > 0 {
        REFRESH.store(us, Ordering::Relaxed);
    }
}

/// The display's refresh interval as known now.
#[must_use]
pub fn refresh() -> Duration {
    match REFRESH.load(Ordering::Relaxed) {
        0 => Duration::from_micros(16_667),
        us => Duration::from_micros(us),
    }
}

/// How long from `now` until a frame asked for now should start.
///
/// Zero when the last frame did not wait for its image, or a refresh has
/// passed since it ended (or none ever did); else until one refresh
/// after it, less [`LEAD`].
#[must_use]
pub fn wait_from(now: Instant) -> Duration {
    let last = LAST_END.load(Ordering::Relaxed);
    if last == 0 || !LAST_WAITED.load(Ordering::Relaxed) {
        return Duration::ZERO;
    }
    let due =
        last.saturating_add(u64::try_from(refresh().saturating_sub(LEAD).as_micros()).unwrap_or(0));
    Duration::from_micros(due.saturating_sub(micros(now)))
}

/// What asks a window for a frame (its shell's redraw request).
pub type Redraw = Arc<dyn Fn() + Send + Sync>;

/// One view's frame requests, paced: at most one waits at a time.
#[derive(Clone)]
pub struct Pacer {
    redraw: Redraw,
    pending: Arc<AtomicBool>,
}

impl Pacer {
    #[must_use]
    pub fn new(redraw: Redraw) -> Self {
        Self {
            redraw,
            pending: Arc::new(AtomicBool::new(false)),
        }
    }

    /// A frame, at the next refresh's turn: now, or after a wait (one
    /// request covers every batch until it is made).
    pub fn request(&self) {
        let wait = if enabled() {
            wait_from(Instant::now())
        } else {
            Duration::ZERO
        };
        if wait.is_zero() {
            crate::frames::asked(crate::frames::Asked::Now);
            (self.redraw)();
            return;
        }
        if self.pending.swap(true, Ordering::AcqRel) {
            return;
        }
        let at = Instant::now() + wait;
        timer::schedule(at, self.clone());
    }

    fn fire(&self) {
        self.pending.store(false, Ordering::Release);
        crate::frames::asked(crate::frames::Asked::Paced);
        (self.redraw)();
    }
}

mod timer {
    use super::{Instant, Mutex, OnceLock, Pacer, PoisonError};
    use std::sync::Condvar;

    struct Queue {
        due: Mutex<Vec<(Instant, Pacer)>>,
        changed: Condvar,
    }

    fn queue() -> &'static Queue {
        static Q: OnceLock<Queue> = OnceLock::new();
        Q.get_or_init(|| {
            let _ = std::thread::Builder::new()
                .name("architect-frame-pacer".into())
                .spawn(run);
            Queue {
                due: Mutex::new(Vec::new()),
                changed: Condvar::new(),
            }
        })
    }

    pub(super) fn schedule(at: Instant, pacer: Pacer) {
        let q = queue();
        q.due
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((at, pacer));
        q.changed.notify_one();
    }

    fn run() {
        let q = queue();
        let mut due = q.due.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            let now = Instant::now();
            let (ready, waiting): (Vec<_>, Vec<_>) = due.drain(..).partition(|(at, _)| *at <= now);
            *due = waiting;
            if !ready.is_empty() {
                drop(due);
                for (_, pacer) in ready {
                    pacer.fire();
                }
                due = q.due.lock().unwrap_or_else(PoisonError::into_inner);
                continue;
            }
            due = match due.iter().map(|(at, _)| *at).min() {
                Some(next) => {
                    q.changed
                        .wait_timeout(due, next.saturating_duration_since(now))
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
                None => q.changed.wait(due).unwrap_or_else(PoisonError::into_inner),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn a_frame_asked_for_right_after_one_waits_for_the_next_refresh() {
        let _g = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        set_refresh(Duration::from_micros(8_333));
        let now = Instant::now();
        frame_ended(now, Duration::from_millis(7));
        let wait = wait_from(now);
        assert!(
            wait > Duration::from_millis(6) && wait <= Duration::from_micros(8_333),
            "{wait:?}"
        );
        // A refresh later, at once.
        assert_eq!(wait_from(now + Duration::from_millis(9)), Duration::ZERO);
        // After a frame that did not wait for its image, at once: when
        // the next comes free is not known, so no wait is added.
        frame_ended(now, Duration::from_millis(1));
        assert_eq!(wait_from(now), Duration::ZERO);
    }

    #[test]
    fn batches_before_the_frame_share_one_request() {
        let _g = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        if !enabled() {
            return;
        }
        set_refresh(Duration::from_micros(8_333));
        let asked = Arc::new(AtomicU32::new(0));
        let pacer = Pacer::new({
            let asked = Arc::clone(&asked);
            Arc::new(move || {
                asked.fetch_add(1, Ordering::SeqCst);
            })
        });
        frame_ended(Instant::now(), Duration::from_millis(7));
        for _ in 0..10 {
            pacer.request();
        }
        assert_eq!(asked.load(Ordering::SeqCst), 0, "asked before its refresh");
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(asked.load(Ordering::SeqCst), 1, "one frame for ten batches");
        // The window drew it long ago: the next is asked for at once.
        LAST_END.store(1, Ordering::Relaxed);
        pacer.request();
        assert_eq!(asked.load(Ordering::SeqCst), 2);
    }
}
