//! `APP_FRAMES`: the window's frames, as the app drew them (#839).
//!
//! Two measures, one action:
//!
//! - **The meter**, always on: each redraw of the main window is timed
//!   where the window's thread hands it to Blitz ([`crate::native`]'s
//!   `RedrawRequested`), its work on that thread (styles, layout, paint,
//!   the GPU's submit and the present) and the time from one frame's
//!   start to the next, the last [`KEPT`] of each ([`record`],
//!   [`report`], [`reset`]).
//! - **A recording** ([`start`], [`stop`]): for a few seconds, every
//!   frame any window draws is a [`FrameRecord`]: when it began and
//!   ended, how long the scene's commands took to build (Blitz's
//!   `LAST_ENCODE_MICROS`, "draw") and how long the renderer took to
//!   rasterise, composite and present them (`LAST_PRESENT_MICROS`,
//!   "render", waiting for a swapchain image included), the display's
//!   refresh interval, the window's size, and what it shows that is new:
//!   the first input the window had since its last frame, and the first
//!   runtime view batch that arrived and was applied since then
//!   ([`batch_arrived`], [`batch_applied`]). [`Recorded::of`] sums it
//!   up: presented fps, refreshes dropped, the frame-time distribution,
//!   input to frame. A recording may feed the main window input of its
//!   own ([`Drive`]) through its event handler, so a run needs no OS
//!   input.
//!
//! `APP_FRAMES {reset?}` answers the meter; with `seconds` it records
//! first and writes the frames to a file (feature `native`,
//! [`register`]). Off, each recording mark is one atomic load.
//!
//! A frame "ends" when the renderer returned from presenting it, which
//! is when the swapchain took the image, not when the display showed it
//! (one refresh later or so, the same for a linked view and a runtime
//! one).

// Times in microseconds and frame counts: far from any bound, and the
// percentiles index sorted, non-empty slices.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::suboptimal_flops
)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// At most this many frames are kept a recording (about 70 s at 240 Hz).
pub const MAX_FRAMES: usize = 1 << 14;

/// A gap between two frames longer than this is the window resting, not
/// frames the display missed.
pub const IDLE_GAP: Duration = Duration::from_millis(250);

/// How many frames the meter keeps: a minute at 60 Hz, 8.5 s at 240.
pub const KEPT: usize = 2048;

/// The main window's frames so far (the meter).
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

fn with_log<T>(f: impl FnOnce(&mut Log) -> T) -> T {
    let mut log = LOG.lock().unwrap_or_else(PoisonError::into_inner);
    f(log.get_or_insert_with(Log::default))
}

/// One frame of the main window: it started at `start` and its work on
/// the window's thread ended at `end`.
pub fn record(start: Instant, end: Instant) {
    with_log(|log| {
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

/// Forgets every frame the meter kept.
pub fn reset() {
    with_log(|log| *log = Log::default());
}

/// p50, p99 and max of some durations, in milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, facet::Facet)]
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

/// What `APP_FRAMES` answers: the meter, and a recording's summary when
/// it made one.
#[derive(Debug, Clone, Default, PartialEq, facet::Facet)]
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
    /// The recording asked for (`seconds`), summed up.
    #[facet(default)]
    pub recorded: Option<Recorded>,
    /// Where its frames were written.
    #[facet(default)]
    pub path: Option<String>,
    /// Of the window's last second's frames, how many asked for the next
    /// themselves (Blitz's `frame_stats`): the widget layer set still
    /// changing, a document animation, a widget still moving.
    #[facet(default)]
    pub again_unsettled: u32,
    #[facet(default)]
    pub again_animating: u32,
    #[facet(default)]
    pub again_widgets: u32,
}

/// The meter now.
#[must_use]
pub fn report() -> Report {
    with_log(|log| {
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
            ..Report::default()
        }
    })
}

/// The main window's display refresh rate as last seen, Hz ×1000 (0: not
/// known): a drive's default rate.
static DISPLAY_MHZ: AtomicU64 = AtomicU64::new(0);

/// The main window's display refreshes `hz` times a second.
pub fn set_display_hz(hz: f64) {
    if hz.is_finite() && hz > 1.0 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::as_conversions)]
        DISPLAY_MHZ.store((hz * 1000.0) as u64, Ordering::Relaxed);
    }
}

