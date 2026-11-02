//! The app's presence, bound (feature `ui`): [`AppPresence`] draws the
//! declared Dock menu and tray item from the commands, keeps the badge
//! and progress the app gives it, runs the window's
//! [`Lifecycle`](crate::Lifecycle) and puts every part of it on the Action
//! List. Mount it once, under the commands and the settings.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use architect::action::{ActionList, ActionMeta, Effect as ActionEffect};
use architect_commands::{try_use_commands, use_action_list};
use architect_settings::{Control, Section, Setting, Value, try_use_settings, use_section};
use dioxus::prelude::*;
use facet::Facet;
use futures_util::StreamExt as _;

use crate::instance::Handoff;
use crate::lifecycle::{Effect, Event, Lifecycle, Place};
use crate::model::{
    Attention, Badge, HIDE_WINDOW, KEEP_RUNNING_SETTING, MenuTree, OPEN_AT_LOGIN_SETTING, Presence,
    QUIT, SHOW_WINDOW, STATE, TOGGLE_BACKGROUND, TOGGLE_LOGIN, progress as clamp,
};
use crate::platform::{Ask, Events, Heard, Platform, TrayView, UsePlatform};

/// How often the Action Lists' events are looked at, for the menus' ticks.
const POLL: Duration = Duration::from_millis(100);

/// What [`STATE`] answers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet)]
pub struct PresenceState {
    /// `window` or `background`.
    pub place: String,
    /// What the Dock's badge says; empty for none.
    pub badge: String,
    pub keep_running: bool,
    pub open_at_login: bool,
    /// Why the app cannot open at login, when it cannot.
    pub login_problem: String,
}

/// The presence in context: to show or hide the window, quit, or ask for
/// attention from the app's own code.
#[derive(Clone, Copy)]
pub struct PresenceHandle {
    tx: Coroutine<Heard>,
    platform: CopyValue<Rc<dyn Platform>>,
    /// Where the app is now.
    pub place: Signal<Place>,
}

impl PresenceHandle {
    /// The window back, the Dock icon with it, in front.
    pub fn open(&self) {
        self.tx.send(Heard::Ask(Ask::Show));
    }

    /// The window away; the app keeps running.
    pub fn hide(&self) {
        self.tx.send(Heard::Ask(Ask::Hide));
    }

    pub fn quit(&self) {
        self.tx.send(Heard::Ask(Ask::Quit));
    }

    /// Bounce the Dock icon (once, or until the app comes forward).
    pub fn request_attention(&self, attention: Attention) {
        self.platform.read().request_attention(attention);
    }

    /// Says `heard` as the platform would: a test's Dock click.
    pub fn hear(&self, heard: Heard) {
        self.tx.send(heard);
    }
}

/// The presence above, if any.
#[must_use]
pub fn try_use_presence() -> Option<PresenceHandle> {
    try_use_context()
}

/// The settings a presence adds (Settings › App): keep running when the
/// window closes, open at login. `None` when it declares neither.
#[must_use]
pub fn settings_section(presence: &Presence) -> Option<Section> {
    let mut settings = Vec::new();
    if let Some(on) = presence.keep_running {
        settings.push(
            Setting::new(
                KEEP_RUNNING_SETTING,
                "Keep running when the window closes",
                Control::Switch,
                Value::Bool(on),
            )
            .describe(format!(
                "{} stays in the menu bar with no window and no Dock icon; Quit quits.",
                presence.name
            ))
            .aliases(["background", "menu bar", "tray", "close"]),
        );
    }
    if presence.login {
        settings.push(
            Setting::new(
                OPEN_AT_LOGIN_SETTING,
                "Open at login",
                Control::Switch,
                Value::Bool(false),
            )
            .describe(format!(
                "{} starts when you log in, in the menu bar, with no window.",
                presence.name
            ))
            .aliases(["startup", "login item", "autostart"]),
        );
    }
    (!settings.is_empty()).then(|| {
        Section::new("app", "App")
            .icon("app-window")
            .block(None, settings)
    })
}

