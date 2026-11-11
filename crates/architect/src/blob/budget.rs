//! A byte budget for a [`LocalFs`]: a size limit and a keep-free floor.
//!
//! [`StoreBudget`] is checked **before** bytes are written: a put reserves
//! each chunk before writing it, a fetch reserves the bytes it still
//! lacks before it asks a remote for them. A reservation that would take
//! the store past [`StoreBudget::max`], or the filesystem holding it
//! under [`StoreBudget::keep_free`], is refused with [`NoRoom`]; nothing
//! is written. Concurrent writers reserve against one running total, so
//! together they cannot overshoot either.
//!
//! The total is kept as bytes are written and removed through the store
//! ([`LocalFs::remove`], [`LocalFs::adopt`]), and
//! reconciled by a walk ([`StoreBudget::reconcile`]): at start and
//! whenever the store's owner looks at it again, never per write.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use super::LocalFs;

/// A filesystem's size and its bytes free to an unprivileged writer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Space {
    pub total: u64,
    pub free: u64,
}

/// What the filesystem holding a path has: [`fs_space`] by default; a
/// test passes its own.
pub type SpaceProbe = Arc<dyn Fn(&Path) -> Option<Space> + Send + Sync>;

/// The filesystem holding `path` (or its nearest existing parent).
///
/// `None` where it cannot be asked (not unix, or every parent missing).
#[must_use]
pub fn fs_space(path: &Path) -> Option<Space> {
    let mut at = Some(path);
    while let Some(p) = at {
        if p.exists() {
            return statvfs(p);
        }
        at = p.parent();
    }
    None
}

#[cfg(unix)]
fn statvfs(path: &Path) -> Option<Space> {
    let s = rustix::fs::statvfs(path).ok()?;
    let block = s.f_frsize;
    Some(Space {
        total: s.f_blocks.saturating_mul(block),
        free: s.f_bavail.saturating_mul(block),
    })
}

#[cfg(not(unix))]
fn statvfs(_path: &Path) -> Option<Space> {
    None
}

/// A write refused for space: what it needed and what there was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoRoom {
    /// The area that refused (its root).
    pub area: PathBuf,
    /// Bytes the write asked for.
    pub need: u64,
    /// Bytes in the store, and promised to writes in flight.
    pub used: u64,
    pub reserved: u64,
    /// The store's limit.
    pub max: u64,
    /// The filesystem's free bytes, when known, and the floor kept free.
    pub free: Option<u64>,
    pub keep_free: u64,
}

impl NoRoom {
    /// Whether it can never fit: more than the whole limit. Waiting for
    /// room does not help; raise the limit.
    #[must_use]
    pub const fn too_large(&self) -> bool {
        self.need > self.max
    }
}

impl fmt::Display for NoRoom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let area = self.area.display();
        if self.too_large() {
            return write!(
                f,
                "{} bytes are more than the whole limit of {} of {area}",
                self.need, self.max
            );
        }
        write!(
            f,
            "no room for {} bytes in {area}: it has {} (+{} in flight) of {}",
            self.need, self.used, self.reserved, self.max
        )?;
        match self.free {
            Some(free) => write!(
                f,
                ", its filesystem {free} free of a {} floor",
                self.keep_free
            ),
            None if self.keep_free > 0 => write!(
                f,
                "; its filesystem's free space cannot be read, so a write that needs its {} floor is refused",
                self.keep_free
            ),
            None => Ok(()),
        }
    }
}

/// Where a store stands against its budget.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StoreUsage {
    /// Bytes in the store (blobs, partial fetches, temporary files).
    pub used: u64,
    /// Bytes promised to writes in flight.
    pub reserved: u64,
    /// The limit, and the floor kept free on its filesystem.
    pub max: u64,
    pub keep_free: u64,
    /// The filesystem holding it, when it can be asked.
    pub space: Option<Space>,
    /// Writes refused for space since start.
    pub refused: u64,
}

#[derive(Default)]
struct Books {
    used: u64,
    reserved: u64,
}

