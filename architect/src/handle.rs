//! Resource handles — keep megabyte-scale payloads server-side, send an
//! opaque id over the wire.
//!
//! A vox RPC return value serializes whole. For a 10 MB FX-state chunk or
//! an audio sample block, that's 10 MB of facet encoding per call even
//! when the client only needs a slice of it. The handle pattern (promoted
//! from FastTrackStudio's hand-rolled audio-accessor registry) keeps the
//! large allocation in the server process behind an id:
//!
//! - [`Handle<T>`] — a typed, `Copy`-able id. Send + Sync regardless of
//!   `T` (the phantom is `fn() -> T`), because the handle never carries
//!   the value, only names it.
//! - [`RawHandle`] — the non-generic **wire** form: a `#[repr(transparent)]`
//!   newtype over [`Uuid`] deriving `facet::Facet`, so it travels through
//!   vox like any other architect wire type (as a bare UUID string).
//! - [`HandleRegistry<T>`] — the server-side table the handles index into.
//!   Thread-safe (`RwLock<HashMap>`), values shared out as `Arc<T>`.
//!
//! # Why two types? (the facet-generic outcome)
//!
//! facet's derive on a generic struct emits a `T: Facet<'ʄ>` bound for
//! every type parameter — a derived `Facet` on `Handle<T>` would poison
//! every handle whose target isn't itself a wire type (a registry of FFI
//! accessor pointers, an mmap, a decoder). The whole point is that `T`
//! **stays server-side**, so the wire type must not constrain it. Hence
//! the split: [`RawHandle`] is what service signatures and wire structs
//! carry; [`Handle<T>`] is the thin typed wrapper both ends convert
//! through ([`RawHandle::typed`] / [`Handle::raw`], or plain `From`).
//!
//! # Type safety
//!
//! Registries are typed: a `HandleRegistry<AudioBlock>` only accepts
//! `Handle<AudioBlock>`, so a handle minted for one resource type cannot
//! dereference a registry of another — the mismatch is a compile error,
//! not a runtime lookup miss:
//!
//! ```compile_fail,E0308
//! use architect::handle::{Handle, HandleRegistry, RawHandle};
//! struct AudioBlock;
//! struct FxChunk;
//! let registry: HandleRegistry<AudioBlock> = HandleRegistry::new();
//! let h: Handle<FxChunk> = RawHandle::new().typed();
//! registry.get(&h); // ERROR: expected `&Handle<AudioBlock>`, found `&Handle<FxChunk>`
//! ```
//!
//! That guarantee holds per registry, not globally: the type system can't
//! stop a client from re-tagging a `RawHandle` with the wrong `T` before
//! handing it back. A wrongly-tagged (or stale) handle simply misses —
//! every lookup returns `Option`.
//!
//! # Lifetime / eviction
//!
//! Eviction is **the caller's policy**. The registry never drops entries
//! on its own — no TTL, no capacity bound. Services pair every
//! `create`-returning method with a `release` method (mirroring the DAW's
//! `destroy_accessor`), and can run [`HandleRegistry::retain`] as a
//! manual sweep for sessions that leak. (A built-in TTL/capacity story
//! may grow later; this pass stays simple on purpose.)
//!
//! # Example: the DAW shape
//!
//! Instead of returning a 10 MB sample block, the service mints a handle
//! and derefs ranges server-side:
//!
//! ```ignore
//! use architect::handle::{HandleRegistry, RawHandle};
//!
//! pub struct AudioSampleData {
//!     pub samples: Vec<f32>,        // megabytes — never serialized whole
//!     pub sample_rate: u32,
//! }
//!
//! #[vox::service]
//! pub trait AudioAccess {
//!     /// Render the take server-side; the reply is 16 bytes, not 10 MB.
//!     async fn render_take(&self, take: TakeRef) -> Result<RawHandle, RepoError>;
//!     /// Deref a window of an existing render — only the slice travels.
//!     async fn get_samples(
//!         &self,
//!         handle: RawHandle,
//!         start: u64,
//!         len: u32,
//!     ) -> Result<Vec<f32>, RepoError>;
//!     /// Pair every create with a release; eviction is the caller's policy.
//!     async fn release(&self, handle: RawHandle) -> Result<bool, RepoError>;
//! }
//!
//! struct Backend {
//!     renders: HandleRegistry<AudioSampleData>,
//! }
//!
//! impl AudioAccess for Backend {
//!     async fn render_take(&self, take: TakeRef) -> Result<RawHandle, RepoError> {
//!         let data = render(take)?;                       // the 10 MB stays here
//!         Ok(self.renders.create(data).raw())
//!     }
//!     async fn get_samples(
//!         &self,
//!         handle: RawHandle,
//!         start: u64,
//!         len: u32,
//!     ) -> Result<Vec<f32>, RepoError> {
//!         let data = self
//!             .renders
//!             .get(&handle.typed())
//!             .ok_or(RepoError::NotFound)?;
//!         Ok(slice_samples(&data, start, len))
//!     }
//!     async fn release(&self, handle: RawHandle) -> Result<bool, RepoError> {
//!         Ok(self.renders.release(&handle.typed()))
//!     }
//! }
//! ```
//!
//! The registry itself is plain std and runs anywhere (wasm included),
//! though in practice it lives on the server side of a service.

