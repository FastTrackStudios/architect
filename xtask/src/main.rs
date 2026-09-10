//! Repo tooling. `cargo xtask <command>`.
//!
//! Everything here is about the seam between this repo and the ones that
//! consume it — the release tags they pin, and the manifests they pin
//! them in. Nothing here is part of the published crate surface.

mod fleet;
mod git;
mod tags;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xtask", about = "architect repo tooling", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check that release tags order the commits the way their numbers do.
    ///
    /// Six repos pin architect by tag, and the only question they ask of a
    /// tag is "am I ahead or behind?". That has an answer only while the
    /// numbering matches history.
    Tags {
        /// Also print every tag with its commit, date and subject.
        #[arg(short, long)]
        verbose: bool,
    },
    /// Repoint every architect-owned git dep in a manifest at one tag.
    ///
    /// Bumping these one crate at a time is how a consumer ends up with
    /// several checkouts of this repo in one lockfile.
    FleetBump {
        /// The tag to pin, e.g. `v0.9.0`.
        tag: String,
        /// Consumer manifests (or the directories holding them).
        #[arg(required = true)]
        manifests: Vec<PathBuf>,
        /// Report the rewrite without performing it.
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Tags { verbose } => tags::run(verbose),
        Command::FleetBump {
            tag,
            manifests,
            dry_run,
        } => manifests
            .iter()
            .try_for_each(|manifest| fleet::run(manifest, &tag, dry_run)),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}
