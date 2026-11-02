//! The workspace actions on an Action List (feature `actions`), run where
//! the workspace lives: an [`Exec`] says how. [`HeadlessWorkspace`] keeps
//! one behind a lock, for a server, a test or a model with no window; the
//! Blitz host runs writes on its window's thread
//! (`features/workspace/src/host/actions.rs`).

use std::sync::{Arc, Mutex, PoisonError};

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect};
use facet::Facet;

use super::{
    ApplyArgs, CloseArgs, DeleteArgs, DockBackArgs, DockPopOutArgs, DockSetArgs, Env, FloatArgs,
    FocusArgs, GroupSetArgs, MoveArgs, NavigateArgs, NeighborArgs, OpenArgs, PaneArgs, RecallArgs,
    ReopenArgs, ResizeArgs, RestoreArgs, RevisionArgs, SaveAsArgs, SplitArgs, State, TabSetArgs,
    WindowArgs, WindowSetArgs, Write, ZoomArgs, transact,
};

/// The actions' ids. Each is a contract (keymaps, scripts and models
/// store it): never renamed.
pub mod ids {
    pub use crate::guide::LAYOUT_ACTION as LAYOUT;
    pub const PANE_GET: &str = "WORKSPACE_PANE_GET";
    pub const NEIGHBOR: &str = "WORKSPACE_NEIGHBOR";
    pub const VIEWS: &str = "WORKSPACE_VIEWS";
    pub const EXPORT: &str = "WORKSPACE_EXPORT";
    pub const OPEN: &str = "WORKSPACE_OPEN";
    pub const MOVE: &str = "WORKSPACE_MOVE";
    pub const SPLIT: &str = "WORKSPACE_SPLIT";
    pub const FOCUS: &str = "WORKSPACE_FOCUS";
    pub const ZOOM: &str = "WORKSPACE_ZOOM";
    pub const RESIZE: &str = "WORKSPACE_RESIZE";
    pub const GROUP_SET: &str = "WORKSPACE_GROUP_SET";
    pub const TAB_SET: &str = "WORKSPACE_TAB_SET";
    pub const NAVIGATE: &str = "WORKSPACE_NAVIGATE";
    pub const DOCK_SET: &str = "WORKSPACE_DOCK_SET";
    pub const DOCK_POP_OUT: &str = "WORKSPACE_DOCK_POP_OUT";
    pub const DOCK_BACK: &str = "WORKSPACE_DOCK_BACK";
    pub const FLOAT: &str = "WORKSPACE_FLOAT";
    pub const MINIMISE: &str = "WORKSPACE_MINIMISE";
    pub const RESTORE: &str = "WORKSPACE_RESTORE";
    pub const WINDOW_SET: &str = "WORKSPACE_WINDOW_SET";
    pub const REOPEN: &str = "WORKSPACE_REOPEN";
    pub const APPLY: &str = "WORKSPACE_APPLY";
    pub const CLOSE: &str = "WORKSPACE_CLOSE";
    pub const CLOSE_OTHERS: &str = "WORKSPACE_CLOSE_OTHERS";
    pub const CLOSE_WINDOW: &str = "WORKSPACE_CLOSE_WINDOW";
    pub const SCREENSET_LIST: &str = "SCREENSET_LIST";
    pub const SCREENSET_RECALL: &str = "SCREENSET_RECALL";
    pub const SCREENSET_SAVE: &str = "SCREENSET_SAVE";
    pub const SCREENSET_SAVE_AS: &str = "SCREENSET_SAVE_AS";
    pub const SCREENSET_RESET: &str = "SCREENSET_RESET";
    pub const SCREENSET_DELETE: &str = "SCREENSET_DELETE";

    /// Every id [`super::register`] puts on a list.
    pub const ALL: &[&str] = &[
        LAYOUT,
        PANE_GET,
        NEIGHBOR,
        VIEWS,
        EXPORT,
        OPEN,
        MOVE,
        SPLIT,
        FOCUS,
        ZOOM,
        RESIZE,
        GROUP_SET,
        TAB_SET,
        NAVIGATE,
        DOCK_SET,
        DOCK_POP_OUT,
        DOCK_BACK,
        FLOAT,
        MINIMISE,
        RESTORE,
        WINDOW_SET,
        REOPEN,
        APPLY,
        CLOSE,
        CLOSE_OTHERS,
        CLOSE_WINDOW,
        SCREENSET_LIST,
        SCREENSET_RECALL,
        SCREENSET_SAVE,
        SCREENSET_SAVE_AS,
        SCREENSET_RESET,
        SCREENSET_DELETE,
    ];
}

