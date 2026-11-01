//! Guide media: one scenario file, three outputs
//! (`docs/content/architecture/guide-media.md`).
//!
//! A [`Scenario`] is a list of [`Step`]s over an app — its root component
//! and its Action List. Each step is `do` (an action by id, or a gesture
//! on a semantic [`Target`]), and `done` (an Action List event to wake on,
//! plus [`Check`]s: read-only actions with what their result must
//! contain). The same file is
//!
//! - **media**: run headlessly on Blitz with a virtual clock, filmed with
//!   a drawn pointer, encoded to GIF, `WebM` and MP4 and a poster PNG
//!   (feature `capture`, [`capture`]);
//! - **a test**: the same run with no frames, every `done` asserted
//!   (feature `run`, [`run::run`]);
//! - **a tour**: `architect-shell`'s tour overlay rings each target, shows
//!   the step's text and moves on when its `done` holds.
//!
//! The default build is the model and its loader ([`load`]): facet types
//! read from styx, and [`contains`], the predicates' matching — no Dioxus,
//! wasm-clean, so a browser tour reads the same file.

#[cfg(feature = "capture")]
pub mod capture;
#[cfg(feature = "capture")]
pub mod cli;
mod model;
#[cfg(feature = "run")]
pub mod run;

pub use model::{
    Capture, Check, DesktopSize, Do, Done, Edge, Modifier, Output, Scenario, Step, Target,
    ViewSize, Wake,
};

use facet_value::{DestructuredRef, Value};

/// Why a scenario file could not be used.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// A file or directory could not be read.
    #[error("{0}")]
    Read(String),
    /// It is not a scenario (styx's own message, with where).
    #[error("{0}")]
    Parse(String),
    /// It reads, but says something the runner cannot do.
    #[error("scenario {scenario}: {problem}")]
    Invalid { scenario: String, problem: String },
}

/// Reads a scenario out of styx source, and checks it can be run.
///
/// # Errors
///
/// [`LoadError::Parse`] when the source is not a scenario;
/// [`LoadError::Invalid`] when two steps share an id, a step has no id, or
/// a gesture has no target to act on.
pub fn load(source: &str) -> Result<Scenario, LoadError> {
    let scenario: Scenario =
        facet_styx::from_str(source).map_err(|e| LoadError::Parse(e.to_string()))?;
    check(&scenario).map_err(|problem| LoadError::Invalid {
        scenario: scenario.id.clone(),
        problem,
    })?;
    Ok(scenario)
}

/// Every scenario in `dir` (its `*.styx` files, by file name), with the
/// file each came from.
///
/// # Errors
///
/// [`LoadError::Read`] when the directory or a file cannot be read; the
/// first file's [`load`] error, named.
pub fn load_dir(dir: &std::path::Path) -> Result<Vec<(std::path::PathBuf, Scenario)>, LoadError> {
    let read = |e: std::io::Error| LoadError::Read(format!("{}: {e}", dir.display()));
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map_err(read)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "styx"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let source = std::fs::read_to_string(&path)
                .map_err(|e| LoadError::Read(format!("{}: {e}", path.display())))?;
            let scenario = load(&source).map_err(|e| match e {
                LoadError::Parse(m) => LoadError::Parse(format!("{}: {m}", path.display())),
                other => other,
            })?;
            Ok((path, scenario))
        })
        .collect()
}

/// What is wrong with a scenario, if anything.
fn check(s: &Scenario) -> Result<(), String> {
    if s.id.trim().is_empty() {
        return Err("it has no id".to_owned());
    }
    if s.capture.fps == 0 {
        return Err("capture.fps is 0".to_owned());
    }
    let mut seen = std::collections::BTreeSet::new();
    for step in &s.steps {
        if step.id.trim().is_empty() {
            return Err("a step has no id".to_owned());
        }
        if !seen.insert(step.id.as_str()) {
            return Err(format!("two steps are called {:?}", step.id));
        }
        let gesture = matches!(
            step.act,
            Some(Do::Click { .. } | Do::Drag { .. } | Do::Scroll { .. })
        );
        if gesture && step.target.is_none() {
            return Err(format!("step {:?}: a gesture needs a target", step.id));
        }
    }
    Ok(())
}

