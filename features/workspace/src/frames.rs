//! The window's frames, one record each (#839): what a frame-time report
//! is made of, recorded by the window itself.
//!
//! While a recording runs ([`start`]), every frame a window draws is a
//! [`FrameRecord`]: when it began and ended, how long the scene's
//! commands took to build (Blitz's `LAST_ENCODE_MICROS`, "draw") and how
//! long the renderer took to rasterise, composite and present them
//! (`LAST_PRESENT_MICROS`, "render"), the display's refresh interval, and
//! what it shows that is new: the first input the window had since its
//! last frame, and the first view batch that arrived and was applied
//! since then (a runtime extension's frame, [`batch_arrived`],
//! [`batch_applied`]). [`stop`] ends it and [`Report::of`] sums it up:
//! the frame-time distribution, the presented frame rate, the frames the
//! display waited for, input to frame.
//!
//! The launcher records ([`crate::native`]); `APP_FRAMES` (feature
//! `native`, [`use_app_frames`]) runs a recording for a few seconds,
//! optionally driving the window with input of its own ([`Drive`]), and
//! writes the report to a file. Off, each mark is one atomic load.
//!
//! The times are the app's: a frame "ends" when the renderer returned
//! from presenting it, which is when the swapchain took the image, not
//! when the display showed it (one refresh later or so, the same for a
//! linked view and a runtime one).

// Times in microseconds and frame counts: far from any bound, and the
// percentiles index sorted, non-empty slices.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::suboptimal_flops
)]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// At most this many frames are kept a recording (about 70 s at 240 Hz).
pub const MAX_FRAMES: usize = 1 << 14;

