//! Runs the example's scripts against a fresh board, from the repo root:
//!
//! ```text
//! cargo run -p example-scripting -- run  examples/scripting/scripts/close_tag.luau '{"tag":"home"}'
//! cargo run -p example-scripting -- live examples/scripting/scripts/burndown.luau
//! cargo run -p example-scripting -- save "Tidy the board" examples/scripting/scripts/tidy.luau
//! cargo run -p example-scripting -- mcp      # an MCP server on stdio, for an agent
//! cargo run -p example-scripting -- serve    # the board on its control socket
//! cargo run -p example-scripting -- actions list --headless
//! ```
//!
//! `actions …` is the board's Action List from the command line, the same
//! as over `mcp`: from a `serve`d board, or `--headless` from a fresh one.
//!
//! Each prints the run's report (`{result, log, ran, undo}`, or the error
//! with its line), then the board and its undo steps.

use std::io::Write as _;
use std::path::Path;
use std::process::ExitCode;

use architect::action::ActionCaller;
use architect_control::cli::Cli;
use architect_control::server::Control;
use architect_scripting::Script;
use architect_scripting::luau::Report;
use example_scripting::{App, HeadlessBoard, agent};
use futures::executor::block_on;
use serde_json::Value;

const USAGE: &str = "usage:
  scripting run  <script.luau> [args as JSON]   run it once, as one undo step
  scripting live <script.luau>                  start it, and run its frames until it stops
  scripting save <name> <script.luau>           save it as an action, then run that action
  scripting mcp                                 serve the board to an agent over stdio
  scripting serve                               serve the board on its control socket
  scripting actions <list|describe|run|…>       the board's actions from the command line";

/// The most frames `live` runs.
const MAX_FRAMES: usize = 100;

type Out = std::io::StdoutLock<'static>;

/// The board's name to `--app`.
const APP: &str = "scripting-example";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("actions") {
        return Cli::new(APP)
            .headless(|| Ok(Box::new(HeadlessBoard::open()?)))
            .main(std::env::args_os().skip(1));
    }
    let mut out = std::io::stdout().lock();
    let done = App::new().and_then(|app| {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        match words.as_slice() {
            ["run", path, rest @ ..] => run(&app, &mut out, path, rest.first().copied()),
            ["live", path] => live(&app, &mut out, path),
            ["save", name, path] => save(&app, &mut out, name, path),
            ["mcp"] => architect_mcp::serve_stdio(&app.mcp(agent()))
                .map(|()| true)
                .map_err(|e| e.to_string()),
            ["serve"] => serve(&app, &mut out),
            _ => Err(USAGE.to_owned()),
        }
    });
    match done {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(message) => {
            writeln!(std::io::stderr(), "{message}").ok();
            ExitCode::from(2)
        }
    }
}

/// Serves the board on its control socket, as the agent `mcp` serves it
/// to, until the process is stopped.
fn serve(app: &App, out: &mut Out) -> Result<bool, String> {
    let serving = Control::new(APP, app.surface(agent()))
        .and_then(Control::serve)
        .map_err(|e| e.to_string())?;
    line(out, &format!("{}", serving.path().display()))?;
    out.flush().map_err(|e| e.to_string())?;
    loop {
        std::thread::park();
    }
}

/// Runs the script at `path` once, as the app (a key, ⌘K): whether it
/// finished.
fn run(app: &App, out: &mut Out, path: &str, args: Option<&str>) -> Result<bool, String> {
    let source = read(path)?;
    let args = match args {
        Some(text) => serde_json::from_str(text).map_err(|e| format!("args: {e}"))?,
        None => Value::Null,
    };
    let step = format!("Script: {}", stem(path));
    let report = block_on(
        app.scripting
            .run(&ActionCaller::app(), &source, args, &step),
    );
    show(out, app, &report)?;
    Ok(report.result.is_ok())
}

/// Starts the live script at `path` and runs its frames, each one step,
/// until it stops deferring.
fn live(app: &App, out: &mut Out, path: &str) -> Result<bool, String> {
    let source = read(path)?;
    let step = format!("Script: {}", stem(path));
    let runtime = app.runtime();
    let (first, live) = block_on(runtime.start(&ActionCaller::app(), &source, Value::Null, &step));
    print(out, &first.to_json())?;
    let Some(live) = live else {
        board(out, app)?;
        return Ok(false);
    };
    for _ in 0..MAX_FRAMES {
        if !live.running() {
            break;
        }
        let frame = block_on(live.frame());
        print(out, &frame.to_json())?;
        if frame.result.is_err() {
            board(out, app)?;
            return Ok(false);
        }
    }
    live.stop();
    board(out, app)?;
    Ok(true)
}

/// Saves the script at `path` under `name` — checked, then an action of
/// its own — and runs it the way ⌘K would: by its id, from the list.
fn save(app: &App, out: &mut Out, name: &str, path: &str) -> Result<bool, String> {
    let source = read(path)?;
    let id = app
        .scripting
        .save(Script::new(name, source).described(format!("Runs {path}.")))?;
    let info = app.list.info(&id).ok_or("saved, and not on the list")?;
    line(
        out,
        &format!(
            "saved as {id} (“{}”, in “{}”)",
            info.display_name, info.group
        ),
    )?;
    let ran = block_on(app.list.run_json_async(&id, b"")).map_err(|e| e.to_string())?;
    let report: Value = serde_json::from_slice(&ran).map_err(|e| e.to_string())?;
    print(out, &report)?;
    board(out, app)?;
    Ok(true)
}

fn show(out: &mut Out, app: &App, report: &Report) -> Result<(), String> {
    print(out, &report.to_json())?;
    board(out, app)
}

fn board(out: &mut Out, app: &App) -> Result<(), String> {
    for task in app.board.tasks() {
        let mark = if task.done { "x" } else { " " };
        line(
            out,
            &format!("[{mark}] {} {} #{}", task.id, task.title, task.tag),
        )?;
    }
    line(out, &format!("undo steps: {:?}", app.board.steps()))
}

fn print(out: &mut Out, value: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    line(out, &text)
}

fn line(out: &mut Out, text: &str) -> Result<(), String> {
    writeln!(out, "{text}").map_err(|e| e.to_string())
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}

fn stem(path: &str) -> &str {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("script")
}