/// A size limit and a keep-free floor for one [`LocalFs`] (see the
/// module docs). Shared: every clone of the store holds the same one.
pub struct StoreBudget {
    root: PathBuf,
    max: u64,
    keep_free: u64,
    probe: SpaceProbe,
    books: Mutex<Books>,
    refused: AtomicU64,
}

impl fmt::Debug for StoreBudget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoreBudget")
            .field("root", &self.root)
            .field("max", &self.max)
            .field("keep_free", &self.keep_free)
            .finish_non_exhaustive()
    }
}

/// The default store limit on a filesystem of `total` bytes.
///
/// A quarter of it, at most 100 GiB: the store is history and a cache,
/// never where the files live.
#[must_use]
pub const fn default_max(total: u64) -> u64 {
    let quarter = total / 4;
    let cap = 100 << 30;
    if quarter < cap { quarter } else { cap }
}

/// The default keep-free floor on a filesystem of `total` bytes.
///
/// 15% of it (above the kubelet's 10% eviction threshold), at most 64 GiB.
#[must_use]
pub const fn default_keep_free(total: u64) -> u64 {
    let part = (total / 100).saturating_mul(15);
    let cap = 64 << 30;
    if part < cap { part } else { cap }
}

impl StoreBudget {
    /// A budget of `max` bytes for the store at `root`, keeping
    /// `keep_free` bytes free on its filesystem. Its total starts at 0:
    /// [`Self::reconcile`] it before the first write.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, max: u64, keep_free: u64) -> Self {
        Self {
            root: root.into(),
            max,
            keep_free,
            probe: Arc::new(fs_space),
            books: Mutex::new(Books::default()),
            refused: AtomicU64::new(0),
        }
    }

    /// Ask `probe` for the filesystem's space instead of `statvfs`.
    #[must_use]
    pub fn with_probe(mut self, probe: SpaceProbe) -> Self {
        self.probe = probe;
        self
    }

    /// The store's limit.
    #[must_use]
    pub const fn max(&self) -> u64 {
        self.max
    }

    /// The floor kept free on the store's filesystem.
    #[must_use]
    pub const fn keep_free(&self) -> u64 {
        self.keep_free
    }

    fn books(&self) -> std::sync::MutexGuard<'_, Books> {
        self.books.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Promise `need` more bytes, or say why not.
    fn take(&self, books: &mut Books, need: u64) -> Result<(), NoRoom> {
        let free = (self.probe)(&self.root).map(|s| s.free);
        let over_max = books
            .used
            .saturating_add(books.reserved)
            .saturating_add(need)
            > self.max;
        // Bytes promised are not on disk yet: they come out of `free` too.
        // A floor that cannot be checked (statvfs failed) refuses: closed,
        // not open.
        let under_floor = match free {
            Some(free) => {
                free.saturating_sub(books.reserved).saturating_sub(need) < self.keep_free
                    || free < books.reserved.saturating_add(need)
            }
            None => self.keep_free > 0,
        };
        if over_max || under_floor {
            self.refused.fetch_add(1, Ordering::Relaxed);
            return Err(NoRoom {
                area: self.root.clone(),
                need,
                used: books.used,
                reserved: books.reserved,
                max: self.max,
                free,
                keep_free: self.keep_free,
            });
        }
        books.reserved = books.reserved.saturating_add(need);
        Ok(())
    }

    /// Reserve `need` bytes for a write about to happen.
    ///
    /// # Errors
    ///
    /// [`NoRoom`] when they would take the store past its limit or its
    /// filesystem under the floor; nothing is reserved then.
    pub fn reserve(self: &Arc<Self>, need: u64) -> Result<Reservation, NoRoom> {
        self.take(&mut self.books(), need)?;
        Ok(Reservation {
            budget: self.clone(),
            left: need,
        })
    }

    /// Whether `need` bytes would fit now, or why not (a check, nothing
    /// reserved, not counted as a refusal).
    ///
    /// # Errors
    ///
    /// [`NoRoom`].
    pub fn check(&self, need: u64) -> Result<(), NoRoom> {
        let result = {
            let mut books = self.books();
            let result = self.take(&mut books, need);
            if result.is_ok() {
                books.reserved = books.reserved.saturating_sub(need);
            }
            result
        };
        if result.is_err() {
            self.refused.fetch_sub(1, Ordering::Relaxed);
        }
        result
    }

    /// Whether `need` bytes would fit now (a check, nothing reserved).
    #[must_use]
    pub fn has_room(&self, need: u64) -> bool {
        let mut books = self.books();
        if self.take(&mut books, need).is_err() {
            // A check is not a refused write.
            self.refused.fetch_sub(1, Ordering::Relaxed);
            return false;
        }
        books.reserved = books.reserved.saturating_sub(need);
        true
    }

    /// `n` bytes left the store.
    pub fn removed(&self, n: u64) {
        let mut books = self.books();
        books.used = books.used.saturating_sub(n);
    }

    /// `n` bytes came into the store without a reservation (a walk will
    /// correct the total if this was wrong).
    pub fn added(&self, n: u64) {
        let mut books = self.books();
        books.used = books.used.saturating_add(n);
    }

    /// Set the total to what a walk of the store found.
    pub fn reconcile(&self, used: u64) {
        self.books().used = used;
    }

    /// Where the store stands now.
    #[must_use]
    pub fn usage(&self) -> StoreUsage {
        let (used, reserved) = {
            let books = self.books();
            (books.used, books.reserved)
        };
        StoreUsage {
            used,
            reserved,
            max: self.max,
            keep_free: self.keep_free,
            space: (self.probe)(&self.root),
            refused: self.refused.load(Ordering::Relaxed),
        }
    }

    /// Whether the store is over `percent` of its limit, or its
    /// filesystem within `percent`-scaled reach of the floor.
    #[must_use]
    pub fn over(&self, percent: u64) -> bool {
        let usage = self.usage();
        let soft = (self.max / 100).saturating_mul(percent);
        let floor = self
            .keep_free
            .saturating_mul(100)
            .checked_div(percent)
            .unwrap_or(u64::MAX);
        usage.used.saturating_add(usage.reserved) > soft
            || usage.space.is_some_and(|s| s.free < floor)
    }
}