/// The main window's display refresh rate, if known.
#[must_use]
pub fn display_hz() -> Option<f64> {
    #[allow(clippy::cast_precision_loss, clippy::as_conversions)]
    match DISPLAY_MHZ.load(Ordering::Relaxed) {
        0 => None,
        mhz => Some(mhz as f64 / 1000.0),
    }
}

static ON: AtomicBool = AtomicBool::new(false);
/// The first batch to arrive and to be applied since the last frame drawn
/// (µs since the recording's start, +1 so 0 = none), and how many.
static BATCH_ARRIVED: AtomicU64 = AtomicU64::new(0);
static BATCH_APPLIED: AtomicU64 = AtomicU64::new(0);
static BATCHES: AtomicU64 = AtomicU64::new(0);

struct Recording {
    began: Instant,
    frames: Vec<FrameRecord>,
    /// Per window: the first input since its last frame (µs +1), how many.
    inputs: Vec<(u64, u64, u32)>,
    /// Per window: the display's refresh interval, µs.
    refresh: Vec<(u64, u32)>,
    /// Per window: its size, pixels.
    sizes: Vec<(u64, (u32, u32))>,
    dropped_records: u64,
}

fn recording() -> &'static Mutex<Option<Recording>> {
    static R: OnceLock<Mutex<Option<Recording>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(None))
}

fn with<T>(f: impl FnOnce(&mut Recording) -> T) -> Option<T> {
    let mut r = recording().lock().unwrap_or_else(PoisonError::into_inner);
    r.as_mut().map(f)
}

/// One frame a window drew.
#[derive(Debug, Clone, Copy, PartialEq, Eq, facet::Facet)]
pub struct FrameRecord {
    /// The window (its platform id).
    pub window: u64,
    /// When it began and ended, µs from the recording's start.
    pub began_us: u64,
    pub ended_us: u64,
    /// Its scene's commands built (Blitz's paint pass), µs.
    pub draw_us: u32,
    /// Rasterised, composited and presented, µs: the renderer's whole
    /// call, waiting for a free swapchain image included.
    pub render_us: u32,
    /// The display's refresh interval, µs (0: not known).
    pub refresh_us: u32,
    /// The first input the window had since its last frame, µs from the
    /// start, and how many (0: none).
    pub input_us: Option<u64>,
    pub inputs: u32,
    /// The first view batch to arrive since the last frame any window
    /// drew, and to be applied, µs from the start; how many were applied.
    pub batch_arrived_us: Option<u64>,
    pub batch_applied_us: Option<u64>,
    pub batches: u32,
}

/// Starts a recording (dropping one under way). `false` if one was
/// under way.
pub fn start() -> bool {
    let was = recording()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .replace(Recording {
        began: Instant::now(),
        frames: Vec::with_capacity(4096),
        inputs: Vec::new(),
        refresh: Vec::new(),
        sizes: Vec::new(),
        dropped_records: 0,
    })
        .is_some();
    BATCH_ARRIVED.store(0, Ordering::Relaxed);
    BATCH_APPLIED.store(0, Ordering::Relaxed);
    BATCHES.store(0, Ordering::Relaxed);
    ON.store(true, Ordering::Release);
    !was
}

/// Whether a recording is under way.
#[must_use]
pub fn recording_now() -> bool {
    ON.load(Ordering::Acquire)
}

/// Ends the recording: its frames, in the order drawn, and how many were
/// past [`MAX_FRAMES`] and not kept.
#[must_use = "the recording's frames"]
pub fn stop() -> (Vec<FrameRecord>, u64) {
    let (frames, not_kept, _) = stop_with_sizes();
    (frames, not_kept)
}

/// [`stop`], with each window's size in pixels.
pub fn stop_with_sizes() -> (Vec<FrameRecord>, u64, Vec<(u64, (u32, u32))>) {
    ON.store(false, Ordering::Release);
    let mut r = recording().lock().unwrap_or_else(PoisonError::into_inner);
    r.take()
        .map(|r| (r.frames, r.dropped_records, r.sizes))
        .unwrap_or_default()
}

