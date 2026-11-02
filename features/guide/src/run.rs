//! The runner (feature `run`): a scenario played over an app, headlessly.
//!
//! The app is its root component and its Action List ([`App`]). The
//! runner opens a Blitz document of the root — a [`Desktop`] of them, one
//! per window, since a tab dragged out opens a pop-out
//! (`architect_workspace::WindowSystem`, played here by the desktop) —
//! and for each step:
//!
//! 1. resolves the step's [`Target`] to a rectangle (an element by id or
//!    `data-testid`, a workspace tab, a palette row, …: [`Resolver`]s);
//! 2. does it: runs the action on the Action List, or turns the gesture
//!    into pointer, wheel and key [`UiEvent`]s, tweened over frames, sent
//!    through each document's own event path (what a window's shell does);
//! 3. waits for `done`: one of its events on the Action List, then every
//!    check — a read-only action whose JSON result must contain `has`.
//!
//! Time is virtual: frame *n* is `n / fps` seconds. Stylo's animations
//! and transitions are resolved at that time, and the app's motions
//! (`architect_shell::motion`) run on it — a [`VirtualClock`] the runner
//! moves on each frame — so a film is the same on every run and shows a
//! motion's in-between frames. (A Dioxus task's own timers still run on
//! the wall clock; the runner waits on state for those, never on time.) A
//! [`Film`] is shown each frame kept; with none, the run is a test.

// Geometry reads plainer as `a + b * c`; its precision is a pixel's.
#![allow(clippy::suboptimal_flops)]

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use architect::action::{ActionError, ActionList};
use architect_shell::motion::VirtualClock;
use architect_workspace::{Popout, Rect, WindowId, WindowSystem, Windows};
use blitz_dom::{Document as _, DocumentConfig, FontContext};
use blitz_traits::events::{
    BlitzKeyEvent, BlitzPointerEvent, BlitzPointerId, BlitzWheelDelta, BlitzWheelEvent, KeyState,
    MouseEventButton, MouseEventButtons, PointerCoords, PointerDetails, UiEvent,
};
use blitz_traits::events::{Code, Key, Location, Modifiers};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::{Element, VirtualDom};
use dioxus_native_dom::DioxusDocument;

pub use crate::locate::Resolver;
use crate::predicate::{Failed, checks, woke};
use crate::{Do, Modifier, Scenario, Step, Target, to_json};

/// An app, as the runner opens it: its root and its Action List.
#[derive(Clone)]
pub struct App {
    /// The root component, as a window mounts it. It finds in context the
    /// Action List (`Arc<ActionList>`), the desktop's windows
    /// (`architect_workspace::Windows`) and the window's size
    /// ([`crate::ViewSize`]).
    pub root: fn() -> Element,
    /// The app's Action List: steps run actions on it, checks read it,
    /// `done` waits on its events.
    pub list: Arc<ActionList>,
    /// The faces the documents lay text out with (the app's own fonts and
    /// the system's). `None`: Blitz's default.
    pub fonts: Option<FontContext>,
    /// What turns targets into selectors, tried in order.
    pub resolvers: Vec<Resolver>,
    /// What else the root finds in context: [`App::context`].
    contexts: Vec<Rc<dyn Fn(&VirtualDom)>>,
}

impl App {
    /// The app, with the [`standard`] resolvers.
    #[must_use]
    pub fn new(root: fn() -> Element, list: Arc<ActionList>) -> Self {
        Self {
            root,
            list,
            fonts: None,
            resolvers: standard(),
            contexts: Vec::new(),
        }
    }

    /// Puts `value` in the root's context too: what the root needs beside
    /// the list (a story's root, the story it draws).
    #[must_use]
    pub fn context<T: Clone + 'static>(mut self, value: T) -> Self {
        self.contexts.push(Rc::new(move |dom: &VirtualDom| {
            dom.provide_root_context(value.clone());
        }));
        self
    }

    #[must_use]
    pub fn fonts(mut self, fonts: FontContext) -> Self {
        self.fonts = Some(fonts);
        self
    }

    /// Tries `resolver` first: an app's own kinds (`@app{kind lane, …}`).
    #[must_use]
    pub fn resolver(mut self, resolver: Resolver) -> Self {
        self.resolvers.insert(0, resolver);
        self
    }
}

