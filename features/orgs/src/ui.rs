//! The current org in a window (feature `ui`): [`use_org`] for any
//! component, the shell's switcher built from the person's orgs, and
//! [`use_orgs_live`], which fetches them again on every (re)connect.

use std::future::Future;
use std::rc::Rc;

use architect_atom::Connection;
use architect_shell::{Workspace, Workspaces};
use dioxus::prelude::*;

use crate::model::{Capabilities, Org, OrgId, Orgs, Selection};
use crate::refresh::Refresh;
use crate::scope::Scope;

/// The window's orgs, in context.
#[derive(Clone, Copy)]
pub struct OrgHandle {
    pub orgs: Signal<Orgs>,
    caps: CopyValue<Capabilities>,
    on_change: CopyValue<Option<Callback<Orgs>>>,
    fetched: Signal<Refresh>,
}

/// Puts the person's orgs (and the app's capabilities by role) in context.
/// `on_change` keeps them (the device's cache).
pub fn use_orgs_provider(
    orgs: impl FnOnce() -> Orgs,
    caps: Capabilities,
    on_change: Option<Callback<Orgs>>,
) -> OrgHandle {
    use_context_provider(|| OrgHandle {
        orgs: Signal::new(orgs()),
        caps: CopyValue::new(caps),
        on_change: CopyValue::new(on_change),
        fetched: Signal::new(Refresh::default()),
    })
}

/// Keeps the window's orgs live, fetched again on every (re)connect.
///
/// `fetch` (`architect_orgs::fetch` over the app's identity-server client)
/// runs when `conn` connects and again on every reconnect
/// ([`Connection::generation`]), refreshing `org`. A fetch that fails
/// leaves the cached list ([`OrgHandle::stale`] says why).
pub fn use_orgs_live<C, F, Fut>(org: OrgHandle, conn: Connection<C>, fetch: F)
where
    C: Clone + 'static,
    F: Fn(C) -> Fut + 'static,
    Fut: Future<Output = Result<Vec<Org>, String>> + 'static,
{
    let fetch = use_hook(|| Rc::new(fetch));
    let _live = use_resource(move || {
        // Reactive on purpose: the resource re-runs on every (re)connect.
        let generation = conn.generation();
        let ready = conn.ready();
        let fetch = Rc::clone(&fetch);
        async move {
            let Some(client) = ready else {
                return;
            };
            // `peek`: a read here would subscribe the resource to its own
            // writes below and restart it.
            if !org.fetched.peek().due(generation) {
                return;
            }
            let answer = fetch(client).await;
            let mut fetched = org.fetched;
            let fresh = fetched.write().finish(generation, answer);
            if let Some(list) = fresh {
                org.refresh(list);
            }
        }
    });
}

/// The orgs above.
#[must_use]
pub fn use_org() -> Option<OrgHandle> {
    try_use_context()
}

impl OrgHandle {
    /// The org the window is in (none in All).
    #[must_use]
    pub fn current(&self) -> Option<Org> {
        self.orgs.read().current().cloned()
    }

    /// What the window's reads cover: its org, or every org in All.
    #[must_use]
    pub fn scope(&self) -> Scope {
        self.orgs.read().scope()
    }

    /// Why the orgs shown are the device's cached list: the last fetch's
    /// failure, if it failed.
    #[must_use]
    pub fn stale(&self) -> Option<String> {
        self.fetched.read().error().map(str::to_owned)
    }

    /// Whether the person may do `cap` here.
    #[must_use]
    pub fn can(&self, cap: &str) -> bool {
        self.orgs.read().can(&self.caps.read(), cap)
    }

    /// Moves the window to `selection` (the switcher).
    pub fn switch(&self, selection: Selection) {
        let mut orgs = self.orgs;
        let moved = orgs.write().switch(selection);
        if moved {
            if let Some(cb) = *self.on_change.peek() {
                cb.call(self.orgs.peek().clone());
            }
        }
    }

    /// Puts `orgs` (the list and the selection) in the window as they are,
    /// `on_change` not told: where another owner keeps them (the active
    /// account's, `architect_accounts`) and the window follows it. Nothing
    /// is written when they are the same.
    pub fn replace(&self, orgs: Orgs) {
        let mut shown = self.orgs;
        if *shown.peek() != orgs {
            shown.set(orgs);
        }
    }

    /// The identity server's fresh list.
    pub fn refresh(&self, list: Vec<Org>) {
        let mut orgs = self.orgs;
        orgs.write().refresh(list);
        if let Some(cb) = *self.on_change.peek() {
            cb.call(self.orgs.peek().clone());
        }
    }

    /// The shell's switcher: every org (its mark, its role and members),
    /// "All organizations" when the app has an all-orgs view, the current
    /// one ticked; choosing one switches the window.
    #[must_use]
    pub fn switcher(&self, all: Option<&str>) -> Workspaces {
        let handle = *self;
        let mut w = Workspaces::new("Organization", move |id: Option<String>| {
            handle.switch(id.map_or(Selection::All, |i| Selection::One(OrgId::new(i))));
        });
        if let Some(label) = all {
            w = w.all(label);
        }
        let orgs = self.orgs.read();
        for o in &orgs.orgs {
            let detail = if o.personal {
                "Just you".to_owned()
            } else if o.members > 1 {
                format!("{} · {} members", o.role.label(), o.members)
            } else {
                o.role.label()
            };
            w = w.with(
                Workspace::new(o.id.0.clone(), o.name.clone())
                    .detail(detail)
                    .color(leak(&o.color)),
            );
        }
        let current = match &orgs.selection {
            Selection::One(id) => Some(id.0.clone()),
            Selection::All => None,
        };
        w.current(current)
    }
}

/// A switcher mark's fill: the shell takes a `'static` colour, and an
/// org's is one of a few, so each is kept once.
fn leak(color: &str) -> &'static str {
    use std::sync::{Mutex, OnceLock, PoisonError};
    static KEPT: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
    let kept = KEPT.get_or_init(|| Mutex::new(Vec::new()));
    let mut kept = kept.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(c) = kept.iter().find(|c| **c == color) {
        return c;
    }
    let c: &'static str = Box::leak(color.to_owned().into_boxed_str());
    kept.push(c);
    c
}
