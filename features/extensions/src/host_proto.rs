//! The host service over a process extension's connection (features
//! `process` and `guest`): the other way from [`crate::proto`] — the
//! extension is the client, the host serves.
//!
//! The host serves [`ExtensionHost`] on the connection it accepted from
//! the program, so the program's calls are its own (the host knows whose
//! connection it is); the guest opens a lane to it as it connects
//! (`guest::host`). One method: a `HostRequest` in, a `HostAnswer` out,
//! both JSON (`crate::hostcall`) — the same calls a Luau script, an
//! in-process extension and a WASM component make.

// The router that serves it asks for `Send` futures itself.
#![allow(async_fn_in_trait)]

use crate::proto::ProcessError;

/// What the host does for a process extension.
#[architect::rpc]
pub trait ExtensionHost {
    /// One call (`request`: a `HostRequest` as JSON); its answer as JSON
    /// (`HostAnswer`: `{"ok": …}` or `{"error": {kind, …}}`). An error
    /// here is the wire's, never a refusal.
    async fn call(&self, request: String) -> Result<String, ProcessError>;
}