/// The resolvers every app has.
///
/// `data-testid` and CSS, the workspace's tabs, panes, groups, tab bars
/// and dividers
/// (`architect_workspace::target_selector`), and the palette's rows
/// (`architect_commands::target_selector`). An element `id` is looked up
/// directly.
#[must_use]
pub fn standard() -> Vec<Resolver> {
    vec![
        crate::locate::builtin,
        architect_workspace::target_selector,
        architect_commands::target_selector,
    ]
}

/// Why a scenario stopped.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// A step did not happen: which, why, and what the failing check's
    /// action gave back.
    #[error("step {step:?}: {reason}{}", shown(actual.as_deref()))]
    Step {
        step: String,
        reason: String,
        /// The failing check's result, as JSON.
        actual: Option<String>,
    },
}

fn shown(actual: Option<&str>) -> String {
    actual.map_or_else(String::new, |a| format!("\n  it gave: {a}"))
}

/// What a run did, step by step.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub steps: Vec<StepReport>,
    /// Frames shown to the film.
    pub frames: u64,
}

#[derive(Debug, Clone)]
pub struct StepReport {
    pub id: String,
    /// Frames of the virtual clock it took.
    pub frames: u64,
}

/// Where the pointer is, for a film to draw (a headless document has no
/// cursor of its own).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pointer {
    /// Desktop points.
    pub x: f32,
    pub y: f32,
    pub down: bool,
    /// A press's ripple, `0` to `1` as it spreads; `None` when none is.
    pub ripple: Option<f32>,
}

/// What is shown each frame a run keeps.
pub trait Film {
    /// One frame: every window as it is now, and the pointer.
    fn frame(&mut self, desktop: &Desktop, pointer: Pointer);
}

/// A window's frame on the desktop: its content's top left and size,
/// desktop points.
const fn content_rect(view: &crate::ViewSize) -> Rect {
    Rect::new(view.x, view.y, to_f32(view.width), to_f32(view.height))
}

/// One window of the desktop: its document and where it is.
pub struct Window {
    pub id: WindowId,
    pub title: String,
    /// Its content, in desktop points.
    pub rect: Rect,
    pub doc: DioxusDocument,
}

/// Every window the app has open, headless: the main one and each
/// pop-out the workspace opened (through [`Windows`], which the desktop
/// is to the app).
pub struct Desktop {
    windows: Vec<Window>,
    shared: Rc<Shared>,
    fonts: Option<FontContext>,
    scale: f32,
}

#[derive(Default)]
struct Shared {
    asks: RefCell<Vec<Ask>>,
    rects: RefCell<Vec<(WindowId, Rect)>>,
    releases: RefCell<Vec<(WindowId, Rc<dyn Fn(f32, f32)>)>>,
}

enum Ask {
    Open(Box<Popout>),
    Close(WindowId),
}

/// The desktop, as the workspace's hosts see it.
struct HeadlessWindows(Rc<Shared>);

impl WindowSystem for HeadlessWindows {
    fn open(&self, popout: Popout) {
        self.0.asks.borrow_mut().push(Ask::Open(Box::new(popout)));
    }

    fn close(&self, window: &WindowId) {
        self.0.asks.borrow_mut().push(Ask::Close(window.clone()));
    }

    fn origin(&self, window: &WindowId) -> Option<(f32, f32)> {
        self.0
            .rects
            .borrow()
            .iter()
            .find(|(id, _)| id == window)
            .map(|(_, r)| (r.x, r.y))
    }

    fn window_at(&self, x: f32, y: f32) -> Option<WindowId> {
        self.0
            .rects
            .borrow()
            .iter()
            .rev()
            .find(|(_, r)| architect_workspace::contains(*r, x, y))
            .map(|(id, _)| id.clone())
    }

    fn on_release_outside(&self, window: &WindowId, release: Rc<dyn Fn(f32, f32)>) {
        self.0.releases.borrow_mut().push((window.clone(), release));
    }
}

/// Where a pop-out with no place of its own opens, from the main window's
/// top left, and its size.
const POPOUT_AT: (f32, f32, f32, f32) = (80.0, 80.0, 640.0, 480.0);

