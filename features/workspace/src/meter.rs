//! How an extension's view keeps up: each frame's time against a budget,
//! and whether it is busy now (`docs/content/architecture/extensions.md`,
//! "Responsive by design: views and workers").
//!
//! A host measures every frame of a view it does not draw itself — a
//! Luau script's window, a process's remote view, a component's — and
//! records it in the view's [`ViewMeter`]. A frame past the budget is
//! *slow*; a view whose frame is still running past the budget is *busy*:
//! the host keeps showing its last good frame with a small busy mark,
//! and the rest of the app goes on. [`Meters`] holds every view's meter
//! for an app (`EXT_VIEWS` reads it), and tells a [`ViewReport`] when a
//! view turns slow or busy and when it comes back: the seam an app wires
//! to its status board.
//!
//! The model only: no clock. The host measures; the meter keeps count.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// A view's frame budget when its host gives none: one frame at 60 Hz.
pub const FRAME_BUDGET: Duration = Duration::from_millis(16);

/// What a view's meter says now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewSummary {
    /// Frames measured.
    pub frames: u64,
    /// Of those, past the budget.
    pub slow: u64,
    /// Of the last [`ViewMeter::KEPT`]: the median, the 99th percentile
    /// and the slowest.
    pub p50: Duration,
    pub p99: Duration,
    pub max: Duration,
    pub budget: Duration,
    /// A frame is running past the budget now.
    pub busy: bool,
}

/// What a [`ViewReport`] is told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewHealth {
    /// A frame took this long, past the budget.
    Slow(Duration),
    /// A frame is still running past the budget: the host shows the last
    /// good one, marked busy.
    Busy,
    /// It is drawing within its budget again.
    Recovered,
}

/// Where a view's health goes: an app's status board, a log, a test.
///
/// Called on the thread that measured, outside the meter's lock; keep it
/// cheap (hand the news on).
pub trait ViewReport: Send + Sync {
    fn report(&self, view: &str, health: ViewHealth, summary: &ViewSummary);
}

#[derive(Debug)]
struct Times {
    budget: Duration,
    last: VecDeque<Duration>,
    frames: u64,
    slow: u64,
    busy: bool,
    /// Its last frame was slow, or it went busy: the next good frame is
    /// a recovery.
    struggling: bool,
}

/// One view's frame times. Cheap to clone; clones share it.
#[derive(Clone)]
pub struct ViewMeter {
    name: Arc<str>,
    times: Arc<Mutex<Times>>,
    report: Option<Arc<dyn ViewReport>>,
}

impl std::fmt::Debug for ViewMeter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ViewMeter")
            .field("name", &self.name)
            .field("summary", &self.summary())
            .finish_non_exhaustive()
    }
}

impl PartialEq for ViewMeter {
    /// The same meter, not an equal one.
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.times, &other.times)
    }
}

impl ViewMeter {
    /// Frames kept for the percentiles.
    pub const KEPT: usize = 512;

    /// A meter on its own (in no [`Meters`]), named `name`, with `budget`.
    #[must_use]
    pub fn new(name: &str, budget: Duration) -> Self {
        Self {
            name: Arc::from(name),
            times: Arc::new(Mutex::new(Times {
                budget,
                last: VecDeque::new(),
                frames: 0,
                slow: 0,
                busy: false,
                struggling: false,
            })),
            report: None,
        }
    }

    /// Its view's name (`lab.clicker/counter`).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn budget(&self) -> Duration {
        self.lock().budget
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Times> {
        self.times.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A frame took `took`; whether it was past the budget. A frame ends
    /// the busy state.
    #[allow(clippy::must_use_candidate)] // Most hosts only record.
    pub fn record(&self, took: Duration) -> bool {
        let (health, summary) = {
            let mut t = self.lock();
            if t.last.len() == Self::KEPT {
                t.last.pop_front();
            }
            t.last.push_back(took);
            t.frames = t.frames.saturating_add(1);
            let slow = took > t.budget;
            let health = if slow {
                t.slow = t.slow.saturating_add(1);
                Some(ViewHealth::Slow(took))
            } else if t.struggling {
                Some(ViewHealth::Recovered)
            } else {
                None
            };
            t.struggling = slow;
            t.busy = false;
            let summary = summarize(&t);
            drop(t);
            (health, summary)
        };
        if let Some(health) = health {
            self.tell(health, &summary);
        }
        matches!(health, Some(ViewHealth::Slow(_)))
    }

    /// A frame is running past the budget (`true`), or none is.
    pub fn set_busy(&self, busy: bool) {
        let summary = {
            let mut t = self.lock();
            if t.busy == busy {
                return;
            }
            t.busy = busy;
            if busy {
                t.struggling = true;
            }
            let summary = summarize(&t);
            drop(t);
            summary
        };
        if busy {
            self.tell(ViewHealth::Busy, &summary);
        }
    }

    /// Whether a frame is running past the budget now.
    #[must_use]
    pub fn busy(&self) -> bool {
        self.lock().busy
    }

    #[must_use]
    pub fn summary(&self) -> ViewSummary {
        summarize(&self.lock())
    }

    fn tell(&self, health: ViewHealth, summary: &ViewSummary) {
        if let Some(report) = &self.report {
            report.report(&self.name, health, summary);
        }
    }
}

fn summarize(t: &Times) -> ViewSummary {
    let mut sorted: Vec<Duration> = t.last.iter().copied().collect();
    sorted.sort_unstable();
    ViewSummary {
        frames: t.frames,
        slow: t.slow,
        p50: percentile(&sorted, 50),
        p99: percentile(&sorted, 99),
        max: sorted.last().copied().unwrap_or_default(),
        budget: t.budget,
        busy: t.busy,
    }
}

/// The nearest-rank `p`th percentile of `sorted`.
#[must_use]
pub fn percentile(sorted: &[Duration], p: usize) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let rank = sorted.len().saturating_mul(p).div_ceil(100).max(1);
    sorted
        .get(rank.saturating_sub(1))
        .copied()
        .unwrap_or_default()
}

