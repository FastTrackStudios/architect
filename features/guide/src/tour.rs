//! The in-app tour (feature `ui`): a scenario played live, over the
//! running app, for a person to follow
//! (`docs/content/architecture/guide-media.md`).
//!
//! [`use_tours`] offers an app's tours ([`TourSet`]) and puts them on its
//! Action List: [`START_ACTION`] (`GUIDE_TOUR_START {id}`) starts one, so
//! ⌘K, a model over MCP and a scenario can; [`STATE_ACTION`] reads where it
//! is; next, back and skip move it. [`TourLayer`], mounted once at the
//! window's root, draws it:
//!
//! - the window dimmed around a cutout over the step's [`Target`], ringed
//!   (found in the live document: [`crate::locate::find_live`]); a drag's
//!   end ringed dashed. A gesture is *shown*, never performed: the person
//!   does it;
//! - a popover with the step's `text`, a line on what to do, and Back,
//!   Skip and Next;
//! - it moves on when the step's `done` holds — one of its events on the
//!   Action List, in process, then its checks — or on Next for a step with
//!   no `done` (text only, or a gesture nothing reports);
//! - a hidden step is setup: the tour runs its `@run` itself and moves on;
//!   Back to a step runs the setup right before it again, so a tour of
//!   chapters (setup, then a step to try) puts the app back where that
//!   chapter needs it;
//! - [`TourSet::on_move`] hears every move, so an app can remember where a
//!   person left a tour and start it there again (`GUIDE_TOUR_START {id,
//!   step}`).
//!
//! Safety: a scenario not marked `safe` never plays over the person's own
//! data. The app's playground ([`TourSet::playground`]: a sample document,
//! a scratch session) is set up first; an app with none cannot start it.
//!
//! The cutout and the popover move on the shell's motion
//! (`architect_shell::motion`), so a tour filmed headlessly shows them
//! glide.

// Geometry reads plainer as `a + b * c`; its precision is a pixel's.
#![allow(clippy::suboptimal_flops)]

use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use architect::action::{ActionList, ActionMeta, Effect};
use architect_shell::motion::{Spring, is_virtual, next_frame, use_motion};
use architect_workspace::Rect;
use dioxus::prelude::*;
use facet::Facet;
use futures_util::StreamExt as _;

use crate::locate::{Resolver, find_live, size_live};
use crate::predicate::{call, checks, woke};
use crate::{Do, LoadError, Modifier, Scenario, Step, to_json};

/// Starts a tour: `{id}`, a scenario's id.
pub const START_ACTION: &str = "GUIDE_TOUR_START";
/// Where the tour is: [`TourState`].
pub const STATE_ACTION: &str = "GUIDE_TOUR_STATE";
/// The tours on offer: [`TourInfo`]s.
pub const LIST_ACTION: &str = "GUIDE_TOUR_LIST";
/// Moves on a step, whatever it waits for.
pub const NEXT_ACTION: &str = "GUIDE_TOUR_NEXT";
/// Goes back a step.
pub const BACK_ACTION: &str = "GUIDE_TOUR_BACK";
/// Ends the tour.
pub const SKIP_ACTION: &str = "GUIDE_TOUR_SKIP";
/// The topic of the `Changed` event each move of the tour is.
pub const TOPIC: &str = "guide";

/// The tours an app offers, and what playing them needs.
#[derive(Clone, Default)]
pub struct TourSet {
    scenarios: Vec<Scenario>,
    resolvers: Vec<Resolver>,
    playground: Option<Rc<dyn Fn(&Scenario) -> Result<(), String>>>,
    moved: Option<Rc<dyn Fn(&Moved)>>,
}

/// A move of a tour, as [`TourSet::on_move`] hears it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Moved {
    /// Tour `scenario` shows step `step` (by id).
    At { scenario: String, step: String },
    /// The person left tour `scenario` at step `step` (Skip).
    Left { scenario: String, step: String },
    /// Tour `scenario` was played to its end.
    Finished { scenario: String },
}

impl TourSet {
    /// These scenarios as tours.
    #[must_use]
    pub fn new(scenarios: Vec<Scenario>) -> Self {
        Self {
            scenarios,
            ..Self::default()
        }
    }

    /// Scenarios read from styx sources (an app's `include_str!`s).
    ///
    /// # Errors
    ///
    /// The first source's [`crate::load`] error.
    pub fn from_sources<'a>(sources: impl IntoIterator<Item = &'a str>) -> Result<Self, LoadError> {
        let scenarios = sources
            .into_iter()
            .map(crate::load)
            .collect::<Result<_, _>>()?;
        Ok(Self::new(scenarios))
    }

    /// Resolves an app's own targets, tried before `data-testid` and CSS:
    /// `architect_workspace::target_selector` for its tabs and panes,
    /// `architect_commands::target_selector` for the palette's rows, an
    /// app's `@app{…}` kinds.
    #[must_use]
    pub fn resolver(mut self, resolver: Resolver) -> Self {
        self.resolvers.push(resolver);
        self
    }

    /// Sets up the app's playground for a scenario not marked `safe`
    /// (opens a sample document, a scratch session), or says why it
    /// cannot. Unset, such a scenario does not start.
    #[must_use]
    pub fn playground(mut self, setup: impl Fn(&Scenario) -> Result<(), String> + 'static) -> Self {
        self.playground = Some(Rc::new(setup));
        self
    }

    /// Hears every move of a tour: a step shown, the tour left, or played
    /// to its end. Where an app remembers a person's place in a tour.
    #[must_use]
    pub fn on_move(mut self, moved: impl Fn(&Moved) + 'static) -> Self {
        self.moved = Some(Rc::new(moved));
        self
    }

    /// The scenarios on offer.
    #[must_use]
    pub fn scenarios(&self) -> &[Scenario] {
        &self.scenarios
    }

    /// Why `scenario` cannot start, if it cannot: not `safe`, and no
    /// playground, or the playground could not be set up.
    fn may_start(&self, scenario: &Scenario) -> Result<(), String> {
        if scenario.safe {
            return Ok(());
        }
        let playground = self.playground.as_ref().ok_or_else(|| {
            format!(
                "tour {} is not safe over your own data, and the app has no playground for it",
                scenario.id
            )
        })?;
        playground(scenario)
    }
}

