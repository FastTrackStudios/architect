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
//! - The app is its node ([`node`]): the window starts it in-process
//!   (`start_node`), with defaults for a person's machine (HTTP on
//!   loopback, on the account as the app's signed-in account). It holds
//!   vaults as a sync peer over iroh, is a device on the account, and
//!   serves the HTTP face and same-origin vox; Sites, share links and MCP
//!   when configured.
//! - `architect-desktop serve` is the same node with no window and nothing
//!   drawn: what a server runs. Built with `--no-default-features` (no
//!   `gui`), the binary is that alone, with no Blitz, wgpu or wayland in
//!   it: the server image (`flake.nix`).

#[cfg(feature = "gui")]
mod app;
#[cfg(feature = "gui")]
pub mod dev;
#[cfg(feature = "gui")]
pub mod install;
#[cfg(feature = "gui")]
mod mcp_port;

#[cfg(feature = "gui")]
pub use app::*;

pub mod links;
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

/// `<app> web-ext <out-dir>`: `dist`'s `/_ext/` as static files.
///
/// Everything a node of `dist` answers under `/_ext/` ([`write_web_ext`]
/// of its islands and favourites), written below `<out-dir>/_ext/` for a
/// static server to serve with the web bundle (`scripts/web-bundle.sh`'s
/// `ARCHITECT_WEB_EXT`).
#[must_use]
pub fn web_ext_command(dist: &Distribution, out: Option<&str>) -> ExitCode {
    let Some(out) = out.filter(|o| !o.is_empty() && !o.starts_with('-')) else {
        let _ = writeln!(std::io::stderr(), "{}: web-ext <out-dir>", dist.app_name());
        return ExitCode::from(64);
    };
    let out = std::path::Path::new(out);
    match write_web_ext(out, &dist.islands(), &dist.favourites()) {
        Ok(files) => {
            let _ = writeln!(
                std::io::stdout(),
                "{files} files in {}",
                out.join("_ext").display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            let _ = writeln!(
                std::io::stderr(),
                "{}: web-ext {}: {e}",
                dist.app_name(),
                out.display()
            );
            ExitCode::FAILURE
        }
    }
}

/// Writes the static `/_ext/` tree of `islands` and `favourites` below `out`.
///
/// Each path of `architect_sites::island::export` as a file
/// (`<out>/_ext/…`, the body a node answers there), and
/// `<out>/_ext/headers.json` (`architect_sites::island::headers_json`:
/// `{"/_ext/…": {"<header>": "<value>"}}`, the headers a static server
/// must send with each). An `_ext` folder already in `out` is replaced
/// whole, so nothing of an older build stays. How many files it wrote,
/// `headers.json` not counted.
///
/// # Errors
///
/// A folder or file that does not write, or a path of the tree that is
/// not a plain one below `/_ext/` (none is; nothing is written past
/// `out`).
pub fn write_web_ext(
    out: &std::path::Path,
    islands: &[architect_sites::SiteIsland],
    favourites: &[architect_favourites::Listed],
) -> std::io::Result<usize> {
    use architect_sites::island;
    let files = island::export(islands, favourites);
    let root = out.join("_ext");
    if root.exists() {
        std::fs::remove_dir_all(&root)?;
    }
    std::fs::create_dir_all(&root)?;
    for file in &files {
        let rest = file
            .path
            .strip_prefix(island::ISLANDS_PATH)
            .filter(|r| {
                !r.is_empty()
                    && r.split('/').all(|seg| {
                        !seg.is_empty() && seg != "." && seg != ".." && !seg.contains('\\')
                    })
            })
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("{}: not a path below /_ext/", file.path),
                )
            })?;
        let to = root.join(rest);
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(to, &file.body)?;
    }
    let headers = serde_json::to_string_pretty(&island::headers_json(&files))
        .map_err(std::io::Error::other)?;
    let name = island::HEADERS_PATH
        .strip_prefix(island::ISLANDS_PATH)
        .unwrap_or("headers.json");
    std::fs::write(root.join(name), format!("{headers}\n"))?;
    Ok(files.len())
}

/// The whole of a binary's `main` without the window
/// (`--no-default-features`): the node of `distribution` (`serve`, or the
/// same with `serve` left out), its `files`/`agent` commands,
/// `distribution` or `web-ext`.
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
        Some("web-ext") => {
            let out = args.nth(1).and_then(|a| a.into_string().ok());
            return web_ext_command(dist, out.as_deref());
        }
        _ => {}
    }
    node::main(args, dist)
}
