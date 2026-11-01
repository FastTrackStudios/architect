//! What the platform does for an app's presence (feature `ui`): the
//! [`Platform`] the component drives, and what it hears back. The
//! native one is feature `native`'s; [`Recorder`] keeps every call, for a
//! test or a headless run.

use std::cell::RefCell;
use std::rc::Rc;

use architect_commands::Pick;

use crate::instance::Handoff;
use crate::lifecycle::Effect;
use crate::login::Applied;
use crate::model::{Attention, MenuTree, Tray};

/// What the platform tells the app, on the window's thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Heard {
    /// A Dock or tray menu row was picked.
    Pick(Pick),
    /// The Dock icon was clicked; `visible` when a window is on screen.
    Reopen { visible: bool },
    /// The tray item was clicked (its [`TrayClick::Run`](crate::TrayClick::Run)).
    TrayClicked,
    /// A second launch, or URLs to open.
    Handoff(Handoff),
    /// The close hid the window (the state moved; this says so to the UI).
    Hidden,
    /// An Action List entry asked.
    Ask(Ask),
}

/// What an Action List entry asks (its handler runs on any thread, so it
/// asks the window's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    Show,
    Hide,
    Quit,
    ToggleBackground,
    ToggleLogin,
}

/// Where the platform sends what it hears.
pub type Events = futures_channel::mpsc::UnboundedSender<Heard>;

/// The tray item as drawn: its look and its menu now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayView {
    pub tray: Tray,
    pub menu: MenuTree,
}

/// The platform's side of an app's presence. Every method is called on
/// the window's thread.
pub trait Platform {
    /// Start hearing the platform (the Dock's clicks and menu, the tray's,
    /// URLs to open): what it hears goes to `events`.
    fn start(&self, events: Events);
    /// What right-clicking the Dock icon lists; `None` for the platform's
    /// own items alone.
    fn set_dock_menu(&self, menu: Option<MenuTree>);
    /// The tray item, or none.
    fn set_tray(&self, tray: Option<TrayView>);
    /// The badge over the Dock icon; `None` for none.
    fn set_badge(&self, label: Option<String>);
    /// Progress on the Dock tile, `0.0`–`1.0`; `None` for none.
    fn set_progress(&self, value: Option<f64>);
    fn request_attention(&self, attention: Attention);
    /// Carry out what the window's state machine says.
    fn apply(&self, effect: Effect);
    /// Open the app at login (`on`) or not; Err with why it cannot.
    fn set_login(&self, id: &str, name: &str, on: bool) -> Result<Applied, String>;
    /// Whether this copy can open at login (an installed one); Err with why
    /// not, which the toggle shows.
    fn login_available(&self) -> Result<(), String>;
}

/// The platform in context: what [`AppPresence`](crate::AppPresence)
/// drives when one is given (a test's [`Recorder`]); else the native one.
#[derive(Clone)]
pub struct UsePlatform(pub Rc<dyn Platform>);

/// One call a [`Recorder`] kept.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    Start,
    DockMenu(Option<MenuTree>),
    Tray(Option<TrayView>),
    Badge(Option<String>),
    Progress(Option<f64>),
    Attention(Attention),
    Apply(Effect),
    Login { on: bool },
}

/// A platform that keeps every call (and does nothing): a test reads them
/// back. Login items are refused with `refuse_login`, when set.
#[derive(Clone, Default)]
pub struct Recorder {
    pub calls: Rc<RefCell<Vec<Call>>>,
    pub events: Rc<RefCell<Option<Events>>>,
    pub refuse_login: Option<String>,
}

impl Recorder {
    /// Every call so far.
    #[must_use]
    pub fn calls(&self) -> Vec<Call> {
        self.calls.borrow().clone()
    }

    /// Says `heard` to the app, as the platform would.
    pub fn hear(&self, heard: Heard) {
        if let Some(tx) = self.events.borrow().as_ref() {
            let _ = tx.unbounded_send(heard);
        }
    }

    fn keep(&self, call: Call) {
        self.calls.borrow_mut().push(call);
    }
}

impl Platform for Recorder {
    fn start(&self, events: Events) {
        *self.events.borrow_mut() = Some(events);
        self.keep(Call::Start);
    }

    fn set_dock_menu(&self, menu: Option<MenuTree>) {
        self.keep(Call::DockMenu(menu));
    }

    fn set_tray(&self, tray: Option<TrayView>) {
        self.keep(Call::Tray(tray));
    }

    fn set_badge(&self, label: Option<String>) {
        self.keep(Call::Badge(label));
    }

    fn set_progress(&self, value: Option<f64>) {
        self.keep(Call::Progress(value));
    }

    fn request_attention(&self, attention: Attention) {
        self.keep(Call::Attention(attention));
    }

    fn apply(&self, effect: Effect) {
        self.keep(Call::Apply(effect));
    }

    fn login_available(&self) -> Result<(), String> {
        self.refuse_login.clone().map_or(Ok(()), Err)
    }

    fn set_login(&self, _id: &str, _name: &str, on: bool) -> Result<Applied, String> {
        self.keep(Call::Login { on });
        match &self.refuse_login {
            Some(why) if on => Err(why.clone()),
            _ => Ok(Applied::Unchanged),
        }
    }
}
