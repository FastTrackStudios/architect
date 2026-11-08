//! Decks in the app, as scope tabs: their group at the leading edge of
//! the main window's first tab bar ([`ScopeTabs`]), their keys and menus,
//! a scope tab popped out into a window of its own, and their part in a
//! phone's switcher.
//!
//! [`use_decks`] holds the [`Decks`] beside the app's `Signal<Workspace>`
//! and `Signal<Screensets>` (the same ones its [`super::WorkspaceView`]
//! draws): the deck on show draws into them, the others are kept as
//! values (`docs/content/architecture/workspace.md`, "Decks"). Every
//! change goes through [`DeckHandle::act`], the [`DeckState`] operations;
//! a person's rearranging is recorded into the deck on show as it
//! settles, and what changed is kept: the session in the app's
//! `session` store, each deck's layout with its context.

use std::rc::Rc;

use architect_commands::{Command, use_command};
use architect_shell::ShellTheme;
use dioxus::prelude::*;
use lucide_dioxus::{AppWindow, X};

use super::Views;
use super::screens::WorkspaceStore;
use super::windows::{Bridge, Popout, Windows};
use crate::deck::{ContextLayout, DeckContext, DeckId, DeckState, DeckTemplate, Decks, Load, NOUN};
use crate::model::{WindowId, Workspace};
use crate::screenset::{FormClass, Screensets};
use crate::store::{self, ContextStore};

/// Where the decks' things are kept, as [`use_decks`] is given them.
#[derive(Clone, Default)]
pub struct DeckStores {
    /// The session (the decks open, their order, the one on show, the
    /// person's defaults), on this device.
    pub session: Option<WorkspaceStore>,
    /// The screensets the person saved (the working copy is each deck's).
    pub screensets: Option<WorkspaceStore>,
    /// Where each context keeps its layout (`home::ContextFiles`).
    pub contexts: Option<Rc<dyn ContextStore>>,
}

