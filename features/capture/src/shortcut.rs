//! The desk's side (feature `native`): the system-wide shortcut that
//! opens the capture window from any app, and the window itself raised
//! over every app where the person is looking.
//!
//! The key is the platform's hot key (`global-hotkey`: Carbon's on macOS,
//! `RegisterHotKey` on Windows, an X11 grab on Linux), registered from the
//! main window, on the event loop's thread as macOS requires; a press
//! comes to the window's thread on a channel that wakes it. Wayland has
//! no global keys: the registration fails and says so, and the tray item
//! and ⇧⌘N still open the composer.

use std::rc::Rc;
use std::sync::Arc;

use dioxus::prelude::*;
use dioxus_native::winit::dpi::{LogicalSize, PhysicalPosition};
use dioxus_native::winit::window::{Window as OsWindow, WindowLevel};
use futures_util::StreamExt as _;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::ui::WINDOW_SIZE;

/// The capture window's key when the app names none: ⌃⌥N (macOS keeps
/// ⌃Space and ⌥⌘Space for itself).
pub const DEFAULT: &str = "control+alt+KeyN";

/// Registers `spec` (`"control+alt+KeyN"`, `"shift+super+Space"`: the
/// modifiers and a key code) system-wide for as long as the calling
/// component lives; `on_press` runs on each press. The registration's
/// failure, if any (a key another app holds, a Wayland session).
pub fn use_global_shortcut(spec: &str, on_press: Callback<()>) -> Option<String> {
    let pressed = use_coroutine(move |mut rx: UnboundedReceiver<()>| async move {
        while rx.next().await.is_some() {
            on_press.call(());
        }
    });
    let held = use_hook(|| {
        let tx = pressed.tx();
        register(spec, move || {
            let _ = tx.unbounded_send(());
        })
        .map(Rc::new)
        .map_err(|e| format!("the capture shortcut {spec}: {e}"))
    });
    {
        let held = held.clone();
        use_drop(move || {
            if let Ok(h) = held {
                let _ = h.manager.unregister(h.key);
                GlobalHotKeyEvent::set_event_handler(None::<fn(GlobalHotKeyEvent)>);
            }
        });
    }
    held.err()
}

/// A registered key: dropped, the platform lets it go.
struct Held {
    manager: GlobalHotKeyManager,
    key: HotKey,
}

fn register(spec: &str, press: impl Fn() + Send + Sync + 'static) -> Result<Held, String> {
    let key: HotKey = spec.parse().map_err(|e| format!("{e}"))?;
    let manager = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
    manager.register(key).map_err(|e| e.to_string())?;
    let id = key.id();
    GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
        if e.id() == id && e.state() == HotKeyState::Pressed {
            press();
        }
    }));
    Ok(Held { manager, key })
}

/// In the capture window's document: the window over every app, its
/// size, a third of the way down the screen it is on, and the keys.
pub fn use_float_window() {
    let window = try_consume_context::<Arc<dyn OsWindow>>();
    use_hook(move || {
        let Some(w) = window else { return };
        w.set_window_level(WindowLevel::AlwaysOnTop);
        let (width, height) = WINDOW_SIZE;
        let _ = w.request_surface_size(LogicalSize::new(width, height).into());
        if let Some(screen) = w.current_monitor().or_else(|| w.primary_monitor())
            && let (Some(at), Some(mode)) = (screen.position(), screen.current_video_mode())
        {
            let scale = screen.scale_factor();
            let size = mode.size();
            let (sw, sh) = (f64::from(size.width), f64::from(size.height));
            let x = f64::from(at.x) + (sw - width * scale) / 2.0;
            let y = f64::from(at.y) + sh / 4.0;
            // Screen pixels fit an i32.
            #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
            w.set_outer_position(PhysicalPosition::new(x as i32, y as i32).into());
        }
        w.focus_window();
    });
}
