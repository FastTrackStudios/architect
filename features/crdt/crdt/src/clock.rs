//! The shared clock presence is stamped in.
//!
//! A playhead in presence (`PlayState`) says where it was *and when*, so a
//! peer extrapolates it instead of streaming it. "When" is only useful if
//! both sides read the same clock, and two processes never do: each
//! [`crate::presence::clock_ms`] starts at its own first call. So the
//! presence host serves its clock ([`crate::sync::DocPresence::now`]), each
//! peer's presence driver pings it ten times a second
//! ([`crate::sync::PresenceDriver::run`]) into the peer's [`SharedClock`],
//! and a playhead is stamped in the host's clock
//! ([`crate::presence::Here::play_now`]). A peer reading another's
//! playhead extrapolates it with [`SharedClock::now_ms`].
//!
//! The estimate is NTP's: one exchange gives four stamps, `t1` sent here,
//! `t2` received there, `t3` answered there, `t4` received here;
//! `offset = ((t2 − t1) + (t3 − t4)) / 2` (there − here), wrong by at most
//! half the exchange's asymmetry. [`ClockEstimator`] keeps a window of
//! exchanges and averages the offsets of the fastest quarter, the round
//! trips nearest both paths' minimum, so a congested packet moves it by
//! nothing. (The same algorithm as the session primitive's
//! `session::sync::ClockEstimator`, FastTrackStudios/session.)

use std::sync::{Arc, Mutex, PoisonError};

/// One exchange, as recorded.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Exchange {
    offset: f64,
    round_trip: f64,
}

/// The offset to one other clock, from a window of exchanges.
#[derive(Clone, Debug)]
pub struct ClockEstimator {
    window: Vec<Exchange>,
    cap: usize,
    cursor: usize,
    /// The window's answer, smoothed, so a change in which exchanges are
    /// fastest does not jump it.
    smoothed: Option<f64>,
}

/// How much of each new window answer the smoothed offset takes (per
/// exchange; at 10 Hz, about a second's time constant).
const SMOOTHING: f64 = 0.1;

/// Exchanges before which the window's answer is taken as it is.
const WARMUP: usize = 4;

impl Default for ClockEstimator {
    fn default() -> Self {
        Self::new(32)
    }
}

impl ClockEstimator {
    /// Keeps the last `cap` exchanges (32 at 10 Hz is about 3 s).
    #[must_use]
    pub fn new(cap: usize) -> Self {
        Self {
            window: Vec::with_capacity(cap.max(1)),
            cap: cap.max(1),
            cursor: 0,
            smoothed: None,
        }
    }

    /// Records one exchange: `t1` sent here, `t2` received there, `t3`
    /// answered there, `t4` received here, each in its own clock (any
    /// unit, the same on both sides). One whose stamps cannot be right (a
    /// negative round trip) is ignored.
    pub fn record(&mut self, t1: f64, t2: f64, t3: f64, t4: f64) {
        let round_trip = (t4 - t1) - (t3 - t2);
        if !round_trip.is_finite() || round_trip < 0.0 {
            return;
        }
        let exchange = Exchange {
            offset: f64::midpoint(t2 - t1, t3 - t4),
            round_trip,
        };
        if self.window.len() < self.cap {
            self.window.push(exchange);
        } else if let Some(slot) = self.window.get_mut(self.cursor) {
            *slot = exchange;
        }
        self.cursor = self
            .cursor
            .saturating_add(1)
            .checked_rem(self.cap)
            .unwrap_or(0);
        if let Some(raw) = self.window_offset() {
            self.smoothed = Some(match self.smoothed {
                Some(was) if self.window.len() > WARMUP => (raw - was).mul_add(SMOOTHING, was),
                _ => raw,
            });
        }
    }

    /// The offset to the other clock (there − here), smoothed; `None`
    /// before the first exchange.
    #[must_use]
    pub const fn offset(&self) -> Option<f64> {
        self.smoothed
    }

    /// The window's own answer: the mean offset of its fastest quarter.
    #[must_use]
    pub fn window_offset(&self) -> Option<f64> {
        if self.window.is_empty() {
            return None;
        }
        let mut by_speed = self.window.clone();
        by_speed.sort_by(|a, b| a.round_trip.total_cmp(&b.round_trip));
        let keep = (by_speed.len() / 4).max(1);
        let fastest = by_speed.get(..keep)?;
        let n = f64::from(u32::try_from(fastest.len()).unwrap_or(u32::MAX));
        Some(fastest.iter().map(|e| e.offset).sum::<f64>() / n)
    }

    /// The fastest round trip in the window: half of it bounds the
    /// estimate's error.
    #[must_use]
    pub fn round_trip(&self) -> Option<f64> {
        self.window
            .iter()
            .map(|e| e.round_trip)
            .min_by(f64::total_cmp)
    }
}

/// A peer's estimate of its presence host's clock, in milliseconds of
/// [`crate::presence::clock_ms`]. Cloning shares it.
#[derive(Clone, Debug, Default)]
pub struct SharedClock {
    estimator: Arc<Mutex<ClockEstimator>>,
}

impl SharedClock {
    fn with<R>(&self, f: impl FnOnce(&mut ClockEstimator) -> R) -> R {
        f(&mut self
            .estimator
            .lock()
            .unwrap_or_else(PoisonError::into_inner))
    }

    /// Records an exchange with the host, all in milliseconds:
    /// `sent` and `received` on this clock, `there` the host's answer.
    pub fn record(&self, sent: f64, there: f64, received: f64) {
        self.with(|e| e.record(sent, there, there, received));
    }

    /// The host's clock minus this one, milliseconds; `None` before the
    /// first exchange (or when the host serves no clock).
    #[must_use]
    pub fn offset_ms(&self) -> Option<f64> {
        self.with(|e| e.offset())
    }

    /// The fastest recent round trip to the host, milliseconds.
    #[must_use]
    pub fn round_trip_ms(&self) -> Option<f64> {
        self.with(|e| e.round_trip())
    }

    /// The host's clock now, milliseconds; `None` before the first
    /// exchange.
    #[must_use]
    #[cfg(feature = "vox")]
    pub fn now_ms(&self) -> Option<f64> {
        self.offset_ms().map(|o| crate::presence::clock_ms() + o)
    }
}