impl PartialEq for DeckStores {
    fn eq(&self, other: &Self) -> bool {
        self.session == other.session
            && self.screensets == other.screensets
            && match (&self.contexts, &other.contexts) {
                (Some(a), Some(b)) => Rc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}

/// What [`use_decks`] is given.
#[derive(Clone)]
pub struct DecksConfig {
    /// The live layout the deck on show draws into (the host's).
    pub workspace: Signal<Workspace>,
    /// The screensets (the app's built-in ones to begin with).
    pub screensets: Signal<Screensets>,
    pub form: FormClass,
    pub stores: DeckStores,
    /// Each context type's default screenset, the app's and the
    /// extensions' (`[[deck_templates]]`).
    pub templates: Vec<DeckTemplate>,
    /// The scope tabs a first run (nothing kept) opens with.
    pub start: Vec<DeckContext>,
    /// The general tabs' context: its type picks their default screenset
    /// (the app's `vault`, shown in "Write"), and an older session's deck
    /// of that type folds into them ([`Decks::fold_general`]).
    pub general: Option<DeckContext>,
    /// The views and contexts a deck popped out into a window draws with;
    /// without `views` a deck does not pop out.
    pub views: Option<Views>,
    pub bridge: Bridge,
    /// The app's Action List: the `SCOPE_*` actions go on it, run here.
    pub actions: Option<super::WorkspaceActions>,
    /// Fills in what a context loaded by its type and id alone lacks
    /// (`SCOPE_OPEN {context: {kind: project, id}}`: a project's folder),
    /// before what it kept is read.
    pub resolve: Option<DeckResolve>,
    /// A context loaded onto a deck with nothing kept, its first time: the
    /// app opens what its deck starts on (a project's page). Told for
    /// every load, the navigator's and `SCOPE_OPEN`'s alike.
    pub on_fresh: Option<Callback<DeckContext>>,
}

/// What [`DecksConfig::resolve`] is.
pub type DeckResolve = Rc<dyn Fn(DeckContext) -> DeckContext>;

/// A deck in a window of its own: its own live layout.
#[derive(Clone, Copy, PartialEq)]
struct Popped {
    deck: CopyValue<DeckId>,
    workspace: Signal<Workspace>,
}

/// The decks, in context under [`use_decks`]: the strip and the host read
/// them, the app and its views act on them.
#[derive(Clone, Copy)]
pub struct DeckHandle {
    pub decks: Signal<Decks>,
    workspace: Signal<Workspace>,
    screensets: Signal<Screensets>,
    form: Signal<FormClass>,
    templates: Signal<Vec<DeckTemplate>>,
    stores: CopyValue<DeckStores>,
    popped: Signal<Vec<Popped>>,
    views: CopyValue<Option<Views>>,
    bridge: CopyValue<Bridge>,
    resolve: CopyValue<Option<DeckResolve>>,
    on_fresh: CopyValue<Option<Callback<DeckContext>>>,
    /// Bumped when what is kept should be written (settled changes).
    dirty: Signal<u64>,
    /// The platform's windows and the shell's colours, as the app's root
    /// has them (read where the decks are mounted: a click has no scope
    /// to look them up from).
    windows: CopyValue<Option<Windows>>,
    theme: CopyValue<Option<ShellTheme>>,
}

impl PartialEq for DeckHandle {
    fn eq(&self, other: &Self) -> bool {
        self.decks == other.decks
    }
}

/// Holds the decks beside `config`'s workspace and screensets.
///
/// Puts a [`DeckHandle`] in context: reads what was kept (or opens
/// `start`), shows the active deck, records the person's rearranging into
/// it, and keeps what changes.
pub fn use_decks(config: DecksConfig) -> DeckHandle {
    let DecksConfig {
        workspace,
        screensets,
        form,
        stores,
        templates,
        start,
        general,
        views,
        bridge,
        actions,
        resolve,
        on_fresh,
    } = config;
    let handle = use_context_provider(|| DeckHandle {
        decks: Signal::new(Decks::default()),
        workspace,
        screensets,
        form: Signal::new(form),
        templates: Signal::new(templates.clone()),
        stores: CopyValue::new(stores.clone()),
        popped: Signal::new(Vec::new()),
        views: CopyValue::new(views.clone()),
        bridge: CopyValue::new(bridge.clone()),
        resolve: CopyValue::new(resolve.clone()),
        on_fresh: CopyValue::new(on_fresh),
        dirty: Signal::new(0),
        windows: CopyValue::new(None),
        theme: CopyValue::new(None),
    });
    let windows_now = try_use_context::<Windows>();
    if handle.windows.peek().as_ref() != windows_now.as_ref() {
        handle.windows.clone().set(windows_now);
    }
    let theme_now = architect_shell::try_use_shell().map(|s| s.theme);
    if *handle.theme.peek() != theme_now {
        handle.theme.clone().set(theme_now);
    }
    // Props change under a live strip: keep them current.
    if *handle.templates.peek() != templates {
        let mut t = handle.templates;
        t.set(templates);
    }
    if *handle.form.peek() != form {
        let mut f = handle.form;
        f.set(form);
        handle.act(DeckState::show);
    }
    handle.views.clone().set(views);
    handle.bridge.clone().set(bridge);
    handle.resolve.clone().set(resolve);
    handle.on_fresh.clone().set(on_fresh);
    // What was kept, once.
    use_hook(move || {
        let stores = handle.stores.peek().clone();
        if let Some(saved) = &stores.screensets {
            let builtins: Vec<_> = screensets
                .peek()
                .sets
                .iter()
                .filter(|s| s.builtin)
                .cloned()
                .collect();
            let none = crate::MemoryStore::new();
            let mut loaded = store::load_split(saved.0.as_ref(), &none, builtins);
            loaded.options = screensets.peek().options;
            let mut s = screensets;
            s.set(loaded);
        }
        let decks = match (&stores.session, &stores.contexts) {
            (Some(session), Some(contexts)) => {
                store::load_decks(session.0.as_ref(), contexts.as_ref())
            }
            (Some(session), None) => {
                store::load_decks(session.0.as_ref(), &crate::MemoryContextStore::new())
            }
            _ => Decks::default(),
        };
        let mut decks = decks;
        if let Some(g) = &general {
            decks.fold_general(&g.kind);
        }
        let first = decks.decks.is_empty() && decks.general.screenset.is_empty();
        let mut d = handle.decks;
        d.set(decks);
        if let Some(g) = general {
            handle.act(|st| st.set_general(g));
        }
        // What was kept on show first: a load records the live layout into
        // the deck on show, which must be its own by then.
        handle.act(DeckState::show);
        if first {
            for context in start {
                let kept = handle.kept(&context);
                let _ = handle.act(|st| st.open(context, Load::NewDeck, None, kept));
            }
            // The first run starts on the general tabs.
            let _ = handle.act(DeckState::show_general);
        }
        handle.act(DeckState::show);
    });
    use_deck_actions(handle, actions);
    // Unmounted (a page switch, the app closing): kept now.
    use_drop(move || handle.keep_all());
    // A person's rearranging, a screenset loaded: into the deck on show.
    use_effect(move || {
        let _ = (workspace.read(), screensets.read());
        let changed = handle.act(DeckState::record);
        if changed {
            handle.settle();
        }
    });
    // What changed is kept once things settle (not every frame of a drag).
    use_effect(move || {
        let n = *handle.dirty.read();
        if n == 0 {
            return;
        }
        spawn(async move {
            futures_timer::Delay::new(std::time::Duration::from_millis(400)).await;
            if *handle.dirty.peek() == n {
                handle.keep_all();
            }
        });
    });
    handle
}

/// What a context kept, read from the app's store, and the app's
/// [`DecksConfig::resolve`] (for `SCOPE_OPEN`).
struct UiKept(Option<Rc<dyn ContextStore>>, Option<DeckResolve>);

impl crate::control::deck::Kept for UiKept {
    fn kept(&self, context: &DeckContext) -> Option<ContextLayout> {
        store::load_context(self.0.as_ref()?.as_ref(), context)
    }

    fn resolve(&self, context: DeckContext) -> DeckContext {
        match &self.1 {
            Some(f) => f(context),
            None => context,
        }
    }
}

type DeckMirror = std::sync::Arc<std::sync::Mutex<DeckState>>;

/// Runs deck writes on the window's thread; reads from the mirror.
struct OnWindow {
    tx: futures_channel::mpsc::UnboundedSender<crate::control::deck::DeckJob>,
    mirror: DeckMirror,
}

impl crate::control::deck::DeckExec for OnWindow {
    fn exec(&self, job: crate::control::deck::DeckJob) -> bool {
        self.tx.unbounded_send(job).is_ok()
    }

    fn read(&self, f: &mut dyn FnMut(&DeckState)) {
        let m = self
            .mirror
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&m);
    }
}

/// Puts the `SCOPE_*` actions on the app's list while the decks are
/// mounted: a write runs here as a click does ([`DeckHandle::act`]), a
/// read answers from a mirror kept current; each change is a `decks`
/// event.
fn use_deck_actions(handle: DeckHandle, actions: Option<super::WorkspaceActions>) {
    use futures_util::StreamExt as _;
    let mirror: DeckMirror =
        use_hook(|| std::sync::Arc::new(std::sync::Mutex::new(handle.act(|st| st.clone()))));
    let list = actions.map(|a| a.0);
    let runner = use_coroutine({
        let (mirror, list) = (mirror.clone(), list.clone());
        move |mut jobs: UnboundedReceiver<crate::control::deck::DeckJob>| {
            let (mirror, list) = (mirror.clone(), list.clone());
            async move {
                while let Some(job) = jobs.next().await {
                    let kept = UiKept(
                        handle.stores.peek().contexts.clone(),
                        handle.resolve.peek().clone(),
                    );
                    let before = handle.decks.peek().clone();
                    let now = handle.act(|st| {
                        job(st, &kept);
                        st.clone()
                    });
                    handle.tell_fresh(&before);
                    *mirror
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = now;
                    let after = handle.decks.peek().clone();
                    if after != before {
                        handle.keep_all();
                        if let Some(list) = &list {
                            let active = after.active.map(|a| a.0).unwrap_or_default();
                            list.changed(crate::control::deck::TOPIC, &active);
                        }
                    }
                }
            }
        }
    });
    // The mirror follows every change, whoever made it.
    {
        let mirror = mirror.clone();
        use_effect(move || {
            let _ = (
                handle.decks.read(),
                handle.screensets.read(),
                handle.workspace.read(),
            );
            let now = handle.act(|st| st.clone());
            *mirror
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = now;
        });
    }
    let registered = use_hook(move || {
        let list = list?;
        let exec: std::sync::Arc<dyn crate::control::deck::DeckExec> =
            std::sync::Arc::new(OnWindow {
                tx: runner.tx(),
                mirror: mirror.clone(),
            });
        crate::control::deck::register(&list, &exec).ok()?;
        Some(list)
    });
    use_drop(move || {
        if let Some(list) = registered {
            for id in crate::control::deck::ids::ALL {
                list.unregister(id);
            }
        }
    });
}

/// Scope tabs take ⌘1–9 and ⌃Tab (and show) once one is open; with
/// none, the keys are the screensets' and a group's tabs' as without them.
#[must_use]
pub(super) fn switching() -> bool {
    try_use_decks().is_some_and(|h| !h.decks.read().decks.is_empty())
}

/// The decks under [`use_decks`], if there are any.
#[must_use]
pub fn try_use_decks() -> Option<DeckHandle> {
    try_use_context::<DeckHandle>()
}

/// The context of the deck on show: what a view reads to show its song or
/// project (its scope tab's while that one shows; the general tabs' while
/// they do).
#[must_use]
pub fn use_deck_context() -> Option<DeckContext> {
    let handle = try_use_decks()?;
    let decks = handle.decks.read();
    Some(decks.shown().context.clone())
}

impl DeckHandle {
    /// Runs `f` over the decks as the operations see them, and puts back
    /// what it changed: the decks, the live layout, the screensets.
    pub fn act<R>(&self, f: impl FnOnce(&mut DeckState) -> R) -> R {
        let mut st = DeckState::new(
            self.decks.peek().clone(),
            self.screensets.peek().clone(),
            self.workspace.peek().clone(),
        )
        .with_form(*self.form.peek())
        .with_templates(self.templates.peek().clone());
        let decks_before = st.decks.clone();
        let out = f(&mut st);
        if *self.workspace.peek() != st.live {
            let mut w = self.workspace;
            w.set(st.live);
        }
        if *self.screensets.peek() != st.screensets {
            let mut s = self.screensets;
            s.set(st.screensets);
        }
        if st.decks != decks_before {
            let mut d = self.decks;
            d.set(st.decks);
            self.settle();
        }
        out
    }