use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::sync::{Arc, RwLock};

use uuid::Uuid;

// ── RawHandle — the wire type ───────────────────────────────────────────

/// The wire form of a resource handle: an opaque id, serialized via facet
/// as a bare UUID string (`#[repr(transparent)]` newtype).
///
/// Deliberately **non-generic** — see the [module docs](self) for why the
/// typed wrapper can't be the wire type under facet 0.46. Service traits
/// and wire structs carry `RawHandle`; both ends convert to/from
/// [`Handle<T>`] with [`typed`](RawHandle::typed) / [`Handle::raw`].
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, facet::Facet)]
#[repr(transparent)]
pub struct RawHandle(pub Uuid);

impl RawHandle {
    /// Mint a fresh (random, v4) handle id.
    #[allow(clippy::new_without_default)] // a random Default would surprise
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Wrap an existing id (e.g. one parsed from a log line).
    pub const fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    /// The underlying id.
    pub const fn uuid(&self) -> Uuid {
        self.0
    }

    /// Tag this raw id with a resource type, for use against a
    /// [`HandleRegistry<T>`]. The tag is an assertion, not a proof —
    /// a wrong tag means lookups miss (`None`), never a wrong-type deref.
    pub const fn typed<T>(self) -> Handle<T> {
        Handle::from_raw(self)
    }
}

impl fmt::Display for RawHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

// ── Handle<T> — the typed wrapper ───────────────────────────────────────

/// A typed resource handle: [`RawHandle`] plus a compile-time tag naming
/// what it indexes. Always `Copy + Send + Sync` — the phantom is
/// `fn() -> T`, so the handle never inherits `T`'s auto traits (a handle
/// to a non-`Send` FFI resource is still freely shareable; only the
/// *registry* holding the values needs `T: Send + Sync`).
///
/// Not a wire type itself (see the [module docs](self)); convert with
/// [`raw`](Handle::raw) / [`RawHandle::typed`] at the service boundary.
pub struct Handle<T> {
    raw: RawHandle,
    _marker: PhantomData<fn() -> T>,
}

// Manual impls: derives would add spurious `T: Clone/Debug/…` bounds.
impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Handle<T> {}
impl<T> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Handle")
            .field(&self.raw.0)
            .field(&std::any::type_name::<T>())
            .finish()
    }
}
impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}
impl<T> Eq for Handle<T> {}
impl<T> std::hash::Hash for Handle<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}
impl<T> fmt::Display for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.raw.fmt(f)
    }
}

impl<T> Handle<T> {
    /// Tag a wire id with this handle's resource type.
    pub const fn from_raw(raw: RawHandle) -> Self {
        Self {
            raw,
            _marker: PhantomData,
        }
    }

    /// The untyped wire form — what goes in a service signature.
    pub const fn raw(&self) -> RawHandle {
        self.raw
    }

