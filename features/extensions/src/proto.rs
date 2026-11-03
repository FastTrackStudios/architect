//! The service a process extension serves the host, over vox (features
//! `process` and `guest`). One trait, so the host and every extension
//! share its types with no codegen; the host is the client.
//!
//! A view is a subscription held open for as long as the pane shows it:
//! [`ProcessExtensionStreamClient::view`] streams its [`Frame`]s, and
//! [`ProcessExtension::event`] hands it the pane's events. Dropping the
//! subscription, or [`ProcessExtension::close`], unmounts it.

// The router that serves it asks for `Send` futures itself.
#![allow(async_fn_in_trait)]

use crate::wire::{Frame, Hello, Open, ThemeData, ViewEvent};

/// The environment variable a host starts a process extension with: the
/// path of the local socket to connect to.
pub const SOCKET: &str = "ARCHITECT_EXT_SOCKET";

/// Why a process extension did not do what it was asked.
#[architect::error]
#[derive(Eq)]
pub enum ProcessError {
    /// It has no action of that id.
    #[error("no action {0}")]
    NoAction(String),
    /// No view is open in that pane.
    #[error("no view open in pane {0}")]
    NoPane(u64),
    /// The action ran and failed.
    #[error("{0}")]
    Failed(String),
}

/// What a process extension does for the host.
#[architect::rpc]
pub trait ProcessExtension {
    /// Who it is: the API and wire it speaks, its views and actions.
    async fn hello(&self) -> Result<Hello, ProcessError>;
    /// Runs its action `action` with JSON `args`; the result is JSON.
    async fn run(&self, action: String, args: String) -> Result<String, ProcessError>;
    /// An event in the view open in `pane`.
    async fn event(&self, pane: u64, event: ViewEvent) -> Result<(), ProcessError>;
    /// Unmounts the view open in `pane`.
    async fn close(&self, pane: u64) -> Result<(), ProcessError>;
    /// Mounts a view: its frames, from the first render on.
    #[subscribe]
    fn view(&self, open: Open) -> Frame;
    /// The app's theme: as it connects, and on every change. Only to a
    /// process that said wire [`THEMED_WIRE`](crate::wire::THEMED_WIRE)
    /// or later (last, so the methods before it keep their places).
    async fn theme(&self, theme: ThemeData) -> Result<(), ProcessError>;
}