/// Whether this launch was asked to stay in the background (a login
/// launch: `--background`).
#[must_use]
pub fn launched_in_background() -> bool {
    std::env::args().any(|a| a == crate::model::BACKGROUND_ARG)
}

static SHOW_META: ActionMeta = meta(
    SHOW_WINDOW,
    "Show the window",
    "Brings the main window back where it was, with the Dock icon, in front.",
    false,
);
static HIDE_META: ActionMeta = meta(
    HIDE_WINDOW,
    "Close the window, keep running",
    "Hides the window and the Dock icon; the app stays in the menu bar.",
    false,
);
static QUIT_META: ActionMeta = meta(
    QUIT,
    "Quit",
    "Quits the app, its window hidden or not.",
    false,
);
static BACKGROUND_META: ActionMeta = meta(
    TOGGLE_BACKGROUND,
    "Keep running when the window closes",
    "On, closing the window leaves the app in the menu bar; off, it quits.",
    true,
);
static LOGIN_META: ActionMeta = meta(
    TOGGLE_LOGIN,
    "Open at login",
    "Starts the app at login, in the menu bar with no window (an installed copy only).",
    true,
);
static STATE_META: ActionMeta = meta(
    STATE,
    "App presence state",
    "Where the app is (window or background), its badge, and whether it keeps running and opens at login.",
    false,
);

const fn meta(
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    toggleable: bool,
) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "",
        method_name: "",
        display_name,
        description,
        category: "App",
        group: "App",
        shortcut: "",
        toggleable,
        undo: false,
    }
}

/// What the Action List's handlers read (any thread).
#[derive(Default)]
struct Mirror {
    keep_running: AtomicBool,
    open_at_login: AtomicBool,
    login_problem: Mutex<Option<String>>,
    state: Mutex<PresenceState>,
}

