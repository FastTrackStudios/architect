//! Process views: a pane of the app's window drawn by another program.
//!
//! A process view is a [`PaintView`](architect_workspace::PaintView)
//! that runs in a program of its own: its own wgpu device, its own crash
//! domain, built and released on its own. The program renders each
//! frame with Vello into memory the app's GPU device reads directly (a
//! dma-buf texture, [`wire::Transport::Dmabuf`]), or, where the app's
//! device can't import one, into shared memory the app uploads
//! ([`wire::Transport::Shm`]). The app forwards the pane's input and
//! size, and draws the program's newest frame in the pane's place
//! (`docs/content/architecture/process-views.md`).
//!
//! - [`wire`]: the messages, and `sock`: the socket pair they travel on,
//!   with the frames' descriptors beside them (Linux).
//! - `guest` (feature): `guest::serve` serves a view from a program.
//! - `host` (feature): `host::ProcessViewPane`, the pane, and its
//!   session (started, read, stopped, restarted).
//! - `demo` (feature): `demo::Orbits`, the lab's scene, and the
//!   `process-view-demo` program that serves it.
//!
//! Linux only for now. macOS (an IOSurface-backed texture) and Windows (a
//! DXGI shared handle) are designed in the docs, not built.

/// The process view's kind id: a view drawn by a program of its own.
pub const PROCESS: &str = "process";

/// The [`PROCESS`] kind: a pane drawn by another program, which takes
/// input (IME too) and draws on the GPU.
#[must_use]
pub fn kind_spec() -> architect_workspace::KindSpec {
    use architect_workspace::{Capability, KindSpec, Placement, ViewHost};
    KindSpec::new(PROCESS, "architect")
        .host(ViewHost::Blitz)
        .placement(Placement::Pane)
        .capability(Capability::Input)
        .capability(Capability::Ime)
        .capability(Capability::Gpu)
}

pub mod wire;

#[cfg(target_os = "linux")]
pub mod sock;

#[cfg(all(target_os = "linux", any(feature = "guest", feature = "host")))]
pub mod gpu;

#[cfg(all(target_os = "linux", feature = "guest"))]
pub mod guest;

#[cfg(all(target_os = "linux", feature = "host"))]
pub mod host;

#[cfg(feature = "demo")]
pub mod demo;
