//! The Architect app on a desk (Blitz), and its Action List from the
//! command line and over MCP, windowed or headless: [`run`] is the whole
//! of `architect-desktop`'s `main` (the Architect app) and of
//! `architect-sandbox`'s (`apps/sandbox`, the dev sandbox), and of any
//! build of someone's own, each with its [`Distribution`] (#491).
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

pub use architect_distribution::{self as distribution, Bundle, Distribution, Icon};

use std::io::Write as _;
use std::process::ExitCode;

/// `<app> distribution [icon]`: what the distribution is, as JSON
/// (`Distribution::describe`: the web bundle's `/app/distribution.json`,
/// what `cargo xtask dist` packages by), or its icon's SVG.
#[must_use]
pub fn distribution_command(dist: &Distribution, verb: Option<&str>) -> ExitCode {
    let text = match verb {
        None | Some("describe") => {
            serde_json::to_string_pretty(&dist.describe()).unwrap_or_default()
        }
        Some("icon") => dist.icons().svg.to_owned(),
        Some(other) => {
            let _ = writeln!(
                std::io::stderr(),
                "{}: distribution [describe|icon], not {other}",
                dist.app_name()
            );
            return ExitCode::from(64);
        }
    };
    let _ = writeln!(std::io::stdout(), "{text}");
    ExitCode::SUCCESS
}

/// The whole of a binary's `main` without the window
/// (`--no-default-features`): the node of `distribution` (`serve`, or the
/// same with `serve` left out), its `files`/`agent` commands, or
/// `distribution`.
#[cfg(not(feature = "gui"))]
#[must_use]
pub fn run(distribution: Distribution) -> ExitCode {
    let dist = distribution.install();
    let mut args = std::env::args_os().skip(1).peekable();
    match args.peek().and_then(|a| a.to_str()) {
        Some("serve") => {
            args.next();
        }
        Some("files") => return node::cli::files(args.skip(1)),
        Some("agent") => return node::cli::agent(args.skip(1)),
        Some(node::cli::GUARD) => return node::cli::guard(args.skip(1)),
        Some("distribution") => {
            let verb = args.nth(1).and_then(|a| a.into_string().ok());
            return distribution_command(dist, verb.as_deref());
        }
        _ => {}
    }
    node::main(args, dist)
}