/// The app's presence while mounted: its Dock menu, tray item, badge and
/// progress, the window's life, and the Action List entries
/// ([`SHOW_WINDOW`], [`HIDE_WINDOW`], [`QUIT`], [`TOGGLE_BACKGROUND`],
/// [`TOGGLE_LOGIN`], [`STATE`]).
///
/// `badge` and `progress` are bound: give what the app has now each time
/// it draws (`Badge::count(inbox.len())`). `on_handoff` hears a second
/// launch's arguments, after the window has come forward. `background`
/// starts it with no window (default: the launch's `--background`).
#[component]
pub fn AppPresence(
    presence: Presence,
    #[props(default)] badge: Badge,
    #[props(default)] progress: Option<f64>,
    #[props(default)] on_handoff: Option<EventHandler<Handoff>>,
    #[props(default)] background: Option<bool>,
) -> Element {
    let commands = try_use_commands();
    let settings = try_use_settings();
    // The app's Action List (its own when there is none in context),
    // listed as commands so the menus and ⌘K reach the presence's entries.
    let list = use_hook(|| {
        try_consume_context::<Arc<ActionList>>().unwrap_or_else(|| Arc::new(ActionList::new()))
    });
    use_action_list(list.clone());
    let platform: Rc<dyn Platform> =
        use_hook(|| try_consume_context::<UsePlatform>().map_or_else(native, |p| p.0));
    let section = use_hook({
        let presence = presence.clone();
        move || settings_section(&presence)
    });
    if let Some(section) = section {
        use_section(move || section);
    }
    // The person's settings, or this presence's own when there are none.
    let mut own_keep = use_signal(|| presence.keep_running.unwrap_or(false));
    let mut own_login = use_signal(|| false);
    let setting = |key: &str| {
        settings.and_then(|s| {
            let registry = s.registry.read();
            s.values
                .read()
                .get(&registry, key)
                .and_then(|v| v.as_bool())
        })
    };
    let keep_running = presence.keep_running.is_some()
        && setting(KEEP_RUNNING_SETTING).unwrap_or_else(|| own_keep());
    let open_at_login =
        presence.login && setting(OPEN_AT_LOGIN_SETTING).unwrap_or_else(|| own_login());

    let lifecycle = use_hook(|| Rc::new(RefCell::new(Lifecycle::new(keep_running))));
    let mut place = use_signal(|| Place::Window);
    let mirror = use_hook(|| Arc::new(Mirror::default()));
    let mut login_problem = use_signal(|| {
        presence
            .login
            .then(|| platform.login_available().err())
            .flatten()
    });

    let tx = use_coroutine({
        let lifecycle = lifecycle.clone();
        let platform = platform.clone();
        let list = list.clone();
        let presence = presence.clone();
        let mirror = mirror.clone();
        move |mut heard: UnboundedReceiver<Heard>| {
            let lifecycle = lifecycle.clone();
            let platform = platform.clone();
            let list = list.clone();
            let presence = presence.clone();
            let mirror = mirror.clone();
            async move {
                let step = |event: Event| {
                    let mut place = place;
                    let effects = lifecycle.borrow_mut().handle(event);
                    for effect in effects {
                        platform.apply(effect);
                    }
                    let now = lifecycle.borrow().place;
                    if *place.peek() != now {
                        place.set(now);
                    }
                };
                while let Some(h) = heard.next().await {
                    match h {
                        Heard::Pick(pick) => {
                            if let Some(c) = commands {
                                let _ = c.run_pick(&pick);
                            }
                        }
                        Heard::Reopen { visible } => step(Event::Reopen { visible }),
                        Heard::TrayClicked => {
                            let run = presence.tray.as_ref().and_then(|t| match &t.click {
                                crate::TrayClick::Run(id) => Some(id.clone()),
                                crate::TrayClick::Menu => None,
                            });
                            if let (Some(c), Some(id)) = (commands, run) {
                                let _ = c.run(&id);
                            }
                        }
                        Heard::Handoff(h) => {
                            if !h.background() {
                                step(Event::Open);
                            }
                            if let Some(f) = on_handoff {
                                f.call(h);
                            }
                        }
                        Heard::Hidden => {
                            let now = lifecycle.borrow().place;
                            let mut place = place;
                            place.set(now);
                        }
                        Heard::Ask(Ask::Show) => step(Event::Open),
                        Heard::Ask(Ask::Hide) => step(Event::Hide),
                        Heard::Ask(Ask::Quit) => step(Event::Quit),
                        Heard::Ask(Ask::ToggleBackground) => {
                            let on = !lifecycle.borrow().keep_running;
                            match settings {
                                Some(s) => s.set(KEEP_RUNNING_SETTING, Value::Bool(on)),
                                None => own_keep.set(on),
                            }
                        }
                        Heard::Ask(Ask::ToggleLogin) => {
                            let on = !mirror.open_at_login.load(Ordering::Acquire);
                            match settings {
                                Some(s) => s.set(OPEN_AT_LOGIN_SETTING, Value::Bool(on)),
                                None => own_login.set(on),
                            }
                        }
                    }
                    list.changed("presence", "");
                }
            }
        }
    });
    let handle = use_context_provider(|| PresenceHandle {
        tx,
        platform: CopyValue::new(platform.clone()),
        place,
    });
    let _ = handle;

    // Started once: the platform heard, the launch's place taken, the
    // window's close asked of the state machine, the one instance's
    // hand-offs heard.
    use_hook({
        let platform = platform.clone();
        let lifecycle = lifecycle.clone();
        let wanted = background.unwrap_or_else(launched_in_background);
        move || {
            let events: Events = tx.tx();
            platform.start(events.clone());
            let effects = lifecycle
                .borrow_mut()
                .handle(Event::Launched { background: wanted });
            for effect in effects {
                platform.apply(effect);
            }
            place.set(lifecycle.borrow().place);
            hear_close(&lifecycle, &platform, &events);
            hear_handoffs(events);
        }
    });

    // Background mode follows the setting.
    if lifecycle.borrow().keep_running != keep_running {
        let _ = lifecycle
            .borrow_mut()
            .handle(Event::KeepRunning(keep_running));
    }

    // The menus, resolved again when the commands change or the Action
    // Lists' events move on (a tick read again).
    let mut heard = use_signal(|| 0_u64);
    use_future(move || async move {
        loop {
            futures_timer::Delay::new(POLL).await;
            if let Some(c) = commands {
                let now = c.heard();
                if *heard.peek() != now {
                    heard.set(now);
                }
            }
        }
    });
    let _ = heard();
    let all = commands.map(|c| c.all()).unwrap_or_default();
    let dock = presence
        .dock_menu
        .as_ref()
        .map(|m| MenuTree::resolve(m, &all));
    let tray = presence.tray.as_ref().map(|t| TrayView {
        tray: t.clone(),
        menu: MenuTree::resolve(&t.menu, &all),
    });
    let label = badge.label();
    let value = clamp(progress);
    let drawn = use_hook(|| Rc::new(RefCell::new(Drawn::default())));
    {
        let mut d = drawn.borrow_mut();
        let first = !d.sent;
        d.sent = true;
        if first || d.dock != dock {
            platform.set_dock_menu(dock.clone());
            d.dock = dock;
        }
        if first || d.tray != tray {
            platform.set_tray(tray.clone());
            d.tray = tray;
        }
        if first || d.badge != label {
            platform.set_badge(label.clone());
            d.badge.clone_from(&label);
        }
        if first || d.progress != value {
            platform.set_progress(value);
            d.progress = value;
        }
        // The login item: written on, removed off, and written again at
        // each launch while on (the app may have moved).
        if presence.login && d.login != Some(open_at_login) {
            d.login = Some(open_at_login);
            let outcome = platform.set_login(&presence.id, &presence.name, open_at_login);
            if let Err(why) = outcome {
                // Not written: the toggle goes back off, greyed with why.
                login_problem.set(Some(why));
                if open_at_login {
                    d.login = Some(false);
                    match settings {
                        Some(s) => s.set(OPEN_AT_LOGIN_SETTING, Value::Bool(false)),
                        None => own_login.set(false),
                    }
                }
            }
        }
    }

    // What the Action List's handlers read.
    mirror.keep_running.store(keep_running, Ordering::Release);
    mirror.open_at_login.store(open_at_login, Ordering::Release);
    let problem = login_problem();
    mirror
        .login_problem
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone_from(&problem);
    let state = PresenceState {
        place: match place() {
            Place::Window => "window".to_owned(),
            Place::Background => "background".to_owned(),
        },
        badge: label.unwrap_or_default(),
        keep_running,
        open_at_login,
        login_problem: problem.unwrap_or_default(),
    };
    {
        let mut kept = mirror.state.lock().unwrap_or_else(PoisonError::into_inner);
        if *kept != state {
            *kept = state;
            drop(kept);
            list.changed("presence", "");
        }
    }
    let registered = use_hook(move || {
        register(&list, &presence, &mirror, &tx.tx());
        list
    });
    use_drop(move || {
        for id in [
            SHOW_WINDOW,
            HIDE_WINDOW,
            QUIT,
            TOGGLE_BACKGROUND,
            TOGGLE_LOGIN,
            STATE,
        ] {
            registered.unregister(id);
        }
    });
    rsx! {}
}

