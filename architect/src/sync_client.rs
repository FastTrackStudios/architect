//! Blocking driver for the emitted `<Trait>SyncClient` facades.
//!
//! The `#[architect::rpc]` user trait is already sync for **in-process**
//! callers — call it directly, zero overhead. This module is for sync
//! code that must talk to a *remote* backend (a REAPER extension's
//! action handlers calling a daw-control server, a CLI without an async
//! main): the facade wraps the async vox client and drives each call to
//! completion on a background tokio runtime.
//!
//! ```ignore
//! // once, at startup (native only)
//! let caller = BlockingCaller::background()?;          // owns a runtime
//! let tracks = TracksSyncClient::new(tracks_client, caller.clone());
//!
//! // hot path: plain sync calls, no .await
//! let all = tracks.all()?;
//! tracks.set_muted("guid".into(), true)?;
//! ```
//!
//! Never call a sync facade from inside an async context — blocking a
//! runtime worker on itself deadlocks. The facade is the boundary
//! *between* sync and async worlds, not a tool for use within the async
//! one.

use std::sync::Arc;

/// Drives futures to completion from sync code. Cheap to clone; all
/// clones share the runtime.
#[derive(Clone)]
pub struct BlockingCaller {
    inner: Inner,
}

#[derive(Clone)]
enum Inner {
    /// Handle into a runtime someone else owns and keeps alive.
    Handle(tokio::runtime::Handle),
    /// A runtime owned by this caller (and its clones), built by
    /// [`BlockingCaller::background`]. Dropped with the last clone.
    Owned(Arc<tokio::runtime::Runtime>),
}

impl BlockingCaller {
    /// Drive calls on an existing runtime. The runtime must outlive
    /// every call (`Handle::block_on` panics if its runtime is gone).
    pub fn from_handle(handle: tokio::runtime::Handle) -> Self {
        Self {
            inner: Inner::Handle(handle),
        }
    }

    /// Build and own a small background runtime (one worker thread) —
    /// the right default for an otherwise-sync binary or plugin.
    pub fn background() -> std::io::Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()?;
        Ok(Self {
            inner: Inner::Owned(Arc::new(rt)),
        })
    }

    /// Block the current thread until `fut` resolves.
    ///
    /// # Panics
    ///
    /// Panics if called from within an async execution context (a
    /// tokio worker) — that's a deadlock, not a slow path.
    pub fn block_on<F: std::future::Future>(&self, fut: F) -> F::Output {
        match &self.inner {
            Inner::Handle(h) => h.block_on(fut),
            Inner::Owned(rt) => rt.block_on(fut),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_runtime_drives_futures() {
        let caller = BlockingCaller::background().expect("runtime");
        let out = caller.block_on(async { 21 * 2 });
        assert_eq!(out, 42);
    }

    #[test]
    fn clones_share_the_runtime() {
        let caller = BlockingCaller::background().expect("runtime");
        let clone = caller.clone();
        drop(caller);
        assert_eq!(clone.block_on(async { 1 }), 1);
    }
}