/// What [`STATE_ACTION`] answers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet)]
pub struct TourState {
    /// The scenario playing; empty when none is.
    pub scenario: String,
    /// The step on show, by id.
    pub step: String,
    /// Its place among the steps a person sees, from 1.
    pub index: u32,
    /// How many steps a person sees.
    pub of: u32,
    /// Whether it waits for its `done` (else for Next).
    pub waiting: bool,
    /// Whether its target is on screen (or it has none).
    pub found: bool,
}

/// One tour on offer, as [`LIST_ACTION`] answers.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct TourInfo {
    pub id: String,
    pub title: String,
    /// Whether it may play over the person's own data.
    pub safe: bool,
}

/// What [`START_ACTION`] takes.
#[derive(Debug, Clone, Facet)]
pub struct StartTour {
    /// The scenario's id.
    pub id: String,
    /// The step to start at, by id (from the setup right before it).
    /// Unset, or no such step: the first.
    #[facet(default)]
    pub step: Option<String>,
}

/// Where a tour is: which scenario, which step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Playing {
    pub scenario: usize,
    pub step: usize,
}

/// The app's tours, in context under [`use_tours`]: start one, move it.
#[derive(Clone, Copy)]
pub struct Tours {
    set: CopyValue<TourSet>,
    list: CopyValue<Arc<ActionList>>,
    playing: Signal<Option<Playing>>,
    /// The step's target and a drag's end, from the layer's top left.
    spot: Signal<Option<Rect>>,
    end: Signal<Option<Rect>>,
    /// The layer's size and the popover's.
    area: Signal<(f32, f32)>,
    card: Signal<(f32, f32)>,
    /// The layer and the popover, as mounted (how a target is found).
    layer: CopyValue<Option<Rc<MountedData>>>,
    popover: CopyValue<Option<Rc<MountedData>>>,
    /// The driver's inbox: asks from the Action List, and a nudge when a
    /// tour starts.
    driver: CopyValue<Option<UnboundedSender<Msg>>>,
}

impl PartialEq for Tours {
    fn eq(&self, other: &Self) -> bool {
        self.playing == other.playing
    }
}

/// What the tour's driver is told: a move asked through the Action List
/// (answered), or to look again.
enum Msg {
    Ask(
        Ask,
        futures_channel::oneshot::Sender<Result<TourState, String>>,
    ),
    Wake,
}

#[derive(Debug, Clone)]
enum Ask {
    Start(String, Option<String>),
    Next,
    Back,
    Skip,
}

static START_META: ActionMeta = meta(
    START_ACTION,
    "Start a tour",
    "Starts the in-app tour of a guide scenario, by its id (GUIDE_TOUR_LIST lists them): its steps shown over the app, one at a time. `step` starts it at that step (by id) instead of the first.",
);
static STATE_META: ActionMeta = meta(
    STATE_ACTION,
    "Where the tour is",
    "The tour playing (empty when none is), its step, the step's place among those shown, whether it waits for the step to be done, and whether its target is on screen.",
);
static LIST_META: ActionMeta = meta(
    LIST_ACTION,
    "Tours on offer",
    "Every tour the app offers: its id, title, and whether it may play over your own data.",
);
static NEXT_META: ActionMeta = meta(
    NEXT_ACTION,
    "Next step of the tour",
    "Moves the tour on a step, whatever the step waits for; past the last, it ends.",
);
static BACK_META: ActionMeta = meta(
    BACK_ACTION,
    "Back a step in the tour",
    "Goes back to the tour's step before, running the setup steps right before it again.",
);
static SKIP_META: ActionMeta = meta(SKIP_ACTION, "End the tour", "Ends the tour playing.");

