//! Proves a *family* of runtime-generated actions — a data-driven set
//! whose count/ids/descriptions aren't known until runtime (e.g. one
//! action per time signature) — can register through the exact same
//! `ActionBackend` a `#[architect::actions]`-declared trait uses, via
//! `ActionMeta::leaked`. No change to `ActionBackend` itself: this is a
//! second, complementary construction path for `&'static ActionMeta`,
//! not a new registration mechanism.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use architect::action::{ActionBackend, ActionMeta, DynamicActionMeta};

type RegisteredAction = (&'static ActionMeta, Arc<dyn Fn() -> Result<(), String> + Send + Sync>);

#[derive(Default)]
struct InMemoryActionBackend {
    registered: Mutex<Vec<RegisteredAction>>,
}

impl ActionBackend for InMemoryActionBackend {
    fn register(&self, meta: &'static ActionMeta, handler: Arc<dyn Fn() -> Result<(), String> + Send + Sync>) {
        self.registered.lock().unwrap().push((meta, handler));
    }
}

impl InMemoryActionBackend {
    fn invoke(&self, id: &str) -> bool {
        let registered = self.registered.lock().unwrap();
        match registered.iter().find(|(meta, _)| meta.id == id) {
            Some((_, handler)) => {
                handler().expect("action handler should succeed in this test");
                true
            }
            None => false,
        }
    }
}

/// Mimics fts-extensions' per-time-signature action loop: a runtime list
/// (not known at compile time) of (numerator, denominator) pairs, one
/// action generated per entry.
fn register_time_signature_family(backend: &dyn ActionBackend, hits: Arc<AtomicUsize>) {
    let time_signatures = [(3, 4), (4, 4), (6, 8)];

    for (num, den) in time_signatures {
        let meta = DynamicActionMeta {
            id: format!("FTS_TEMPO_SET_{num}_{den}"),
            trait_name: "TempoActions",
            method_name: format!("set_time_sig_{num}_{den}"),
            display_name: format!("Set {num}/{den}"),
            description: format!("Set the project time signature to {num}/{den}"),
            category: "Tempo",
            group: "Time Signatures",
            toggleable: false,
            undo: false,
        }
        .leak();
        let hits = hits.clone();
        backend.register(
            meta,
            Arc::new(move || {
                hits.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
        );
    }
}

#[test]
fn dynamic_family_registers_and_invokes_every_member() {
    let backend = InMemoryActionBackend::default();
    let hits = Arc::new(AtomicUsize::new(0));
    register_time_signature_family(&backend, hits.clone());

    assert_eq!(backend.registered.lock().unwrap().len(), 3);

    let ids: Vec<_> = backend
        .registered
        .lock()
        .unwrap()
        .iter()
        .map(|(m, _)| m.id)
        .collect();
    assert_eq!(
        ids,
        [
            "FTS_TEMPO_SET_3_4",
            "FTS_TEMPO_SET_4_4",
            "FTS_TEMPO_SET_6_8"
        ]
    );

    assert!(backend.invoke("FTS_TEMPO_SET_3_4"));
    assert!(backend.invoke("FTS_TEMPO_SET_6_8"));
    assert!(!backend.invoke("FTS_TEMPO_SET_5_4"));
    assert_eq!(hits.load(Ordering::SeqCst), 2);

    // Metadata round-trips correctly, same as a macro-declared action.
    let (meta, _) = backend
        .registered
        .lock()
        .unwrap()
        .iter()
        .find(|(m, _)| m.id == "FTS_TEMPO_SET_4_4")
        .map(|(m, h)| (*m, h.clone()))
        .unwrap();
    assert_eq!(meta.display_name, "Set 4/4");
    assert_eq!(meta.category, "Tempo");
    assert_eq!(meta.group, "Time Signatures");
    assert_eq!(meta.trait_name, "TempoActions");
    assert!(!meta.toggleable);
}
