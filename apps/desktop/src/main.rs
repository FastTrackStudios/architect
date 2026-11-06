//! The Architect app on a desk, painted by Blitz: the person's vault (its
//! first run asks for one), the shell, the workspace and extensions.
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
//! ```
//!
//! In a window it keeps `<app dir>/AGENTS.md` and `actions.schema.json`
//! written from its live list (`architect_control::guide`). The dev
//! sandbox, on made-up data, is `architect-sandbox` (`apps/sandbox`).

fn main() -> std::process::ExitCode {
    architect_desktop::run(architect_app::Mode::Architect)
}
