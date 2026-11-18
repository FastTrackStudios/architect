//! The engine's values, written lock-free: meters, levels, a playhead an
//! audio callback knows first.
//!
//! An engine runs on a thread that must never wait (an audio callback
//! has a few milliseconds and no business taking a lock or allocating).
//! It writes `engine.` values ([`crate::anim::Space::Engine`]) into a
//! fixed table of slots ([`EngineTable`]): each slot an atomic holding
//! an `f64`'s bits, each write two atomic stores ([`EngineWriter::write`]).
//! The names are fixed when the table is made, so a write never looks a
//! name up or grows anything; the writer finds its slot once
//! ([`EngineTable::slot`]).
//!
//! A pane reads the table at compose time through its values
//! ([`crate::anim::Values::with_engine`]), so a layer bound to
//! `engine.track.1.peak` is drawn at the level the engine last wrote,
//! with no message to the view and none from it. Only a pane granted the
//! engine's values is given the table; elsewhere they read as nothing.
//!
//! A host draws every frame while the table's generation moves, and
//! rests when it stops; a table it [`watch`]es wakes it when the engine
//! starts writing again. One table may be shared by every pane that shows
//! the same engine: the writer writes once, whoever reads.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use crate::anim::{Name, Space};
use crate::{Error, MAX_COORDINATE};

/// The most slots a table may have.
pub const MAX_SLOTS: usize = 1024;

/// What a slot holds before its first write: no value.
const UNSET: u64 = u64::MAX;

/// A slot in a table: where a writer writes one name's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Slot(usize);

/// An engine's values: a fixed table of slots by name.
pub struct EngineTable {
    names: BTreeMap<Name, usize>,
    slots: Box<[AtomicU64]>,
    /// Moves on with every write: what a host compares to know whether
    /// a meter moved.
    generation: AtomicU64,
}

impl std::fmt::Debug for EngineTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineTable")
            .field("slots", &self.slots.len())
            .field("generation", &self.generation())
            .finish_non_exhaustive()
    }
}

impl EngineTable {
    /// A table of `names`, each unwritten, and the writer for it.
    ///
    /// # Errors
    ///
    /// A name not in the engine's space (`engine.`), or more than
    /// [`MAX_SLOTS`].
    pub fn new(names: impl IntoIterator<Item = Name>) -> Result<(Arc<Self>, EngineWriter), Error> {
        let mut map = BTreeMap::new();
        for name in names {
            if name.space() != Space::Engine {
                return Err(Error::Denied("write", name.to_string()));
            }
            let next = map.len();
            map.entry(name).or_insert(next);
        }
        if map.len() > MAX_SLOTS {
            return Err(Error::TooManyValues(MAX_SLOTS));
        }
        let slots = (0..map.len()).map(|_| AtomicU64::new(UNSET)).collect();
        let table = Arc::new(Self {
            names: map,
            slots,
            generation: AtomicU64::new(0),
        });
        let writer = EngineWriter {
            table: Arc::clone(&table),
        };
        Ok((table, writer))
    }

    /// Where `name` is written, if the table has it.
    #[must_use]
    pub fn slot(&self, name: &Name) -> Option<Slot> {
        self.names.get(name).copied().map(Slot)
    }

    /// `name`'s value as last written: none before its first write, or
    /// when what was written is not a number a renderer takes.
    #[must_use]
    pub fn read(&self, name: &Name) -> Option<f64> {
        let bits = self
            .slots
            .get(*self.names.get(name)?)?
            .load(Ordering::Acquire);
        let v = f64::from_bits(bits);
        (bits != UNSET && v.is_finite() && v.abs() <= MAX_COORDINATE).then_some(v)
    }

    /// How many writes it has had.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Its names.
    pub fn names(&self) -> impl Iterator<Item = &Name> {
        self.names.keys()
    }
}