impl Desktop {
    fn open(app: &App, scenario: &Scenario) -> Self {
        let shared = Rc::new(Shared::default());
        let vdom = VirtualDom::new(app.root)
            .with_root_context(app.list.clone())
            .with_root_context(Windows::new(HeadlessWindows(shared.clone())))
            .with_root_context(scenario.view);
        for provide in &app.contexts {
            provide(&vdom);
        }
        let rect = content_rect(&scenario.view);
        let scale = scenario.view.scale;
        let doc = document(vdom, rect, scale, app.fonts.clone());
        let desk = Self {
            windows: vec![Window {
                id: WindowId::main(),
                title: scenario.title.clone(),
                rect,
                doc,
            }],
            shared,
            fonts: app.fonts.clone(),
            scale,
        };
        desk.publish_rects();
        desk
    }

    /// Every window, back to front (the main one first).
    #[must_use]
    pub fn windows(&self) -> &[Window] {
        &self.windows
    }

    /// Device pixels per point.
    #[must_use]
    pub const fn scale(&self) -> f32 {
        self.scale
    }

    fn publish_rects(&self) {
        *self.shared.rects.borrow_mut() = self
            .windows
            .iter()
            .map(|w| (w.id.clone(), w.rect))
            .collect();
    }

    /// Opens and closes what the hosts asked for since.
    fn windows_asked(&mut self) {
        let asks = std::mem::take(&mut *self.shared.asks.borrow_mut());
        if asks.is_empty() {
            return;
        }
        for ask in asks {
            match ask {
                Ask::Open(popout) => {
                    if self.windows.iter().any(|w| w.id == *popout.id()) {
                        continue;
                    }
                    let main = self
                        .windows
                        .first()
                        .map_or(Rect::new(0.0, 0.0, 0.0, 0.0), |w| w.rect);
                    let (dx, dy, w, h) = POPOUT_AT;
                    let rect = popout
                        .bounds()
                        .unwrap_or_else(|| Rect::new(main.x + dx, main.y + dy, w, h));
                    let doc = document(popout.dom(), rect, self.scale, self.fonts.clone());
                    self.windows.push(Window {
                        id: popout.id().clone(),
                        title: popout.title().to_owned(),
                        rect,
                        doc,
                    });
                }
                Ask::Close(id) => self
                    .windows
                    .retain(|w| w.id != id || w.id == WindowId::main()),
            }
        }
        self.publish_rects();
    }

    /// Runs every document until none has work, then lays each out at
    /// `time` (seconds of the virtual clock). True when any had work.
    fn settle(&mut self, time: f64) -> bool {
        let mut any = false;
        for _ in 0..SETTLE_ROUNDS {
            self.windows_asked();
            let mut progressed = false;
            for w in &mut self.windows {
                progressed |= w.doc.poll(None);
            }
            if !progressed {
                break;
            }
            any = true;
        }
        self.windows_asked();
        for w in &self.windows {
            w.doc.inner.borrow_mut().resolve(time);
        }
        any
    }

    /// The front window whose content holds the desktop point.
    fn window_at(&self, x: f32, y: f32) -> Option<usize> {
        self.windows
            .iter()
            .rposition(|w| architect_workspace::contains(w.rect, x, y))
    }

    /// Where `target` is, in desktop points: the first window (front
    /// first) where it resolves to something with a size.
    #[must_use]
    pub fn find(&self, target: &Target, resolvers: &[Resolver]) -> Option<Rect> {
        if let Target::At { x, y } = target {
            return Some(Rect::new(*x, *y, 0.0, 0.0));
        }
        self.windows.iter().rev().find_map(|w| {
            let doc = w.doc.inner.borrow();
            let r = crate::locate::find_in(&doc, target, resolvers, None)?;
            Some(Rect::new(w.rect.x + r.x, w.rect.y + r.y, r.w, r.h))
        })
    }
}

/// How many times a frame polls the documents for work before it lays
/// them out (a signal written in one redraws another).
const SETTLE_ROUNDS: usize = 16;

fn document(
    vdom: VirtualDom,
    rect: Rect,
    scale: f32,
    fonts: Option<FontContext>,
) -> DioxusDocument {
    let viewport = Viewport::new(
        device_px(rect.w, scale),
        device_px(rect.h, scale),
        scale,
        ColorScheme::Dark,
    );
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(viewport),
            font_ctx: fonts,
            ..Default::default()
        },
    );
    doc.initial_build();
    doc
}

