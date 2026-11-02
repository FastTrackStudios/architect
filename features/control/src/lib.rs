//! An app's Action List from the command line, the same as over MCP.
//!
//! Everything an architect app can do is an action on its Action List
//! (`architect::action::ActionList`). A model reaches it over MCP
//! (`architect-mcp`); this crate is the other face: a person, a script or
//! CI on the command line, through a socket the running app serves, or
//! through the app's list started with no window.
//!
//! - [`protocol`]: what is asked ([`protocol::Request`]) and what is told
//!   ([`protocol::Answer`], [`protocol::Failure`] and its [`protocol::Code`]).
//!   MCP's generic tools are these requests: one parse, one answer, two
//!   renderings — so the two faces cannot drift.
//! - [`Surface`]: an app's list, name and default caller, answering a
//!   request for a caller through the list's authorizer.
//! - [`endpoint`]: where an app's socket is (`<runtime dir>/fts/<app>.sock`;
//!   a named pipe on Windows) and whether it serves one (`FTS_CONTROL`, on
//!   in debug builds by default).
//! - `server` (feature `server`): `server::Control` serves a surface on its
//!   socket — newline-delimited JSON, `{id, method, params}` in,
//!   `{id, result}` or `{id, error}` out — on a thread and runtime of its
//!   own.
//! - `client` (feature `client`): `client::Client`, blocking.
//! - `cli` (feature `cli`): the `actions` command an app embeds
//!   (`cli::Cli`), its headless mode (`cli::Headless`), and the generic
//!   client `architect-actions`.

pub mod endpoint;
pub mod protocol;
mod surface;

#[cfg(feature = "cli")]
pub mod cli;
#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "server")]
pub mod server;

pub use surface::Surface;
