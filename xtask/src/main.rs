//! Repo tooling. `cargo xtask <command>`.
//!
//! Everything here is about the seam between this repo and the ones that
//! consume it — the release tags they pin, and the manifests they pin
//! them in. Nothing here is part of the published crate surface.

mod boundaries;
mod docs;
mod fleet;
mod git;
mod mirror;
mod scaffold;
mod tags;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xtask", about = "architect repo tooling", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scaffold a feature: ONE crate at `features/<name>/` (layout ADR,
    /// `docs/content/architecture/layout.md`). The contract in `src/lib.rs`
    /// (default features, wasm-clean), an in-memory backend behind
    /// `memory`, `vox` and `ui` flags, a `spec/` and a passing test. Adds
    /// it to the workspace members and `[workspace.dependencies]`.
    Feature {
        #[command(subcommand)]
        action: FeatureAction,
    },
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
    /// Pull a sanitised copy of a running auth server's database.
    ///
    /// The scrubbing happens server-side: password hashes, sessions,
    /// OAuth tokens, API keys, passkeys and two-factor secrets never
    /// leave the machine that holds them. What arrives is the
    /// organization graph — who is in what, with which role — which is
    /// what makes a local server useful for reproducing a bug or
    /// replaying an incident.
    ///
    /// Needs an administrator's session token in `AUTH_ADMIN_TOKEN`.
    AuthMirror {
        /// Base URL of the server to copy, e.g.
        /// `https://auth.fasttrackstudio.app`.
        #[arg(long)]
        from: String,
        /// Where to write the snapshot.
        #[arg(long, default_value = "auth-snapshot.json")]
        out: PathBuf,
        /// Replace every address with `user-<id>@local.invalid`.
        #[arg(long)]
        redact_emails: bool,
    },
    /// Check the crate-layout boundaries in `boundaries.toml`.
    ///
    /// Resolves every layer-table row with `cargo tree -p` and fails when
    /// a contract or ui feature set reaches an engine, storage or host
    /// dependency, a crate outside `apps/` turns a transport host on, a
    /// member lacks workspace lints, the lockfile pins architect twice,
    /// or a feature crate has no row. Counts are ratcheted against the
    /// table's `[baseline]`. See `docs/content/architecture/layout.md`.
    Boundaries {
        /// The layer table.
        #[arg(long, default_value = "boundaries.toml")]
        config: PathBuf,
        /// Also print the rows and the rules that held.
        #[arg(short, long)]
        verbose: bool,
    },
    /// Check that every path and command the docs name exists.
    ///
    /// Reads `docs/content/**/*.md` and `README.md`: repo paths in code,
    /// page links, source links into `main`, `just` recipes, `cargo xtask`
    /// commands and `cargo -p` packages. Other repos' paths are written
    /// `task:apps/…`; sketches sit between `<!-- doc-check: off -->` and
    /// `<!-- doc-check: on -->`.
    Docs,
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

#[derive(Subcommand)]
enum FeatureAction {
    /// Create the crate for a new feature.
    New {
        /// Lowercase kebab-case crate name (`inventory`, `tour-dates`).
        name: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Tags { verbose } => tags::run(verbose),
        Command::Boundaries { config, verbose } => {
            let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            boundaries::run_cli(&root, &config, verbose)
        }
        Command::Docs => {
            let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let commands = Cli::command()
                .get_subcommands()
                .map(|c| c.get_name().to_owned())
                .collect();
            docs::run_cli(&root, commands)
        }
        Command::Feature {
            action: FeatureAction::New { name },
        } => {
            let repo = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            scaffold::feature_new(&repo, &name).map(|written| {
                for path in written {
                    println!("wrote {}", path.display());
                }
                println!(
                    "next: `cargo test -p {name}`; the contract alone: \
                     `cargo check -p {name} --target wasm32-unknown-unknown`"
                );
            })
        }
        Command::AuthMirror {
            from,
            out,
            redact_emails,
        } => mirror::run(&from, &out, redact_emails),
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