/// A gap between two frames longer than this is the window resting, not
/// frames the display missed.
pub const IDLE_GAP: Duration = Duration::from_millis(250);

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
pub struct Spread {
    pub n: u32,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

impl Spread {
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
pub struct Report {
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
    pub interval: Spread,
    /// Refreshes with no new frame between two frames less than
    /// [`IDLE_GAP`] apart: frames dropped, when something moves every
    /// frame (playback, continuous input).
    pub dropped: u32,
    /// The window's work a frame, start to end.
    pub work: Spread,
    pub draw: Spread,
    pub render: Spread,
    /// From the first input a frame shows to its end.
    pub input_to_frame: Spread,
    /// From the first batch a frame shows arriving, and being applied, to
    /// its end.
    pub batch_to_frame: Spread,
    pub applied_to_frame: Spread,
    /// Frames that showed input, inputs in all; frames that showed a
    /// batch, batches in all.
    pub frames_with_input: u32,
    pub inputs: u32,
    pub frames_with_batch: u32,
    pub batches: u32,
    /// Records past [`MAX_FRAMES`], not kept.
    pub not_kept: u64,
}

impl Report {
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
            interval: Spread::of(intervals),
            dropped,
            work: Spread::of(
                mine.iter()
                    .map(|f| f.ended_us.saturating_sub(f.began_us))
                    .collect(),
            ),
            draw: Spread::of(mine.iter().map(|f| u64::from(f.draw_us)).collect()),
            render: Spread::of(mine.iter().map(|f| u64::from(f.render_us)).collect()),
            input_to_frame: Spread::of(
                mine.iter()
                    .filter_map(|f| after(f.input_us, f.ended_us))
                    .collect(),
            ),
            batch_to_frame: Spread::of(
                mine.iter()
                    .filter_map(|f| after(f.batch_arrived_us, f.ended_us))
                    .collect(),
            ),
            applied_to_frame: Spread::of(
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
pub use action::{APP_FRAMES, FramesArgs, FramesAnswer, use_app_frames};

#[cfg(feature = "native")]
mod action {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use architect::action::{ActionCaller, ActionList, ActionMeta, Effect};
    use dioxus::prelude::*;
    use facet::Facet;

    use super::{Drive, Driving, FrameRecord, Report};

    /// `APP_FRAMES`'s id.
    pub const APP_FRAMES: &str = "APP_FRAMES";

    /// `APP_FRAMES`'s arguments.
    #[derive(Debug, Clone, Facet)]
    pub struct FramesArgs {
        /// How long to record, seconds (default 5, at most 60).
        #[facet(default)]
        pub seconds: Option<f64>,
        /// What it records, said in the report.
        #[facet(default)]
        pub label: Option<String>,
        /// Where to write the report (JSON): an absolute path ending in
        /// `.json`. `<temp>/app-frames.json` unless given.
        #[facet(default)]
        pub path: Option<String>,
        /// Input to feed the window while recording (default none).
        #[facet(default)]
        pub drive: Option<Drive>,
        /// The drive's events a second (default the display's refresh).
        #[facet(default)]
        pub rate: Option<f64>,
    }

    /// What `APP_FRAMES` answers and writes.
    #[derive(Debug, Clone, Facet)]
    pub struct FramesAnswer {
        pub path: String,
        pub report: Report,
        /// Of the window's last second's frames, how many asked for the
        /// next themselves (Blitz's `frame_stats`): the widget layer set
        /// still changing, a document animation, a widget still moving.
        pub again_unsettled: u32,
        pub again_animating: u32,
        pub again_widgets: u32,
    }

    /// The file: the report and every frame.
    #[derive(Facet)]
    struct File {
        report: Report,
        frames: Vec<FrameRecord>,
    }

    static META: ActionMeta = ActionMeta {
        id: APP_FRAMES,
        trait_name: "",
        method_name: "",
        display_name: "Record the window's frames",
        description: "Records every frame the window draws for `seconds` (default 5): when it began and ended, its \
                      draw and render times, the display's refresh, and the input and extension batches it shows. \
                      `drive` feeds the window input meanwhile (`Wheel`, `Drag`) at `rate` a second. Writes the frames \
                      and their summary (frame-time percentiles, presented fps, dropped frames, input to frame) as \
                      JSON to `path` and answers the summary.",
        category: "App",
        group: "Agent tools",
        shortcut: "",
        toggleable: false,
        undo: false,
    };

    /// Puts `APP_FRAMES` on `list` while the calling component is mounted,
    /// in a window the launcher opened.
    pub fn use_app_frames(list: &Arc<ActionList>) {
        let window = try_use_context::<Arc<dyn dioxus_native::winit::window::Window>>();
        let registered = use_hook({
            let list = Arc::clone(list);
            move || {
                let hz = window.as_ref().and_then(|w| crate::native::refresh_hz(w.as_ref()));
                list.register_async_with(
                    &META,
                    Effect::ReadOnly,
                    move |_: &ActionCaller, args: FramesArgs| async move { run(args, hz).await },
                )
                .ok()?;
                Some(list)
            }
        });
        use_drop(move || {
            if let Some(list) = registered {
                list.unregister(APP_FRAMES);
            }
        });
    }

    async fn run(args: FramesArgs, hz: Option<f64>) -> Result<FramesAnswer, String> {
        let seconds = args.seconds.unwrap_or(5.0);
        if !(seconds.is_finite() && seconds > 0.0 && seconds <= 60.0) {
            return Err(format!("seconds: {seconds} is not between 0 and 60"));
        }
        let path = args
            .path
            .map_or_else(|| std::env::temp_dir().join("app-frames.json"), PathBuf::from);
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) {
            return Err(format!("{}: the report's path ends in .json", path.display()));
        }
        let path = std::path::absolute(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if super::recording_now() {
            return Err("a recording is under way".to_owned());
        }
        super::start();
        if let Some(drive) = args.drive.filter(|d| *d != Drive::Still) {
            let rate = args.rate.or(hz).unwrap_or(60.0);
            if !(rate.is_finite() && (1.0..=2000.0).contains(&rate)) {
                let _ = super::stop();
                return Err(format!("rate: {rate} is not between 1 and 2000"));
            }
            super::drive(Driving::new(drive, rate, seconds));
        }
        futures_timer::Delay::new(Duration::from_secs_f64(seconds)).await;
        // The drive's last input (a release) lands in a frame of its own.
        futures_timer::Delay::new(Duration::from_millis(50)).await;
        let blitz = blitz_shell::frame_stats::snapshot();
        let (frames, not_kept, sizes) = super::stop_with_sizes();
        let mut report = Report::of(args.label.as_deref().unwrap_or(""), &frames, not_kept);
        if let Some((_, (w, h))) = sizes.iter().find(|(w, _)| *w == report.window) {
            report.width = *w;
            report.height = *h;
        }
        let json = facet_json::to_string(&File {
            report: report.clone(),
            frames,
        })
        .map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(FramesAnswer {
            path: path.display().to_string(),
            report,
            again_unsettled: blitz.again_unsettled,
            again_animating: blitz.again_animating,
            again_widgets: blitz.again_widgets,
        })
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
        let r = Report::of("", &frames, 0);
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
        let r = Report::of("", &frames, 0);
        assert_eq!(r.dropped, 1);
        assert_eq!(r.interval.n, 4);
    }

    #[test]
    fn the_window_that_drew_most_is_reported() {
        let frames = vec![at(2, 10), at(1, 11), at(2, 18), at(2, 26)];
        let r = Report::of("", &frames, 0);
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
}