/// A write, sent to where the workspace lives.
pub type Job = Box<dyn FnOnce(&mut State, &Env) + Send>;

/// Where the workspace lives, as the actions reach it.
pub trait Exec: Send + Sync + 'static {
    /// Runs `job` against the workspace (on its thread, in its turn);
    /// false when the workspace is gone and it never will.
    fn exec(&self, job: Job) -> bool;

    /// Reads the workspace as it is now, at once.
    fn read(&self, f: &mut dyn FnMut(&State, &Env));
}

const GONE: &str = "the workspace is gone";

macro_rules! metas {
    ($($name:ident = $id:ident, $group:literal, $display:literal, $desc:literal;)*) => {
        $(static $name: ActionMeta = ActionMeta {
            id: ids::$id,
            trait_name: "",
            method_name: "",
            display_name: $display,
            description: $desc,
            category: "Workspace",
            group: $group,
            shortcut: "",
            toggleable: false,
            undo: false,
        };)*
    };
}

metas! {
    LAYOUT = LAYOUT, "Panes", "Workspace layout", "Every window's tab groups (their ids, their tabs in order, the one showing, stacked or not), its splits and its docks' groups, and the active pane.";
    PANE_GET = PANE_GET, "Panes", "Workspace pane", "One pane: its view, title and state, its window, place (center, a dock, a float), group and index, whether it shows and is active, and its history.";
    NEIGHBOR = NEIGHBOR, "Panes", "Workspace neighbour", "The pane showing next to `pane` toward `direction` (left, right, up, down), and its group; unset at an edge.";
    VIEWS = VIEWS, "Panes", "Workspace views", "The views a pane can show: each kind (what WORKSPACE_OPEN takes as `view`), its title, and whether at most one is open.";
    EXPORT = EXPORT, "Panes", "Export the workspace layout", "The layout as a portable tree WORKSPACE_APPLY takes back: every window's splits and shares, groups and tabs (pane id, view, state, title), docks and floats.";
    OPEN = OPEN, "Panes", "Open a view", "Opens a view in a new pane at a named place (`at`: replace, beside, group, split, window, dock, float). An id already open answers `already_open`. The person's focus stays unless `focus: true`.";
    MOVE = MOVE, "Panes", "Move a pane", "Moves a pane (`to`: a group, beside a pane, a group's edge, a split beside a pane, a window, a dock, a float). The focus stays unless `focus: true`.";
    SPLIT = SPLIT, "Panes", "Split a pane off", "Moves a pane out of its group into a new group on `side` of it. Its group's only tab answers `only_tab`.";
    FOCUS = FOCUS, "Panes", "Focus a pane", "Makes `pane` (or the pane next to `from` toward `direction`) the active one: the person's keyboard goes there.";
    ZOOM = ZOOM, "Panes", "Zoom a pane's group", "Shows the center group holding `pane` alone in its window (`mode`: on, off, toggle); the layout is kept.";
    RESIZE = RESIZE, "Panes", "Resize", "A split's shares (or even them), a dock's size, or a pane grown toward a direction by an amount of its split.";
    GROUP_SET = GROUP_SET, "Panes", "Set a group", "A group's stacked tabs, its tab bar (tabs, icons, bare), or the tab it shows.";
    TAB_SET = TAB_SET, "Tabs", "Set a tab", "A pane's pin, link (\"\" unlinks) or title.";
    NAVIGATE = NAVIGATE, "Tabs", "Navigate a pane", "Back or forward through a pane's history, or to a new state (the old one kept to go back to).";
    DOCK_SET = DOCK_SET, "Docks", "Set a dock", "A dock's shown state (open, collapsed, hidden) and size.";
    DOCK_POP_OUT = DOCK_POP_OUT, "Docks", "Pop a dock out", "The dock on `side`, with its ribbon and groups, into a window of its own (`dock-left`, `dock-right`, `dock-bottom`).";
    DOCK_BACK = DOCK_BACK, "Docks", "Dock back", "A popped-out dock back to its side of the main window.";
    FLOAT = FLOAT, "Panes", "Float a pane", "Floats a pane over the main window (out of its group when it is not floating), with its rectangle, snap and corner pin.";
    MINIMISE = MINIMISE, "Panes", "Minimise a float", "A floating pane down to its chip.";
    RESTORE = RESTORE, "Panes", "Restore a float", "A minimised float back from its chip.";
    WINDOW_SET = WINDOW_SET, "Windows", "Set a window", "A pop-out window's bounds, or whether a window is maximised.";
    REOPEN = REOPEN, "Tabs", "Reopen the last closed pane", "Reopens the last closed pane where it was.";
    APPLY = APPLY, "Panes", "Apply a workspace layout", "Makes the workspace match a portable layout (WORKSPACE_EXPORT's): built whole first, then put in place, so one that does not fit changes nothing. Open panes it does not place stay as tabs unless `keep_unplaced: false` (then they close, kept to reopen).";
    CLOSE = CLOSE, "Tabs", "Close panes", "Closes the panes named (all or none), each kept to reopen.";
    CLOSE_OTHERS = CLOSE_OTHERS, "Tabs", "Close the other tabs", "Closes every other unpinned tab in the pane's group.";
    CLOSE_WINDOW = CLOSE_WINDOW, "Windows", "Close a window", "Closes a pop-out window; its tabs come back to the main window.";
    SCREENSET_LIST = SCREENSET_LIST, "Screensets", "Screensets", "Every screenset (id, name, slot, built in, the classes of screen it has layouts for), the active one, and whether the live layout differs from it.";
    SCREENSET_RECALL = SCREENSET_RECALL, "Screensets", "Recall a screenset", "Recalls a screenset by `id` or `slot`: its layout for this class of screen becomes the live one.";
    SCREENSET_SAVE = SCREENSET_SAVE, "Screensets", "Save the screenset", "Saves the live layout into the active screenset.";
    SCREENSET_SAVE_AS = SCREENSET_SAVE_AS, "Screensets", "Save as a new screenset", "Saves every live layout as a new screenset named `name` (in `slot` 1–9 when given), which becomes active.";
    SCREENSET_RESET = SCREENSET_RESET, "Screensets", "Reset to the saved layout", "Throws the live changes away: back to the active screenset's saved layout.";
    SCREENSET_DELETE = SCREENSET_DELETE, "Screensets", "Delete a screenset", "Deletes a screenset the person made (a built-in one answers `builtin`).";
}