const fn meta(
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "",
        method_name: "",
        display_name,
        description,
        category: "Guide",
        group: "Tours",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

/// Offers `set`'s tours while the calling component is mounted: puts
/// [`Tours`] in context (for [`TourLayer`] and the app's own buttons) and
/// the tour's actions on `list`.
#[must_use = "the tours, to start one from the app's own buttons"]
pub fn use_tours(list: Arc<ActionList>, set: TourSet) -> Tours {
    let set = use_hook(|| CopyValue::new(set));
    let held = use_hook(|| CopyValue::new(list.clone()));
    let playing = use_signal(|| None::<Playing>);
    let spot = use_signal(|| None::<Rect>);
    let end = use_signal(|| None::<Rect>);
    let area = use_signal(|| (0.0_f32, 0.0_f32));
    let card = use_signal(|| (0.0_f32, 0.0_f32));
    let layer = use_hook(|| CopyValue::new(None::<Rc<MountedData>>));
    let popover = use_hook(|| CopyValue::new(None::<Rc<MountedData>>));
    let mirror = use_hook(|| Arc::new(Mutex::new(TourState::default())));
    let mut inbox_tx = use_hook(|| CopyValue::new(None::<UnboundedSender<Msg>>));
    let tours = Tours {
        set,
        list: held,
        playing,
        spot,
        end,
        area,
        card,
        layer,
        popover,
        driver: inbox_tx,
    };
    let driver = use_coroutine(move |mut inbox: UnboundedReceiver<Msg>| async move {
        let mut at = None::<Playing>;
        let mut step = StepWatch::default();
        loop {
            // Asks first; then, while a tour plays, a look a frame.
            while let Ok(msg) = inbox.try_recv() {
                tours.hear(msg);
            }
            let Some(now) = *playing.peek() else {
                at = None;
                match inbox.next().await {
                    Some(msg) => tours.hear(msg),
                    None => return,
                }
                continue;
            };
            if at != Some(now) {
                at = Some(now);
                step = StepWatch::new(&held.peek());
            }
            tours.look(now, &mut step).await;
            pause().await;
        }
    });
    use_hook(move || inbox_tx.set(Some(driver.tx())));
    use_context_provider(|| tours);
    let registered = use_hook({
        let list = list.clone();
        let mirror = mirror.clone();
        move || register(&list, &driver.tx(), &mirror, &set.peek())
    });
    // The mirror the state action reads, and a `guide` event each move.
    let told = list.clone();
    use_effect(move || {
        let now = playing();
        let found = spot().is_some();
        let next = set.with_peek(|s| state_of(s, now, found));
        let mut kept = mirror.lock().unwrap_or_else(PoisonError::into_inner);
        if *kept != next {
            let moved = kept.scenario != next.scenario || kept.step != next.step;
            let detail = next.step.clone();
            *kept = next;
            drop(kept);
            if moved {
                told.changed(TOPIC, &detail);
            }
        }
    });
    use_drop(move || {
        if registered {
            for id in [
                START_ACTION,
                STATE_ACTION,
                LIST_ACTION,
                NEXT_ACTION,
                BACK_ACTION,
                SKIP_ACTION,
            ] {
                list.unregister(id);
            }
        }
    });
    tours
}

/// Puts the tour's actions on `list`; false when one was there already.
fn register(
    list: &ActionList,
    driver: &UnboundedSender<Msg>,
    mirror: &Arc<Mutex<TourState>>,
    set: &TourSet,
) -> bool {
    let asker = |ask: fn(StartTour) -> Ask| {
        let tx = driver.clone();
        move |args: StartTour| {
            let (answer, heard) = futures_channel::oneshot::channel();
            let posted = tx.unbounded_send(Msg::Ask(ask(args), answer)).is_ok();
            async move {
                if !posted {
                    return Err("the window is gone".to_owned());
                }
                heard.await.map_err(|_| "the window is gone".to_owned())?
            }
        }
    };
    let mover = |ask: Ask| {
        let tx = driver.clone();
        move |(): ()| {
            let (answer, heard) = futures_channel::oneshot::channel();
            let posted = tx.unbounded_send(Msg::Ask(ask.clone(), answer)).is_ok();
            async move {
                if !posted {
                    return Err("the window is gone".to_owned());
                }
                heard.await.map_err(|_| "the window is gone".to_owned())?
            }
        }
    };
    let read = mirror.clone();
    let offered: Vec<TourInfo> = set
        .scenarios
        .iter()
        .map(|s| TourInfo {
            id: s.id.clone(),
            title: s.title.clone(),
            safe: s.safe,
        })
        .collect();
    [
        list.register_async(
            &START_META,
            Effect::Mutating,
            asker(|a| Ask::Start(a.id, a.step)),
        ),
        list.register(&STATE_META, Effect::ReadOnly, move |(): ()| {
            Ok(read.lock().unwrap_or_else(PoisonError::into_inner).clone())
        }),
        list.register(&LIST_META, Effect::ReadOnly, move |(): ()| {
            Ok(offered.clone())
        }),
        list.register_async(&NEXT_META, Effect::Mutating, mover(Ask::Next)),
        list.register_async(&BACK_META, Effect::Mutating, mover(Ask::Back)),
        list.register_async(&SKIP_META, Effect::Mutating, mover(Ask::Skip)),
    ]
    .into_iter()
    .all(|r| r.is_ok())
}

/// A frame of the motion clock under a film or a test (so a tour is
/// filmed frame by frame); on the wall clock, a few frames.
#[allow(clippy::future_not_send)]
async fn pause() {
    if is_virtual() {
        next_frame().await;
    } else {
        futures_timer::Delay::new(Duration::from_millis(50)).await;
    }
}

impl Tours {
    /// Starts tour `id` from its first step.
    ///
    /// # Errors
    ///
    /// No such tour, or it is not `safe` and the app's playground could
    /// not be set up ([`TourSet::playground`]).
    pub fn start(&self, id: &str) -> Result<(), String> {
        self.start_at(id, None)
    }

    /// Starts tour `id` at step `step` (by id), from the setup steps right
    /// before it; `None`, or no such step, from its first.
    ///
    /// # Errors
    ///
    /// As [`Tours::start`].
    pub fn start_at(&self, id: &str, step: Option<&str>) -> Result<(), String> {
        let found = self.set.with_peek(|s| {
            let (n, scenario) = s
                .scenarios
                .iter()
                .enumerate()
                .find(|(_, sc)| sc.id == id)
                .ok_or_else(|| format!("no tour {id:?}"))?;
            s.may_start(scenario)?;
            let first = first_step(scenario).ok_or_else(|| format!("tour {id} has no steps"))?;
            let first = step
                .and_then(|want| scenario.steps.iter().position(|s| s.id == want))
                .map_or(first, |at| setup_of(&scenario.steps, at));
            Ok::<_, String>(Playing {
                scenario: n,
                step: first,
            })
        })?;
        self.go(Some(found));
        Ok(())
    }

    /// Moves on a step (past the last, the tour ends).
    pub fn next(&self) {
        let Some(now) = *self.playing.peek() else {
            return;
        };
        let to = self.set.with_peek(|s| {
            s.scenarios.get(now.scenario).and_then(|sc| {
                (now.step.saturating_add(1) < sc.steps.len()).then_some(now.step.saturating_add(1))
            })
        });
        let finished = to.is_none();
        self.go_as(to.map(|step| Playing { step, ..now }), finished);
    }

    /// Back to the step a person saw before this one (none: stays).
    pub fn back(&self) {
        let Some(now) = *self.playing.peek() else {
            return;
        };
        let to = self.set.with_peek(|s| {
            s.scenarios
                .get(now.scenario)
                .and_then(|sc| shown_before(&sc.steps, now.step).map(|at| setup_of(&sc.steps, at)))
        });
        if let Some(step) = to {
            self.go(Some(Playing { step, ..now }));
        }
    }

    /// Ends the tour.
    pub fn stop(&self) {
        self.go(None);
    }

    /// The tour playing.
    #[must_use]
    pub fn playing(&self) -> Option<Playing> {
        (self.playing)()
    }

    /// The tours on offer, by id and title (an app's ⌘K rows for them).
    #[must_use]
    pub fn offered(&self) -> Vec<(String, String)> {
        self.set.with_peek(|s| {
            s.scenarios
                .iter()
                .map(|sc| (sc.id.clone(), sc.title.clone()))
                .collect()
        })
    }

    fn go(&self, to: Option<Playing>) {
        self.go_as(to, false);
    }

    /// Moves to `to`; `finished`: a tour ending was played to its end.
    fn go_as(&self, to: Option<Playing>, finished: bool) {
        let mut playing = self.playing;
        let from = *playing.peek();
        if from != to {
            playing.set(to);
            self.set.with_peek(|s| {
                if let (Some(heard), Some(moved)) = (&s.moved, moved_of(s, from, to, finished)) {
                    heard(&moved);
                }
            });
        }
        if let Some(tx) = self.driver.peek().as_ref() {
            let _ = tx.unbounded_send(Msg::Wake);
        }
    }

    /// Does what the driver was told.
    fn hear(&self, msg: Msg) {
        let Msg::Ask(ask, answer) = msg else {
            return;
        };
        let done = match ask {
            Ask::Start(id, step) => self.start_at(&id, step.as_deref()),
            Ask::Next => {
                self.next();
                Ok(())
            }
            Ask::Back => {
                self.back();
                Ok(())
            }
            Ask::Skip => {
                self.stop();
                Ok(())
            }
        };
        let now = *self.playing.peek();
        let found = self.spot.peek().is_some();
        let state = self.set.with_peek(|s| state_of(s, now, found));
        let _ = answer.send(done.map(|()| state));
    }

    /// One look at the step on show: run a setup step, find its targets,
    /// and move on when its `done` holds.
    // The window's thread runs it, as it runs the component that made it.
    #[allow(clippy::future_not_send)]
    async fn look(&self, now: Playing, watch: &mut StepWatch) {
        let list = self.list.peek().clone();
        let Some(step) = self.set.with_peek(|s| {
            s.scenarios
                .get(now.scenario)
                .and_then(|sc| sc.steps.get(now.step))
                .cloned()
        }) else {
            self.stop();
            return;
        };
        if step.hide {
            self.put(None, None);
            if !watch.ran {
                watch.ran = true;
                setup(&list, &step).await;
                watch.since = std::time::Instant::now();
            }
            let gave_up = step
                .done
                .as_ref()
                .is_none_or(|d| watch.since.elapsed().as_secs_f32() > d.timeout);
            if gave_up || watch.holds(&list, &step) {
                self.next();
            }
            return;
        }
        let resolvers = self.set.with_peek(|s| s.resolvers.clone());
        let layer = self.layer.peek().clone();
        let found = |t: &crate::Target| layer.as_deref().and_then(|m| find_live(m, t, &resolvers));
        let spot = step.target.as_ref().and_then(found);
        let end = match &step.act {
            Some(Do::Drag { to: Some(t), .. }) => found(t),
            _ => None,
        };
        self.put(spot, end);
        self.measure();
        if step.done.is_some() && watch.holds(&list, &step) {
            self.next();
        }
    }

    fn put(&self, spot: Option<Rect>, end: Option<Rect>) {
        let (mut s, mut e) = (self.spot, self.end);
        if !same(*s.peek(), spot) {
            s.set(spot);
        }
        if !same(*e.peek(), end) {
            e.set(end);
        }
    }

    /// The layer's size and the popover's, as laid out.
    fn measure(&self) {
        let size = |m: CopyValue<Option<Rc<MountedData>>>| m.peek().as_deref().and_then(size_live);
        let (mut area, mut card) = (self.area, self.card);
        if let Some(a) = size(self.layer).filter(|a| a.0 > 0.0) {
            if *area.peek() != a {
                area.set(a);
            }
        }
        if let Some(c) = size(self.popover).filter(|c| c.0 > 0.0) {
            if *card.peek() != c {
                card.set(c);
            }
        }
    }
}

/// Whether two places are the same to half a point.
fn same(a: Option<Rect>, b: Option<Rect>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            (a.x - b.x).abs() < 0.5
                && (a.y - b.y).abs() < 0.5
                && (a.w - b.w).abs() < 0.5
                && (a.h - b.h).abs() < 0.5
        }
        _ => false,
    }
}