fn since(began: Instant, at: Instant) -> u64 {
    u64::try_from(at.saturating_duration_since(began).as_micros()).unwrap_or(u64::MAX)
}

fn stamp(cell: &AtomicU64) {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    let now = Instant::now();
    let Some(us) = with(|r| since(r.began, now)) else {
        return;
    };
    let _ = cell.compare_exchange(0, us.saturating_add(1), Ordering::Relaxed, Ordering::Relaxed);
}

/// A runtime view's batch (a frame from its process) arrived.
pub fn batch_arrived() {
    stamp(&BATCH_ARRIVED);
}

/// A runtime view's batch that changes something was applied: the next
/// frame shows it.
pub fn batch_applied() {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    stamp(&BATCH_APPLIED);
    BATCHES.fetch_add(1, Ordering::Relaxed);
}

/// Window `window` had input (a pointer, a wheel, a key, a touch).
pub fn input(window: u64) {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    let now = Instant::now();
    with(|r| {
        let us = since(r.began, now).saturating_add(1);
        match r.inputs.iter_mut().find(|(w, _, _)| *w == window) {
            Some((_, first, n)) => {
                if *first == 0 {
                    *first = us;
                }
                *n = n.saturating_add(1);
            }
            None => r.inputs.push((window, us, 1)),
        }
    });
}

/// Whether the recording knows window `window`'s refresh (`true` with
/// no recording: nothing to learn).
#[must_use]
pub fn knows_refresh(window: u64) -> bool {
    with(|r| r.refresh.iter().any(|(w, _)| *w == window)).unwrap_or(true)
}

/// Window `window`'s display refreshes every `interval`.
pub fn refresh(window: u64, interval: Duration) {
    with(|r| {
        let us = u32::try_from(interval.as_micros()).unwrap_or(u32::MAX);
        match r.refresh.iter_mut().find(|(w, _)| *w == window) {
            Some((_, at)) => *at = us,
            None => r.refresh.push((window, us)),
        }
    });
}

/// Window `window` is `width` × `height` pixels.
pub fn size(window: u64, width: u32, height: u32) {
    with(|r| match r.sizes.iter_mut().find(|(w, _)| *w == window) {
        Some((_, at)) => *at = (width, height),
        None => r.sizes.push((window, (width, height))),
    });
}

/// Window `window` drew a frame from `began` to `ended`, its commands
/// built in `draw` and rendered and presented in `render`.
pub fn frame(window: u64, began: Instant, ended: Instant, draw: Duration, render: Duration) {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    let arrived = BATCH_ARRIVED.swap(0, Ordering::Relaxed);
    let applied = BATCH_APPLIED.swap(0, Ordering::Relaxed);
    let batches = BATCHES.swap(0, Ordering::Relaxed);
    let at = |v: u64| (v > 0).then(|| v - 1);
    with(|r| {
        let (input, inputs) = r
            .inputs
            .iter_mut()
            .find(|(w, _, _)| *w == window)
            .map_or((0, 0), |(_, first, n)| {
                (std::mem::take(first), std::mem::take(n))
            });
        if r.frames.len() >= MAX_FRAMES {
            r.dropped_records = r.dropped_records.saturating_add(1);
            return;
        }
        let refresh_us = r
            .refresh
            .iter()
            .find(|(w, _)| *w == window)
            .map_or(0, |(_, us)| *us);
        let micros = |d: Duration| u32::try_from(d.as_micros()).unwrap_or(u32::MAX);
        r.frames.push(FrameRecord {
            window,
            began_us: since(r.began, began),
            ended_us: since(r.began, ended),
            draw_us: micros(draw),
            render_us: micros(render),
            refresh_us,
            input_us: at(input),
            inputs,
            batch_arrived_us: at(arrived),
            batch_applied_us: at(applied),
            batches: u32::try_from(batches).unwrap_or(u32::MAX),
        });
    });
}

