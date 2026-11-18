//! Process views: a pane of the app's window drawn by another program.
//!
//! A process view is a [`PaintView`](architect_workspace::PaintView)
//! that runs in a program of its own: its own wgpu device, its own crash
//! domain, built and released on its own. The program renders each
//! frame with Vello into memory the app's GPU device reads directly (a
//! dma-buf texture on Linux, [`wire::Transport::Dmabuf`]; an `IOSurface`
//! on macOS, [`wire::Transport::IoSurface`]), or, where the app's
//! device can't import one, into shared memory the app uploads
//! ([`wire::Transport::Shm`]). The app forwards the pane's input and
//! size, and draws the program's newest frame in the pane's place
//! (`docs/content/architecture/process-views.md`).
//!
//! - [`wire`]: the messages, and `sock`: the socket pair they travel on,
//!   with the frames' descriptors beside them.
//! - `shm`: the shared memory a frame is read back into where the GPU
//!   can't share it, the same on every platform.
//! - `surface` (macOS): `IOSurface`s, the frames the GPU shares there, and
//!   the Mach port they travel on beside the socket.
//! - `guest` (feature): `guest::serve` serves a view from a program.
//! - `host` (feature): `host::ProcessViewPane`, the pane, and its
//!   session (started, read, stopped, restarted).
//! - `extension` (feature): extensions' `kind = "process"` views drawn
//!   by the pane, whichever copy of the extension is loaded (#838).
//! - `demo` (feature): `demo::Orbits`, the lab's scene, and the
//!   `process-view-demo` program that serves it.
//!
//! Linux (a dma-buf texture, else shared memory) and macOS (an
//! `IOSurface`-backed texture, else shared memory). Windows (a DXGI
//! shared handle) is designed in the docs, not built.

/// The process view's kind id: a view drawn by a program of its own.
pub const PROCESS: &str = architect_workspace::PROCESS;

/// The [`PROCESS`] kind: a pane drawn by another program, which takes
/// input (IME too) and draws on the GPU.
#[must_use]
pub fn kind_spec() -> architect_workspace::KindSpec {
    architect_workspace::KindSpec::process()
}

pub mod wire;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod sock;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod shm;

#[cfg(target_os = "macos")]
pub mod surface;

#[cfg(all(
    any(target_os = "linux", target_os = "macos"),
    any(feature = "guest", feature = "host")
))]
pub mod gpu;

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "guest"))]
pub mod guest;

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "host"))]
pub mod host;

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "extension"))]
pub mod extension;

#[cfg(feature = "demo")]
pub mod demo;