/// A styx value as JSON, for an action's arguments.
///
/// Styx scalars are untyped: one that spells `true`, `false` or a number
/// is sent as that (what a typed argument expects), anything else as a
/// string. A digit string an action wants as a string cannot be said.
#[must_use]
pub fn to_json(value: &Value) -> serde_json::Value {
    match value.destructure_ref() {
        DestructuredRef::Bool(b) => serde_json::Value::Bool(b),
        DestructuredRef::Number(n) => n
            .to_i64()
            .map(serde_json::Value::from)
            .or_else(|| n.to_f64().map(serde_json::Value::from))
            .unwrap_or(serde_json::Value::Null),
        DestructuredRef::String(s) => scalar(s.as_str()),
        DestructuredRef::Array(a) => {
            serde_json::Value::Array(a.as_slice().iter().map(to_json).collect())
        }
        DestructuredRef::Object(o) => serde_json::Value::Object(
            o.iter()
                .map(|(k, v)| (k.as_str().to_owned(), to_json(v)))
                .collect(),
        ),
        DestructuredRef::Char(c) => serde_json::Value::String(c.to_string()),
        // Null, and what styx never produces (bytes, dates, names, ids).
        _ => serde_json::Value::Null,
    }
}

/// A styx scalar's JSON: a bool or a number when it spells one.
fn scalar(text: &str) -> serde_json::Value {
    match text {
        "true" => return serde_json::Value::Bool(true),
        "false" => return serde_json::Value::Bool(false),
        _ => {}
    }
    if let Ok(i) = text.parse::<i64>() {
        return serde_json::Value::from(i);
    }
    if let Ok(f) = text.parse::<f64>() {
        if f.is_finite() && text.chars().any(|c| c.is_ascii_digit()) {
            return serde_json::Value::from(f);
        }
    }
    serde_json::Value::String(text.to_owned())
}