/// A distribution of times, ms.
#[derive(Debug, Clone, Copy, Default, PartialEq, facet::Facet)]
pub struct Times {
    pub n: u32,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

impl Times {
    /// Of `values`, µs.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::as_conversions
    )]
    pub fn of(mut values: Vec<u64>) -> Self {
        if values.is_empty() {
            return Self::default();
        }
        values.sort_unstable();
        let n = values.len();
        let at = |q: f64| {
            let i = ((q * n as f64).ceil() as usize).clamp(1, n) - 1;
            values[i] as f64 / 1000.0
        };
        Self {
            n: u32::try_from(n).unwrap_or(u32::MAX),
            p50: at(0.50),
            p95: at(0.95),
            p99: at(0.99),
            max: values[n - 1] as f64 / 1000.0,
        }
    }
}

/// A recording summed up: one window's (the one that drew most).
#[derive(Debug, Clone, Default, PartialEq, facet::Facet)]
pub struct Recorded {
    /// What was recorded (the caller's label).
    pub label: String,
    /// The window reported on, and how many windows drew.
    pub window: u64,
    pub windows: u32,
    /// The display's refresh rate, Hz (0: not known).
    pub refresh_hz: f64,
    /// The window's size, pixels (0: not known).
    pub width: u32,
    pub height: u32,
    /// Seconds from its first frame's end to its last's.
    pub seconds: f64,
    pub frames: u32,
    /// Frames presented a second.
    pub presented_fps: f64,
    /// Between one frame's end and the next's, while not resting
    /// ([`IDLE_GAP`]).
    pub interval: Times,
    /// Refreshes with no new frame between two frames less than
    /// [`IDLE_GAP`] apart: frames dropped, when something moves every
    /// frame (playback, continuous input).
    pub dropped: u32,
    /// The window's work a frame, start to end.
    pub work: Times,
    pub draw: Times,
    pub render: Times,
    /// From the first input a frame shows to its end.
    pub input_to_frame: Times,
    /// From the first batch a frame shows arriving, and being applied, to
    /// its end.
    pub batch_to_frame: Times,
    pub applied_to_frame: Times,
    /// Frames that showed input, inputs in all; frames that showed a
    /// batch, batches in all.
    pub frames_with_input: u32,
    pub inputs: u32,
    pub frames_with_batch: u32,
    pub batches: u32,
    /// Records past [`MAX_FRAMES`], not kept.
    pub not_kept: u64,
}

