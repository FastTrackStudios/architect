//! Which distribution of Architect is running (#491): the Architect app,
//! the dev sandbox, or a build of someone's own
//! (`architect_distribution::Distribution`).
//!
//! The binary says which before it launches
//! (`architect_distribution::Distribution::install`, done by
//! `architect_desktop::run`). A test or a film that mounts the app
//! in-process says nothing and gets the sandbox ([`sandbox`]), so the
//! goldens, scenarios and films run on its made-up data.
//!
//! Each has a folder of its own (`Distribution::app_dir`) and a bundle id
//! of its own, so they run side by side and the sandbox never touches
//! the person's settings.

use std::path::PathBuf;
use std::sync::OnceLock;

pub use architect_distribution::{Bundle, Distribution, Icon, architect};

/// The dev sandbox: the Architect app on made-up data (the Inbox, Tasks,
/// Schedule, the Board and the Clicker), for framework demos, goldens,
/// scenarios and films. Compiled in: the Board (`apps/sandbox/board`)
/// and the setlists decks demo (#441).
#[must_use]
pub fn sandbox() -> Distribution {
    Distribution::new("Architect Sandbox")
        .app("architect-sandbox")
        .folder("architect-sandbox")
        .bundle_id("app.fasttrackstudio.architect-sandbox")
        .icon(Icon::ARCHITECT.tray("testtube.2", "applications-science"))
        .sandbox()
        .linked(&architect_sandbox_board::Board)
        // Setlists: the decks demo (#441), a setlist's songs as decks.
        .linked(&crate::setlists::SetlistsExtension)
}

static SANDBOX: OnceLock<Distribution> = OnceLock::new();

/// The distribution this process runs: the one its binary installed,
/// else (a test or a film mounting the app) the sandbox.
#[must_use]
pub fn distribution() -> &'static Distribution {
    architect_distribution::current().unwrap_or_else(|| SANDBOX.get_or_init(sandbox))
}

/// The running app's folder: its settings, themes, layouts, extensions in
/// development and the agents' guide (`AGENTS.md`).
#[must_use]
pub fn app_dir() -> Option<PathBuf> {
    distribution().app_dir()
}

/// [`app_dir`], or a folder of its name under the temp dir where the
/// platform has no config dir.
#[must_use]
pub fn app_dir_or_temp() -> PathBuf {
    distribution().app_dir_or_temp()
}
