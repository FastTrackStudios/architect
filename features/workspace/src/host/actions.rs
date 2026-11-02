//! The workspace on the app's Action List: every action of
//! [`crate::control`] (`ids::ALL`), run by the host that draws it.
//!
//! A write is a job sent to the window's thread, where the workspace's
//! signals live; it runs there as a command does ([`Host::act`]) and its
//! answer goes back to the caller. A read is answered at once from a
//! mirror the host keeps current, so a guide scenario's check (which runs
//! actions synchronously) and a model see the layout as it is. Each change
//! to the arrangement moves the revision on and is a
//! `Changed { topic: "workspace" }` event ([`control::Change`]), whoever
//! made it: a person, a command, an agent. The layout's rectangles are
//! the host's measure of each group, taken after the frame that drew it.

use std::sync::{Arc, Mutex, PoisonError};

use architect::action::ActionList;
use dioxus::prelude::*;
use futures_util::StreamExt as _;

use super::{Host, Mount};
use crate::control::{self, Env, Exec, Job, State, ViewSpec};
use crate::model::{Docks, GroupId, WindowId, Workspace};
use crate::tree::Measured;

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

/// What the host knows of the layout's revision: the arrangement it
/// stands for, and the list its changes are told on.
#[derive(Default)]
pub(super) struct Revisions {
    pub(super) revision: u64,
    pub(super) seen: Option<Workspace>,
    pub(super) list: Option<Arc<ActionList>>,
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
        env.measured = self.measured.peek().clone();
        for side in Docks::SIDES {
            if let Some(bounds) = self.dock_bounds(side) {
                env.set_dock_bounds(side, bounds);
            }
        }
        env
    }

    /// The workspace, its screensets and class, at its revision, as the
    /// actions see them.
    fn state(&self) -> State {
        let mut st = State::new(self.workspace.peek().clone())
            .with_screensets(self.screensets.peek().clone());
        st.form = *self.form.peek();
        st.revision = self.revisions.peek().revision;
        st
    }

    /// Runs `f` over the workspace as an action does — what a command and
    /// an action from the list both come down to — and keeps what it
    /// changed. A new revision is told on the list, with who made it.
    pub(super) fn act<R>(&self, f: impl FnOnce(&mut State, &Env) -> R) -> R {
        self.finish_closing();
        let env = self.env();
        let mut st = self.state();
        let before = st.workspace.clone();
        let out = f(&mut st, &env);
        if st.revision != self.revisions.peek().revision {
            self.moved_on(&before, &st);
        }
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

    /// The layout is at `st.revision` now, from `before`: kept, and told.
    fn moved_on(&self, before: &Workspace, st: &State) {
        let mut revisions = self.revisions;
        let list = {
            let mut r = revisions.write();
            r.revision = st.revision;
            r.seen = Some(st.workspace.clone());
            r.list.clone()
        };
        if let Some(list) = list {
            list.changed("workspace", &control::Change::between(before, st).to_json());
        }
    }

    /// Runs a write as a command: on the active pane's behalf, its answer
    /// for the caller to ignore (a command has no one to tell).
    pub(super) fn write<A: control::Guarded>(&self, f: control::Write<A>, args: A) {
        let _ = self.act(|st, env| control::transact(st, env, f, args));
    }

    /// Measures the main window's area and each of its groups, for the
    /// layout's rectangles (laid out by Blitz, so after the frame that
    /// drew them).
    async fn measure(&self) -> Measured {
        let mut out = Measured::default();
        if self.window_id() != WindowId::main() {
            return out;
        }
        if let Some(area) = self.rect(Mount::Host).await.filter(|r| r.w > 0.0) {
            out.areas.insert("main".to_owned(), area);
        }
        let groups: Vec<u64> = {
            let w = self.workspace.peek();
            control::groups_in(&w, 0)
        };
        for g in groups {
            if let Some(r) = self
                .rect(Mount::Group(GroupId(g)))
                .await
                .filter(|r| r.w > 0.0)
            {
                out.groups.insert(g, r);
            }
        }
        out
    }
}

/// Puts the workspace's actions on `actions`' list while the host is
/// mounted.
pub(super) fn use_actions(host: Host, actions: Option<WorkspaceActions>) {
    let mirror: Mirror = use_hook(|| Arc::new(Mutex::new((host.state(), host.env()))));
    let list = actions.map(|a| a.0);
    use_hook({
        let list = list.clone();
        move || {
            let mut revisions = host.revisions;
            let mut r = revisions.write();
            r.list = list;
            r.seen = Some(host.workspace.peek().clone());
        }
    });
    // The window's thread runs what the list was asked to.
    let runner = use_coroutine({
        let mirror = mirror.clone();
        move |mut jobs: UnboundedReceiver<Job>| {
            let mirror = mirror.clone();
            async move {
                while let Some(job) = jobs.next().await {
                    host.act(|st, env| job(st, env));
                    *mirror.lock().unwrap_or_else(PoisonError::into_inner) =
                        (host.state(), host.env());
                }
            }
        }
    });
    let registered = use_hook({
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
    // A person's change (a drag, a key) moves the revision on, is heard,
    // and reaches the mirror; the groups are measured again after it.
    use_effect(move || {
        let now = host.workspace.read().clone();
        let _ = (host.screensets.read(), host.form.read());
        let seen = host.revisions.peek().seen.clone();
        if let Some(before) = seen.filter(|b| *b != now) {
            if control::rearranged(&before, &now) {
                let mut st = host.state();
                st.revision = st.revision.saturating_add(1);
                host.moved_on(&before, &st);
            } else {
                let mut revisions = host.revisions;
                revisions.write().seen = Some(now);
            }
        }
        *mirror.lock().unwrap_or_else(PoisonError::into_inner) = (host.state(), host.env());
        let mirror = mirror.clone();
        spawn(async move {
            let measured = host.measure().await;
            if *host.measured.peek() != measured {
                let mut m = host.measured;
                m.set(measured);
                *mirror.lock().unwrap_or_else(PoisonError::into_inner) = (host.state(), host.env());
            }
        });
    });
    use_drop(move || {
        if let Some(list) = registered {
            for id in control::ids::ALL {
                list.unregister(id);
            }
        }
    });
}