impl Recorded {
    /// The report of `frames` (a recording, in order).
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::as_conversions
    )]
    pub fn of(label: &str, frames: &[FrameRecord], not_kept: u64) -> Self {
        let mut windows: Vec<(u64, u32)> = Vec::new();
        for f in frames {
            match windows.iter_mut().find(|(w, _)| *w == f.window) {
                Some((_, n)) => *n += 1,
                None => windows.push((f.window, 1)),
            }
        }
        let Some(&(window, _)) = windows.iter().max_by_key(|(_, n)| *n) else {
            return Self {
                label: label.to_owned(),
                not_kept,
                ..Self::default()
            };
        };
        let mine: Vec<&FrameRecord> = frames.iter().filter(|f| f.window == window).collect();
        let refresh_us = mine
            .iter()
            .rev()
            .map(|f| f.refresh_us)
            .find(|r| *r > 0)
            .unwrap_or(0);
        let idle = IDLE_GAP.as_micros() as u64;
        let mut intervals = Vec::new();
        let mut dropped = 0_u32;
        for pair in mine.windows(2) {
            let gap = pair[1].ended_us.saturating_sub(pair[0].ended_us);
            if gap >= idle {
                continue;
            }
            intervals.push(gap);
            if refresh_us > 0 {
                let refreshes = (gap as f64 / f64::from(refresh_us)).round() as u64;
                dropped = dropped.saturating_add(u32::try_from(refreshes.saturating_sub(1)).unwrap_or(u32::MAX));
            }
        }
        let first = mine.first().map_or(0, |f| f.ended_us);
        let last = mine.last().map_or(0, |f| f.ended_us);
        let seconds = last.saturating_sub(first) as f64 / 1e6;
        let n = mine.len();
        let after = |at: Option<u64>, end: u64| at.map(|a| end.saturating_sub(a));
        Self {
            label: label.to_owned(),
            window,
            windows: u32::try_from(windows.len()).unwrap_or(u32::MAX),
            refresh_hz: if refresh_us > 0 {
                1e6 / f64::from(refresh_us)
            } else {
                0.0
            },
            width: 0,
            height: 0,
            seconds,
            frames: u32::try_from(n).unwrap_or(u32::MAX),
            presented_fps: if seconds > 0.0 {
                (n.saturating_sub(1)) as f64 / seconds
            } else {
                0.0
            },
            interval: Times::of(intervals),
            dropped,
            work: Times::of(
                mine.iter()
                    .map(|f| f.ended_us.saturating_sub(f.began_us))
                    .collect(),
            ),
            draw: Times::of(mine.iter().map(|f| u64::from(f.draw_us)).collect()),
            render: Times::of(mine.iter().map(|f| u64::from(f.render_us)).collect()),
            input_to_frame: Times::of(
                mine.iter()
                    .filter_map(|f| after(f.input_us, f.ended_us))
                    .collect(),
            ),
            batch_to_frame: Times::of(
                mine.iter()
                    .filter_map(|f| after(f.batch_arrived_us, f.ended_us))
                    .collect(),
            ),
            applied_to_frame: Times::of(
                mine.iter()
                    .filter_map(|f| after(f.batch_applied_us, f.ended_us))
                    .collect(),
            ),
            frames_with_input: count(&mine, |f| f.inputs > 0),
            inputs: mine.iter().map(|f| f.inputs).sum(),
            frames_with_batch: count(&mine, |f| f.batches > 0),
            batches: mine.iter().map(|f| f.batches).sum(),
            not_kept,
        }
    }
}

fn count(frames: &[&FrameRecord], test: impl Fn(&FrameRecord) -> bool) -> u32 {
    u32::try_from(frames.iter().filter(|f| test(f)).count()).unwrap_or(u32::MAX)
}

/// Input a recording feeds its window, through the same path a person's
/// takes (the window's event handler), at `rate` events a second.
#[derive(Debug, Clone, PartialEq, facet::Facet)]
#[repr(u8)]
pub enum Drive {
    /// Nothing: record what the window does (playback, a meter).
    Still,
    /// The wheel, one notch an event, at `at`, `dy` lines (+ down),
    /// turning round every `turn` notches so the view stays in range.
    Wheel { at: (f64, f64), dy: f64, turn: u32 },
    /// A press at `from`, then moves along a back-and-forth line of
    /// `reach` points (x, y), and a release at the end: a pan, a drag.
    Drag {
        from: (f64, f64),
        reach: (f64, f64),
        /// Seconds there and back.
        period: f64,
    },
}

/// One input the driver makes now, in window points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Driven {
    Move(f64, f64),
    Press(f64, f64),
    Release(f64, f64),
    Wheel(f64, f64, f64),
}

/// A [`Drive`] under way: what it makes at `t` seconds.
#[derive(Debug, Clone)]
pub struct Driving {
    pub drive: Drive,
    pub rate: f64,
    pub seconds: f64,
    sent: u64,
    pressed: bool,
    done: bool,
}

impl Driving {
    #[must_use]
    pub const fn new(drive: Drive, rate: f64, seconds: f64) -> Self {
        Self {
            drive,
            rate,
            seconds,
            sent: 0,
            pressed: false,
            done: false,
        }
    }

    /// Whether it has made its last input.
    #[must_use]
    pub const fn done(&self) -> bool {
        self.done
    }