    /// Something changed that is kept: written when things settle.
    fn settle(&self) {
        let mut dirty = self.dirty;
        let n = dirty.peek().wrapping_add(1);
        dirty.set(n);
    }

    /// Writes the session, the saved screensets and every deck's layout
    /// with its context, now.
    pub fn keep_all(&self) {
        let stores = self.stores.peek().clone();
        let decks = self.decks.peek().clone();
        if let Some(session) = &stores.session {
            let _ = store::save_session(session.0.as_ref(), &decks);
        }
        if let Some(saved) = &stores.screensets {
            if let Ok(json) = facet_json::to_string(&self.screensets.peek().saved()) {
                saved.0.save(&json);
            }
        }
        if let Some(contexts) = &stores.contexts {
            for d in decks.decks.iter().chain(decks.closed.iter()) {
                let _ = store::save_context(contexts.as_ref(), d);
            }
        }
    }

    /// What `context` kept, if its store has it.
    #[must_use]
    pub fn kept(&self, context: &DeckContext) -> Option<ContextLayout> {
        let contexts = self.stores.peek().contexts.clone()?;
        store::load_context(contexts.as_ref(), context)
    }

    /// Loads `context` onto the deck on show, or onto a new deck: the
    /// navigator's click, and its ⌘-click or middle-click. A deck in a
    /// window of its own comes forward instead.
    pub fn load(&self, context: DeckContext, load: Load) {
        let context = match self.resolve.peek().as_ref() {
            Some(f) => f(context),
            None => context,
        };
        let open = self.decks.peek().find(&context).map(|d| d.id.clone());
        if let Some(open) = open {
            if self.is_popped(&open) && load == Load::Here {
                self.pop_out(&open);
                return;
            }
        }
        let kept = self.kept(&context);
        let before = self.decks.peek().clone();
        let _ = self.act(|st| st.open(context, load, None, kept));
        self.tell_fresh(&before);
    }

    /// Tells [`DecksConfig::on_fresh`] of each context on a deck now that
    /// was on none in `before` and kept nothing.
    fn tell_fresh(&self, before: &Decks) {
        let Some(on_fresh) = *self.on_fresh.peek() else {
            return;
        };
        let fresh: Vec<DeckContext> = self
            .decks
            .peek()
            .decks
            .iter()
            .map(|d| d.context.clone())
            .filter(|c| before.find(c).is_none())
            .filter(|c| self.kept(c).is_none_or(|k| k.layouts.is_empty()))
            .collect();
        for c in fresh {
            on_fresh.call(c);
        }
    }

    /// Shows the general tabs: no scope tab active.
    pub fn show_general(&self) {
        let _ = self.act(DeckState::show_general);
    }

    /// Scope tab `id`'s colour (`None`: its context's own again is gone,
    /// its kind's shows).
    pub fn set_colour(&self, id: &DeckId, colour: Option<String>) {
        let mut decks = self.decks;
        if decks
            .peek()
            .get(id)
            .is_some_and(|d| d.context.colour != colour)
        {
            if let Some(d) = decks.write().get_mut(id) {
                d.context.colour = colour;
            }
            self.settle();
        }
    }

    /// Shows deck `id` (a deck in its own window comes forward).
    pub fn activate(&self, id: &DeckId) {
        if self.is_popped(id) {
            self.pop_out(id);
            return;
        }
        let _ = self.act(|st| st.activate(id));
    }

    /// Shows the deck `by` places along (⌃Tab), passing popped-out ones.
    pub fn cycle(&self, by: isize) {
        let n = self.decks.peek().decks.len();
        for _ in 0..n {
            let _ = self.act(|st| st.cycle(by));
            let active = self.decks.peek().active.clone();
            if active.is_none_or(|a| !self.is_popped(&a)) {
                return;
            }
        }
    }

