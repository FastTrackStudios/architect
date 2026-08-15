//! [`Reactivity`] — keyed invalidation, effect-atom's
//! `Reactivity.invalidate(keys)` ported to signals.
//!
//! A data hook *tracks* a key inside its loader; anything — typically a
//! mutation — *invalidates* the key, and every loader tracking it re-runs.
//! This covers the refreshes the store can't reconcile by itself: derived
//! server data (search results, counts, aggregations) that a write makes
//! stale.
//!
//! ```ignore
//! // in a data hook's `use_resource` loader:
//! reactivity.track("examples");
//! let res = clients.search(q, 100).await;
//!
//! // after a mutation settles:
//! reactivity.invalidate("examples");   // every tracker re-fetches
//! ```
//!
//! Each key gets its own revision signal (created lazily, owned by the
//! root scope), so invalidating one key only restarts loaders that track
//! *that* key.

use std::collections::HashMap;

use dioxus::prelude::*;

/// A `Copy` handle to the keyed revision registry.
pub struct Reactivity {
    keys: CopyValue<HashMap<&'static str, Signal<u64>>>,
}

impl Clone for Reactivity {
    fn clone(&self) -> Self {
        *self
    }
}
impl Copy for Reactivity {}

impl Reactivity {
    fn entry(&self, key: &'static str) -> Signal<u64> {
        if let Some(sig) = self.keys.read().get(key) {
            return *sig;
        }
        // Lazily create the key's revision signal, owned by the root scope
        // so it outlives whichever component first touched the key.
        let sig = Signal::new_in_scope(0u64, ScopeId::ROOT);
        let mut keys = self.keys;
        keys.write().insert(key, sig);
        sig
    }

    /// Subscribe to a key. Call inside a reactive scope (a `use_resource`
    /// loader, a memo): when the key is invalidated, the scope re-runs.
    pub fn track(&self, key: &'static str) {
        let _ = *self.entry(key).read();
    }

    /// Bump a key's revision — every loader tracking it re-runs. Call from
    /// event handlers / after mutations settle, never during render.
    pub fn invalidate(&self, key: &'static str) {
        let mut sig = self.entry(key);
        let next = *sig.peek() + 1;
        sig.set(next);
    }
}

/// Provide the registry at the app root.
pub fn provide_reactivity() -> Reactivity {
    let keys = use_hook(|| CopyValue::new(HashMap::new()));
    use_context_provider(|| Reactivity { keys })
}

/// Pull the registry anywhere under the provider.
pub fn use_reactivity() -> Reactivity {
    use_context::<Reactivity>()
}

/// Like [`use_reactivity`] but tolerant of a missing provider — generated
/// data hooks use this so invalidation is opt-in.
pub fn try_use_reactivity() -> Option<Reactivity> {
    use_hook(try_consume_context::<Reactivity>)
}