    /// The underlying id.
    pub const fn uuid(&self) -> Uuid {
        self.raw.0
    }
}

impl<T> From<RawHandle> for Handle<T> {
    fn from(raw: RawHandle) -> Self {
        Self::from_raw(raw)
    }
}
impl<T> From<Handle<T>> for RawHandle {
    fn from(h: Handle<T>) -> Self {
        h.raw
    }
}

// ── HandleRegistry<T> — the server-side table ───────────────────────────

/// Server-side store mapping [`Handle<T>`]s to live values. Thread-safe;
/// values are handed out as `Arc<T>` so a loan from [`get`](Self::get)
/// outlives the registry entry (a concurrent [`release`](Self::release)
/// drops the table's reference, not the borrower's).
///
/// Entries live until explicitly removed — see "Lifetime / eviction" in
/// the [module docs](self).
///
/// ```
/// use architect::handle::HandleRegistry;
///
/// let registry: HandleRegistry<Vec<f32>> = HandleRegistry::new();
/// let handle = registry.create(vec![0.0; 1024]);   // the Vec stays here
///
/// // …`handle.raw()` travels over the wire and comes back…
///
/// let block = registry.get(&handle).expect("still registered");
/// assert_eq!(block.len(), 1024);
/// drop(block);
///
/// assert!(registry.release(&handle));
/// assert!(registry.get(&handle).is_none());
/// ```
pub struct HandleRegistry<T> {
    entries: RwLock<HashMap<Uuid, Arc<T>>>,
}

impl<T> Default for HandleRegistry<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> HandleRegistry<T> {
    /// An empty registry.
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Store `value` and mint the handle that names it.
    pub fn create(&self, value: T) -> Handle<T> {
        let handle = RawHandle::new().typed();
        self.entries
            .write()
            .expect("handle registry poisoned")
            .insert(handle.uuid(), Arc::new(value));
        handle
    }

    /// Borrow the value behind `handle` (cheap `Arc` clone), or `None`
    /// for an unknown / already-released handle.
    pub fn get(&self, handle: &Handle<T>) -> Option<Arc<T>> {
        self.entries
            .read()
            .expect("handle registry poisoned")
            .get(&handle.uuid())
            .cloned()
    }

    /// Remove the entry and return the value by **ownership**. Succeeds
    /// only when the registry holds the sole reference — if a loan from
    /// [`get`](Self::get) is still live, the entry is left in place and
    /// `take` returns `None` (retry once the borrower drops its `Arc`).
    pub fn take(&self, handle: &Handle<T>) -> Option<T> {
        // Hold the write lock across remove + unwrap so no concurrent
        // `get` can observe the entry missing while we put it back.
        let mut entries = self.entries.write().expect("handle registry poisoned");
        let arc = entries.remove(&handle.uuid())?;
        match Arc::try_unwrap(arc) {
            Ok(value) => Some(value),
            Err(arc) => {
                // Outstanding loans — restore the entry untouched.
                entries.insert(handle.uuid(), arc);
                None
            }
        }
    }

    /// Drop the registry's reference to the value. Returns `true` if the
    /// handle was registered. Outstanding `Arc` loans keep the value
    /// alive until they drop; new lookups miss immediately.
    pub fn release(&self, handle: &Handle<T>) -> bool {
        self.entries
            .write()
            .expect("handle registry poisoned")
            .remove(&handle.uuid())
            .is_some()
    }

    /// Keep only the entries `keep` approves — the manual-sweep hook for
    /// whatever eviction policy the caller runs (session end, LRU
    /// sidecar, "drop everything for project X").
    pub fn retain(&self, mut keep: impl FnMut(Handle<T>, &T) -> bool) {
        self.entries
            .write()
            .expect("handle registry poisoned")
            .retain(|id, value| keep(RawHandle::from_uuid(*id).typed(), value));
    }

    /// Number of live entries.
    pub fn len(&self) -> usize {
        self.entries.read().expect("handle registry poisoned").len()
    }

    /// True when no entries are registered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_get_take_release_round_trip() {
        let registry: HandleRegistry<Vec<u8>> = HandleRegistry::new();
        assert!(registry.is_empty());

