//! Decks in the app: the strip at the top of the main window, its keys
//! and menus, a deck popped out into a window of its own, and the decks'
//! part in a phone's switcher.
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
use lucide_dioxus::{AppWindow, Plus, X};

use super::screens::WorkspaceStore;
use super::windows::{Bridge, Popout, Windows};
use super::{TitleRow, Views};
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
    /// The decks a first run (nothing kept) opens with.
    pub start: Vec<DeckContext>,
    /// The views and contexts a deck popped out into a window draws with;
    /// without `views` a deck does not pop out.
    pub views: Option<Views>,
    pub bridge: Bridge,
    /// What the strip's `+` loads onto a new deck: the app asks (a
    /// picker), or it is the deck on show again.
    pub on_new: Option<Callback<()>>,
    /// The app's Action List: the `DECK_*` actions go on it, run here.
    pub actions: Option<super::WorkspaceActions>,
}

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
    on_new: CopyValue<Option<Callback<()>>>,
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
        views,
        bridge,
        on_new,
        actions,
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
        on_new: CopyValue::new(on_new),
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
    handle.on_new.clone().set(on_new);
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
        let first = decks.decks.is_empty();
        let mut d = handle.decks;
        d.set(decks);
        if first {
            for context in start {
                let kept = handle.kept(&context);
                let _ = handle.act(|st| st.open(context, Load::NewDeck, None, kept));
            }
            let first = handle.decks.peek().decks.first().map(|d| d.id.clone());
            if let Some(id) = first {
                let _ = handle.act(|st| st.activate(&id));
            }
        } else {
            handle.act(DeckState::show);
        }
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

/// What a context kept, read from the app's store (for `DECK_OPEN`).
struct UiKept(Option<Rc<dyn ContextStore>>);