/// Whether `actual` contains `expected`: what a [`Check`]'s `has` means.
///
/// - an object contains another when it has each of its keys, each value
///   containing the other's (extra keys are fine);
/// - an array contains another of the same length whose items each
///   contain the other's, in order (order is what a tab bar is about);
/// - a scalar contains an equal one (numbers by value).
#[must_use]
pub fn contains(actual: &serde_json::Value, expected: &serde_json::Value) -> bool {
    use serde_json::Value as J;
    match (actual, expected) {
        (J::Object(a), J::Object(e)) => e
            .iter()
            .all(|(k, ev)| a.get(k).is_some_and(|av| contains(av, ev))),
        (J::Array(a), J::Array(e)) => {
            a.len() == e.len() && a.iter().zip(e).all(|(av, ev)| contains(av, ev))
        }
        (J::Number(a), J::Number(e)) => a.as_f64() == e.as_f64(),
        (a, e) => a == e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PALETTE: &str = r#"
id palette-run
title "Run a command from ⌘K"
view {width 1024, height 640}
capture {fps 10, crop @palette-row"LAB_GO_SCHEDULE", width 640, outputs (gif webm png)}
steps (
    {
        id open
        text "⌘K opens every command."
        do @run{id LAB_SHOW_COMMANDS, args {focus true, count 3}}
        done {
            on (@ran"COMMANDS_RUN" @changed"commands")
            check ({id COMMANDS_STATE, has {overlay palette}})
        }
        hold 0.5
    }
    {
        id pick
        target @palette-row"LAB_GO_SCHEDULE"
        do @click{}
        hide true
    }
    {
        id drag
        target @tab"note-welcome"
        do @drag{to @app{kind lane, name Drums}, edge right, seconds 1.2}
    }
    {
        id pull
        target @split"note-welcome"
        do @drag{dx 120}
    }
    {
        id keys
        do @key{key Enter, mods (meta shift)}
    }
)
"#;

    #[test]
    fn a_scenario_reads_from_styx() {
        let s = load(PALETTE).unwrap();
        assert_eq!(s.id, "palette-run");
        assert_eq!(s.view.width, 1024);
        assert!((s.view.scale - 1.0).abs() < f32::EPSILON);
        assert_eq!(s.capture.fps, 10);
        assert_eq!(
            s.capture.crop,
            Some(Target::PaletteRow("LAB_GO_SCHEDULE".into()))
        );
        assert_eq!(
            s.capture.outputs,
            vec![Output::Gif, Output::Webm, Output::Png]
        );
        assert!(s.capture.cursor);
        assert_eq!(s.steps.len(), 5);
        let open = &s.steps[0];
        let Some(Do::Run { id, args }) = &open.act else {
            panic!("{:?}", open.act);
        };
        assert_eq!(id, "LAB_SHOW_COMMANDS");
        assert_eq!(
            to_json(args.as_ref().unwrap()),
            json!({"focus": true, "count": 3})
        );
        let done = open.done.as_ref().unwrap();
        assert_eq!(
            done.on,
            vec![
                Wake::Ran("COMMANDS_RUN".into()),
                Wake::Changed("commands".into())
            ]
        );
        assert_eq!(to_json(&done.check[0].has), json!({"overlay": "palette"}));
        assert!((done.timeout - 5.0).abs() < f32::EPSILON);
        assert!(s.steps[1].hide);
        assert_eq!(s.steps[1].act, Some(Do::Click { seconds: 0.4 }));
        assert_eq!(
            s.steps[2].act,
            Some(Do::Drag {
                to: Some(Target::App {
                    kind: "lane".into(),
                    name: "Drums".into()
                }),
                edge: Some(Edge::Right),
                dx: 0.0,
                dy: 0.0,
                seconds: 1.2
            })
        );
        assert_eq!(
            s.steps[3].act,
            Some(Do::Drag {
                to: None,
                edge: None,
                dx: 120.0,
                dy: 0.0,
                seconds: 0.8
            })
        );
        assert_eq!(
            s.steps[4].act,
            Some(Do::Key {
                key: "Enter".into(),
                mods: vec![Modifier::Meta, Modifier::Shift]
            })
        );
    }

    #[test]
    fn a_target_by_its_text_reads_from_styx() {
        let s = load("id x\nsteps ({id a, target @text\"View\", do @click{}})").unwrap();
        assert_eq!(s.steps[0].target, Some(Target::Text("View".into())));
        assert_eq!(
            s.steps[0].target.as_ref().and_then(Target::kind_and_name),
            Some(("text", "View"))
        );
    }

    #[test]
    fn a_gesture_without_a_target_is_refused() {
        let err = load("id x\nsteps ({id a, do @click{}})").unwrap_err();
        assert!(err.to_string().contains("needs a target"), "{err}");
    }

    #[test]
    fn two_steps_with_one_id_are_refused() {
        let err = load("id x\nsteps ({id a} {id a})").unwrap_err();
        assert!(err.to_string().contains("two steps"), "{err}");
    }

    #[test]
    fn contains_is_a_subset_of_objects_and_an_ordered_match_of_arrays() {
        let actual = json!({"tabs": ["a", "b"], "active": "a", "n": 2.0, "x": {"y": 1, "z": 2}});
        assert!(contains(&actual, &json!({"active": "a"})));
        assert!(contains(&actual, &json!({"n": 2, "x": {"z": 2}})));
        assert!(contains(&actual, &json!({"tabs": ["a", "b"]})));
        assert!(!contains(&actual, &json!({"tabs": ["b", "a"]})));
        assert!(!contains(&actual, &json!({"tabs": ["a"]})));
        assert!(!contains(&actual, &json!({"missing": 1})));
    }
}
