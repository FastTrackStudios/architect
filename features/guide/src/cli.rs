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
//! architect-lab-media -- showcase-layout --out target/media`. With
//! feature `story`, [`main_stories`] is the same command over the
//! scenarios of the stories linked in.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;

use crate::Scenario;
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

/// A scenario to play, where it came from, and its app.
struct Job<'a> {
    path: PathBuf,
    scenario: Scenario,
    make: &'a dyn Fn() -> App,
}

impl Clone for Job<'_> {
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
            scenario: self.scenario.clone(),
            make: self.make,
        }
    }
}

/// Runs the command over the scenarios in `dir`, a fresh app from `make`
/// for each.
#[must_use]
pub fn main(make: impl Fn() -> App, dir: &Path) -> ExitCode {
    let args = Args::parse();
    let found = crate::load_dir(dir).map(|all| {
        all.into_iter()
            .map(|(path, scenario)| Job {
                path,
                scenario,
                make: &make,
            })
            .collect::<Vec<_>>()
    });
    finish(found.map_err(|e| e.to_string()).and_then(|all| {
        let from = dir.display().to_string();
        go(
            &args,
            all,
            Some(&make),
            &from,
            &mut std::io::stdout().lock(),
        )
    }))
}

/// Runs the command over the scenarios of every story linked in that has
/// any (`#[story(scenario = "…")]`), each over its story alone
/// ([`crate::story::app`]).
#[cfg(feature = "story")]
#[must_use]
pub fn main_stories() -> ExitCode {
    let args = Args::parse();
    let stories: Vec<_> = crate::story::with_scenarios().collect();
    let makers: Vec<Box<dyn Fn() -> App>> = stories
        .iter()
        .map(|story| {
            let story = *story;
            let make: Box<dyn Fn() -> App> = Box::new(move || crate::story::app(story));
            make
        })
        .collect();
    let mut all = Vec::new();
    for (story, make) in stories.iter().zip(&makers) {
        match crate::story::scenarios(story) {
            Ok(list) => all.extend(list.into_iter().map(|scenario| Job {
                path: PathBuf::from(story.source),
                scenario,
                make: make.as_ref(),
            })),
            Err(e) => return finish(Err(e.to_string())),
        }
    }
    finish(go(
        &args,
        all,
        None,
        "the stories",
        &mut std::io::stdout().lock(),
    ))
}

fn finish(done: Result<(), String>) -> ExitCode {
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn go(
    args: &Args,
    all: Vec<Job<'_>>,
    for_files: Option<&dyn Fn() -> App>,
    from: &str,
    out: &mut impl std::io::Write,
) -> Result<(), String> {
    let mut picked = Vec::new();
    for want in &args.scenarios {
        if let Some(make) =
            for_files.filter(|_| Path::new(want).extension().is_some_and(|x| x == "styx"))
        {
            let source = std::fs::read_to_string(want).map_err(|e| format!("{want}: {e}"))?;
            picked.push(Job {
                path: PathBuf::from(want),
                scenario: crate::load(&source).map_err(|e| e.to_string())?,
                make,
            });
            continue;
        }
        let found = all
            .iter()
            .find(|j| j.scenario.id == *want)
            .ok_or_else(|| format!("no scenario {want:?} in {from}"))?;
        picked.push(found.clone());
    }
    if args.scenarios.is_empty() {
        picked = all;
    }
    let say = |out: &mut dyn std::io::Write, line: String| {
        let _ = writeln!(out, "{line}");
    };
    if args.list {
        for j in &picked {
            let s = &j.scenario;
            say(out, format!("{}\t{}\t{}", s.id, s.title, j.path.display()));
        }
        return Ok(());
    }
    for job in &picked {
        let scenario = &job.scenario;
        let app = (job.make)();
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