/// What a step on show has heard since it came up.
struct StepWatch {
    cursor: u64,
    heard: bool,
    ran: bool,
    since: std::time::Instant,
}

impl Default for StepWatch {
    fn default() -> Self {
        Self {
            cursor: 0,
            heard: false,
            ran: false,
            since: std::time::Instant::now(),
        }
    }
}

impl StepWatch {
    fn new(list: &ActionList) -> Self {
        Self {
            cursor: list.latest(),
            ..Self::default()
        }
    }

    /// Whether `step`'s `done` holds: one of its events since it came up
    /// (or none to wait for), then every check.
    fn holds(&mut self, list: &ActionList, step: &Step) -> bool {
        let Some(done) = &step.done else {
            return false;
        };
        if !self.heard {
            let (events, next) = list.events_since(self.cursor, 256, Duration::ZERO);
            self.cursor = next;
            self.heard = done.on.is_empty() || events.iter().any(|e| woke(&done.on, &e.kind));
        }
        self.heard && checks(list, &done.check).is_ok()
    }
}

/// Runs a setup step's action (a tour performs only those: a gesture is
/// the person's to make).
#[allow(clippy::future_not_send)]
async fn setup(list: &ActionList, step: &Step) {
    let Some(Do::Run { id, args }) = &step.act else {
        return;
    };
    let (id, args) = call(list, id, args.as_ref().map(to_json));
    let bytes = args.to_string().into_bytes();
    // A setup that fails leaves the app as it was; the steps after say
    // what they need.
    let _ = if list.is_async(&id) {
        list.run_json_async(&id, &bytes).await
    } else {
        list.run_json(&id, &bytes)
    };
}