/// Bytes promised to one write; what it does not use goes back on drop.
#[derive(Debug)]
pub struct Reservation {
    budget: Arc<StoreBudget>,
    left: u64,
}

impl Reservation {
    /// Promise `more` bytes to this write.
    ///
    /// # Errors
    ///
    /// [`NoRoom`], as [`StoreBudget::reserve`].
    pub fn grow(&mut self, more: u64) -> Result<(), NoRoom> {
        self.budget.take(&mut self.budget.books(), more)?;
        self.left = self.left.saturating_add(more);
        Ok(())
    }

    /// `n` of the promised bytes are now in the store.
    pub fn wrote(&mut self, n: u64) {
        let n = n.min(self.left);
        {
            let mut books = self.budget.books();
            books.reserved = books.reserved.saturating_sub(n);
            books.used = books.used.saturating_add(n);
        }
        self.left = self.left.saturating_sub(n);
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        let mut books = self.budget.books();
        books.reserved = books.reserved.saturating_sub(self.left);
    }
}

/// Every byte under `dir` (blobs, partial fetches, temporary files).
pub async fn walk_size(dir: &Path) -> u64 {
    let dir = dir.to_owned();
    tokio::task::spawn_blocking(move || dir_size(&dir))
        .await
        .unwrap_or(0)
}

/// [`walk_size`], blocking (once, at start).
#[must_use]
pub fn dir_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir).map_or(0, |entries| {
        entries
            .flatten()
            .map(|e| match e.file_type() {
                Ok(t) if t.is_dir() => dir_size(&e.path()),
                Ok(_) => e.metadata().map_or(0, |m| m.len()),
                Err(_) => 0,
            })
            .fold(0_u64, u64::saturating_add)
    })
}

impl LocalFs {
    /// This store under `budget` (shared by every clone made after).
    #[must_use]
    pub fn with_budget(mut self, budget: Arc<StoreBudget>) -> Self {
        self.budget = Some(budget);
        self
    }