    /// Shows the deck at `index` (⌘1–9).
    pub fn activate_nth(&self, index: usize) {
        let id = {
            let decks = self.decks.peek();
            let i = if index >= 8 {
                decks.decks.len().saturating_sub(1)
            } else {
                index
            };
            decks.decks.get(i).map(|d| d.id.clone())
        };
        if let Some(id) = id {
            self.activate(&id);
        }
    }

    pub fn close(&self, id: &DeckId) {
        if self.is_popped(id) {
            self.dock_back(id);
        }
        let _ = self.act(|st| st.close(id));
        self.keep_all();
    }

    pub fn duplicate(&self, id: &DeckId) {
        let _ = self.act(|st| st.duplicate(id));
    }

    pub fn move_to(&self, id: &DeckId, index: usize) {
        let _ = self.act(|st| st.move_to(id, index));
    }

    pub fn reopen(&self) {
        let _ = self.act(DeckState::reopen);
    }

    pub fn set_screenset(&self, id: &DeckId, screenset: &str) {
        let _ = self.act(|st| st.set_screenset(id, screenset));
    }

    /// Saves deck `id`'s layout as a screenset called `name`; its id.
    #[must_use]
    pub fn save_as_screenset(&self, id: &DeckId, name: &str) -> Option<String> {
        self.act(|st| st.save_as_screenset(id, name)).ok()
    }

    pub fn reset(&self, id: &DeckId) {
        let _ = self.act(|st| st.reset(id));
    }

    /// The app's badge on deck `id` (a song's unsaved session).
    pub fn set_badge(&self, id: &DeckId, badge: Option<String>) {
        let mut decks = self.decks;
        if decks.peek().get(id).is_some_and(|d| d.badge != badge) {
            if let Some(d) = decks.write().get_mut(id) {
                d.badge = badge;
            }
        }
    }

    /// Whether deck `id` is in a window of its own.
    #[must_use]
    pub fn is_popped(&self, id: &DeckId) -> bool {
        self.popped.peek().iter().any(|p| &*p.deck.peek() == id)
    }

    /// Decks pop out here: the app gave its views and the platform opens
    /// windows.
    #[must_use]
    pub fn can_pop_out(&self) -> bool {
        self.views.peek().is_some() && self.windows.peek().is_some()
    }

    /// Deck `id` into a window of its own (or that window forward): the
    /// main window shows its neighbour.
    pub fn pop_out(&self, id: &DeckId) {
        let (Some(views), Some(windows)) = (self.views.peek().clone(), self.windows.peek().clone())
        else {
            return;
        };
        let title = self
            .decks
            .peek()
            .get(id)
            .map(|d| d.context.title.clone())
            .unwrap_or_default();
        let window = WindowId(format!("{NOUN}-{}", id.0));
        let existing = self
            .popped
            .peek()
            .iter()
            .find(|p| &*p.deck.peek() == id)
            .copied();
        let popped = existing.unwrap_or_else(|| self.popped_for(id));
        let carried = Bridge::new()
            .context::<Signal<architect_shell::Shell>>()
            .context::<architect_theme::ThemeHandle>()
            .context::<architect_theme::RootCss>()
            .and(&self.bridge.peek());
        let theme = self.theme.peek().clone();
        let handle = *self;
        let draw: Rc<dyn Fn() -> Element> = Rc::new(move || {
            rsx! {
                DeckWindow { handle, popped, views: views.clone(), theme: theme.clone() }
            }
        });
        let back = id.clone();
        let on_closed: Rc<dyn Fn()> = Rc::new(move || handle.dock_back(&back));
        windows
            .0
            .open(Popout::page(window, title, None, carried, draw, on_closed));
    }

    /// A new window's own layout for deck `id`, apart from the main
    /// window's from now on (which shows the next deck when it showed it).
    fn popped_for(&self, id: &DeckId) -> Popped {
        if self.decks.peek().active.as_ref() == Some(id) {
            let _ = self.act(DeckState::record);
            let next = {
                let decks = self.decks.peek();
                decks
                    .decks
                    .iter()
                    .find(|d| &d.id != id && !self.is_popped(&d.id))
                    .map(|d| d.id.clone())
            };
            if let Some(next) = next {
                let _ = self.act(|st| st.activate(&next));
            }
        }
        let live = self.act(|st| st.arrangement(id));
        let p = Popped {
            deck: CopyValue::new_in_scope(id.clone(), ScopeId::ROOT),
            workspace: Signal::new_in_scope(live, ScopeId::ROOT),
        };
        let mut all = self.popped;
        all.write().push(p);
        p
    }

    /// Deck `id` back from its window into the strip, its layout kept.
    pub fn dock_back(&self, id: &DeckId) {
        let found = self
            .popped
            .peek()
            .iter()
            .find(|p| &*p.deck.peek() == id)
            .copied();
        let Some(p) = found else {
            return;
        };
        let live = p.workspace.peek().clone();
        let _ = self.act(|st| st.keep(id, &live));
        let mut all = self.popped;
        all.write().retain(|q| &*q.deck.peek() != id);
        if let Some(windows) = self.windows.peek().clone() {
            windows.0.close(&WindowId(format!("{NOUN}-{}", id.0)));
        }
        self.settle();
    }