        let handle = registry.create(vec![1, 2, 3]);
        assert_eq!(registry.len(), 1);

        // get: shared loan
        let loan = registry.get(&handle).expect("entry exists");
        assert_eq!(*loan, vec![1, 2, 3]);

        // take refuses while the loan is live, entry stays put
        assert!(registry.take(&handle).is_none());
        assert_eq!(registry.len(), 1);
        drop(loan);

        // take succeeds once sole owner; entry is gone afterwards
        let owned = registry.take(&handle).expect("sole reference now");
        assert_eq!(owned, vec![1, 2, 3]);
        assert!(registry.get(&handle).is_none());
        assert!(registry.is_empty());

        // release reports presence
        let h2 = registry.create(vec![9]);
        assert!(registry.release(&h2));
        assert!(!registry.release(&h2)); // second release: already gone
    }

    #[test]
    fn release_does_not_invalidate_outstanding_loans() {
        let registry: HandleRegistry<String> = HandleRegistry::new();
        let handle = registry.create("payload".to_string());
        let loan = registry.get(&handle).unwrap();
        assert!(registry.release(&handle));
        // The borrower's Arc is unaffected; new lookups miss.
        assert_eq!(*loan, "payload");
        assert!(registry.get(&handle).is_none());
    }

    #[test]
    fn retain_sweeps_by_predicate() {
        let registry: HandleRegistry<u32> = HandleRegistry::new();
        let small = registry.create(1);
        let big = registry.create(1_000_000);
        registry.retain(|_, v| *v < 100);
        assert!(registry.get(&small).is_some());
        assert!(registry.get(&big).is_none());
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn unknown_and_wrongly_tagged_handles_miss() {
        let registry: HandleRegistry<u32> = HandleRegistry::new();
        // Fresh id never registered.
        let stranger: Handle<u32> = RawHandle::new().typed();
        assert!(registry.get(&stranger).is_none());
        assert!(registry.take(&stranger).is_none());
        assert!(!registry.release(&stranger));
    }

    #[test]
    fn handle_identity_follows_the_id_not_the_type_path() {
        let raw = RawHandle::new();
        let a: Handle<u32> = raw.typed();
        let b: Handle<u32> = raw.typed();
        assert_eq!(a, b);
        assert_eq!(RawHandle::from(a), raw);
        assert_eq!(a.uuid(), raw.uuid());
        // Round-trip through the wire form.
        let back: Handle<u32> = a.raw().typed();
        assert_eq!(back, a);
    }

    // Compile-level: handles are Send + Sync + Copy even when T is not.
    // (Cross-type registry safety is the `compile_fail` doctest in the
    // module docs.)
    #[test]
    fn handle_is_send_sync_copy_regardless_of_t() {
        struct NotSendNotSync(#[allow(dead_code)] *mut u8);
        fn assert_traits<H: Send + Sync + Copy>() {}
        assert_traits::<Handle<NotSendNotSync>>();
        assert_traits::<RawHandle>();
    }

    #[test]
    fn concurrent_access_smoke() {
        let registry = Arc::new(HandleRegistry::<Vec<u8>>::new());
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let registry = Arc::clone(&registry);
                std::thread::spawn(move || {
                    for n in 0..100 {
                        let handle = registry.create(vec![i; 64]);
                        let loan = registry.get(&handle).expect("just created");
                        assert_eq!(loan[0], i);
                        drop(loan);
                        if n % 2 == 0 {
                            assert!(registry.release(&handle));
                        } else {
                            assert_eq!(registry.take(&handle).expect("sole owner")[0], i);
                        }
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().expect("worker panicked");
        }
        assert!(registry.is_empty());
    }

    #[test]
    fn raw_handle_facet_shape_is_transparent() {
        // The wire type must serialize via facet like every other
        // architect wire struct; transparent => it reflects as the inner
        // Uuid (a scalar), not a one-field struct.
        let shape = <RawHandle as facet::Facet>::SHAPE;
        assert!(shape.is_transparent());
    }
}