/// The engine's side of a table: cheap to clone, `Send`, never waits.
#[derive(Clone, Debug)]
pub struct EngineWriter {
    table: Arc<EngineTable>,
}

impl EngineWriter {
    /// `value` into `slot`: two atomic stores, no lock, no allocation. A
    /// slot of another table is ignored.
    #[inline]
    pub fn write(&self, slot: Slot, value: f64) {
        if let Some(cell) = self.table.slots.get(slot.0) {
            cell.store(value.to_bits(), Ordering::Release);
            self.table.generation.fetch_add(1, Ordering::Release);
        }
    }

    /// The table it writes.
    #[must_use]
    pub const fn table(&self) -> &Arc<EngineTable> {
        &self.table
    }
}

type Waker = Arc<dyn Fn() + Send + Sync>;

struct Watched {
    table: Weak<EngineTable>,
    waker: Weak<dyn Fn() + Send + Sync>,
    seen: u64,
}

static WATCHED: Mutex<Vec<Watched>> = Mutex::new(Vec::new());

/// How often a resting table is looked at for new writes.
pub const WATCH_EVERY: std::time::Duration = std::time::Duration::from_millis(8);

/// Wakes `waker` when `table` is written after a rest.
///
/// For as long as both are held elsewhere: a host's window, resting while its meters
/// stood, draws again when the engine moves them. The engine is never
/// asked to do anything for it; a thread looks at the table's generation
/// every [`WATCH_EVERY`] while any table is watched.
pub fn watch(table: &Arc<EngineTable>, waker: &Waker) {
    let mut all = WATCHED.lock().unwrap_or_else(PoisonError::into_inner);
    let first = all.is_empty();
    all.push(Watched {
        table: Arc::downgrade(table),
        waker: Arc::downgrade(waker),
        seen: table.generation(),
    });
    drop(all);
    if first {
        watcher::start();
    }
}

#[cfg(not(target_family = "wasm"))]
mod watcher {
    use std::sync::PoisonError;
    use std::sync::atomic::{AtomicBool, Ordering};

    static RUNNING: AtomicBool = AtomicBool::new(false);

    pub fn start() {
        if RUNNING.swap(true, Ordering::AcqRel) {
            return;
        }
        let spawned = std::thread::Builder::new()
            .name("architect-scene-engine".to_owned())
            .spawn(run);
        if spawned.is_err() {
            RUNNING.store(false, Ordering::Release);
        }
    }

    fn run() {
        loop {
            std::thread::sleep(super::WATCH_EVERY);
            let wake: Vec<super::Waker> = {
                let mut all = super::WATCHED
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                all.retain(|w| w.table.strong_count() > 0 && w.waker.strong_count() > 0);
                if all.is_empty() {
                    RUNNING.store(false, Ordering::Release);
                    return;
                }
                all.iter_mut()
                    .filter_map(|w| {
                        let now = w.table.upgrade()?.generation();
                        (now != w.seen).then(|| {
                            w.seen = now;
                            w.waker.upgrade()
                        })?
                    })
                    .collect()
            };
            for waker in wake {
                waker();
            }
        }
    }
}

#[cfg(target_family = "wasm")]
mod watcher {
    // No thread: a browser's host draws a granted pane every frame.
    pub const fn start() {}
}

/// The engines the app runs, by the extension whose views read them: an
/// engine publishes its table here, and a pane of that extension's
/// granted the engine's values reads it.
static PUBLISHED: Mutex<BTreeMap<String, Arc<EngineTable>>> = Mutex::new(BTreeMap::new());

/// Makes `table` the engine's values that `extension`'s views read.
pub fn publish(extension: &str, table: Arc<EngineTable>) {
    PUBLISHED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(extension.to_owned(), table);
}

/// The table `extension`'s views read, if an engine published one.
#[must_use]
pub fn published(extension: &str) -> Option<Arc<EngineTable>> {
    PUBLISHED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(extension)
        .cloned()
}