    fn keep_popped(&self, id: &DeckId, live: &Workspace) {
        if self.act(|st| st.keep(id, live)) {
            self.settle();
        }
    }
}

/// A deck in a window of its own: a workspace over its own layout, kept
/// into the deck as it changes.
#[component]
fn DeckWindow(
    handle: DeckHandle,
    popped: Popped,
    views: Views,
    theme: Option<ShellTheme>,
) -> Element {
    let workspace = popped.workspace;
    use_effect(move || {
        let live = workspace.read().clone();
        let id = popped.deck.peek().clone();
        handle.keep_popped(&id, &live);
    });
    rsx! {
        div { style: "width:100%; height:100%; display:flex;",
            super::WorkspaceView { workspace, views, theme, arrange_key: None }
        }
    }
}

/// The scope tab commands, in the palette and on their keys while the
/// decks are mounted (the host draws them): ⌘1–9, ⌃Tab and ⌃⇧Tab, the
/// general tabs, close, reopen, duplicate, the screenset (switch, save
/// as, reset), pop out.
#[component]
pub(super) fn DeckCommands(handle: DeckHandle) -> Element {
    let decks = handle.decks.read();
    let n = decks.decks.len();
    let active = decks.active.clone();
    let titles: Vec<String> = decks
        .decks
        .iter()
        .take(9)
        .map(|d| d.context.title.clone())
        .collect();
    let modified = decks
        .active_deck()
        .is_some_and(|d| d.modified(*handle.form.peek()));
    let on_screenset = decks.active_deck().map(|d| d.screenset.clone());
    let closed = !decks.closed.is_empty();
    drop(decks);
    let sets: Vec<(String, String)> = handle
        .screensets
        .read()
        .sets
        .iter()
        .map(|s| (s.id.clone(), s.name.clone()))
        .collect();
    let group = crate::deck::NOUN_PLURAL_TITLE;
    for i in 0..9_usize {
        let title = if i == 8 { titles.last() } else { titles.get(i) };
        let label = title.map_or_else(
            || format!("Show {NOUN} {}", i.saturating_add(1)),
            |t| format!("Show {NOUN}: {t}"),
        );
        let mut c = Command::new(
            format!(
                "{}SHOW_{}",
                crate::control::deck::ids::PREFIX,
                i.saturating_add(1)
            ),
            label,
        )
        .group(group)
        .shortcut(if n >= 1 {
            format!("⌘{}", i.saturating_add(1))
        } else {
            String::new()
        });
        if title.is_none() {
            c = c.disabled(format!("no {NOUN} is there"));
        }
        use_command(c, move || handle.activate_nth(i));
    }
    let several = |c: Command| {
        if n >= 1 {
            c
        } else {
            c.disabled(format!("no {NOUN} is open"))
        }
    };
    use_command(
        several(
            Command::new(crate::control::deck::ids::NEXT, format!("Next {NOUN}"))
                .group(group)
                .shortcut(if n >= 1 { "⌃Tab" } else { "" }),
        ),
        move || handle.cycle(1),
    );
    use_command(
        several(
            Command::new(crate::control::deck::ids::PREV, format!("Previous {NOUN}"))
                .group(group)
                .shortcut(if n >= 1 { "⌃⇧Tab" } else { "" }),
        ),
        move || handle.cycle(-1),
    );
    let general_shown = active.is_none();
    let general = Command::new(
        crate::control::deck::ids::GENERAL,
        format!("Show the {}", crate::deck::GENERAL_NOUN),
    )
    .group(group);
    use_command(
        if general_shown {
            general.disabled(format!("the {} show", crate::deck::GENERAL_NOUN))
        } else {
            general
        },
        move || handle.show_general(),
    );
    let on_active = |c: Command| {
        if active.is_some() {
            c
        } else {
            c.disabled(format!("no {NOUN} is open"))
        }
    };
    let a1 = active.clone();
    use_command(
        on_active(
            Command::new(crate::control::deck::ids::CLOSE, format!("Close {NOUN}")).group(group),
        ),
        move || {
            if let Some(id) = &a1 {
                handle.close(id);
            }
        },
    );
    use_command(
        if closed {
            Command::new(
                crate::control::deck::ids::REOPEN,
                format!("Reopen closed {NOUN}"),
            )
        } else {
            Command::new(
                crate::control::deck::ids::REOPEN,
                format!("Reopen closed {NOUN}"),
            )
            .disabled("nothing was closed")
        }
        .group(group),
        move || handle.reopen(),
    );
    let a2 = active.clone();
    use_command(
        on_active(
            Command::new(
                crate::control::deck::ids::DUPLICATE,
                format!("Duplicate {NOUN}"),
            )
            .group(group),
        ),
        move || {
            if let Some(id) = &a2 {
                handle.duplicate(id);
            }
        },
    );
    let a3 = active.clone();
    let reset =
        Command::new(crate::control::deck::ids::RESET, "Reset to the screenset").group(group);
    use_command(
        if modified {
            reset
        } else {
            reset.disabled(format!("the {NOUN} shows its screenset as saved"))
        },
        move || {
            if let Some(id) = &a3 {
                handle.reset(id);
            }
        },
    );
    let a4 = active.clone();
    use_command(
        on_active(
            Command::new(
                crate::control::deck::ids::SAVE_AS_SCREENSET,
                "Save as screenset",
            )
            .group(group),
        ),
        move || {
            let Some(id) = &a4 else {
                return;
            };
            let title = handle
                .decks
                .peek()
                .get(id)
                .map(|d| d.context.title.clone())
                .unwrap_or_default();
            let _ = handle.save_as_screenset(id, &format!("{title} layout"));
        },
    );
    let a5 = active.clone();
    use_command(
        if handle.can_pop_out() {
            on_active(
                Command::new(
                    concat!(scope_word!(prefix), "POP_OUT"),
                    format!("Pop {NOUN} out into a window"),
                )
                .group(group),
            )
        } else {
            Command::new(
                concat!(scope_word!(prefix), "POP_OUT"),
                format!("Pop {NOUN} out into a window"),
            )
            .group(group)
            .disabled("windows do not open here")
        },
        move || {
            if let Some(id) = &a5 {
                handle.pop_out(id);
            }
        },
    );
    rsx! {
        for (id, name) in sets {
            SwitchCommand {
                key: "{id}",
                handle,
                on: on_screenset.as_deref() == Some(id.as_str()),
                id,
                name,
                deck: active.clone(),
            }
        }
    }
}

/// "Switch this deck's screenset: Mixing": one palette row per screenset.
#[component]
fn SwitchCommand(
    handle: DeckHandle,
    id: String,
    name: String,
    on: bool,
    deck: Option<DeckId>,
) -> Element {
    let command_id = format!(
        "{}{}",
        crate::control::deck::ids::SET_SCREENSET,
        id.to_uppercase()
            .replace(|c: char| !c.is_ascii_alphanumeric(), "_")
    );
    let c = Command::new(
        command_id,
        format!("Switch this {NOUN}'s screenset: {name}"),
    )
    .group(crate::deck::NOUN_PLURAL_TITLE);
    let c = match (&deck, on) {
        (None, _) => c.disabled(format!("no {NOUN} is open")),
        (_, true) => c.disabled(format!("the {NOUN} is shown in it")),
        _ => c,
    };
    use_command(c, move || {
        if let Some(d) = &deck {
            handle.set_screenset(d, &id);
        }
    });
    rsx! {}
}

/// What a scope tab's menu is open on.
#[derive(Clone, PartialEq)]
struct ScopeMenuAt {
    deck: DeckId,
    x: f32,
    y: f32,
}

/// A drag along the scope tabs: the one held, and where it started.
#[derive(Clone, PartialEq)]
struct ScopeDrag {
    deck: DeckId,
    from_x: f32,
    moved: bool,
}

const COMPACT_MAX: f32 = 180.0;
const WIDE_W: f32 = 196.0;
const SLOP: f32 = 6.0;

/// The colours a person can give a scope tab from its menu.
const SWATCHES: [&str; 7] = [
    "#e05a47", "#e8913a", "#d9b23a", "#4fa36b", "#3f8fd1", "#7c5cff", "#c2569e",
];

/// The scope tabs, at the leading edge of the main window's first tab bar
/// (`docs/content/architecture/workspace.md`, "Decks"): one per deck, drawn
/// its kind's way ([`crate::TabKind`]), then a divider before the tabs of
/// the scope on show. Nothing with no scope tab open, so the bar is as
/// without them. A click shows one (the one on show again: the general
/// tabs); a middle-click closes; a drag reorders; its menu has close,
/// colour, the screenset's rows and its kind's own.
#[component]
pub(super) fn ScopeTabs(height: f32) -> Element {
    let host = super::use_host();
    let Some(handle) = *host.decks.peek() else {
        return rsx! {};
    };
    let t = host.theme.read().clone();
    let mut menu = use_signal(|| None::<ScopeMenuAt>);
    let mut drag = use_signal(|| None::<ScopeDrag>);
    let decks = handle.decks.read().clone();
    if decks.decks.is_empty() {
        return rsx! {};
    }
    let kinds = super::try_use_tab_kinds().unwrap_or_default();
    let form = *handle.form.read();
    let popped: Vec<DeckId> = handle
        .popped
        .read()
        .iter()
        .map(|p| p.deck.peek().clone())
        .collect();
    let tabs = decks.decks.iter().enumerate().map(|(i, d)| {
        let id = d.id.clone();
        let active = decks.active.as_ref() == Some(&d.id);
        let face = kinds.kinds.face(&d.context, d.badge.clone());
        let wide = face.width == crate::TabWidth::Wide;
        let render = kinds.render_for(&face.kind);
        let pill_h = if wide { (height - 6.0).min(36.0) } else { (height - 10.0).min(28.0) };
        let modified = d.modified(form);
        let out = popped.contains(&d.id);
        let title = face.title.clone();
        let colour = face.colour.clone().unwrap_or_else(|| t.accent.to_string());
        let (bg, edge, ink) = if active {
            (t.sheet.to_string(), t.rule.to_string(), t.ink.to_string())
        } else {
            ("transparent".to_owned(), "transparent".to_owned(), t.ink_2.to_string())
        };
        let lifted = drag.read().as_ref().is_some_and(|g| g.moved && g.deck == id);
        let shadow = if lifted { format!("box-shadow:{};", t.shadow_lift) } else { String::new() };
        // They keep their room: the scope's own tabs give way first.
        let size = if wide {
            format!("flex:0 0 {WIDE_W}px; width:{WIDE_W}px;")
        } else {
            format!("flex:0 0 auto; max-width:{COMPACT_MAX}px;")
        };
        let hint = if active {
            format!("{title} — click again for the {}", crate::deck::GENERAL_NOUN)
        } else if modified {
            format!("{title} — changed from its screenset")
        } else {
            title.clone()
        };
        let label = format!("{title}, {NOUN}");
        let inner = render.map_or_else(
            || rsx! { FaceBody { face: face.clone(), on: active, theme: t.clone() } },
            |r| r(super::TabRenderCtx { face: face.clone(), on: active, height: pill_h, theme: t.clone() }),
        );
        let (id_click, id_close, id_menu, id_down, id_move) =
            (id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
        rsx! {
            div {
                key: "{id.0}",
                role: "tab",
                "aria-selected": if active { "true" } else { "false" },
                "aria-label": "{label}",
                "data-scope-tab": "{id.0}",
                "data-tab-kind": "{face.kind}",
                "data-tab-width": face.width.as_str(),
                "data-screenset": "{d.screenset}",
                title: "{hint}",
                style: "position:relative; {size} height:{pill_h}px; align-self:center; box-sizing:border-box; \
                        display:flex; align-items:center; gap:6px; padding:0 6px 0 8px; overflow:hidden; \
                        border-radius:8px; border:1px solid {edge}; background:{bg}; color:{ink}; \
                        cursor:default; user-select:none; touch-action:none; {shadow}",
                onpointerdown: move |e: PointerEvent| {
                    e.stop_propagation();
                    match e.data().trigger_button() {
                        Some(dioxus::html::input_data::MouseButton::Auxiliary) => handle.close(&id_down),
                        Some(dioxus::html::input_data::MouseButton::Primary) | None => {
                            drag.set(Some(ScopeDrag {
                                deck: id_down.clone(),
                                from_x: super::narrow(e.data().client_coordinates().x),
                                moved: false,
                            }));
                        }
                        _ => {}
                    }
                },
                onpointermove: move |e: PointerEvent| {
                    let x = super::narrow(e.data().client_coordinates().x);
                    let held = drag.peek().clone();
                    let Some(g) = held else { return };
                    if !g.moved && (x - g.from_x).abs() < SLOP {
                        return;
                    }
                    if !g.moved {
                        drag.set(Some(ScopeDrag { moved: true, ..g.clone() }));
                    }
                    // Over another scope tab: the held one takes its place.
                    if g.deck != id_move {
                        handle.move_to(&g.deck, i);
                    }
                },
                onpointerup: move |_| {
                    let held = drag.peek().clone();
                    drag.set(None);
                    if held.is_some_and(|g| !g.moved) {
                        // The one on show again: back to the general tabs.
                        if handle.decks.peek().active.as_ref() == Some(&id_click) {
                            handle.show_general();
                        } else {
                            handle.activate(&id_click);
                        }
                    }
                },
                oncontextmenu: move |e: MouseEvent| {
                    e.prevent_default();
                    e.stop_propagation();
                    let p = e.data().client_coordinates();
                    menu.set(Some(ScopeMenuAt { deck: id_menu.clone(), x: super::narrow(p.x), y: super::narrow(p.y) }));
                },
                // Its colour along its foot while it shows (a wide one's
                // runs down its side always).
                if wide {
                    span { style: "flex:0 0 3px; width:3px; align-self:stretch; margin:5px 0; border-radius:2px; background:{colour};" }
                } else if active {
                    span { style: "position:absolute; left:10px; right:10px; bottom:0; height:2px; border-radius:2px; background:{colour};" }
                }
                if out {
                    span { style: "flex:0 0 auto; display:flex;", AppWindow { size: 12 } }
                }
                {inner}
                if modified {
                    span {
                        "aria-label": "changed from its screenset",
                        style: "flex:0 0 auto; width:6px; height:6px; border-radius:999px; background:{t.accent};",
                    }
                }
                if active {
                    button {
                        "aria-label": "Close {title}",
                        title: "Close {title}",
                        style: "flex:0 0 auto; width:18px; height:18px; display:flex; align-items:center; justify-content:center; \
                                border:none; border-radius:4px; background:transparent; color:{t.ink_3}; padding:0;",
                        onpointerdown: move |e: PointerEvent| e.stop_propagation(),
                        onclick: move |e: MouseEvent| {
                            e.stop_propagation();
                            handle.close(&id_close);
                        },
                        X { size: 12 }
                    }
                }
            }
        }
    });
    let open_menu = menu.read().clone();
    rsx! {
        div {
            role: "tablist",
            "aria-label": crate::deck::NOUN_PLURAL_TITLE,
            "data-scope-tabs": "{decks.decks.len()}",
            style: "flex:0 0 auto; height:{height}px; display:flex; align-items:center; gap:3px; \
                    position:relative;",
            onpointerup: move |_| drag.set(None),
            onpointerleave: move |_| drag.set(None),
            {tabs}
        }
        // The divider between the scope tabs and the scope's own tabs.
        div {
            "data-scope-divider": "true",
            style: "flex:0 0 1px; width:1px; height:{(height * 0.5).max(14.0)}px; align-self:center; margin:0 8px 0 6px; \
                    background:{t.rule};",
        }
        if let Some(m) = open_menu {
            ScopeMenu { handle, menu: m, on_done: move |()| menu.set(None) }
        }
    }
}

/// What a tab of a kind with no renderer shows: its icon in its colour,
/// its title, a wide kind's subtitle under it, its badge.
#[component]
pub(super) fn FaceBody(face: crate::TabFace, on: bool, theme: ShellTheme) -> Element {
    let t = theme;
    let wide = face.width == crate::TabWidth::Wide;
    let glyph = face.icon.clone().map(|i| {
        let i = i.trim().trim_matches('"').to_owned();
        i.strip_prefix("lucide:")
            .map_or_else(|| i.clone(), str::to_owned)
    });
    let lucide = glyph
        .clone()
        .filter(|g| architect_shell::lucide::glyphs(g).is_some());
    let emoji = glyph
        .filter(|g| lucide.is_none() && !g.is_ascii())
        .map(|g| g.strip_prefix("emoji:").unwrap_or(&g).to_owned());
    let weight = if on { 600 } else { 500 };
    let icon_colour = face.colour.clone();
    rsx! {
        if let Some(name) = lucide {
            span { "data-tab-icon": "{name}", style: "flex:0 0 auto; display:flex;",
                architect_shell::lucide::LucideIcon { name: name.clone(), size: if wide { 15 } else { 14 }, color: icon_colour }
            }
        }
        if let Some(e) = emoji {
            span { "data-tab-icon": "{e}", style: "flex:0 0 auto; font-size:13px; line-height:1;", "{e}" }
        }
        span { style: "flex:1 1 auto; min-width:0; display:flex; flex-direction:column; justify-content:center; overflow:hidden;",
            span { style: "white-space:nowrap; overflow:hidden; font-size:12px; font-weight:{weight}; line-height:1.25;",
                "{face.title}"
            }
            if let Some(sub) = face.subtitle.clone().filter(|_| wide) {
                span { "data-tab-subtitle": "{sub}",
                    style: "white-space:nowrap; overflow:hidden; font-size:10.5px; color:{t.ink_3}; line-height:1.25;",
                    "{sub}"
                }
            }
        }
        if let Some(b) = face.badge {
            span { style: "flex:0 0 auto; padding:0 6px; border-radius:999px; background:{t.accent}; \
                           color:{t.accent_ink}; font-size:10px; font-weight:700;",
                "{b}"
            }
        }
    }
}

/// A scope tab's menu, where it was opened: duplicate, pop out, its
/// colour, its screenset (switch, save as, reset), its kind's own rows,
/// close.
#[component]
fn ScopeMenu(handle: DeckHandle, menu: ScopeMenuAt, on_done: EventHandler<()>) -> Element {
    let t = architect_shell::try_use_shell()
        .map(|s| s.theme)
        .unwrap_or_default();
    let id = menu.deck.clone();
    let deck = handle.decks.read().get(&id).cloned();
    let Some(deck) = deck else {
        return rsx! {};
    };
    let kind = super::try_use_tab_kinds().and_then(|k| k.kinds.scope(&deck.context.kind).cloned());
    let form = *handle.form.read();
    let sets: Vec<(String, String)> = handle
        .screensets
        .read()
        .sets
        .iter()
        .map(|s| (s.id.clone(), s.name.clone()))
        .collect();
    let can_pop = handle.can_pop_out();
    let popped = handle.is_popped(&id);
    let row = "display:block; width:100%; text-align:left; padding:6px 12px; border:none; background:transparent; \
               font-size:12px;";
    let (i1, i2, i3, i4, i5, i6) = (
        id.clone(),
        id.clone(),
        id.clone(),
        id.clone(),
        id.clone(),
        id.clone(),
    );
    let reset_ink = if deck.modified(form) {
        t.ink.clone()
    } else {
        t.ink_3.clone()
    };
    let items = kind.map(|k| k.menu).unwrap_or_default();
    let context_json = facet_json::to_string(&deck.context).unwrap_or_default();
    rsx! {
        div {
            style: "position:fixed; inset:0; z-index:{architect_shell::layer::MENU};",
            onpointerdown: move |e: PointerEvent| {
                e.stop_propagation();
                on_done.call(());
            },
        }
        div {
            role: "menu",
            "aria-label": "{deck.context.title}",
            "data-scope-menu": "{id.0}",
            style: "position:fixed; left:{(menu.x - 20.0).max(4.0)}px; top:{menu.y + 6.0}px; min-width:220px; padding:4px 0; \
                    border-radius:8px; background:{t.sheet}; color:{t.ink}; border:1px solid {t.rule}; \
                    box-shadow:{t.shadow_float}; z-index:{architect_shell::layer::MENU}; font-family:{t.font};",
            onpointerdown: move |e: PointerEvent| e.stop_propagation(),
            button { role: "menuitem", style: "{row} color:{t.ink};",
                onclick: move |_| { handle.duplicate(&i1); on_done.call(()); },
                "Duplicate {NOUN}"
            }
            if can_pop {
                button { role: "menuitem", style: "{row} color:{t.ink};",
                    onclick: move |_| {
                        if popped { handle.dock_back(&i2) } else { handle.pop_out(&i2) }
                        on_done.call(());
                    },
                    if popped { "Back into the tab bar" } else { "Pop out into a window" }
                }
            }
            div { style: "height:1px; margin:4px 0; background:{t.rule};" }
            div { style: "padding:4px 12px; font-size:11px; color:{t.ink_3};", "Colour" }
            div { style: "display:flex; gap:6px; padding:2px 12px 6px;",
                for c in SWATCHES {
                    button {
                        key: "{c}",
                        role: "menuitemradio",
                        "aria-label": "Colour {c}",
                        "aria-checked": if deck.context.colour.as_deref() == Some(c) { "true" } else { "false" },
                        style: "width:16px; height:16px; border-radius:999px; border:1px solid {t.rule}; background:{c}; padding:0;",
                        onclick: {
                            let id = i6.clone();
                            move |_| { handle.set_colour(&id, Some(c.to_owned())); on_done.call(()); }
                        },
                    }
                }
            }
            div { style: "height:1px; margin:4px 0; background:{t.rule};" }
            div { style: "padding:4px 12px; font-size:11px; color:{t.ink_3};", "Switch this {NOUN}'s screenset" }
            for (sid, name) in sets {
                SwitchRow {
                    key: "{sid}",
                    handle,
                    deck: i3.clone(),
                    on: deck.screenset == sid,
                    sid,
                    name,
                    on_done,
                }
            }
            div { style: "height:1px; margin:4px 0; background:{t.rule};" }
            button { role: "menuitem", style: "{row} color:{t.ink};",
                onclick: {
                    let title = deck.context.title.clone();
                    move |_| {
                        let _ = handle.save_as_screenset(&i4, &format!("{title} layout"));
                        on_done.call(());
                    }
                },
                "Save as screenset"
            }
            button { role: "menuitem",
                disabled: !deck.modified(form),
                style: "{row} color:{reset_ink};",
                onclick: move |_| { handle.reset(&i5); on_done.call(()); },
                "Reset to the screenset"
            }
            if !items.is_empty() {
                div { style: "height:1px; margin:4px 0; background:{t.rule};" }
                for item in items {
                    button { key: "{item.action}", role: "menuitem", style: "{row} color:{t.ink};",
                        onclick: {
                            let (action, args, on) = (item.action, format!("{{\"context\":{context_json}}}"), id.0.clone());
                            move |_| {
                                if let Some(c) = architect_commands::try_use_commands() {
                                    let invocation = architect::action::Invocation::new(
                                        architect::action::InvocationSource::ContextMenu,
                                    )
                                    .on("scope", &on);
                                    let _ = c.run_invoked(&action, Some(&args), invocation);
                                }
                                on_done.call(());
                            }
                        },
                        "{item.label}"
                    }
                }
            }
            div { style: "height:1px; margin:4px 0; background:{t.rule};" }
            button { role: "menuitem", style: "{row} color:{t.danger};",
                onclick: move |_| { handle.close(&id); on_done.call(()); },
                "Close {NOUN}"
            }
        }
    }
}

#[component]
fn SwitchRow(
    handle: DeckHandle,
    deck: DeckId,
    sid: String,
    name: String,
    on: bool,
    on_done: EventHandler<()>,
) -> Element {
    let t = architect_shell::try_use_shell()
        .map(|s| s.theme)
        .unwrap_or_default();
    let tick = if on { "✓ " } else { "" };
    rsx! {
        button {
            role: "menuitemradio",
            "aria-checked": if on { "true" } else { "false" },
            style: "display:block; width:100%; text-align:left; padding:6px 12px 6px 20px; border:none; \
                    background:transparent; font-size:12px; color:{t.ink};",
            onclick: move |_| {
                handle.set_screenset(&deck, &sid);
                on_done.call(());
            },
            "{tick}{name}"
        }
    }
}

/// The scope tabs as a phone's switcher lists them (read as its menu is
/// drawn): the general tabs and each scope tab, its id, title, whether it
/// shows.
#[must_use]
pub(super) fn switcher_rows() -> Option<(DeckHandle, Vec<(DeckId, String, bool)>)> {
    let handle = try_consume_context::<DeckHandle>()?;
    let decks = handle.decks.peek();
    let mut rows: Vec<(DeckId, String, bool)> = decks
        .decks
        .iter()
        .map(|d| {
            (
                d.id.clone(),
                d.context.title.clone(),
                decks.active.as_ref() == Some(&d.id),
            )
        })
        .collect();
    // The general tabs first, once a scope tab is open.
    if !rows.is_empty() {
        rows.insert(
            0,
            (
                decks.general.id.clone(),
                scope_word!(general_title).to_owned(),
                decks.general_shown(),
            ),
        );
    }
    drop(decks);
    Some((handle, rows))
}