    /// The inputs due by `t` seconds from its start, in order.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::as_conversions
    )]
    pub fn due(&mut self, t: f64) -> Vec<Driven> {
        let mut out = Vec::new();
        if self.done {
            return out;
        }
        let ending = t >= self.seconds;
        let due = ((t.min(self.seconds) * self.rate).floor() as u64).saturating_add(1);
        // A long stall (the window busy) catches up with the latest only:
        // a mouse reports where it is, not where it was.
        if due > self.sent.saturating_add(4) {
            self.sent = due - 1;
        }
        while self.sent < due {
            let at = self.sent as f64 / self.rate;
            self.sent += 1;
            match &self.drive {
                Drive::Still => {}
                Drive::Wheel { at: (x, y), dy, turn } => {
                    // The pointer goes there once; a wheel or a trackpad
                    // scrolls with no move between its events.
                    if !self.pressed {
                        self.pressed = true;
                        out.push(Driven::Move(*x, *y));
                    }
                    let turn = u64::from((*turn).max(1));
                    let leg = (self.sent - 1) / turn;
                    let sign = if leg.is_multiple_of(2) { 1.0 } else { -1.0 };
                    out.push(Driven::Wheel(*x, *y, dy * sign));
                }
                Drive::Drag {
                    from: (x, y),
                    reach: (rx, ry),
                    period,
                } => {
                    if !self.pressed {
                        self.pressed = true;
                        out.push(Driven::Move(*x, *y));
                        out.push(Driven::Press(*x, *y));
                        continue;
                    }
                    let period = period.max(0.05);
                    let phase = (at / period).fract();
                    // 0 → 1 → 0 over a period.
                    let k = 1.0 - (2.0 * phase - 1.0).abs();
                    out.push(Driven::Move(x + rx * k, y + ry * k));
                }
            }
        }
        if ending {
            self.done = true;
            if let Drive::Drag { from: (x, y), .. } = &self.drive
                && self.pressed
            {
                out.push(Driven::Release(*x, *y));
            }
        }
        out
    }
}

/// The drive the window is fed now, if any: set by `APP_FRAMES`, read by
/// the launcher on each turn of its loop.
static DRIVING: Mutex<Option<(Instant, Driving)>> = Mutex::new(None);
/// What wakes the launcher's loop (its proxy), set at launch.
static WAKE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// The launcher's loop is woken by `wake` (once, at launch).
pub fn set_waker(wake: impl Fn() + Send + Sync + 'static) {
    let _ = WAKE.set(Box::new(wake));
}

/// Feeds the window `driving` from now, waking its loop at its rate
/// until it is done.
pub fn drive(driving: Driving) {
    let interval = Duration::from_secs_f64(1.0 / driving.rate.max(1.0));
    let seconds = driving.seconds;
    *DRIVING.lock().unwrap_or_else(PoisonError::into_inner) = Some((Instant::now(), driving));
    let _ = std::thread::Builder::new()
        .name("architect-frames-drive".into())
        .spawn(move || {
            let end = Instant::now() + Duration::from_secs_f64(seconds + 0.05);
            while Instant::now() < end {
                if let Some(wake) = WAKE.get() {
                    wake();
                }
                std::thread::sleep(interval);
            }
            if let Some(wake) = WAKE.get() {
                wake();
            }
        });
}

/// The inputs due now (the launcher's loop calls this each turn).
pub fn driven_now() -> Vec<Driven> {
    let mut d = DRIVING.lock().unwrap_or_else(PoisonError::into_inner);
    let Some((began, driving)) = d.as_mut() else {
        return Vec::new();
    };
    let out = driving.due(began.elapsed().as_secs_f64());
    if driving.done() {
        *d = None;
    }
    out
}

#[cfg(feature = "native")]
pub use action::{APP_FRAMES, FramesArgs, register};

#[cfg(feature = "native")]
mod action {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use architect::action::{ActionCaller, ActionList, ActionMeta, Effect};
    use facet::Facet;

    use super::{Drive, Driving, FrameRecord, Recorded, Report};

    /// `APP_FRAMES`' id.
    pub const APP_FRAMES: &str = "APP_FRAMES";