/// Plays `scenario` over `app`, showing each frame kept to `film` (none:
/// a test, no frames drawn).
///
/// # Errors
///
/// [`RunError::Step`] for the first step whose action failed, whose
/// target was not found, or whose `done` did not hold in time.
pub fn run(
    app: &App,
    scenario: &Scenario,
    film: Option<&mut dyn Film>,
) -> Result<Report, RunError> {
    // Before the documents: a motion started as they build is on it too.
    let clock = VirtualClock::start();
    let desk = Desktop::open(app, scenario);
    let main = desk
        .windows
        .first()
        .map_or(Rect::new(0.0, 0.0, 0.0, 0.0), |w| w.rect);
    let mut runner = Runner {
        app,
        fps: f64::from(scenario.capture.fps.max(1)),
        desk,
        frame: 0,
        pointer: Pointer {
            x: main.x + main.w * 0.5,
            y: main.y + main.h * 0.5,
            down: false,
            ripple: None,
        },
        pressed_at: None,
        captured: None,
        focused: 0,
        film,
        clock,
        recording: false,
        pending: None,
        report: Report::default(),
    };
    runner.settle_hidden();
    for step in &scenario.steps {
        let started = runner.frame;
        runner.recording = !step.hide;
        runner.step(step)?;
        runner.report.steps.push(StepReport {
            id: step.id.clone(),
            frames: runner.frame.saturating_sub(started),
        });
    }
    Ok(runner.report)
}

struct Runner<'a, 'f> {
    app: &'a App,
    fps: f64,
    desk: Desktop,
    /// The virtual clock, in frames.
    frame: u64,
    pointer: Pointer,
    /// The frame the pointer last went down at, for its ripple.
    pressed_at: Option<u64>,
    /// The window holding the pointer while its button is down.
    captured: Option<usize>,
    /// The window keys go to: the last one pressed in.
    focused: usize,
    film: Option<&'f mut dyn Film>,
    /// The app's motions' clock, moved on with the frames.
    clock: VirtualClock,
    recording: bool,
    /// An async action under way.
    pending: Option<(String, Pending)>,
    report: Report,
}

/// An async action's run, polled between frames.
type Pending = Pin<Box<dyn Future<Output = Result<Vec<u8>, ActionError>>>>;

/// Frames a press's ripple spreads over, as seconds.
const RIPPLE_SECONDS: f64 = 0.45;
/// How long the pointer takes to reach a drag's start.
const APPROACH_SECONDS: f32 = 0.35;
/// How long the documents must be idle before a frame is taken, and the
/// longest a frame waits for them.
const QUIET: Duration = Duration::from_millis(12);
const QUIET_MOST: Duration = Duration::from_millis(250);
/// The longest the documents are given to settle before the first step.
const SETTLE_WALL: Duration = Duration::from_secs(3);

impl Runner<'_, '_> {
    fn time(&self) -> f64 {
        to_f64(self.frame) / self.fps
    }

    fn frames_for(&self, seconds: f32) -> u64 {
        let n = (f64::from(seconds.max(0.0)) * self.fps).ceil();
        from_f64(n)
    }