/// What a move from `from` to `to` is, for [`TourSet::on_move`]: a step
/// shown (a setup step is not one), the tour left, or finished.
fn moved_of(
    set: &TourSet,
    from: Option<Playing>,
    to: Option<Playing>,
    finished: bool,
) -> Option<Moved> {
    let step_of = |p: Playing| {
        let sc = set.scenarios.get(p.scenario)?;
        Some((sc.id.clone(), sc.steps.get(p.step)?))
    };
    match to {
        Some(p) => {
            let (scenario, step) = step_of(p)?;
            (!step.hide).then(|| Moved::At {
                scenario,
                step: step.id.clone(),
            })
        }
        None => {
            let (scenario, step) = step_of(from?)?;
            Some(if finished {
                Moved::Finished { scenario }
            } else {
                Moved::Left {
                    scenario,
                    step: step.id.clone(),
                }
            })
        }
    }
}

/// Where a tour going to `step` starts: the first of the setup steps right
/// before it (they run again, so the app is where the step expects), or
/// the step itself.
fn setup_of(steps: &[Step], step: usize) -> usize {
    let before = steps.get(..step).unwrap_or_default();
    before
        .iter()
        .rposition(|s| !s.hide)
        .map_or(0, |shown| shown.saturating_add(1))
}

/// The first step of a scenario (setup or not).
fn first_step(scenario: &Scenario) -> Option<usize> {
    (!scenario.steps.is_empty()).then_some(0)
}

/// The step a person saw before `step`: the nearest earlier one not
/// hidden.
fn shown_before(steps: &[Step], step: usize) -> Option<usize> {
    steps.get(..step)?.iter().rposition(|s| !s.hide)
}

/// What [`STATE_ACTION`] says of `playing`.
fn state_of(set: &TourSet, playing: Option<Playing>, found: bool) -> TourState {
    let Some(p) = playing else {
        return TourState::default();
    };
    let Some(scenario) = set.scenarios.get(p.scenario) else {
        return TourState::default();
    };
    let Some(step) = scenario.steps.get(p.step) else {
        return TourState::default();
    };
    let (index, of) = place_of(&scenario.steps, p.step);
    TourState {
        scenario: scenario.id.clone(),
        step: step.id.clone(),
        index,
        of,
        waiting: step.done.is_some(),
        found: step.target.is_none() || found,
    }
}

/// A step's place among the steps a person sees (from 1), and how many
/// they see. A setup step has the place of the next one shown.
fn place_of(steps: &[Step], step: usize) -> (u32, u32) {
    let count =
        |s: &[Step]| u32::try_from(s.iter().filter(|s| !s.hide).count()).unwrap_or(u32::MAX);
    let before = count(steps.get(..step).unwrap_or_default());
    (before.saturating_add(1), count(steps))
}