/// Every extension view's meter in an app, by name. Cheap to clone;
/// clones share them.
#[derive(Clone, Default)]
pub struct Meters {
    all: Arc<Mutex<BTreeMap<String, ViewMeter>>>,
    report: Option<Arc<dyn ViewReport>>,
}

impl std::fmt::Debug for Meters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Meters")
            .field("views", &self.names())
            .field("report", &self.report.is_some())
            .finish_non_exhaustive()
    }
}

impl PartialEq for Meters {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.all, &other.all)
    }
}

impl Meters {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Tells `report` when a view turns slow or busy, and when it comes
    /// back (an app's status board: `APP_STATUS`'s problems).
    #[must_use]
    pub fn with_report(mut self, report: Arc<dyn ViewReport>) -> Self {
        self.report = Some(report);
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, ViewMeter>> {
        self.all.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// View `name`'s meter, made with `budget` the first time it is asked
    /// for. A view mounted again keeps its counts.
    #[must_use]
    pub fn meter(&self, name: &str, budget: Duration) -> ViewMeter {
        self.lock()
            .entry(name.to_owned())
            .or_insert_with(|| ViewMeter {
                report: self.report.clone(),
                ..ViewMeter::new(name, budget)
            })
            .clone()
    }

    /// Forgets view `name` (its extension unloaded).
    pub fn remove(&self, name: &str) {
        self.lock().remove(name);
    }

    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.lock().keys().cloned().collect()
    }

    /// Every view's summary, by name.
    #[must_use]
    pub fn summaries(&self) -> Vec<(String, ViewSummary)> {
        let all: Vec<ViewMeter> = self.lock().values().cloned().collect();
        all.iter()
            .map(|m| (m.name().to_owned(), m.summary()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::{Meters, ViewHealth, ViewReport, ViewSummary};

    #[derive(Default)]
    struct Heard(Mutex<Vec<(String, ViewHealth)>>);

    impl ViewReport for Heard {
        fn report(&self, view: &str, health: ViewHealth, _: &ViewSummary) {
            if let Ok(mut h) = self.0.lock() {
                h.push((view.to_owned(), health));
            }
        }
    }

    #[test]
    fn a_meter_counts_slow_frames_and_reports_busy_and_back() {
        let heard = Arc::new(Heard::default());
        let meters = Meters::new().with_report(heard.clone());
        let m = meters.meter("ext/view", Duration::from_millis(16));
        let ms = Duration::from_millis;
        assert!(!m.record(ms(4)));
        assert!(m.record(ms(40)));
        assert!(!m.record(ms(5)));
        m.set_busy(true);
        m.set_busy(true);
        assert!(m.summary().busy);
        assert!(m.record(ms(300)));
        assert!(!m.summary().busy, "a frame ends the busy state");
        assert!(!m.record(ms(3)));
        let s = m.summary();
        assert_eq!((s.frames, s.slow, s.max), (5, 2, ms(300)));
        assert_eq!(s.p50, ms(5));
        let heard = heard.0.lock().map(|h| h.clone()).unwrap_or_default();
        let health: Vec<ViewHealth> = heard.iter().map(|(_, h)| *h).collect();
        assert_eq!(
            health,
            [
                ViewHealth::Slow(ms(40)),
                ViewHealth::Recovered,
                ViewHealth::Busy,
                ViewHealth::Slow(ms(300)),
                ViewHealth::Recovered,
            ]
        );
        assert_eq!(meters.meter("ext/view", ms(1)).summary().frames, 5);
        assert_eq!(meters.summaries().len(), 1);
    }
}
