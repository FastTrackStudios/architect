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

use architect::action::{ActionList, ActionMeta, Effect};
use dioxus::prelude::*;
use facet::Facet;
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

/// What the host has open over the panes, as `APP_STATUS` tells it.
type Overlays = Arc<Mutex<Vec<String>>>;

/// The workspace's part of `APP_STATUS` (its `workspace` section).
#[derive(Debug, Clone, Facet)]
pub struct WorkspaceStatus {
    /// The layout's revision: what a write's `if_revision` is checked
    /// against.
    pub revision: u64,
    /// The focused pane's id, and its title; empty when none is.
    pub focused: String,
    pub focused_title: String,
    /// Its windows: each one's groups (id, tabs, the tab showing), its
    /// docks' groups and how many floats it has.
    pub windows: Vec<WindowStatus>,
    /// The saved layout (screenset) in use, when one is.
    #[facet(default)]
    pub screenset: Option<ScreensetStatus>,
    /// What is open over the panes: `menu`, `sheet`, `manage`, `drag`.
    pub overlays: Vec<String>,
}

/// One window in [`WorkspaceStatus`].
#[derive(Debug, Clone, Facet)]
pub struct WindowStatus {
    pub id: String,
    pub groups: Vec<crate::guide::ArrangedGroup>,
    pub docked: Vec<crate::guide::ArrangedGroup>,
    pub floats: usize,
}

/// The screenset in [`WorkspaceStatus`].
#[derive(Debug, Clone, Facet)]
pub struct ScreensetStatus {
    pub id: String,
    pub name: String,
    pub builtin: bool,
    /// The live layout differs from what it saved.
    pub dirty: bool,
}

/// The `workspace` section from the mirror and what is open over it.
fn status_of(mirror: &Mirror, overlays: &Overlays) -> String {
    let (st, env) = mirror
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let arranged = crate::guide::Arrangement::measured(&st.workspace, st.revision, &env.measured);
    let focused_title = st
        .workspace
        .active
        .as_ref()
        .and_then(|id| st.workspace.pane(id))
        .map(|p| p.title.clone())
        .unwrap_or_default();
    let screenset = st
        .screensets
        .active
        .as_deref()
        .and_then(|id| st.screensets.get(id))
        .map(|set| ScreensetStatus {
            id: set.id.clone(),
            name: set.name.clone(),
            builtin: set.builtin,
            dirty: st.screensets.is_dirty(st.form),
        });
    let status = WorkspaceStatus {
        revision: st.revision,
        focused: arranged.active.clone(),
        focused_title,
        windows: arranged
            .windows
            .into_iter()
            .map(|w| WindowStatus {
                id: w.id,
                groups: w.groups,
                docked: w.docked,
                floats: w.floats.len(),
            })
            .collect(),
        screenset,
        overlays: overlays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
    };
    facet_json::to_string(&status).unwrap_or_else(|_| "{}".to_owned())
}

/// How many times a measurement that found nothing is asked again.
const MEASURE_AGAIN: u32 = 20;

/// The workspace's section of `APP_STATUS`.
pub const STATUS_SECTION: &str = "workspace";

static MANAGE_META: ActionMeta = ActionMeta {
    id: control::ids::WORKSPACES_MANAGE,
    trait_name: "",
    method_name: "",
    display_name: "Manage workspace layouts",
    description: "Opens the sheet a person manages the saved layouts in: load, save over, rename, slot, delete, and save the current layout under a name.",
    category: "Workspace",
    group: "Workspaces",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// What `WORKSPACES_MANAGE` answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
pub struct Opened {
    pub opened: bool,
}

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
        env.shipped = self.shipped.peek().clone();
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
        let sets = st.screensets.clone();
        let out = f(&mut st, &env);
        if st.revision != self.revisions.peek().revision {
            self.moved_on(&before, &st);
        }
        let list = self.revisions.peek().list.clone();
        if let (Some(active), Some(list)) = (control::sets_changed(&sets, &st.screensets), list) {
            list.changed(control::SETS_TOPIC, &active);
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
    // `WORKSPACES_MANAGE`: the sheet opens on the window's thread.
    let opener = use_coroutine(
        move |mut asks: UnboundedReceiver<futures_channel::oneshot::Sender<bool>>| async move {
            while let Some(answer) = asks.next().await {
                let mut managing = host.managing;
                managing.set(true);
                let _ = answer.send(true);
            }
        },
    );
    // What is open over the panes, for `APP_STATUS`.
    let overlays: Overlays = use_hook(|| Arc::new(Mutex::new(Vec::new())));
    {
        let overlays = overlays.clone();
        use_effect(move || {
            let open: Vec<String> = [
                ("menu", host.menu.read().is_some()),
                ("sheet", host.sheet.read().is_some()),
                ("manage", *host.managing.read()),
                ("drag", host.drag.read().is_some()),
            ]
            .into_iter()
            .filter(|(_, open)| *open)
            .map(|(name, _)| name.to_owned())
            .collect();
            *overlays.lock().unwrap_or_else(PoisonError::into_inner) = open;
        });
    }
    let registered = use_hook({
        let mirror = mirror.clone();
        move || {
            let list = list?;
            let exec: Arc<dyn Exec> = Arc::new(OnWindow {
                tx: runner.tx(),
                mirror: mirror.clone(),
            });
            control::register(&list, &exec).ok()?;
            list.status()
                .section(STATUS_SECTION, move || status_of(&mirror, &overlays));
            let tx = opener.tx();
            list.register_async(&MANAGE_META, Effect::Mutating, move |(): ()| {
                let (answer, heard) = futures_channel::oneshot::channel();
                let sent = tx.unbounded_send(answer).is_ok();
                async move {
                    Ok(Opened {
                        opened: sent && heard.await.unwrap_or(false),
                    })
                }
            })
            .ok()?;
            Some(list)
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
            let mut measured = host.measure().await;
            // Mounted before its first frame was laid out, nothing measures
            // yet: ask again shortly, a few times (`APP_SHOT`'s pane crop
            // and the layout's rects need it before the first change).
            for _ in 0..MEASURE_AGAIN {
                if !measured.groups.is_empty() {
                    break;
                }
                futures_timer::Delay::new(std::time::Duration::from_millis(50)).await;
                measured = host.measure().await;
            }
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
            list.unregister(control::ids::WORKSPACES_MANAGE);
            list.status().remove(STATUS_SECTION);
        }
    });
}