/// What the popover says to do, from the step's `do`: a gesture as words
/// over the ringed target, a key as its glyphs, an action by its name and
/// key (`name_of` looks an action up on the app's list).
#[must_use]
pub fn hint(step: &Step, name_of: impl Fn(&str) -> Option<(String, String)>) -> String {
    match &step.act {
        None | Some(Do::Pause { .. }) => String::new(),
        Some(Do::Click { .. }) => "Click the highlighted spot.".to_owned(),
        Some(Do::Drag { to: Some(_), .. }) => {
            "Drag what is highlighted onto the dashed outline.".to_owned()
        }
        Some(Do::Drag { dx, dy, .. }) => {
            let way = if dx.abs() >= dy.abs() {
                if *dx < 0.0 { "left" } else { "right" }
            } else if *dy < 0.0 {
                "up"
            } else {
                "down"
            };
            format!("Drag what is highlighted {way}.")
        }
        Some(Do::Scroll { dx, dy, .. }) => {
            if dx.abs() > dy.abs() {
                "Scroll sideways over what is highlighted.".to_owned()
            } else {
                "Scroll over what is highlighted.".to_owned()
            }
        }
        Some(Do::Type { text }) => format!("Type “{text}”."),
        Some(Do::Key { key, mods }) => format!("Press {}.", key_label(key, mods)),
        Some(Do::Run { id, .. }) => match name_of(id) {
            Some((name, key)) if !key.is_empty() => format!("Run “{name}” ({key})."),
            Some((name, _)) => format!("Run “{name}”."),
            None => String::new(),
        },
    }
}

/// A key and its modifiers as a Mac menu shows them: `⇧⌘K`.
#[must_use]
pub fn key_label(key: &str, mods: &[Modifier]) -> String {
    let mut out = String::new();
    // A menu's order: ⌃ ⌥ ⇧ ⌘.
    for (m, glyph) in [
        (Modifier::Ctrl, '⌃'),
        (Modifier::Alt, '⌥'),
        (Modifier::Shift, '⇧'),
        (Modifier::Meta, '⌘'),
    ] {
        if mods.contains(&m) {
            out.push(glyph);
        }
    }
    let named = match key {
        "Enter" => "↵",
        "Escape" => "⎋",
        "Backspace" => "⌫",
        "Tab" => "⇥",
        "ArrowUp" => "↑",
        "ArrowDown" => "↓",
        "ArrowLeft" => "←",
        "ArrowRight" => "→",
        " " => "Space",
        _ => "",
    };
    if named.is_empty() {
        out.push_str(&key.to_uppercase());
    } else {
        out.push_str(named);
    }
    out
}

/// The cutout's room around its target, points.
const PAD: f32 = 6.0;
/// Between the target and the popover, and the popover and the edge.
const GAP: f32 = 14.0;
/// The popover's width.
const CARD_W: f32 = 340.0;

/// Where the popover's top left goes in an `area`: under the target if
/// it fits, else over it, else at the area's foot; across, centred on it
/// and kept inside. No target: centred at the foot.
#[must_use]
pub fn place(target: Option<Rect>, card: (f32, f32), area: (f32, f32)) -> (f32, f32) {
    let (cw, ch) = card;
    let (aw, ah) = area;
    let inside = |x: f32| x.clamp(GAP, (aw - cw - GAP).max(GAP));
    let foot = (ah - ch - GAP * 2.0).max(GAP);
    let Some(t) = target else {
        return (inside((aw - cw) * 0.5), foot);
    };
    let x = inside(t.x + t.w * 0.5 - cw * 0.5);
    let below = t.y + t.h + PAD + GAP;
    let above = t.y - PAD - GAP - ch;
    if below + ch + GAP <= ah {
        (x, below)
    } else if above >= GAP {
        (x, above)
    } else {
        (x, foot)
    }
}

/// What the popover shows of the step on show.
#[derive(Debug, Clone, Default, PartialEq)]
struct Card {
    title: String,
    text: String,
    hint: String,
    index: u32,
    of: u32,
    /// It moves on when its `done` holds (Next is not offered).
    waiting: bool,
    first: bool,
    last: bool,
}

fn card_of(set: &TourSet, list: &ActionList, playing: Playing) -> Option<Card> {
    let scenario = set.scenarios.get(playing.scenario)?;
    let step = scenario.steps.get(playing.step)?;
    if step.hide {
        return None;
    }
    let (index, of) = place_of(&scenario.steps, playing.step);
    let name_of = |id: &str| list.info(id).map(|i| (i.display_name, i.shortcut));
    Some(Card {
        title: scenario.title.clone(),
        text: step.text.clone(),
        hint: hint(step, name_of),
        index,
        of,
        waiting: step.done.is_some(),
        first: shown_before(&scenario.steps, playing.step).is_none(),
        last: index == of,
    })
}