/// What the platform was last given.
#[derive(Default)]
struct Drawn {
    /// Nothing given yet: everything is, the first time.
    sent: bool,
    dock: Option<MenuTree>,
    tray: Option<TrayView>,
    badge: Option<String>,
    progress: Option<f64>,
    login: Option<bool>,
}

/// The presence's entries on `list`. Each handler asks the window's
/// thread through `tx`.
fn register(list: &ActionList, presence: &Presence, mirror: &Arc<Mirror>, tx: &Events) {
    let ask = |meta: &'static ActionMeta, ask: Ask| {
        let tx = tx.clone();
        let _ = list.register(meta, ActionEffect::Mutating, move |(): ()| {
            tx.unbounded_send(Heard::Ask(ask))
                .map_err(|_| "the window is gone".to_owned())
        });
    };
    ask(&SHOW_META, Ask::Show);
    ask(&QUIT_META, Ask::Quit);
    if presence.keep_running.is_some() {
        ask(&HIDE_META, Ask::Hide);
        ask(&BACKGROUND_META, Ask::ToggleBackground);
        let m = mirror.clone();
        let _ = list.set_checked(TOGGLE_BACKGROUND, move || {
            m.keep_running.load(Ordering::Acquire)
        });
    }
    if presence.login {
        ask(&LOGIN_META, Ask::ToggleLogin);
        let m = mirror.clone();
        let _ = list.set_checked(TOGGLE_LOGIN, move || {
            m.open_at_login.load(Ordering::Acquire)
        });
        let m = mirror.clone();
        let _ = list.set_enabled(TOGGLE_LOGIN, move || {
            // Off can always be asked; on, only where it can be written.
            let on = m.open_at_login.load(Ordering::Acquire);
            let problem = m
                .login_problem
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            match problem {
                Some(why) if !on => Err(why),
                _ => Ok(()),
            }
        });
    }
    let m = mirror.clone();
    let _ = list.register(&STATE_META, ActionEffect::ReadOnly, move |(): ()| {
        Ok(m.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    });
}