    /// The store's budget, if it has one.
    #[must_use]
    pub const fn budget(&self) -> Option<&Arc<StoreBudget>> {
        self.budget.as_ref()
    }

    /// Reserve `need` bytes (`None` without a budget: no limit).
    ///
    /// # Errors
    ///
    /// [`NoRoom`].
    pub fn reserve(&self, need: u64) -> Result<Option<Reservation>, NoRoom> {
        self.budget.as_ref().map(|b| b.reserve(need)).transpose()
    }

    /// Whether `need` bytes fit now, or why not (nothing reserved).
    ///
    /// # Errors
    ///
    /// [`super::BlobError::NoRoom`].
    pub fn check_room(&self, need: u64) -> Result<(), super::BlobError> {
        self.budget
            .as_ref()
            .map_or(Ok(()), |b| b.check(need))
            .map_err(super::BlobError::NoRoom)
    }

    /// Whether `need` bytes fit now (always without a budget).
    #[must_use]
    pub fn has_room(&self, need: u64) -> bool {
        self.budget.as_ref().is_none_or(|b| b.has_room(need))
    }

    /// Walk the store and set its budget's total to what is there; the
    /// bytes found.
    pub async fn reconcile(&self) -> u64 {
        let used = walk_size(&self.root).await;
        if let Some(b) = &self.budget {
            b.reconcile(used);
        }
        used
    }

    /// Remove the blob `hash`; the bytes freed.
    ///
    /// # Errors
    ///
    /// The file is there and cannot be removed.
    pub fn remove(&self, hash: &super::Sha256) -> std::io::Result<u64> {
        let path = self.path_for(hash);
        let len = match std::fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        std::fs::remove_file(&path)?;
        if let Some(b) = &self.budget {
            b.removed(len);
        }
        Ok(len)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn fixed(free: u64) -> SpaceProbe {
        Arc::new(move |_| {
            Some(Space {
                total: 1 << 30,
                free,
            })
        })
    }

    #[test]
    fn reservations_stop_at_the_limit_and_come_back_on_drop() {
        let b = Arc::new(StoreBudget::new("/nowhere", 100, 0).with_probe(fixed(1 << 30)));
        let a = b.reserve(60).unwrap();
        assert!(b.reserve(50).is_err(), "60 + 50 is past 100");
        drop(a);
        let mut c = b.reserve(50).unwrap();
        c.wrote(50);
        drop(c);
        assert_eq!(b.usage().used, 50);
        assert_eq!(b.usage().reserved, 0);
        assert!(b.reserve(51).is_err());
        b.removed(50);
        assert!(b.reserve(100).is_ok());
        assert_eq!(b.usage().refused, 2);
    }

    #[test]
    fn the_floor_counts_what_is_promised() {
        let b = Arc::new(StoreBudget::new("/nowhere", u64::MAX, 1_000).with_probe(fixed(1_500)));
        let _a = b.reserve(400).unwrap();
        assert!(!b.has_room(200), "1500 - 400 - 200 is under 1000");
        assert!(b.has_room(100));
        assert_eq!(b.usage().refused, 0, "a check is not a refusal");
    }

    #[test]
    fn an_unreadable_filesystem_refuses_what_needs_the_floor() {
        let blind: SpaceProbe = Arc::new(|_| None);
        let b = Arc::new(StoreBudget::new("/nowhere", 1_000, 10).with_probe(blind.clone()));
        let no = b.reserve(1).unwrap_err();
        assert_eq!(no.free, None);
        assert!(no.to_string().contains("cannot be read"));
        let open = Arc::new(StoreBudget::new("/nowhere", 1_000, 0).with_probe(blind));
        assert!(
            open.reserve(1).is_ok(),
            "no floor asked for: the limit alone"
        );
        assert!(open.reserve(2_000).unwrap_err().too_large());
    }

    #[test]
    fn defaults_are_fractions_with_caps() {
        assert_eq!(default_max(20 << 30), 5 << 30);
        assert_eq!(default_max(4 << 40), 100 << 30);
        assert_eq!(default_keep_free(100 << 30), 15 << 30);
        assert_eq!(default_keep_free(4 << 40), 64 << 30);
    }
}
