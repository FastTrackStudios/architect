//! The media command an app's binary is (feature `capture`): its
//! scenarios filmed into `--out`, or played as tests with `--test`.
//!
//! ```rust,ignore
//! fn main() -> std::process::ExitCode {
//!     architect_guide::cli::main(my_app::guide_app, Path::new("scenarios"))
//! }
//! ```
//!
//! `lab-media` (`apps/lab/media`) is one: `cargo run -p
//! architect-lab-media -- showcase-layout --out target/media`.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;

use crate::capture::{Recorder, encode};
use crate::run::{App, run};

/// Film an app's guide scenarios, or play them as tests.
#[derive(Debug, Parser)]
struct Args {
    /// Scenario ids (or `.styx` paths); none: every scenario.
    scenarios: Vec<String>,
    /// Where the media go.
    #[arg(long, default_value = "target/media")]
    out: PathBuf,
    /// Play them with no frames, checking every step.
    #[arg(long)]
    test: bool,
    /// Keep the frames (`<out>/<id>.frames/`) after encoding.
    #[arg(long)]
    keep_frames: bool,
    /// List the scenarios and stop.
    #[arg(long)]
    list: bool,
}

/// Runs the command over the scenarios in `dir`, a fresh app from `make`
/// for each.
#[must_use]
pub fn main(make: impl Fn() -> App, dir: &Path) -> ExitCode {
    let args = Args::parse();
    let mut out = std::io::stdout().lock();
    match go(&args, &make, dir, &mut out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn go(
    args: &Args,
    make: &impl Fn() -> App,
    dir: &Path,
    out: &mut impl std::io::Write,
) -> Result<(), String> {
    let all = crate::load_dir(dir).map_err(|e| e.to_string())?;
    let mut picked = Vec::new();
    for want in &args.scenarios {
        if Path::new(want).extension().is_some_and(|x| x == "styx") {
            let source = std::fs::read_to_string(want).map_err(|e| format!("{want}: {e}"))?;
            picked.push((
                PathBuf::from(want),
                crate::load(&source).map_err(|e| e.to_string())?,
            ));
            continue;
        }
        let found = all
            .iter()
            .find(|(_, s)| s.id == *want)
            .ok_or_else(|| format!("no scenario {want:?} in {}", dir.display()))?;
        picked.push(found.clone());
    }
    if args.scenarios.is_empty() {
        picked = all;
    }
    let say = |out: &mut dyn std::io::Write, line: String| {
        let _ = writeln!(out, "{line}");
    };
    if args.list {
        for (path, s) in &picked {
            say(out, format!("{}\t{}\t{}", s.id, s.title, path.display()));
        }
        return Ok(());
    }
    for (_, scenario) in &picked {
        let app = make();
        if args.test {
            let report = run(&app, scenario, None).map_err(|e| format!("{}: {e}", scenario.id))?;
            say(
                out,
                format!("ok {} ({} steps)", scenario.id, report.steps.len()),
            );
            continue;
        }
        let frames = args.out.join(format!("{}.frames", scenario.id));
        let mut recorder =
            Recorder::new(scenario, &frames, app.resolvers.clone()).map_err(|e| e.to_string())?;
        let ran = run(&app, scenario, Some(&mut recorder));
        // A frame that could not be written says more than the step that
        // failed after it.
        let count = recorder
            .finish()
            .map_err(|e| format!("{}: {e}", scenario.id))?;
        ran.map_err(|e| format!("{}: {e}", scenario.id))?;
        let files = encode(scenario, &frames, count, &args.out)
            .map_err(|e| format!("{}: {e}", scenario.id))?;
        if !args.keep_frames {
            let _ = std::fs::remove_dir_all(&frames);
        }
        let names: Vec<String> = files.iter().map(|f| f.display().to_string()).collect();
        say(
            out,
            format!("{} ({count} frames): {}", scenario.id, names.join(", ")),
        );
    }
    Ok(())
}
