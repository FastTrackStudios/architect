//! The store in a Dioxus app (feature `dioxus`): given once at the root
//! with [`use_theme_store`], read anywhere below with [`use_theme`],
//! [`use_resolved`] or [`use_themed`] — a typed slice that changes only
//! when its own part of the theme does.

use dioxus::prelude::*;
use futures_util::StreamExt;

use crate::model::Scheme;
use crate::resolve::{FromResolved, Resolved};
use crate::store::ThemeStore;

/// The app's theme store and the theme showing, as a signal.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ThemeHandle {
    store: CopyValue<ThemeStore>,
    resolved: Signal<Resolved>,
}

impl ThemeHandle {
    /// The store: set layers, pick, preview (each change reaches the
    /// signal on the next turn).
    #[must_use]
    pub fn store(&self) -> ThemeStore {
        self.store.read().clone()
    }

    /// The theme showing; reading it subscribes the component.
    #[must_use]
    pub const fn resolved(&self) -> Signal<Resolved> {
        self.resolved
    }

    /// The host's hook for the OS's light or dark
    /// ([`ThemeStore::set_system`]): call it at start and on every change
    /// (a Blitz window's `WindowEvent::ThemeChanged`, a browser's
    /// `prefers-color-scheme` media query).
    pub fn set_system(&self, scheme: Scheme) -> bool {
        self.store().set_system(scheme)
    }
}

/// Gives `make`'s store to everything below (made once), and keeps a
/// signal of the theme showing in step with it: every change, from any
/// thread (an action, the file watcher), arrives on the next turn.
pub fn use_theme_store(make: impl FnOnce() -> ThemeStore) -> ThemeHandle {
    let store = use_hook(make);
    let mut resolved = use_signal(|| store.current());
    let handle = use_context_provider(|| ThemeHandle {
        store: CopyValue::new(store.clone()),
        resolved,
    });
    use_hook(move || {
        let (tx, mut rx) = futures_channel::mpsc::unbounded::<Resolved>();
        let subscription = store.subscribe(move |r| {
            let _ = tx.unbounded_send(r.clone());
        });
        // A change between the signal's making and the subscription.
        let now = store.current();
        spawn(async move {
            let _subscription = subscription;
            if *resolved.peek() != now {
                resolved.set(now);
            }
            while let Some(mut next) = rx.next().await {
                // Only the latest of a burst.
                while let Ok(later) = rx.try_recv() {
                    next = later;
                }
                if *resolved.peek() != next {
                    resolved.set(next);
                }
            }
        });
    });
    handle
}

/// The app's theme store ([`use_theme_store`] above).
///
/// # Panics
///
/// When no ancestor gave one.
#[must_use]
pub fn use_theme() -> ThemeHandle {
    use_context::<ThemeHandle>()
}

/// The app's theme store, when an ancestor gave one.
#[must_use]
pub fn try_use_theme() -> Option<ThemeHandle> {
    try_use_context::<ThemeHandle>()
}

/// The theme showing; the built-in default when no store was given.
#[must_use]
pub fn use_resolved() -> Resolved {
    try_use_theme().map_or_else(Resolved::default, |t| t.resolved.read().clone())
}

/// A typed slice of the theme (`ShellTheme`, a view's own struct) as a
/// memo: components reading it rerun only when the slice changes, not on
/// every theme change. With no store above, the built-in default's.
#[must_use]
pub fn use_themed<T: FromResolved + PartialEq + 'static>() -> Memo<T> {
    let resolved = try_use_theme().map(|t| t.resolved);
    use_memo(move || {
        resolved.map_or_else(
            || T::from_resolved(&Resolved::default()),
            |r| T::from_resolved(&r.read()),
        )
    })
}
