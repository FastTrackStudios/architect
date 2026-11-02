//! The workspace on the app's Action List: every action of
//! [`crate::control`] (`ids::ALL`), run by the host that draws it.
//!
//! A write is a job sent to the window's thread, where the workspace's
//! signals live; it runs there as a command does ([`Host::act`]) and its
//! answer goes back to the caller. A read is answered at once from a
//! mirror the host keeps current, so a guide scenario's check (which runs
//! actions synchronously) and a model see the layout as it is. Each change
//! to the arrangement is a `Changed { topic: "workspace" }` event.

use std::sync::{Arc, Mutex, PoisonError};

use architect::action::ActionList;
use dioxus::prelude::*;
use futures_util::StreamExt as _;

use super::Host;
use crate::control::{self, Env, Exec, Job, State, ViewSpec};
use crate::guide::Arrangement;
use crate::model::Docks;

/// The app's Action List, as a [`super::WorkspaceView`] prop (compared by
/// identity): the host puts the workspace's actions on it while mounted.
#[derive(Clone)]
pub struct WorkspaceActions(pub Arc<ActionList>);

impl WorkspaceActions {
    #[must_use]
    pub const fn new(list: Arc<ActionList>) -> Self {
        Self(list)
    }
}

impl PartialEq for WorkspaceActions {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

type Mirror = Arc<Mutex<(State, Env)>>;

/// Runs writes on the window's thread; reads from the mirror.
struct OnWindow {
    tx: futures_channel::mpsc::UnboundedSender<Job>,
    mirror: Mirror,
}

impl Exec for OnWindow {
    fn exec(&self, job: Job) -> bool {
        self.tx.unbounded_send(job).is_ok()
    }

    fn read(&self, f: &mut dyn FnMut(&State, &Env)) {
        let m = self.mirror.lock().unwrap_or_else(PoisonError::into_inner);
        f(&m.0, &m.1);
    }
}

impl Host {
    /// What the app offers the actions, now.
    pub(super) fn env(&self) -> Env {
        let mut env = Env::new(
            self.views
                .peek()
                .iter()
                .map(|v| ViewSpec {
                    kind: v.kind.0.clone(),
                    title: v.title.clone(),
                    single: v.single,
                })
                .collect(),
        );
        env.windows = self.has_windows();
        for side in Docks::SIDES {
            if let Some(bounds) = self.dock_bounds(side) {
                env.set_dock_bounds(side, bounds);
            }
        }
        env
    }

    /// The workspace, its screensets and class, as the actions see them.
    fn state(&self) -> State {
        State {
            workspace: self.workspace.peek().clone(),
            screensets: self.screensets.peek().clone(),
            form: *self.form.peek(),
        }
    }

    /// Runs `f` over the workspace as an action does — what a command and
    /// an action from the list both come down to — and keeps what it
    /// changed.
    pub(super) fn act<R>(&self, f: impl FnOnce(&mut State, &Env) -> R) -> R {
        self.finish_closing();
        let env = self.env();
        let mut st = self.state();
        let out = f(&mut st, &env);
        if *self.workspace.peek() != st.workspace {
            let mut w = self.workspace;
            w.set(st.workspace);
        }
        if *self.screensets.peek() != st.screensets {
            let mut s = self.screensets;
            s.set(st.screensets);
        }
        out
    }

    /// Runs a write as a command: on the active pane's behalf, its answer
    /// for the caller to ignore (a command has no one to tell).
    pub(super) fn write<A>(&self, f: control::Write<A>, args: A) {
        let _ = self.act(|st, env| control::transact(st, env, f, args));
    }
}

/// Puts the workspace's actions on `actions`' list while the host is
/// mounted.
pub(super) fn use_actions(host: Host, actions: Option<WorkspaceActions>) {
    let mirror: Mirror = use_hook(|| Arc::new(Mutex::new((host.state(), host.env()))));
    // The window's thread runs what the list was asked to.
    let runner = use_coroutine({
        let mirror = mirror.clone();
        move |mut jobs: UnboundedReceiver<Job>| {
            let mirror = mirror.clone();
            async move {
                while let Some(job) = jobs.next().await {
                    host.act(|st, env| {
                        job(st, env);
                        *mirror.lock().unwrap_or_else(PoisonError::into_inner) =
                            (st.clone(), env.clone());
                    });
                }
            }
        }
    });
    let list = actions.map(|a| a.0);
    let registered = use_hook({
        let list = list.clone();
        let mirror = mirror.clone();
        move || {
            let list = list?;
            let exec: Arc<dyn Exec> = Arc::new(OnWindow {
                tx: runner.tx(),
                mirror,
            });
            control::register(&list, &exec).ok().map(|()| list)
        }
    });
    // A person's change (a drag, a key) reaches the mirror, and is heard.
    let told = use_hook(|| Arc::new(Mutex::new(Arrangement::of(&host.workspace.peek()))));
    use_effect(move || {
        let st = State {
            workspace: host.workspace.read().clone(),
            screensets: host.screensets.read().clone(),
            form: *host.form.read(),
        };
        let now = Arrangement::of(&st.workspace);
        *mirror.lock().unwrap_or_else(PoisonError::into_inner) = (st, host.env());
        let mut kept = told.lock().unwrap_or_else(PoisonError::into_inner);
        if *kept != now {
            let active = now.active.clone();
            *kept = now;
            drop(kept);
            if let Some(list) = &list {
                list.changed("workspace", &active);
            }
        }
    });
    use_drop(move || {
        if let Some(list) = registered {
            for id in control::ids::ALL {
                list.unregister(id);
            }
        }
    });
}
