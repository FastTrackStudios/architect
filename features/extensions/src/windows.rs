//! Extensions' windows (feature `host`).
//!
//! Which `[[windows]]` are open, with what state, for the app's window
//! machinery to draw (`crate::window_ui::ExtensionWindows`, feature `ui`):
//! a sheet over the main window, or an OS window of its own.
//!
//! A window opens through the action its manifest names (`opens`, whose
//! handler is the host's: its arguments are the window's state), through
//! the host service (`window {id, open}`: an extension's own), or
//! [`crate::host::Host::open_window`]; it closes the same ways, by the
//! person (a click outside a sheet, its own close, an OS window closed), and when its extension
//! unloads.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::Serialize;
use serde_json::Value;

use crate::interop::WindowDecl;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// An open window.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OpenWindow {
    pub extension: String,
    pub window: WindowDecl,
    /// What it was opened with (the opening action's arguments).
    pub state: Value,
    /// How many times it has been opened: a window opened again while open
    /// is told so (a section to show).
    pub opened: u64,
}

impl OpenWindow {
    /// Its key: `<extension>/<window id>`.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}/{}", self.extension, self.window.id)
    }
}

type Listener = Arc<dyn Fn() + Send + Sync>;

/// The open windows, and who listens. Cheap to share inside the host.
#[derive(Default)]
pub struct Windows {
    open: Mutex<BTreeMap<String, OpenWindow>>,
    listeners: Mutex<Vec<Listener>>,
    serial: Mutex<u64>,
}

impl std::fmt::Debug for Windows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Windows")
            .field("open", &lock(&self.open).keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Windows {
    fn tell(&self) {
        let listeners = lock(&self.listeners).clone();
        for l in listeners {
            l();
        }
    }

    /// Opens (or opens again, with new state) `window` of `extension`.
    pub fn open(&self, extension: &str, window: &WindowDecl, state: Value) {
        let opened = {
            let mut serial = lock(&self.serial);
            *serial = serial.saturating_add(1);
            *serial
        };
        lock(&self.open).insert(
            format!("{extension}/{}", window.id),
            OpenWindow {
                extension: extension.to_owned(),
                window: window.clone(),
                state,
                opened,
            },
        );
        self.tell();
    }

    /// Closes `<extension>/<id>`: whether it was open.
    pub fn close(&self, extension: &str, id: &str) -> bool {
        let was = lock(&self.open)
            .remove(&format!("{extension}/{id}"))
            .is_some();
        if was {
            self.tell();
        }
        was
    }

    /// Closes every window of `extension`.
    pub fn close_all(&self, extension: &str) {
        let before = lock(&self.open).len();
        lock(&self.open).retain(|_, w| w.extension != extension);
        if lock(&self.open).len() != before {
            self.tell();
        }
    }

    /// The open windows, by key.
    #[must_use]
    pub fn open_now(&self) -> Vec<OpenWindow> {
        lock(&self.open).values().cloned().collect()
    }

    /// Whether `<extension>/<id>` is open.
    #[must_use]
    pub fn is_open(&self, extension: &str, id: &str) -> bool {
        lock(&self.open).contains_key(&format!("{extension}/{id}"))
    }

    /// Calls `listener` on every change (from any thread).
    pub fn listen(&self, listener: impl Fn() + Send + Sync + 'static) {
        lock(&self.listeners).push(Arc::new(listener));
    }
}