    /// Hidden: the documents settle (fonts, first measurements, the app's
    /// own start-up tasks) before anything is filmed.
    fn settle_hidden(&mut self) {
        let start = Instant::now();
        let mut idle = 0_u32;
        while idle < 24 && start.elapsed() < SETTLE_WALL {
            if self.desk.settle(0.0) {
                idle = 0;
            } else {
                idle = idle.saturating_add(1);
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }

    /// One frame of the virtual clock: settle, then show it to the film.
    fn tick(&mut self) -> Result<(), RunError> {
        let time = self.time();
        self.clock.advance_to(time);
        self.quiesce(time);
        self.poll_pending()?;
        self.show();
        self.frame = self.frame.saturating_add(1);
        Ok(())
    }

    /// Settles the documents, then lets the app's own short tasks finish
    /// (a measurement retried a few milliseconds on, a timer): the frame
    /// is taken once nothing has had work for [`QUIET`], or after
    /// [`QUIET_MOST`] whatever is still running. So a gesture's next move
    /// lands where a person's would, after the app caught up, however fast
    /// frames are made.
    fn quiesce(&mut self, time: f64) {
        let start = Instant::now();
        let mut quiet_since = Instant::now();
        loop {
            if self.desk.settle(time) {
                quiet_since = Instant::now();
            }
            if quiet_since.elapsed() >= QUIET || start.elapsed() >= QUIET_MOST {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn show(&mut self) {
        let ripple = self.pressed_at.and_then(|at| {
            let since = to_f64(self.frame.saturating_sub(at)) / self.fps;
            (since <= RIPPLE_SECONDS).then(|| narrow(since / RIPPLE_SECONDS))
        });
        self.pointer.ripple = ripple;
        if self.recording {
            if let Some(film) = self.film.as_deref_mut() {
                film.frame(&self.desk, self.pointer);
                self.report.frames = self.report.frames.saturating_add(1);
            }
        }
    }

    fn step(&mut self, step: &Step) -> Result<(), RunError> {
        let fail = |reason: String| RunError::Step {
            step: step.id.clone(),
            reason,
            actual: None,
        };
        let since = self.app.list.latest();
        match &step.act {
            // A step that only points at something (a tour's "this is
            // the trigger") is done once it is on screen.
            None => {
                if let Some(target) = &step.target {
                    self.wait_found(step, target)?;
                }
            }
            Some(Do::Run { id, args }) => {
                let args = args.as_ref().map(to_json);
                self.run_action(id, args).map_err(fail)?;
            }
            Some(Do::Pause { seconds }) => {
                for _ in 0..self.frames_for(*seconds) {
                    self.tick()?;
                }
            }
            Some(Do::Type { text }) => {
                for c in text.chars() {
                    let key = Key::Character(c.to_string());
                    let typed = c.to_string();
                    self.key(&key, Modifiers::empty(), Some(&typed));
                    self.tick()?;
                }
            }
            Some(Do::Key { key, mods }) => {
                let parsed = parse_key(key);
                let text = match &parsed {
                    Key::Character(c) => Some(c.clone()),
                    Key::Named(_) => None,
                };
                self.key(&parsed, modifiers(mods), text.as_deref());
                self.tick()?;
            }
            Some(gesture) => {
                let target = step
                    .target
                    .as_ref()
                    .ok_or_else(|| fail("a gesture needs a target".to_owned()))?;
                let at = self.find(target).map_err(fail)?;
                let (x, y) = centre(at);
                match gesture {
                    Do::Click { seconds } => {
                        self.travel(x, y, *seconds)?;
                        self.press(x, y);
                        self.tick()?;
                        self.release(x, y);
                        self.tick()?;
                    }
                    Do::Drag {
                        to,
                        edge,
                        dx,
                        dy,
                        seconds,
                    } => {
                        self.travel(x, y, APPROACH_SECONDS)?;
                        self.press(x, y);
                        self.tick()?;
                        // The end is found again each frame: what is under
                        // the drag moves as it goes (a split opening up).
                        let frames = self.frames_for(*seconds).max(2);
                        for n in 1..=frames {
                            let end = match to {
                                Some(t) => self
                                    .find(t)
                                    .map(|r| {
                                        edge.map_or_else(
                                            || centre(r),
                                            |e| e.point(r.x, r.y, r.w, r.h),
                                        )
                                    })
                                    .map_err(fail)?,
                                None => (x, y),
                            };
                            let end = (end.0 + dx, end.1 + dy);
                            let p = ease(to_f64(n) / to_f64(frames));
                            let (px, py) = (lerp(x, end.0, p), lerp(y, end.1, p));
                            self.move_to(px, py);
                            self.tick()?;
                        }
                        let (ex, ey) = (self.pointer.x, self.pointer.y);
                        self.release(ex, ey);
                        self.tick()?;
                    }
                    Do::Scroll { dx, dy, seconds } => {
                        self.travel(x, y, APPROACH_SECONDS)?;
                        // A trackpad's flick: quick at first, coasting to a
                        // stop, in pixel deltas as macOS sends them.
                        let frames = self.frames_for(*seconds).max(1);
                        let mut done = 0.0_f32;
                        for n in 1..=frames {
                            let p = flick(to_f64(n) / to_f64(frames));
                            let step = p - done;
                            done = p;
                            self.wheel(x, y, *dx * step, *dy * step);
                            self.tick()?;
                        }
                    }
                    Do::Run { .. } | Do::Pause { .. } | Do::Type { .. } | Do::Key { .. } => {}
                }
            }
        }
        self.wait_done(step, since)?;
        for _ in 0..self.frames_for(step.hold).max(1) {
            self.tick()?;
        }
        Ok(())
    }

    /// Waits (as long as the step's `done` allows) for `target` to be on
    /// screen, keeping a frame whenever a document changed.
    fn wait_found(&mut self, step: &Step, target: &Target) -> Result<(), RunError> {
        let timeout = step.done.as_ref().map_or(5.0, |d| d.timeout);
        let deadline = Instant::now()
            .checked_add(Duration::from_secs_f32(timeout.clamp(0.05, 3600.0)))
            .unwrap_or_else(Instant::now);
        loop {
            let Err(reason) = self.find(target) else {
                return Ok(());
            };
            if Instant::now() > deadline {
                return Err(RunError::Step {
                    step: step.id.clone(),
                    reason: format!("{reason} (in {timeout}s)"),
                    actual: None,
                });
            }
            let time = self.time();
            if self.desk.settle(time) {
                self.show();
                self.frame = self.frame.saturating_add(1);
            } else {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    fn find(&self, target: &Target) -> Result<Rect, String> {
        self.desk
            .find(target, &self.app.resolvers)
            .ok_or_else(|| format!("found nothing on screen for {target:?}"))
    }

    /// Runs action `id` (`COMMANDS_RUN` for a command the list does not
    /// have itself: a view's own, `architect_commands`').
    fn run_action(&mut self, id: &str, args: Option<serde_json::Value>) -> Result<(), String> {
        let list = &self.app.list;
        let (id, args) = crate::predicate::call(list, id, args);
        let bytes = args.to_string().into_bytes();
        if list.is_async(&id) {
            let list = Arc::clone(list);
            let named = id.clone();
            let future: Pending =
                Box::pin(async move { list.run_json_async(&named, &bytes).await });
            self.pending = Some((id, future));
            return Ok(());
        }
        list.run_json(&id, &bytes)
            .map(drop)
            .map_err(|e| format!("action {id} failed: {e}"))
    }

    /// Polls an async action under way; its failure fails the step.
    fn poll_pending(&mut self) -> Result<(), RunError> {
        let Some((id, future)) = self.pending.as_mut() else {
            return Ok(());
        };
        let mut cx = Context::from_waker(Waker::noop());
        match future.as_mut().poll(&mut cx) {
            Poll::Pending => Ok(()),
            Poll::Ready(result) => {
                let id = id.clone();
                self.pending = None;
                result.map(drop).map_err(|e| RunError::Step {
                    step: String::new(),
                    reason: format!("action {id} failed: {e}"),
                    actual: None,
                })
            }
        }
    }

    /// Waits for `done`: one of its events (from `since`), then every
    /// check. Frames are kept only when a document changed, so a wait on a
    /// slow task films no stills.
    fn wait_done(&mut self, step: &Step, since: u64) -> Result<(), RunError> {
        let fail = |reason: String, actual: Option<String>| RunError::Step {
            step: step.id.clone(),
            reason,
            actual,
        };
        let done = step.done.clone().unwrap_or_default();
        let timeout = if step.done.is_some() {
            done.timeout
        } else {
            5.0
        };
        let deadline = Instant::now()
            .checked_add(Duration::from_secs_f32(timeout.clamp(0.05, 3600.0)))
            .unwrap_or_else(Instant::now);
        let mut heard = done.on.is_empty();
        let mut cursor = since;
        loop {
            let time = self.time();
            // A motion under way (a closing split folding before the close
            // lands) needs time to pass: the wait is frames then.
            let moving = self.clock.waiting();
            if moving {
                self.clock.advance_to(time);
            }
            let changed = self.desk.settle(time);
            self.poll_pending().map_err(|e| match e {
                RunError::Step { reason, actual, .. } => fail(reason, actual),
            })?;
            if changed || moving {
                self.show();
                self.frame = self.frame.saturating_add(1);
            }
            if !heard {
                let (events, next) = self.app.list.events_since(cursor, 256, Duration::ZERO);
                cursor = next;
                heard = events.iter().any(|e| woke(&done.on, &e.kind));
            }
            let mut failing = None;
            if heard && self.pending.is_none() {
                match checks(&self.app.list, &done.check) {
                    Ok(()) => return Ok(()),
                    Err(Failed::Hard(reason)) => return Err(fail(reason, None)),
                    Err(Failed::Not(reason, actual)) => failing = Some((reason, actual)),
                }
            }
            if Instant::now() > deadline {
                let (reason, actual) = failing.unwrap_or_else(|| {
                    let reason = if self.pending.is_some() {
                        "its action never finished".to_owned()
                    } else {
                        format!("never heard any of {:?}", done.on)
                    };
                    (reason, None)
                });
                return Err(fail(format!("{reason} (in {timeout}s)"), actual));
            }
            if !changed {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    // ── The pointer ───────────────────────────────────────────────────

    /// Moves the pointer to `(x, y)` over `seconds`, a frame at a time.
    fn travel(&mut self, x: f32, y: f32, seconds: f32) -> Result<(), RunError> {
        let (x0, y0) = (self.pointer.x, self.pointer.y);
        let frames = self.frames_for(seconds);
        for n in 1..=frames {
            let p = ease(to_f64(n) / to_f64(frames));
            self.move_to(lerp(x0, x, p), lerp(y0, y, p));
            self.tick()?;
        }
        self.move_to(x, y);
        Ok(())
    }

    /// The window a pointer event at `(x, y)` goes to: the one holding the
    /// pointer, else the one under it.
    fn route(&self, x: f32, y: f32) -> Option<usize> {
        self.captured.or_else(|| self.desk.window_at(x, y))
    }

    fn pointer_event(&self, window: usize, x: f32, y: f32) -> Option<BlitzPointerEvent> {
        let w = self.desk.windows.get(window)?;
        let (lx, ly) = (x - w.rect.x, y - w.rect.y);
        let down = self.pointer.down;
        Some(BlitzPointerEvent {
            id: BlitzPointerId::Mouse,
            is_primary: true,
            coords: PointerCoords {
                page_x: lx,
                page_y: ly,
                screen_x: x,
                screen_y: y,
                client_x: lx,
                client_y: ly,
            },
            button: MouseEventButton::Main,
            buttons: if down {
                MouseEventButtons::Primary
            } else {
                MouseEventButtons::None
            },
            mods: Modifiers::empty(),
            details: PointerDetails {
                pressure: if down { 0.5 } else { 0.0 },
                ..PointerDetails::default()
            },
            element: blitz_traits::events::Point { x: lx, y: ly },
            active_pointers: Arc::default(),
        })
    }

    fn send(&mut self, window: usize, event: UiEvent) {
        if let Some(w) = self.desk.windows.get_mut(window) {
            w.doc.handle_ui_event(event);
        }
    }

    fn move_to(&mut self, x: f32, y: f32) {
        self.pointer.x = x;
        self.pointer.y = y;
        if let Some(window) = self.route(x, y) {
            if let Some(e) = self.pointer_event(window, x, y) {
                self.send(window, UiEvent::PointerMove(e));
            }
        }
    }

    fn press(&mut self, x: f32, y: f32) {
        self.pointer.down = true;
        self.pressed_at = Some(self.frame);
        let Some(window) = self.desk.window_at(x, y) else {
            return;
        };
        self.captured = Some(window);
        self.focused = window;
        if let Some(e) = self.pointer_event(window, x, y) {
            self.send(window, UiEvent::PointerDown(e));
        }
    }

    /// Lets go: the window holding the pointer hears it; let go outside
    /// its content, its host is told where too (the platform's
    /// `on_release_outside`), as on a desk.
    fn release(&mut self, x: f32, y: f32) {
        self.pointer.down = false;
        let Some(window) = self.captured.take().or_else(|| self.desk.window_at(x, y)) else {
            return;
        };
        if let Some(e) = self.pointer_event(window, x, y) {
            self.send(window, UiEvent::PointerUp(e));
        }
        let Some(w) = self.desk.windows.get(window) else {
            return;
        };
        if architect_workspace::contains(w.rect, x, y) {
            return;
        }
        let (lx, ly) = (x - w.rect.x, y - w.rect.y);
        let hear = self
            .desk
            .shared
            .releases
            .borrow()
            .iter()
            .rev()
            .find(|(id, _)| *id == w.id)
            .map(|(_, f)| Rc::clone(f));
        if let Some(hear) = hear {
            hear(lx, ly);
        }
    }

    fn wheel(&mut self, x: f32, y: f32, dx: f32, dy: f32) {
        let Some(window) = self.desk.window_at(x, y) else {
            return;
        };
        let Some(e) = self.pointer_event(window, x, y) else {
            return;
        };
        self.send(
            window,
            UiEvent::Wheel(BlitzWheelEvent {
                delta: BlitzWheelDelta::Pixels(f64::from(dx), f64::from(dy)),
                coords: e.coords,
                buttons: MouseEventButtons::None,
                mods: Modifiers::empty(),
                element: e.element,
            }),
        );
    }

    fn key(&mut self, key: &Key, mods: Modifiers, text: Option<&str>) {
        let event = |state| BlitzKeyEvent {
            key: key.clone(),
            code: Code::Unidentified,
            modifiers: mods,
            location: Location::Standard,
            is_auto_repeating: false,
            is_composing: false,
            state,
            text: text.map(Into::into),
        };
        let window = self.focused;
        self.send(window, UiEvent::KeyDown(event(KeyState::Pressed)));
        self.send(window, UiEvent::KeyUp(event(KeyState::Released)));
    }
}

/// A key by its `KeyboardEvent.key` name (`Enter`, `ArrowDown`) or as the
/// one character it types.
fn parse_key(name: &str) -> Key {
    name.parse::<Key>()
        .unwrap_or_else(|_| Key::Character(name.into()))
}

fn modifiers(mods: &[Modifier]) -> Modifiers {
    mods.iter().fold(Modifiers::empty(), |m, k| {
        m | match k {
            Modifier::Shift => Modifiers::SHIFT,
            Modifier::Ctrl => Modifiers::CONTROL,
            Modifier::Alt => Modifiers::ALT,
            Modifier::Meta => Modifiers::META,
        }
    })
}

fn centre(r: Rect) -> (f32, f32) {
    (r.x + r.w * 0.5, r.y + r.h * 0.5)
}

/// Ease in and out (smoothstep): a hand starts and stops gently.
fn ease(t: f64) -> f32 {
    let t = t.clamp(0.0, 1.0);
    narrow(t * t * 2.0f64.mul_add(-t, 3.0))
}

/// How far a flick has gone at `t` of its time: a cubic ease-out.
fn flick(t: f64) -> f32 {
    let rest = 1.0 - t.clamp(0.0, 1.0);
    narrow(rest.mul_add(-(rest * rest), 1.0))
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    (b - a).mul_add(t, a)
}

// ── Conversions the lints want named ──────────────────────────────────

/// A point count as an `f32` (window sizes fit).
#[allow(clippy::cast_precision_loss, clippy::as_conversions)]
const fn to_f32(n: u32) -> f32 {
    n as f32
}

#[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
const fn narrow(v: f64) -> f32 {
    v as f32
}

#[allow(clippy::cast_precision_loss, clippy::as_conversions)]
const fn to_f64(n: u64) -> f64 {
    n as f64
}

/// A non-negative frame count from a float; 0 for anything else.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::as_conversions
)]
fn from_f64(v: f64) -> u64 {
    if v.is_finite() && v > 0.0 {
        v.min(1.0e9) as u64
    } else {
        0
    }
}

/// Points as device pixels, at least one.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::as_conversions
)]
pub(crate) fn device_px(points: f32, scale: f32) -> u32 {
    let px = f64::from(points) * f64::from(if scale > 0.0 { scale } else { 1.0 });
    if px.is_finite() && px >= 1.0 {
        px.min(16384.0).round() as u32
    } else {
        1
    }
}

/// Plays every scenario in `dir` (`*.styx`, by name) as a test: a fresh
/// app from `make` for each, no frames. Every failure is collected, so one
/// broken step does not hide another scenario's.
///
/// # Errors
///
/// Each scenario that did not load or did not run, with its step and
/// what its check's action gave back, one per line.
pub fn test_dir(make: impl Fn() -> App, dir: &std::path::Path) -> Result<usize, String> {
    let scenarios = crate::load_dir(dir).map_err(|e| e.to_string())?;
    let mut failed = Vec::new();
    for (path, scenario) in &scenarios {
        if let Err(e) = run(&make(), scenario, None) {
            failed.push(format!("{} ({}): {e}", scenario.id, path.display()));
        }
    }
    if failed.is_empty() {
        Ok(scenarios.len())
    } else {
        Err(failed.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_commands_run_action_is_the_one_a_step_goes_through() {
        assert_eq!(
            crate::predicate::RUN_COMMAND,
            architect_commands::RUN_ACTION
        );
    }
}