impl crate::control::deck::Kept for UiKept {
    fn kept(&self, context: &DeckContext) -> Option<ContextLayout> {
        store::load_context(self.0.as_ref()?.as_ref(), context)
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

/// Puts the `DECK_*` actions on the app's list while the decks are
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
                    let kept = UiKept(handle.stores.peek().contexts.clone());
                    let before = handle.decks.peek().clone();
                    let now = handle.act(|st| {
                        job(st, &kept);
                        st.clone()
                    });
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

/// Decks take ⌘1–9 and ⌃Tab (and the strip shows) once two or more are
/// open; with one, the keys are the screensets' and a group's tabs' as
/// without decks.
#[must_use]
pub(super) fn switching() -> bool {
    try_use_decks().is_some_and(|h| h.decks.read().decks.len() >= 2)
}

/// The decks under [`use_decks`], if there are any.
#[must_use]
pub fn try_use_decks() -> Option<DeckHandle> {
    try_use_context::<DeckHandle>()
}

/// The context of the deck on show: what a view reads to show its song or
/// project (its deck's while that deck shows).
#[must_use]
pub fn use_deck_context() -> Option<DeckContext> {
    let handle = try_use_decks()?;
    let decks = handle.decks.read();
    decks.active_deck().map(|d| d.context.clone())
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
        let open = self.decks.peek().find(&context).map(|d| d.id.clone());
        if let Some(open) = open {
            if self.is_popped(&open) && load == Load::Here {
                self.pop_out(&open);
                return;
            }
        }
        let kept = self.kept(&context);
        let _ = self.act(|st| st.open(context, load, None, kept));
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

/// The deck commands, in the palette and on their keys while the strip is
/// mounted: ⌘1–9, ⌃Tab and ⌃⇧Tab, close, reopen, duplicate, the deck's
/// screenset (switch, save as, reset), pop out.
#[component]
fn DeckCommands(handle: DeckHandle) -> Element {
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
        let mut c = Command::new(format!("DECK_SHOW_{}", i.saturating_add(1)), label)
            .group(group)
            .shortcut(if n >= 2 {
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
        if n > 1 {
            c
        } else {
            c.disabled(format!("one {NOUN} is open"))
        }
    };
    use_command(
        several(
            Command::new(crate::control::deck::ids::NEXT, format!("Next {NOUN}"))
                .group(group)
                .shortcut(if n >= 2 { "⌃Tab" } else { "" }),
        ),
        move || handle.cycle(1),
    );
    use_command(
        several(
            Command::new(crate::control::deck::ids::PREV, format!("Previous {NOUN}"))
                .group(group)
                .shortcut(if n >= 2 { "⌃⇧Tab" } else { "" }),
        ),
        move || handle.cycle(-1),
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
                Command::new("DECK_POP_OUT", format!("Pop {NOUN} out into a window")).group(group),
            )
        } else {
            Command::new("DECK_POP_OUT", format!("Pop {NOUN} out into a window"))
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

/// What the strip's menu is open on.
#[derive(Clone, PartialEq)]
struct StripMenu {
    deck: DeckId,
    x: f32,
}

/// A drag along the strip: the deck held, and where it started.
#[derive(Clone, PartialEq)]
struct StripDrag {
    deck: DeckId,
    from_x: f32,
    moved: bool,
}

const TAB_W: f32 = 176.0;
const SLOP: f32 = 6.0;

/// The strip of decks at the top of the main window, above the panes'
/// tab groups: a tab per deck (its title, a mark when its layout differs
/// from its screenset's, the app's badge, a ×), `+`, and a menu per deck.
/// Under a title row it is the window's title row (as tall, clear of the
/// window's buttons, its empty part moving the window). On a phone it
/// draws nothing (the switcher's menu lists the decks); its commands are
/// on either way.
#[component]
pub fn DeckStrip(
    /// The window's title row, when the strip reaches the window's top.
    title_row: Option<TitleRow>,
    #[props(default)] phone: bool,
    /// Shown with one deck too; else it shows once a second is open.
    #[props(default)]
    always: bool,
    /// Its colours; left out, the shell's.
    #[props(default, into)]
    theme: Option<ShellTheme>,
) -> Element {
    let Some(handle) = try_use_decks() else {
        return rsx! {};
    };
    let shown = !phone && (always || handle.decks.read().decks.len() >= 2);
    // One row on a desk: the shell's own title row gives way to it.
    architect_shell::use_title_row(title_row.is_some() && shown);
    let t = theme
        .or_else(|| architect_shell::try_use_shell().map(|s| s.theme))
        .unwrap_or_default();
    let mut menu = use_signal(|| None::<StripMenu>);
    let mut drag = use_signal(|| None::<StripDrag>);
    // Where the strip starts in the window: its menu opens under the tab
    // pressed, in the strip's own points.
    let mut strip_left = use_signal(|| 0.0_f32);
    if !shown {
        return rsx! { DeckCommands { handle } };
    }
    let form = *handle.form.read();
    let decks = handle.decks.read().clone();
    let popped: Vec<DeckId> = handle
        .popped
        .read()
        .iter()
        .map(|p| p.deck.peek().clone())
        .collect();
    let height = title_row.map_or(34.0, |r| r.height.max(30.0));
    let pad_left = title_row.map_or(6.0, |r| r.left.max(6.0));
    let pad_right = title_row.map_or(6.0, |r| r.right.max(6.0));
    let on_drag = title_row.and_then(|r| r.on_drag);
    let on_zoom = title_row.and_then(|r| r.on_zoom);
    let bar = if title_row.is_some() {
        t.title_bar.clone()
    } else {
        t.desk.clone()
    };
    let tabs = decks.decks.iter().enumerate().map(|(i, d)| {
        let id = d.id.clone();
        let active = decks.active.as_ref() == Some(&d.id);
        let out = popped.contains(&d.id);
        let modified = d.modified(form);
        let title = d.context.title.clone();
        let badge = d.badge.clone();
        let screenset = d.screenset.clone();
        let bg = if active { t.sheet.clone() } else { bar.clone() };
        let edge = if active { t.rule.clone() } else { bar.clone() };
        let weight = if active { 600 } else { 500 };
        let ink = if active { t.ink.clone() } else { t.ink_3.clone() };
        let (id_click, id_close, id_menu, id_down, id_move) =
            (id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
        let lifted = drag.read().as_ref().is_some_and(|g| g.moved && g.deck == id);
        let shadow = if lifted { format!("box-shadow:{};", t.shadow_lift) } else { String::new() };
        let label = format!("{title}, {NOUN}");
        rsx! {
            div {
                key: "{id.0}",
                role: "tab",
                "aria-selected": if active { "true" } else { "false" },
                "aria-label": "{label}",
                "data-deck": "{id.0}",
                "data-screenset": "{screenset}",
                title: if modified { format!("{title} — changed from its screenset") } else { title },
                style: "position:relative; flex:0 1 {TAB_W}px; min-width:72px; height:{height - 6.0}px; margin-top:6px; \
                        box-sizing:border-box; display:flex; align-items:center; gap:6px; padding:0 6px 0 10px; \
                        border-radius:8px 8px 0 0; background:{bg}; color:{ink}; cursor:default; {shadow} \
                        border:1px solid {edge}; border-bottom:none;",
                onpointerdown: move |e: PointerEvent| {
                    e.stop_propagation();
                    match e.data().trigger_button() {
                        Some(dioxus::html::input_data::MouseButton::Auxiliary) => {
                            handle.close(&id_down);
                        }
                        Some(dioxus::html::input_data::MouseButton::Primary) | None => {
                            drag.set(Some(StripDrag {
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
                        drag.set(Some(StripDrag { moved: true, ..g.clone() }));
                    }
                    // Over another deck's tab: the held one takes its place.
                    if g.deck != id_move {
                        handle.move_to(&g.deck, i);
                    }
                },
                onpointerup: move |_| {
                    let held = drag.peek().clone();
                    drag.set(None);
                    if held.is_some_and(|g| !g.moved) {
                        handle.activate(&id_click);
                    }
                },
                oncontextmenu: move |e: MouseEvent| {
                    e.prevent_default();
                    let x = super::narrow(e.data().client_coordinates().x);
                    let x = x - *strip_left.peek();
                    menu.set(Some(StripMenu { deck: id_menu.clone(), x }));
                },
                // In a window of its own: says so.
                if out {
                    AppWindow { size: 13 }
                }
                span { style: "flex:1 1 auto; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; \
                               font-size:12px; font-weight:{weight};",
                    "{title}"
                }
                if let Some(b) = badge {
                    span { style: "flex:0 0 auto; padding:0 6px; border-radius:999px; background:{t.accent}; \
                                   color:{t.accent_ink}; font-size:10px; font-weight:700;",
                        "{b}"
                    }
                }
                if modified {
                    span {
                        "aria-label": "changed from its screenset",
                        style: "flex:0 0 auto; width:6px; height:6px; border-radius:999px; background:{t.accent};",
                    }
                }
                // The one on show has its ×; the others keep their room for
                // their names (a middle-click closes any).
                if active {
                    button {
                        "aria-label": "Close {NOUN}",
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
        DeckCommands { handle }
        div {
            role: "tablist",
            "aria-label": crate::deck::NOUN_PLURAL_TITLE,
            "data-deck-strip": "true",
            style: "position:relative; flex:0 0 {height}px; height:{height}px; box-sizing:border-box; display:flex; \
                    align-items:stretch; gap:2px; padding:0 {pad_right}px 0 {pad_left}px; background:{bar}; \
                    border-bottom:1px solid {t.rule}; font-family:{t.font}; z-index:{architect_shell::layer::STATUS};",
            onpointerdown: move |_| {
                if let Some(drag) = on_drag {
                    drag.call(());
                }
            },
            ondoubleclick: move |_| {
                if let Some(zoom) = on_zoom {
                    zoom.call(());
                }
            },
            onpointerup: move |_| drag.set(None),
            onpointerleave: move |_| drag.set(None),
            onmounted: move |e| async move {
                if let Ok(r) = e.data().get_client_rect().await {
                    strip_left.set(super::narrow(r.origin.x));
                }
            },
            {tabs}
            button {
                "aria-label": "Load onto a new {NOUN}",
                title: "Load onto a new {NOUN}",
                style: "flex:0 0 auto; width:28px; height:{height - 10.0}px; margin-top:5px; display:flex; align-items:center; \
                        justify-content:center; border:none; border-radius:6px; background:transparent; color:{t.ink_3};",
                onpointerdown: move |e: PointerEvent| e.stop_propagation(),
                onclick: move |_| {
                    let on_new = *handle.on_new.peek();
                    if let Some(on_new) = on_new {
                        on_new.call(());
                    } else {
                        let active = handle.decks.peek().active.clone();
                        if let Some(id) = active {
                            handle.duplicate(&id);
                        }
                    }
                },
                Plus { size: 14 }
            }
            if let Some(m) = open_menu {
                DeckMenu { handle, menu: m, top: height, on_done: move |()| menu.set(None) }
            }
        }
    }
}

/// A deck's menu, under its tab.
#[component]
fn DeckMenu(handle: DeckHandle, menu: StripMenu, top: f32, on_done: EventHandler<()>) -> Element {
    let t = architect_shell::try_use_shell()
        .map(|s| s.theme)
        .unwrap_or_default();
    let id = menu.deck.clone();
    let deck = handle.decks.read().get(&id).cloned();
    let Some(deck) = deck else {
        return rsx! {};
    };
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
        id,
    );
    let left = (menu.x - 40.0).max(4.0);
    let reset_ink = if deck.modified(form) {
        t.ink.clone()
    } else {
        t.ink_3.clone()
    };
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
            style: "position:absolute; left:{left}px; top:{top}px; min-width:220px; padding:4px 0; border-radius:8px; \
                    background:{t.sheet}; color:{t.ink}; border:1px solid {t.rule}; box-shadow:{t.shadow_float}; \
                    z-index:{architect_shell::layer::MENU}; font-family:{t.font};",
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
                    if popped { "Back into the strip" } else { "Pop out into a window" }
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
            div { style: "height:1px; margin:4px 0; background:{t.rule};" }
            button { role: "menuitem", style: "{row} color:{t.danger};",
                onclick: move |_| { handle.close(&i6); on_done.call(()); },
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

/// The decks as a phone's switcher lists them (read as its menu is
/// drawn): the decks, and each one's id, title, whether it shows.
#[must_use]
pub(super) fn switcher_rows() -> Option<(DeckHandle, Vec<(DeckId, String, bool)>)> {
    let handle = try_consume_context::<DeckHandle>()?;
    let decks = handle.decks.peek();
    let rows = decks
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
    drop(decks);
    Some((handle, rows))
}
