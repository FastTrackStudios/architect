//! `architect-actions`: any running architect app's Action List from the
//! command line. `architect-actions apps` lists them; `--app <name>` picks
//! one (the only one running unless given).
//!
//! ```text
//! architect-actions --app architect-desktop list --filter lab
//! architect-actions --app task run TASKS_CREATE --arg title=Hello
//! ```

fn main() -> std::process::ExitCode {
    architect_control::cli::Cli::generic().main(std::env::args_os())
}
