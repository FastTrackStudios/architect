//! The platforms' side (feature `native`): the window shown, hidden and
//! focused through the workspace's launcher, the app's one instance, the
//! login item; the Dock and the menu bar on macOS (`mac`), the tray on
//! Linux (`linux`). Where a platform has no such thing yet, it says so
//! once, with what it cannot do.

use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};

use architect_workspace::native::MainWindow;
use dioxus::prelude::*;
use dioxus_native::winit::window::Window;

use crate::instance::{Claim, Handoff};
use crate::lifecycle::Effect;
use crate::login::Applied;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
use crate::model::{Attention, MenuTree};
use crate::platform::Platform;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
use crate::platform::{Events, TrayView};

/// The hand-offs [`single_instance`] took, for the presence to hear.
static HANDOFFS: Mutex<Option<futures_channel::mpsc::UnboundedReceiver<Handoff>>> =
    Mutex::new(None);

/// What [`single_instance`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    /// This is the app's one instance: run.
    First,
    /// Another was running and took this launch's arguments: end `main`.
    HandedOff,
}

/// Claims app `id`'s one instance, before anything opens.
///
/// A second launch hands its arguments (and URLs) to the running one,
/// which comes forward, and gets [`Launch::HandedOff`] back to end on. Where there is no
/// instance socket (Windows, or none could be made) every launch runs, and
/// the log says why.
#[must_use]
pub fn single_instance(id: &str) -> Launch {
    match crate::instance::claim(id, &Handoff::this_process()) {
        Ok(Claim::Running) => {
            tracing::info!(
                presence.instance = "handed_off",
                "presence: the app is running already"
            );
            Launch::HandedOff
        }
        Ok(Claim::First { handoffs }) => {
            *HANDOFFS.lock().unwrap_or_else(PoisonError::into_inner) = Some(handoffs);
            Launch::First
        }
        Err(error) => {
            tracing::warn!(presence.instance = "unclaimed", %error, "presence: no single instance");
            Launch::First
        }
    }
}

/// The hand-offs, once: the presence hears them.
pub fn take_handoffs() -> Option<futures_channel::mpsc::UnboundedReceiver<Handoff>> {
    HANDOFFS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
}

/// The main window, from the launcher (or dioxus-native's window alone).
#[derive(Clone)]
pub struct WindowOps {
    main: Option<MainWindow>,
    window: Option<Arc<dyn Window>>,
}

impl WindowOps {
    /// The ones in context (call inside a component).
    pub fn here() -> Self {
        Self {
            main: try_consume_context::<MainWindow>(),
            window: try_consume_context::<Arc<dyn Window>>(),
        }
    }

    /// The OS window, once it is open.
    pub fn os_window(&self) -> Option<Arc<dyn Window>> {
        self.main
            .as_ref()
            .and_then(MainWindow::window)
            .or_else(|| self.window.clone())
    }

    /// Carries out a window effect; false for one that is not the window's
    /// (the Dock icon).
    pub fn apply(&self, effect: Effect) -> bool {
        match effect {
            Effect::ShowWindow => {
                if let Some(w) = self.os_window() {
                    w.set_visible(true);
                    w.request_redraw();
                }
            }
            Effect::HideWindow => {
                if let Some(w) = self.os_window() {
                    w.set_visible(false);
                }
            }
            Effect::Focus => {
                if let Some(w) = self.os_window() {
                    w.set_minimized(false);
                    w.focus_window();
                }
            }
            // The launcher closes it itself.
            Effect::Close => {}
            Effect::Quit => match &self.main {
                Some(main) => main.quit(),
                None => return false,
            },
            Effect::DockIcon(_) => return false,
        }
        true
    }
}

/// This platform's.
pub fn platform() -> Rc<dyn Platform> {
    let window = WindowOps::here();
    #[cfg(target_os = "macos")]
    return Rc::new(crate::mac::Mac::new(window));
    #[cfg(target_os = "linux")]
    return Rc::new(crate::linux::Linux::new(window));
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    Rc::new(Basic { window })
}

/// Where there is no Dock or tray here yet (Windows): the window's life
/// and the login item work; the rest says it is not built.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
struct Basic {
    window: WindowOps,
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
impl Platform for Basic {
    fn start(&self, _events: Events) {
        tracing::warn!(
            presence.tray = "unsupported",
            "presence: no tray item, badge or taskbar menu on this platform yet (the notification area is not built)"
        );
    }
    fn set_dock_menu(&self, _menu: Option<MenuTree>) {}
    fn set_tray(&self, _tray: Option<TrayView>) {}
    fn set_badge(&self, _label: Option<String>) {}
    fn set_progress(&self, _value: Option<f64>) {}
    fn request_attention(&self, _attention: Attention) {
        if let Some(w) = self.window.os_window() {
            w.request_user_attention(Some(
                dioxus_native::winit::window::UserAttentionType::Informational,
            ));
        }
    }
    fn apply(&self, effect: Effect) {
        let _ = self.window.apply(effect);
    }
    fn login_available(&self) -> Result<(), String> {
        crate::login::available()
    }

    fn set_login(&self, id: &str, name: &str, on: bool) -> Result<Applied, String> {
        apply_login(id, name, on)
    }
}

/// The login item written or removed, and said in the log.
pub fn apply_login(id: &str, name: &str, on: bool) -> Result<Applied, String> {
    let outcome = crate::login::apply(id, name, on);
    match &outcome {
        Ok(Applied::Written(path)) => {
            tracing::info!(presence.login = "written", path = %path.display(), "presence: opens at login");
        }
        Ok(Applied::Removed(path)) => {
            tracing::info!(presence.login = "removed", path = %path.display(), "presence: no longer opens at login");
        }
        Ok(Applied::Unchanged) => {}
        Err(why) => {
            tracing::info!(presence.login = "refused", reason = %why, "presence: not opening at login");
        }
    }
    outcome
}
