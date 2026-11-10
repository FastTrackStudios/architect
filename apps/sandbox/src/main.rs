//! The dev sandbox: the Architect app (`architect-desktop`'s window and
//! CLI, `architect_desktop::run`) on made-up data. Its Inbox, Tasks and
//! Schedule are sample lists, its Vault's notes are in memory (or the
//! scratch demo vault `$ARCHITECT_VAULT` names: `just sandbox`, `devenv
//! up sandbox`), and the example extensions beside it (`apps/sandbox`: the
//! Board, the Clicker, the WASI Counter, the registry's metronome, tidy,
//! outline and surface) are loaded. The goldens, scenarios and films
//! (`apps/sandbox/media`) run on the same data in-process.
//!
//! It has a folder of its own (`<config dir>/architect-sandbox`,
//! `$ARCHITECT_SANDBOX_CONFIG_DIR`), a bundle id of its own and a control
//! socket of its own (`architect-sandbox actions list`), so it runs beside
//! the Architect app and never touches the person's settings.

fn main() -> std::process::ExitCode {
    architect_desktop::run(architect_app::sandbox())
}
