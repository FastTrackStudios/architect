//! The Architect app on a desk, painted by Blitz: the person's vault (its
//! first run asks for one), the shell, the workspace and extensions. The
//! app is also its node (`src/node/`): the window starts it in-process,
//! so installing the app is all a machine needs to be a device on the
//! account and a sync peer. A server runs the same app without the
//! window (`serve`).
//! `ARCHITECT_DEVICE=iphone|iphone-side|ipad|ipad-side` opens it standing
//! in for that device. Served with `dx serve`, an `rsx!` edit in
//! `apps/ui` or in `architect-shell` shows at once.
//!
//! With `ARCHITECT_MCP_PORT` set it serves its Action List over MCP on
//! that loopback port. One instance: a second launch brings the first
//! forward and ends. With `--background` (a login launch) it starts in the
//! menu bar, with no window and no Dock icon.
//!
//! Its Action List from the command line and over MCP (`src/lib.rs`):
//!
//! ```text
//! architect-desktop actions list               # ask the running app
//! architect-desktop actions run APP_STATUS --headless
//! architect-desktop --headless                 # the app with no window, serving its socket
//! architect-desktop mcp [--headless]           # MCP on stdio
//! architect-desktop mcp --app <socket>         # MCP over another running app (the iOS app)
//! architect-desktop actions shot /tmp/app.png  # the window, drawn by the app (APP_SHOT)
//! architect-desktop extensions check <folder>  # EXT_CHECK with no window; nonzero when it fails
//! architect-desktop extensions dev <folder>    # EXT_DEV: the folder as a development copy in the running app
//! architect-desktop extensions registries      # EXT_REGISTRIES: the store's and the person's registries
//! architect-desktop serve [--config <file>]    # the app's node with no window: a server (src/node/)
//! architect-desktop dev [--extension <dir>]…  # an isolated instance, as many as wanted (src/dev.rs)
//! ```
//!
//! In a window it keeps `<app dir>/AGENTS.md` and `actions.schema.json`
//! written from its live list (`architect_control::guide`). The dev
//! sandbox, on made-up data, is `architect-sandbox` (`apps/sandbox`).

/// The Architect app: architect's own distribution
/// (`architect_distribution::architect`). The window runs its node in the
/// same process; built without the window (`--no-default-features`), the
/// binary is that node alone: `serve`, or the same with `serve` left out.
fn main() -> std::process::ExitCode {
    architect_desktop::run(architect_distribution::architect())
}