/// The tour over the window, under [`use_tours`].
///
/// Mount it once, the last thing in the app's root (an absolute layer
/// against the root, as Blitz has no `position: fixed`), over every other
/// layer (z-index 200: over the palette's 90). It is always there, drawn
/// only while a tour plays; it takes no presses but its popover's.
#[component]
pub fn TourLayer() -> Element {
    let tours = try_use_context::<Tours>();
    let x = use_motion(0.0, Spring::SHEET, 0.5);
    let y = use_motion(0.0, Spring::SHEET, 0.5);
    let w = use_motion(0.0, Spring::SHEET, 0.5);
    let h = use_motion(0.0, Spring::SHEET, 0.5);
    let px = use_motion(0.0, Spring::SHEET, 0.5);
    let py = use_motion(0.0, Spring::SHEET, 0.5);
    let fade = use_motion(0.0, Spring::SNAPPY, 0.01);
    let mut placed = use_hook(|| CopyValue::new(false));
    // The cutout follows the target: it jumps to the first, glides after.
    use_effect(move || {
        let Some(t) = tours else {
            return;
        };
        let spot = (t.spot)();
        let card_at = place(spot, (t.card)(), (t.area)());
        let Some(r) = spot else {
            placed.set(false);
            px.to(f64::from(card_at.0));
            py.to(f64::from(card_at.1));
            return;
        };
        let parts = [
            (x, r.x - PAD),
            (y, r.y - PAD),
            (w, r.w + PAD * 2.0),
            (h, r.h + PAD * 2.0),
            (px, card_at.0),
            (py, card_at.1),
        ];
        let first = !*placed.peek();
        placed.set(true);
        for (m, to) in parts {
            if first {
                m.jump(f64::from(to));
            } else {
                m.to(f64::from(to));
            }
        }
    });
    // In as a tour starts, out as it ends.
    let on = tours.is_some_and(|t| (t.playing)().is_some());
    use_effect(move || {
        let on = tours.is_some_and(|t| (t.playing)().is_some());
        fade.to(if on { 1.0 } else { 0.0 });
    });
    let Some(tours) = tours else {
        return rsx! {};
    };
    let card = (tours.playing)().and_then(|p| {
        let list = tours.list.peek().clone();
        tours.set.with_peek(|s| card_of(s, &list, p))
    });
    let spot = (tours.spot)().filter(|_| card.is_some());
    let end = (tours.end)().filter(|_| card.is_some());
    let opacity = fade.value();
    let shown = on || opacity > 0.01;
    let (cx, cy, cw, ch) = (x.value(), y.value(), w.value(), h.value());
    let cut = if spot.is_some() { "block" } else { "none" };
    let card = card.unwrap_or_default();
    let has_card = shown && card.of > 0;
    let (ex, ey, ew, eh) = end.map_or((0.0, 0.0, 0.0, 0.0), |r| {
        (r.x - PAD, r.y - PAD, r.w + PAD * 2.0, r.h + PAD * 2.0)
    });
    let end_shown = if end.is_some() { "block" } else { "none" };
    let next_shown = if card.waiting { "none" } else { "block" };
    let wait_shown = if card.waiting { "block" } else { "none" };
    let hint_shown = if card.hint.is_empty() {
        "none"
    } else {
        "block"
    };
    let next_label = if card.last { "Done" } else { "Next" };
    let back_opacity = if card.first { 0.35 } else { 1.0 };
    let layer_display = if shown { "block" } else { "none" };
    let card_display = if has_card { "flex" } else { "none" };
    let (card_x, card_y) = (px.value(), py.value());
    rsx! {
        div {
            "data-testid": "guide-tour",
            style: "position: absolute; left: 0; top: 0; right: 0; bottom: 0; pointer-events: none; \
                    z-index: 200; display: {layer_display}; opacity: {opacity}; \
                    font-family: Inter, system-ui, -apple-system, sans-serif;",
            onmounted: move |e| tours.layer.clone().set(Some(e.data())),
            // The dim, around the cutout: four pieces, so the target shows
            // through untouched.
            div { style: "{DIM} display: {cut}; left: 0; top: 0; right: 0; height: {cy.max(0.0)}px;" }
            div { style: "{DIM} display: {cut}; left: 0; top: {cy + ch}px; right: 0; bottom: 0;" }
            div { style: "{DIM} display: {cut}; left: 0; top: {cy}px; width: {cx.max(0.0)}px; height: {ch}px;" }
            div { style: "{DIM} display: {cut}; left: {cx + cw}px; top: {cy}px; right: 0; height: {ch}px;" }
            div {
                "data-testid": "guide-tour-ring",
                style: "position: absolute; display: {cut}; left: {cx}px; top: {cy}px; width: {cw}px; height: {ch}px; \
                        box-sizing: border-box; border: 2px solid {ACCENT}; border-radius: 10px; \
                        box-shadow: 0 0 0 4px rgba(96,165,250,0.25);",
            }
            // A drag's end.
            div {
                style: "position: absolute; display: {end_shown}; left: {ex}px; top: {ey}px; width: {ew}px; height: {eh}px; \
                        box-sizing: border-box; border: 2px dashed {ACCENT}; border-radius: 10px;",
            }
            div {
                "data-testid": "guide-tour-popover",
                style: "position: absolute; left: {card_x}px; top: {card_y}px; width: {CARD_W}px; \
                        display: {card_display}; flex-direction: column; gap: 8px; \
                        box-sizing: border-box; padding: 14px 16px 12px; pointer-events: auto; \
                        background: #18181b; border: 1px solid #3f3f46; border-radius: 12px; \
                        box-shadow: 0 16px 48px rgba(0,0,0,0.55); color: #e4e4e7; font-size: 13px; line-height: 1.45;",
                onmounted: move |e| tours.popover.clone().set(Some(e.data())),
                div { style: "display: flex; align-items: center; gap: 8px; font-size: 11px; color: #a1a1aa;",
                    span { style: "flex: 1 1 0; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
                        "{card.title}"
                    }
                    span { style: "flex: none;", "{card.index} of {card.of}" }
                }
                div { style: "font-size: 14px; color: #fafafa;", "{card.text}" }
                div { style: "display: {hint_shown}; color: {ACCENT};", "{card.hint}" }
                div { style: "display: flex; align-items: center; gap: 8px; margin-top: 4px;",
                    button {
                        "data-testid": "guide-tour-skip",
                        style: "{GHOST}",
                        onclick: move |_| tours.stop(),
                        "Skip"
                    }
                    span { style: "flex: 1 1 0;" }
                    button {
                        "data-testid": "guide-tour-back",
                        style: "{GHOST} opacity: {back_opacity};",
                        onclick: move |_| tours.back(),
                        "Back"
                    }
                    span {
                        style: "display: {wait_shown}; color: #a1a1aa; font-size: 12px;",
                        "Waiting for you…"
                    }
                    button {
                        "data-testid": "guide-tour-next",
                        style: "display: {next_shown}; height: 28px; padding: 0 14px; border-radius: 7px; border: none; \
                                background: {ACCENT}; color: #0b1220; font-size: 12px; font-weight: 600;",
                        onclick: move |_| tours.next(),
                        "{next_label}"
                    }
                }
            }
        }
    }
}