/// Registers a write: sent to where the workspace lives, answered when it
/// has run there.
fn write<A>(
    list: &ActionList,
    exec: &Arc<dyn Exec>,
    meta: &'static ActionMeta,
    effect: Effect,
    f: Write<A>,
) -> Result<(), ActionError>
where
    A: Facet<'static> + super::Guarded + Send + 'static,
{
    let exec = exec.clone();
    list.register_async_with(meta, effect, move |caller: &ActionCaller, args: A| {
        let actor = caller.principal().map_or_else(
            || super::APP.to_owned(),
            architect::action::Principal::describe,
        );
        let (tx, rx) = futures_channel::oneshot::channel();
        let sent = exec.exec(Box::new(move |st, env| {
            st.actor = actor;
            let _ = tx.send(transact(st, env, f, args));
        }));
        async move {
            if !sent {
                return Err(GONE.to_owned());
            }
            rx.await.map_err(|_| GONE.to_owned())?
        }
    })
}

/// Registers a read: answered at once, from the workspace as it is.
fn read<A, R>(
    list: &ActionList,
    exec: &Arc<dyn Exec>,
    meta: &'static ActionMeta,
    f: fn(&State, &Env, A) -> Result<R, String>,
) -> Result<(), ActionError>
where
    A: Facet<'static> + 'static,
    R: Facet<'static> + 'static,
{
    let exec = exec.clone();
    list.register(meta, Effect::ReadOnly, move |args: A| {
        let mut args = Some(args);
        let mut out = Err(GONE.to_owned());
        exec.read(&mut |st, env| {
            if let Some(args) = args.take() {
                out = f(st, env, args);
            }
        });
        out
    })
}

