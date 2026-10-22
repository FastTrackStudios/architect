//! `architect::testing`: support for an app's own test binaries.
//!
//! Two parts, two features, both meant for `[dev-dependencies]` only:
//!
//! - [`alloc`] (feature `testing-alloc`, std only): a counting
//!   `GlobalAlloc` and its readings. A test binary installs it itself; the
//!   library never does.
//! - The sandbox (feature `testing`, native only): [`sandbox_server`]
//!   boots an app over a fresh temporary directory with `HOME` and the
//!   per-user directory variables pointed inside it, under one
//!   process-wide environment lock ([`env_lock`]), then serves it on an
//!   ephemeral loopback port. A test that boots through it cannot read or
//!   write the developer's real home, whatever the app resolves from the
//!   environment.
//!
//! Extracted from Task's `apps/server/tests/it/support/` (STA-400). See
//! `docs/content/architecture/testing.md`.

pub mod alloc;

#[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
mod sandbox;
#[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
pub use sandbox::{
    EnvGuard, HOME_VARS, IntoSandboxApp, Sandbox, SandboxBuilder, SandboxServer, Sandboxed,
    env_lock, sandbox, sandbox_server,
};