/// The tour's blue, and its dim.
const ACCENT: &str = "#60a5fa";
const DIM: &str = "position: absolute; background: rgba(9,9,11,0.62);";
const GHOST: &str = "height: 28px; padding: 0 10px; border-radius: 7px; border: 1px solid #3f3f46; \
                     background: transparent; color: #d4d4d8; font-size: 12px;";

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(src: &str) -> Vec<Step> {
        crate::load(src).unwrap().steps
    }

    #[test]
    fn a_hidden_step_is_setup_and_is_not_counted() {
        let s = steps(
            "id x\nsteps ({id a, hide true, do @run{id GO}} {id b, text hi} {id c, do @click{}, target @id\"z\"})",
        );
        assert_eq!(place_of(&s, 0), (1, 2));
        assert_eq!(place_of(&s, 1), (1, 2));
        assert_eq!(place_of(&s, 2), (2, 2));
        assert_eq!(shown_before(&s, 2), Some(1));
        assert_eq!(shown_before(&s, 1), None);
    }

    #[test]
    fn going_to_a_chapter_runs_its_setup_again() {
        let s = steps(
            "id x\nsteps ({id a, hide true, do @run{id GO}} {id b, text hi} \
             {id c1, hide true, do @run{id GO}} {id c2, hide true, do @run{id GO}} {id c, text there})",
        );
        // Back from c lands on b, after its setup a.
        assert_eq!(shown_before(&s, 4).map(|at| setup_of(&s, at)), Some(0));
        // Starting at c starts at its setup, c1.
        assert_eq!(setup_of(&s, 4), 2);
        assert_eq!(setup_of(&s, 1), 0);
        assert_eq!(setup_of(&s, 0), 0);
    }

    #[test]
    fn a_move_is_a_step_shown_a_tour_left_or_finished() {
        let sc =
            crate::load("id x\nsteps ({id a, hide true, do @run{id GO}} {id b, text hi})").unwrap();
        let set = TourSet::new(vec![sc]);
        let at = |step| Some(Playing { scenario: 0, step });
        assert_eq!(moved_of(&set, None, at(0), false), None);
        assert_eq!(
            moved_of(&set, at(0), at(1), false),
            Some(Moved::At {
                scenario: "x".into(),
                step: "b".into()
            })
        );
        assert_eq!(
            moved_of(&set, at(1), None, false),
            Some(Moved::Left {
                scenario: "x".into(),
                step: "b".into()
            })
        );
        assert_eq!(
            moved_of(&set, at(1), None, true),
            Some(Moved::Finished {
                scenario: "x".into()
            })
        );
    }

    #[test]
    fn a_gesture_is_said_not_done() {
        let s = steps(
            "id x\nsteps (\
             {id a, target @tab\"t\", do @drag{to @group\"g\", edge right}}\
             {id b, target @split\"t\", do @drag{dx -120}}\
             {id c, do @key{key k, mods (meta shift)}}\
             {id d, do @type{text sched}}\
             {id e, do @run{id LAB_GO}}\
             {id f, do @run{id ELSEWHERE}}\
             {id g})",
        );
        let name_of = |id: &str| (id == "LAB_GO").then(|| ("Go".to_owned(), "⌘G".to_owned()));
        let said: Vec<String> = s.iter().map(|st| hint(st, name_of)).collect();
        assert_eq!(
            said,
            [
                "Drag what is highlighted onto the dashed outline.",
                "Drag what is highlighted left.",
                "Press ⇧⌘K.",
                "Type “sched”.",
                "Run “Go” (⌘G).",
                "",
                "",
            ]
        );
    }

    #[test]
    fn the_popover_sits_under_its_target_over_it_or_at_the_foot() {
        let area = (1000.0, 600.0);
        let card = (340.0, 120.0);
        let (x, y) = place(Some(Rect::new(100.0, 100.0, 80.0, 30.0)), card, area);
        assert!((x - GAP).abs() < 0.01, "kept inside the left edge: {x}");
        assert!(y > 130.0, "under it: {y}");
        let (_, y) = place(Some(Rect::new(500.0, 520.0, 80.0, 30.0)), card, area);
        assert!(y < 520.0, "over it: {y}");
        let (x, y) = place(None, card, area);
        assert!(
            (x - 330.0).abs() < 0.01 && y > 400.0,
            "at the foot: {x}, {y}"
        );
    }

    #[test]
    fn a_tour_not_marked_safe_needs_the_app_playground() {
        let s = crate::load("id x\nsteps ({id a})").unwrap();
        let none = TourSet::new(vec![s.clone()]);
        assert!(none.may_start(&s).unwrap_err().contains("playground"));
        let refused = TourSet::new(vec![s.clone()]).playground(|_| Err("busy".to_owned()));
        assert_eq!(refused.may_start(&s), Err("busy".to_owned()));
        let set = TourSet::new(vec![s.clone()]).playground(|_| Ok(()));
        assert_eq!(set.may_start(&s), Ok(()));
        let safe = crate::load("id y\nsafe true\nsteps ({id a})").unwrap();
        assert_eq!(TourSet::default().may_start(&safe), Ok(()));
    }
}