    /// `APP_FRAMES`' arguments.
    #[derive(Debug, Clone, Default, Facet)]
    pub struct FramesArgs {
        /// Forget the meter's frames (after answering them).
        #[facet(default)]
        pub reset: bool,
        /// Record every frame for this long first, seconds (at most 60).
        #[facet(default)]
        pub seconds: Option<f64>,
        /// What it records, said in its summary.
        #[facet(default)]
        pub label: Option<String>,
        /// Where to write the recording (JSON): an absolute path ending in
        /// `.json`. `<temp>/app-frames.json` unless given.
        #[facet(default)]
        pub path: Option<String>,
        /// Input to feed the main window while recording (default none).
        #[facet(default)]
        pub drive: Option<Drive>,
        /// The drive's events a second (default the display's refresh).
        #[facet(default)]
        pub rate: Option<f64>,
    }

    /// The file a recording writes: its summary and every frame.
    #[derive(Facet)]
    struct File {
        report: Recorded,
        frames: Vec<FrameRecord>,
    }

    static META: ActionMeta = ActionMeta {
        id: APP_FRAMES,
        trait_name: "",
        method_name: "",
        display_name: "The window's frame times",
        description: "The main window's last frames: each one's work on the window's thread (styles, layout, \
                      paint, the GPU's submit and present) and the time from one to the next (what reached the \
                      screen), as p50, p99 and max in milliseconds, the count and the rate; `reset` forgets them \
                      after answering. With `seconds`, first records every frame for that long (each one's draw \
                      and render times, the display's refresh, the input and extension batches it shows), feeding \
                      the window `drive` input (`Wheel`, `Drag`) at `rate` a second meanwhile, writes the frames \
                      as JSON to `path` and answers their summary as `recorded`: presented fps, dropped \
                      refreshes, frame-time and input-to-frame percentiles.",
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
        list.register_async_with(
            &META,
            Effect::ReadOnly,
            move |_: &ActionCaller, args: FramesArgs| async move { run(args).await },
        )
    }

    async fn run(args: FramesArgs) -> Result<Report, String> {
        let recorded = match args.seconds {
            Some(seconds) => Some(record_for(seconds, &args).await?),
            None => None,
        };
        let mut report = super::report();
        if args.reset {
            super::reset();
        }
        if let Some((recorded, path)) = recorded {
            report.recorded = Some(recorded);
            report.path = Some(path);
        }
        let blitz = blitz_shell::frame_stats::snapshot();
        report.again_unsettled = blitz.again_unsettled;
        report.again_animating = blitz.again_animating;
        report.again_widgets = blitz.again_widgets;
        Ok(report)
    }

