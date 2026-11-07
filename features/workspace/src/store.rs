//! Where a workspace is kept.
//!
//! A [`Store`] the app passes the host: the device, the person's synced
//! settings, a project. What is kept is the
//! [`Screensets`]: every set, the active one, and the live layout per
//! form class (so the arrangement comes back as it was left, on each
//! class of screen). Kept in two places ([`load_split`], [`save_split`]),
//! the saved sets go where they are shared (a project's settings scope,
//! the person's synced settings) and the working copy stays on the
//! device.

use std::cell::RefCell;
use std::rc::Rc;

use crate::screenset::{Layout, SavedScreensets, Screensets};

/// Keeps one JSON document. The host reads it once, when it mounts, and
/// writes it when the workspace settles (not on every frame of a drag).
pub trait Store {
    /// What was kept, if anything.
    fn load(&self) -> Option<String>;
    /// Keep `json`. Where it goes, and whether that can fail quietly, is
    /// the store's: a layout that is not kept is not lost work.
    fn save(&self, json: &str);
}

/// A store in memory: for tests, and an app that keeps nothing.
#[derive(Debug, Clone, Default)]
pub struct MemoryStore(Rc<RefCell<Option<String>>>);

impl MemoryStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What it holds now.
    #[must_use]
    pub fn get(&self) -> Option<String> {
        self.0.borrow().clone()
    }
}

impl Store for MemoryStore {
    fn load(&self) -> Option<String> {
        self.get()
    }

    fn save(&self, json: &str) {
        *self.0.borrow_mut() = Some(json.to_owned());
    }
}

/// Read the screensets kept in `store`, joined with the ones the app
/// ships; the app's alone when nothing (or nothing readable) was kept.
#[must_use]
pub fn load(store: &dyn Store, builtins: Vec<crate::Screenset>) -> Screensets {
    store
        .load()
        .and_then(|json| Screensets::from_json(&json).ok())
        .unwrap_or_default()
        .with_builtins(builtins)
}

/// Keep `sets` in `store`. `false` when they could not be written as
/// JSON (nothing is kept then).
pub fn save(store: &dyn Store, sets: &Screensets) -> bool {
    sets.to_json().is_ok_and(|json| {
        store.save(&json);
        true
    })
}

/// Read screensets kept in two places: the saved sets in `saved` (a
/// project's scope, the person's synced settings) and this device's
/// working copy in `working`, joined with the app's own.
#[must_use]
pub fn load_split(
    saved: &dyn Store,
    working: &dyn Store,
    builtins: Vec<crate::Screenset>,
) -> Screensets {
    let sets = saved
        .load()
        .and_then(|json| facet_json::from_str::<SavedScreensets>(&json).ok())
        .unwrap_or_default();
    let live = working
        .load()
        .and_then(|json| facet_json::from_str::<Vec<Layout>>(&json).ok())
        .unwrap_or_default();
    let mut joined = Screensets::joined(sets, live);
    for layout in joined
        .sets
        .iter_mut()
        .flat_map(|s| s.layouts.iter_mut())
        .chain(joined.working.iter_mut())
    {
        layout.workspace.repair();
    }
    joined.with_builtins(builtins)
}

/// Keep `sets` in two places: the saved sets in `saved`, this device's
/// working copy in `working`. `false` when they could not be written.
pub fn save_split(saved: &dyn Store, working: &dyn Store, sets: &Screensets) -> bool {
    let (Ok(shared), Ok(live)) = (
        facet_json::to_string(&sets.saved()),
        facet_json::to_string(&sets.working),
    ) else {
        return false;
    };
    saved.save(&shared);
    working.save(&live);
    true
}

/// Keeps what each deck's context keeps ([`crate::ContextLayout`]): the
/// screenset it was shown in and its override. A project's goes in its
/// folder (`home::ContextFiles`); a test's in memory.
pub trait ContextStore {
    /// What `context` kept, if anything.
    fn load(&self, context: &crate::DeckContext) -> Option<String>;
    /// Keep `json` for `context`.
    fn save(&self, context: &crate::DeckContext, json: &str);
}

/// Contexts' layouts in memory, by [`crate::DeckContext::key`].
#[derive(Debug, Clone, Default)]
pub struct MemoryContextStore(Rc<RefCell<std::collections::BTreeMap<String, String>>>);

impl MemoryContextStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many contexts have something kept.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.borrow().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.borrow().is_empty()
    }
}

impl ContextStore for MemoryContextStore {
    fn load(&self, context: &crate::DeckContext) -> Option<String> {
        self.0.borrow().get(&context.key()).cloned()
    }

    fn save(&self, context: &crate::DeckContext, json: &str) {
        self.0.borrow_mut().insert(context.key(), json.to_owned());
    }
}

/// What `context` kept in `store`, if it reads.
#[must_use]
pub fn load_context(
    store: &dyn ContextStore,
    context: &crate::DeckContext,
) -> Option<crate::ContextLayout> {
    store
        .load(context)
        .and_then(|json| crate::ContextLayout::from_json(&json).ok())
        .filter(|kept| kept.context.same(context))
}

/// Keep `deck`'s screenset and override with its context.
pub fn save_context(store: &dyn ContextStore, deck: &crate::Deck) -> bool {
    deck.context_layout().to_json().is_ok_and(|json| {
        store.save(&deck.context, &json);
        true
    })
}

/// The decks open when the app last ran on this device.
///
/// From `session`, each with what its context kept (`contexts`): the
/// screenset it was in and its override. A context's own word beats the
/// session's (it may have been changed on another device).
#[must_use]
pub fn load_decks(session: &dyn Store, contexts: &dyn ContextStore) -> crate::Decks {
    let mut decks = session
        .load()
        .and_then(|json| crate::Decks::from_json(&json).ok())
        .unwrap_or_default();
    for deck in &mut decks.decks {
        if let Some(kept) = load_context(contexts, &deck.context) {
            deck.screenset = kept.screenset;
            deck.overrides = kept.layouts;
        }
    }
    decks
}

/// Keep the session (the decks open, their order, the active one, the
/// person's defaults) on this device; each deck's override goes to its
/// context through [`save_context`].
pub fn save_session(session: &dyn Store, decks: &crate::Decks) -> bool {
    decks.session().to_json().is_ok_and(|json| {
        session.save(&json);
        true
    })
}
