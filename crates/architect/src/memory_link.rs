//! In-process [`vox_types::Link`] — architect's own in-memory transport,
//! available on **wasm** where vox-core's `memory_link` module is
//! `#[cfg(not(target_arch = "wasm32"))]` and therefore absent.
//!
//! This is a straight port of `vox_core::memory_link` (same channel
//! logic, same `Backing::Boxed` recv), trimmed to the wasm reality:
//! there are no Unix file descriptors to pass, so each direction is a
//! plain [`tokio::sync::mpsc`] channel of `Vec<u8>` frames (tokio's
//! `sync` feature is wasm-clean — no reactor assumptions). It exists so
//! [`crate::local::LocalServer`] can serve a `LayerRouter` in-process in
//! the browser, exactly as it does natively over
//! [`vox_core::memory_link_pair`].
//!
//! Native builds never compile this module — they use vox-core's own
//! `memory_link` (which additionally does `SCM_RIGHTS` fd-passing on
//! Unix). Keeping architect's copy wasm-only is deliberate: it is a
//! stopgap for a capability vox simply didn't enable for wasm, not a
//! replacement for the native path.

use tokio::sync::mpsc;
use vox_types::{Backing, Link, LinkRx, LinkTx};

/// One in-process frame: just bytes (no fd passing on wasm).
type MemItem = Vec<u8>;

/// In-process [`Link`] backed by tokio mpsc channels — the wasm sibling
/// of `vox_core::MemoryLink`.
pub struct MemoryLink {
    tx: mpsc::Sender<MemItem>,
    rx: mpsc::Receiver<MemItem>,
}

/// Create a pair of connected [`MemoryLink`]s.
///
/// Returns `(a, b)` where sending on `a` delivers to `b` and vice versa.
pub fn memory_link_pair(buffer: usize) -> (MemoryLink, MemoryLink) {
    let (tx_a, rx_b) = mpsc::channel(buffer);
    let (tx_b, rx_a) = mpsc::channel(buffer);

    let a = MemoryLink { tx: tx_a, rx: rx_a };
    let b = MemoryLink { tx: tx_b, rx: rx_b };

    (a, b)
}

impl Link for MemoryLink {
    type Tx = MemoryLinkTx;
    type Rx = MemoryLinkRx;

    fn split(self) -> (Self::Tx, Self::Rx) {
        (MemoryLinkTx { tx: self.tx }, MemoryLinkRx { rx: self.rx })
    }
}

// ── Tx ──────────────────────────────────────────────────────────────────

/// Sending half of a [`MemoryLink`].
#[derive(Clone)]
pub struct MemoryLinkTx {
    tx: mpsc::Sender<MemItem>,
}

impl LinkTx for MemoryLinkTx {
    async fn send(&self, bytes: Vec<u8>) -> std::io::Result<()> {
        self.tx.send(bytes).await.map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::ConnectionReset, "receiver dropped")
        })
    }

    async fn close(self) -> std::io::Result<()> {
        drop(self.tx);
        Ok(())
    }
}

// ── Rx ──────────────────────────────────────────────────────────────────

/// Receiving half of a [`MemoryLink`].
pub struct MemoryLinkRx {
    rx: mpsc::Receiver<MemItem>,
}

/// MemoryLink never fails on recv — the only "error" is channel closed
/// (returns `Ok(None)`).
#[derive(Debug)]
pub struct MemoryLinkRxError;

impl std::fmt::Display for MemoryLinkRxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "memory link rx error (unreachable)")
    }
}

impl std::error::Error for MemoryLinkRxError {}

impl LinkRx for MemoryLinkRx {
    type Error = MemoryLinkRxError;

    async fn recv(&mut self) -> Result<Option<Backing>, Self::Error> {
        match self.rx.recv().await {
            Some(bytes) => Ok(Some(Backing::Boxed(bytes.into_boxed_slice()))),
            None => Ok(None),
        }
    }
}
