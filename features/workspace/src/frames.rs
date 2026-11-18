//! `APP_FRAMES` (feature `native`): the main window's frames as the app
//! drew them (#839's first meter).
//!
//! Each redraw of the main window is timed where the window's thread
//! hands it to Blitz ([`crate::native`]'s `RedrawRequested`): the whole
//! frame on that thread (the document's styles, layout, the paint into
//! the scene, the GPU's submit and the present). The time between one
//! redraw's start and the next is what reached the screen: on a 60 Hz
//! output with vsync no faster than 16.7 ms, whatever the work. The GPU's
//! own raster is not separated from the present's wait.
//!
//! `APP_FRAMES` answers the last [`KEPT`] frames' work and gaps (p50,
//! p99, max), how many frames, and the rate; `{reset: true}` starts the
//! count again (before a measurement).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use architect::action::{ActionCaller, ActionList, ActionMeta, Effect};
use facet::Facet;

/// How many frames the meter keeps: a minute at 60 Hz, 8.5 s at 240.
pub const KEPT: usize = 2048;

/// `APP_FRAMES`' id.
pub const APP_FRAMES: &str = "APP_FRAMES";

/// The main window's frames so far.
#[derive(Debug, Default)]
struct Log {
    /// Each frame's work on the window's thread.
    work: VecDeque<Duration>,
    /// The time from each frame's start to the next one's.
    gaps: VecDeque<Duration>,
    last: Option<Instant>,
    first: Option<Instant>,
    frames: u64,
}

static LOG: Mutex<Option<Log>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut Log) -> T) -> T {
    let mut log = LOG.lock().unwrap_or_else(PoisonError::into_inner);
    f(log.get_or_insert_with(Log::default))
}

/// One frame of the main window: it started at `start` and its work on
/// the window's thread ended at `end`.
pub fn record(start: Instant, end: Instant) {
    with(|log| {
        if let Some(last) = log.last {
            push(&mut log.gaps, start.saturating_duration_since(last));
        }
        log.last = Some(start);
        log.first.get_or_insert(start);
        push(&mut log.work, end.saturating_duration_since(start));
        log.frames = log.frames.saturating_add(1);
    });
}

fn push(q: &mut VecDeque<Duration>, d: Duration) {
    if q.len() == KEPT {
        q.pop_front();
    }
    q.push_back(d);
}

/// Forgets every frame so far.
pub fn reset() {
    with(|log| *log = Log::default());
}

/// p50, p99 and max of some durations, in milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Facet)]
pub struct Spread {
    pub p50_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

impl Spread {
    fn of(q: &VecDeque<Duration>) -> Self {
        let mut ms: Vec<f64> = q.iter().map(|d| d.as_secs_f64() * 1e3).collect();
        ms.sort_by(f64::total_cmp);
        // Nearest rank: the p-th percent of n is at ceil(p·n / 100) - 1.
        let at = |percent: usize| {
            let rank = percent
                .saturating_mul(ms.len())
                .div_ceil(100)
                .saturating_sub(1);
            ms.get(rank).copied().unwrap_or_default()
        };
        Self {
            p50_ms: at(50),
            p99_ms: at(99),
            max_ms: ms.last().copied().unwrap_or_default(),
        }
    }
}

/// What `APP_FRAMES` answers.
#[derive(Debug, Clone, Default, PartialEq, Facet)]
pub struct Report {
    /// Frames since the start or the last reset.
    pub frames: u64,
    /// Of them, the last ones kept ([`KEPT`]).
    pub kept: usize,
    /// Each frame's work on the window's thread.
    pub work: Spread,
    /// From one frame's start to the next: what reached the screen.
    pub gap: Spread,
    /// Frames a second over the kept frames' span.
    pub fps: f64,
}

/// The meter now.
#[must_use]
pub fn report() -> Report {
    with(|log| {
        let span: Duration = log.gaps.iter().sum();
        let gaps = f64::from(u32::try_from(log.gaps.len()).unwrap_or(u32::MAX));
        let fps = if span.is_zero() {
            0.0
        } else {
            gaps / span.as_secs_f64()
        };
        Report {
            frames: log.frames,
            kept: log.work.len(),
            work: Spread::of(&log.work),
            gap: Spread::of(&log.gaps),
            fps,
        }
    })
}

/// `APP_FRAMES`' arguments.
#[derive(Debug, Default, Facet)]
pub struct FramesArgs {
    /// Forget the frames so far (after answering them).
    #[facet(default)]
    pub reset: bool,
}

static META: ActionMeta = ActionMeta {
    id: APP_FRAMES,
    trait_name: "",
    method_name: "",
    display_name: "The window's frame times",
    description: "The main window's last frames: each one's work on the window's thread (styles, layout, \
                  paint, the GPU's submit and present) and the time from one to the next (what reached the \
                  screen), as p50, p99 and max in milliseconds, the count and the rate. `reset` forgets them \
                  after answering, to measure from now.",
    category: "App",
    group: "Agent tools",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// Puts `APP_FRAMES` on `list`.
///
/// # Errors
///
/// Its id is taken.
pub fn register(list: &Arc<ActionList>) -> Result<(), architect::action::ActionError> {
    list.register_with(
        &META,
        Effect::ReadOnly,
        move |_: &ActionCaller, args: FramesArgs| {
            let answer = report();
            if args.reset {
                reset();
            }
            Ok::<_, String>(answer)
        },
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp)]

    use super::*;

    #[test]
    fn frames_are_timed_and_spread() {
        reset();
        let t = Instant::now();
        for i in 0..100_u32 {
            let start = t + Duration::from_millis(u64::from(i) * 4);
            record(
                start,
                start + Duration::from_millis(if i == 99 { 9 } else { 1 }),
            );
        }
        let r = report();
        assert_eq!(r.frames, 100);
        assert_eq!(r.work.p50_ms, 1.0);
        assert_eq!(r.work.max_ms, 9.0);
        assert_eq!(r.gap.p99_ms, 4.0);
        assert!((r.fps - 250.0).abs() < 0.01, "{}", r.fps);
        reset();
        assert_eq!(report().frames, 0);
    }
}