/// The window's close asks the state machine (feature `native`, under
/// the workspace's launcher): kept (hidden) in background mode, else it
/// closes and the app quits.
#[cfg(feature = "native")]
fn hear_close(lifecycle: &Rc<RefCell<Lifecycle>>, platform: &Rc<dyn Platform>, events: &Events) {
    let Some(main) = try_consume_context::<architect_workspace::native::MainWindow>() else {
        return;
    };
    let lifecycle = lifecycle.clone();
    let platform = platform.clone();
    let events = events.clone();
    main.on_close(Some(Rc::new(move || {
        close_requested(&lifecycle, platform.as_ref(), &events)
    })));
}

#[cfg(not(feature = "native"))]
const fn hear_close(_: &Rc<RefCell<Lifecycle>>, _: &Rc<dyn Platform>, _: &Events) {}

/// The person closed the window: `true` when it was kept (hidden).
#[must_use]
pub fn close_requested(
    lifecycle: &RefCell<Lifecycle>,
    platform: &dyn Platform,
    events: &Events,
) -> bool {
    let effects = lifecycle.borrow_mut().handle(Event::CloseRequested);
    if effects.contains(&Effect::Close) {
        return false;
    }
    for effect in effects {
        platform.apply(effect);
    }
    let _ = events.unbounded_send(Heard::Hidden);
    true
}

/// The one instance's hand-offs ([`crate::claim`] in `main`), heard.
#[cfg(feature = "native")]
fn hear_handoffs(events: Events) {
    let Some(mut handoffs) = crate::native::take_handoffs() else {
        return;
    };
    spawn(async move {
        while let Some(h) = handoffs.next().await {
            if events.unbounded_send(Heard::Handoff(h)).is_err() {
                return;
            }
        }
    });
}

#[cfg(not(feature = "native"))]
fn hear_handoffs(_: Events) {}

/// The platform this is built for.
#[cfg(feature = "native")]
fn native() -> Rc<dyn Platform> {
    crate::native::platform()
}

/// Built without `native`: nothing drawn, every call kept.
#[cfg(not(feature = "native"))]
fn native() -> Rc<dyn Platform> {
    Rc::new(crate::platform::Recorder::default())
}