/// Puts every workspace action (`ids::ALL`) on `list`, run through `exec`.
/// Reads are read-only; writes are mutating, but for closing others' tabs,
/// a window or a screenset, which are destructive.
///
/// # Errors
///
/// An id already on `list` ([`ActionError::Duplicate`]); the actions
/// registered before it stay.
pub fn register(list: &ActionList, exec: &Arc<dyn Exec>) -> Result<(), ActionError> {
    use Effect::{Destructive, Mutating};
    let e = exec;
    read(list, e, &LAYOUT, super::layout)?;
    read::<PaneArgs, _>(list, e, &PANE_GET, super::pane_get)?;
    read::<NeighborArgs, _>(list, e, &NEIGHBOR, super::neighbor)?;
    read(list, e, &VIEWS, super::views)?;
    read(list, e, &EXPORT, super::export)?;
    read(list, e, &SCREENSET_LIST, super::screensets)?;
    write::<OpenArgs>(list, e, &OPEN, Mutating, super::open)?;
    write::<MoveArgs>(list, e, &MOVE, Mutating, super::move_pane)?;
    write::<SplitArgs>(list, e, &SPLIT, Mutating, super::split)?;
    write::<FocusArgs>(list, e, &FOCUS, Mutating, super::focus)?;
    write::<ZoomArgs>(list, e, &ZOOM, Mutating, super::zoom)?;
    write::<ResizeArgs>(list, e, &RESIZE, Mutating, super::resize)?;
    write::<GroupSetArgs>(list, e, &GROUP_SET, Mutating, super::group_set)?;
    write::<TabSetArgs>(list, e, &TAB_SET, Mutating, super::tab_set)?;
    write::<NavigateArgs>(list, e, &NAVIGATE, Mutating, super::navigate)?;
    write::<DockSetArgs>(list, e, &DOCK_SET, Mutating, super::dock_set)?;
    write::<DockPopOutArgs>(list, e, &DOCK_POP_OUT, Mutating, super::dock_pop_out)?;
    write::<DockBackArgs>(list, e, &DOCK_BACK, Mutating, super::dock_back)?;
    write::<FloatArgs>(list, e, &FLOAT, Mutating, super::float)?;
    write::<PaneArgs>(list, e, &MINIMISE, Mutating, super::minimise)?;
    write::<RestoreArgs>(list, e, &RESTORE, Mutating, super::restore)?;
    write::<WindowSetArgs>(list, e, &WINDOW_SET, Mutating, super::window_set)?;
    write::<ReopenArgs>(list, e, &REOPEN, Mutating, super::reopen)?;
    write::<ApplyArgs>(list, e, &APPLY, Mutating, super::apply)?;
    write::<CloseArgs>(list, e, &CLOSE, Mutating, super::close)?;
    write::<PaneArgs>(list, e, &CLOSE_OTHERS, Destructive, super::close_others)?;
    write::<WindowArgs>(list, e, &CLOSE_WINDOW, Destructive, super::close_window)?;
    write::<RecallArgs>(list, e, &SCREENSET_RECALL, Mutating, super::recall)?;
    write::<RevisionArgs>(list, e, &SCREENSET_SAVE, Mutating, super::save)?;
    write::<SaveAsArgs>(list, e, &SCREENSET_SAVE_AS, Mutating, super::save_as)?;
    write::<RevisionArgs>(list, e, &SCREENSET_RESET, Mutating, super::reset)?;
    write::<DeleteArgs>(list, e, &SCREENSET_DELETE, Destructive, super::delete)?;
    Ok(())
}

/// A workspace with no window, kept behind a lock.
///
/// Its actions run on the caller's thread: for a server, a test, or a
/// model arranging a layout to store. Each change is a
/// `Changed { topic: "workspace" }` on the list, its detail a
/// [`super::Change`].
#[derive(Clone)]
pub struct HeadlessWorkspace {
    shared: Arc<Mutex<(State, Env)>>,
}

impl HeadlessWorkspace {
    #[must_use]
    pub fn new(state: State, env: Env) -> Self {
        Self {
            shared: Arc::new(Mutex::new((state, env))),
        }
    }

    /// Puts its actions on `list`.
    ///
    /// # Errors
    ///
    /// As [`register`].
    pub fn install(&self, list: &ActionList) -> Result<(), ActionError> {
        let exec: Arc<dyn Exec> = Arc::new(Headless {
            shared: self.shared.clone(),
            list: list.clone(),
        });
        register(list, &exec)
    }

    /// A copy of its state as it is now.
    #[must_use]
    pub fn state(&self) -> State {
        self.shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .0
            .clone()
    }
}

struct Headless {
    shared: Arc<Mutex<(State, Env)>>,
    list: ActionList,
}

impl Exec for Headless {
    fn exec(&self, job: Job) -> bool {
        let mut shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        let (st, env) = &mut *shared;
        let before = st.workspace.clone();
        let revision = st.revision;
        job(st, env);
        let detail =
            (st.revision != revision).then(|| super::Change::between(&before, st).to_json());
        super::APP.clone_into(&mut st.actor);
        drop(shared);
        if let Some(detail) = detail {
            self.list.changed("workspace", &detail);
        }
        true
    }

    fn read(&self, f: &mut dyn FnMut(&State, &Env)) {
        let shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        f(&shared.0, &shared.1);
    }
}