    async fn record_for(seconds: f64, args: &FramesArgs) -> Result<(Recorded, String), String> {
        if !(seconds.is_finite() && seconds > 0.0 && seconds <= 60.0) {
            return Err(format!("seconds: {seconds} is not between 0 and 60"));
        }
        let path = args
            .path
            .clone()
            .map_or_else(|| std::env::temp_dir().join("app-frames.json"), PathBuf::from);
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) {
            return Err(format!("{}: the recording's path ends in .json", path.display()));
        }
        let path = std::path::absolute(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if super::recording_now() {
            return Err("a recording is under way".to_owned());
        }
        super::start();
        if let Some(drive) = args.drive.clone().filter(|d| *d != Drive::Still) {
            let rate = args.rate.or_else(super::display_hz).unwrap_or(60.0);
            if !(rate.is_finite() && (1.0..=2000.0).contains(&rate)) {
                let _ = super::stop();
                return Err(format!("rate: {rate} is not between 1 and 2000"));
            }
            super::drive(Driving::new(drive, rate, seconds));
        }
        futures_timer::Delay::new(Duration::from_secs_f64(seconds)).await;
        // The drive's last input (a release) lands in a frame of its own.
        futures_timer::Delay::new(Duration::from_millis(50)).await;
        let (frames, not_kept, sizes) = super::stop_with_sizes();
        let mut recorded = Recorded::of(args.label.as_deref().unwrap_or(""), &frames, not_kept);
        if let Some((_, (w, h))) = sizes.iter().find(|(w, _)| *w == recorded.window) {
            recorded.width = *w;
            recorded.height = *h;
        }
        let json = facet_json::to_string(&File {
            report: recorded.clone(),
            frames,
        })
        .map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok((recorded, path.display().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(window: u64, ended_ms: u64) -> FrameRecord {
        FrameRecord {
            window,
            began_us: ended_ms * 1000 - 1500,
            ended_us: ended_ms * 1000,
            draw_us: 300,
            render_us: 1000,
            refresh_us: 8333,
            input_us: None,
            inputs: 0,
            batch_arrived_us: None,
            batch_applied_us: None,
            batches: 0,
        }
    }

    #[test]
    fn a_frame_every_refresh_drops_none() {
        let frames: Vec<_> = (1..=121).map(|i| at(1, i * 25 / 3)).collect();
        let r = Recorded::of("", &frames, 0);
        assert_eq!(r.frames, 121);
        assert_eq!(r.dropped, 0);
        assert!((r.refresh_hz - 120.0).abs() < 0.1, "{}", r.refresh_hz);
        assert!((r.presented_fps - 120.0).abs() < 2.0, "{}", r.presented_fps);
    }

    #[test]
    fn a_skipped_refresh_is_a_dropped_frame_and_a_rest_is_not() {
        let mut frames = vec![at(1, 10), at(1, 18), at(1, 35), at(1, 43)];
        // A rest: half a second with nothing.
        frames.push(at(1, 543));
        frames.push(at(1, 551));
        let r = Recorded::of("", &frames, 0);
        assert_eq!(r.dropped, 1);
        assert_eq!(r.interval.n, 4);
    }

    #[test]
    fn the_window_that_drew_most_is_reported() {
        let frames = vec![at(2, 10), at(1, 11), at(2, 18), at(2, 26)];
        let r = Recorded::of("", &frames, 0);
        assert_eq!(r.window, 2);
        assert_eq!(r.windows, 2);
        assert_eq!(r.frames, 3);
    }

    #[test]
    fn marks_land_in_the_next_frame_of_their_window() {
        let _guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        start();
        input(7);
        input(7);
        batch_arrived();
        batch_applied();
        let now = Instant::now();
        frame(7, now, now, Duration::ZERO, Duration::ZERO);
        frame(7, now, now, Duration::ZERO, Duration::ZERO);
        let (frames, _) = stop();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].inputs, 2);
        assert!(frames[0].input_us.is_some());
        assert_eq!(frames[0].batches, 1);
        assert!(frames[0].batch_arrived_us.is_some() && frames[0].batch_applied_us.is_some());
        assert_eq!(frames[1].inputs, 0);
        assert_eq!(frames[1].batches, 0);
        // Off, nothing is kept.
        input(7);
        frame(7, now, now, Duration::ZERO, Duration::ZERO);
        assert!(stop().0.is_empty());
    }

    #[test]
    fn a_drag_presses_moves_and_lets_go() {
        let mut d = Driving::new(
            Drive::Drag {
                from: (10.0, 10.0),
                reach: (100.0, 0.0),
                period: 1.0,
            },
            100.0,
            1.0,
        );
        let first = d.due(0.0);
        assert_eq!(first, vec![Driven::Move(10.0, 10.0), Driven::Press(10.0, 10.0)]);
        let mid = d.due(0.5);
        assert!(matches!(mid.last(), Some(Driven::Move(x, _)) if (*x - 110.0).abs() < 2.5));
        let end = d.due(1.0);
        assert_eq!(end.last(), Some(&Driven::Release(10.0, 10.0)));
        assert!(d.done());
        assert!(d.due(2.0).is_empty());
    }

    #[test]
    fn a_wheel_turns_round() {
        let mut d = Driving::new(
            Drive::Wheel {
                at: (0.0, 0.0),
                dy: 1.0,
                turn: 2,
            },
            10.0,
            0.35,
        );
        let all = d.due(0.35);
        assert_eq!(all.first(), Some(&Driven::Move(0.0, 0.0)));
        let dys: Vec<f64> = all
            .iter()
            .filter_map(|e| match e {
                Driven::Wheel(_, _, dy) => Some(*dy),
                _ => None,
            })
            .collect();
        assert_eq!(dys, vec![1.0, 1.0, -1.0, -1.0]);
    }

    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    #[allow(clippy::float_cmp)]
    fn the_meter_times_and_spreads_frames() {
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
