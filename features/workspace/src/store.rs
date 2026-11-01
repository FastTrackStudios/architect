//! Where a workspace is kept.
//!
//! A [`Store`] the app passes the host: the device, the person's synced
//! settings, a project. What is kept is the
//! [`Screensets`]: every set, the active one, and the live layout per
//! form class (so the arrangement comes back as it was left, on each
//! class of screen).

use std::cell::RefCell;
use std::rc::Rc;

use crate::screenset::Screensets;

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
