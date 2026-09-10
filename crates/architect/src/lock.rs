//! Poison-recovering `Mutex` / `RwLock` accessors.
//!
//! Every lock in this crate guards a plain collection — a subscriber
//! list, a finalizer stack, a handle registry, a test clock's elapsed
//! time. None of them has an invariant that spans a panic: a thread
//! that dies mid-update leaves a `Vec` that is still a `Vec`.
//!
//! The standard `lock().unwrap()` turns that unrelated panic into a
//! *second* panic, in whatever thread touches the lock next. Architect
//! runs inside other people's processes — a REAPER extension, an audio
//! host, a long-lived server — where the second panic is the one that
//! takes the host down, and it does so at a call site that had nothing
//! to do with the original fault.
//!
//! So the crate never propagates poison. It recovers the guard and
//! carries on; the original panic has already been reported by whoever
//! raised it.
//!
//! ```ignore
//! use crate::lock::{lock, read, write};
//!
//! let mut subs = lock(&self.inner);          // instead of .lock().expect(..)
//! let entries  = read(&self.entries);        // instead of .read().unwrap()
//! ```
//!
//! If a future lock ever *does* guard a cross-panic invariant, it must
//! not use these helpers — it needs an explicit poison policy at that
//! call site, and a comment saying what the invariant is.

use std::sync::{Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Lock a `Mutex`, recovering the guard if the lock is poisoned.
#[inline]
pub fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Read-lock an `RwLock`, recovering the guard if the lock is poisoned.
#[inline]
pub fn read<T: ?Sized>(l: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(PoisonError::into_inner)
}

/// Write-lock an `RwLock`, recovering the guard if the lock is poisoned.
#[inline]
pub fn write<T: ?Sized>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(PoisonError::into_inner)
}
