//! The Architect app on a desk (Blitz), and its Action List from the
//! command line and over MCP, windowed or headless: [`run`] is the whole
//! of `architect-desktop`'s `main` (the Architect app) and of
//! `architect-sandbox`'s (`apps/sandbox`, the dev sandbox), each with its
//! [`Mode`].
//!
//! - In the window, the app serves its control socket
//!   (`architect-control`; on in a debug build, `FTS_CONTROL=0|1` says
//!   otherwise): `architect-desktop actions list` asks it.
//! - `architect-desktop --headless` is the app with no window, serving the
//!   same socket until it is stopped: its root mounted in a headless
//!   document (`architect_guide::run::Headless`), so every action its
//!   components register is there.
//! - `architect-desktop actions … --headless` starts that for one command.
//! - `architect-desktop mcp [--headless]` is MCP on stdio for a model:
//!   bridged onto the running lab's socket, or over a headless one.
//! - `architect-desktop serve` is the app as an always-on node, with no
//!   window and nothing drawn: [`node`]. It holds the org's vaults as a
//!   sync peer over iroh, signs in to the account as one more device, and
//!   serves Sites, MCP, the HTTP face and same-origin vox. Built with
//!   `--no-default-features` (no `gui`), the binary is that alone, with no
//!   Blitz, wgpu or wayland in it: the server image (`flake.nix`).

#[cfg(feature = "gui")]
mod app;
#[cfg(feature = "gui")]
mod mcp_port;

#[cfg(feature = "gui")]
pub use app::*;

pub mod node;
