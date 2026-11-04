//! `EXT_CHECK`: load an extension from its folder once, on its own, and
//! render each of its views once — `ok`, or the exact error (#268 H5).
//!
//! herdr's `check <mod>` is the dev loop's gate: an agent never calls a
//! mod done without a passing check. Here a [`Checker`] (from the app's
//! host, [`crate::host::Host::checker`], so it knows the same view kinds)
//! takes a folder through every stage a load does, apart from the app:
//!
//! 1. **manifest**: `extension.toml` read and checked; a refusal names the
//!    field (`actions[LAB_X].effect`) and, for TOML, the line;
//! 2. **script**: every Luau file it names (actions' and overlays')
//!    compiled, a syntax error with its file and line;
//! 3. **load**: started in a host of its own (a list, settings and, for a
//!    process extension, a runner of its own: the app's running copy is
//!    not touched), its actions made, its view kinds resolved;
//! 4. **view**: each view rendered once. A process extension's Dioxus
//!    view (feature `remote`) is opened, its first frame applied to a
//!    Blitz document and laid out; a view of a kind the app's provider
//!    draws is checked to have one, and said not to be rendered here.
//!
//! Then it is unloaded and its process stopped.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use architect::action::ActionList;
use architect_settings::Section;
use architect_workspace::ViewKinds;
use facet::Facet;

use crate::Origin;
use crate::host::{Host, LoadError, SectionSink};
use crate::manifest::{Kind, MANIFEST, Manifest, ManifestError};
use crate::package::{Package, PackageError};

/// One thing a check found wrong.
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
pub struct CheckProblem {
    /// Which stage: `manifest`, `files`, `script`, `load` or `view`.
    pub stage: String,
    /// The file it is in, relative to the folder, when it is in one.
    #[facet(default)]
    pub file: Option<String>,
    /// The manifest field (`actions[LAB_X].effect`, `views[counter].kind`).
    #[facet(default)]
    pub field: Option<String>,
    /// The line, when it is known.
    #[facet(default)]
    pub line: Option<u32>,
    pub message: String,
}

/// One view, as the check rendered it.
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
pub struct ViewCheck {
    pub id: String,
    pub kind: String,
    /// It was drawn here, once.
    pub rendered: bool,
    /// How many elements its first frame made, when it was.
    #[facet(default)]
    pub elements: Option<u32>,
    /// Why it was not drawn here, when it was not and that is no error.
    #[facet(default)]
    pub note: Option<String>,
}

/// What `EXT_CHECK` answers.
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
pub struct CheckReport {
    /// Nothing was wrong.
    pub ok: bool,
    /// The folder checked.
    pub folder: String,
    #[facet(default)]
    pub id: Option<String>,
    #[facet(default)]
    pub version: Option<String>,
    #[facet(default)]
    pub kind: Option<String>,
    /// The actions it would add.
    pub actions: Vec<String>,
    pub views: Vec<ViewCheck>,
    pub problems: Vec<CheckProblem>,
}

impl CheckReport {
    fn new(folder: &Path) -> Self {
        Self {
            ok: false,
            folder: folder.display().to_string(),
            id: None,
            version: None,
            kind: None,
            actions: Vec::new(),
            views: Vec::new(),
            problems: Vec::new(),
        }
    }

    fn problem(&mut self, stage: &str, message: impl Into<String>) -> &mut CheckProblem {
        let problem = CheckProblem {
            stage: stage.to_owned(),
            file: None,
            field: None,
            line: None,
            message: message.into(),
        };
        self.problems.push_mut(problem)
    }

    /// The first problem, one line: what a command line prints.
    #[must_use]
    pub fn summary(&self) -> String {
        self.problems.first().map_or_else(
            || "ok".to_owned(),
            |p| {
                let mut at = p.file.clone().unwrap_or_default();
                if let Some(line) = p.line {
                    let _ = write!(at, ":{line}");
                }
                if let Some(field) = &p.field {
                    if !at.is_empty() {
                        at.push(' ');
                    }
                    at.push_str(field);
                }
                if at.is_empty() {
                    format!("{}: {}", p.stage, p.message)
                } else {
                    format!("{} ({at}): {}", p.stage, p.message)
                }
            },
        )
    }
}

/// Settings that go nowhere: a checked extension's sections.
struct Discard;

impl SectionSink for Discard {
    fn put(&self, _: &str, _: Vec<Section>) -> Result<(), String> {
        Ok(())
    }

    fn withdraw(&self, _: &str) {}
}

/// Checks extension folders as an app would load them.
#[derive(Clone, Debug)]
pub struct Checker {
    kinds: ViewKinds,
    processes: bool,
    /// How long a view has to send its first frame.
    pub frame_within: Duration,
}

impl Default for Checker {
    fn default() -> Self {
        Self::new()
    }
}

impl Checker {
    /// A checker knowing only the built-in view kind, running process
    /// extensions where this build can (feature `process`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            kinds: ViewKinds::new(),
            processes: cfg!(all(feature = "process", unix)),
            frame_within: Duration::from_secs(10),
        }
    }

    /// The view kinds the app provides.
    #[must_use]
    pub fn with_view_kinds(mut self, kinds: ViewKinds) -> Self {
        self.kinds = kinds;
        self
    }

    /// Whether it starts process extensions (the app's host does).
    #[must_use]
    pub const fn with_processes(mut self, on: bool) -> Self {
        self.processes = on && cfg!(all(feature = "process", unix));
        self
    }

    /// Checks the extension in `folder`.
    #[must_use]
    pub fn check(&self, folder: &Path) -> CheckReport {
        let mut report = CheckReport::new(folder);
        let span = tracing::info_span!(
            "ext.check",
            ext.folder = %folder.display(),
            ext.ok = tracing::field::Empty,
        );
        let _entered = span.enter();
        let Some(package) = read(folder, &mut report) else {
            span.record("ext.ok", false);
            return report;
        };
        let manifest = &package.manifest;
        report.id = Some(manifest.id.clone());
        report.version = Some(manifest.version.to_string());
        report.kind = Some(manifest.kind.name().to_owned());
        report.actions = manifest.actions.iter().map(|a| a.id.clone()).collect();
        scripts(&package, &mut report);
        if report.problems.is_empty() {
            self.load_and_render(&package, &mut report);
        }
        report.ok = report.problems.is_empty();
        span.record("ext.ok", report.ok);
        report
    }

    fn load_and_render(&self, package: &Package, report: &mut CheckReport) {
        let manifest = &package.manifest;
        let host = Host::new(ActionList::new(), Discard).with_view_kinds(self.kinds.clone());
        #[cfg(all(feature = "process", unix))]
        let processes = self.process_runner(&host, manifest.kind, report);
        if let Err(error) = host.load(package, Origin::Dev) {
            load_problem(&error, report);
            return;
        }
        for view in &manifest.views {
            let mut check = ViewCheck {
                id: view.id.clone(),
                kind: view.kind.clone(),
                rendered: false,
                elements: None,
                note: None,
            };
            if view.kind == architect_workspace::DIOXUS && manifest.kind == Kind::Process {
                #[cfg(all(feature = "remote", unix))]
                match processes
                    .as_ref()
                    .map(|p| self.render_remote(p, &manifest.id, &view.id))
                {
                    Some(Ok(elements)) => {
                        check.rendered = true;
                        check.elements = Some(elements);
                    }
                    Some(Err(why)) => {
                        report.problem("view", why).field = Some(format!("views[{}]", view.id));
                    }
                    None => check.note = Some("this build runs no process extensions".to_owned()),
                }
                #[cfg(not(all(feature = "remote", unix)))]
                {
                    check.note =
                        Some("this build renders no process views (feature `remote`)".to_owned());
                }
            } else {
                check.note = Some(format!(
                    "drawn by the app's `{}` provider: its kind is there, it is not rendered here",
                    view.kind
                ));
            }
            report.views.push(check);
        }
        let _ = host.unload(&manifest.id);
    }

    /// A runner of the check's own for a process extension, its processes
    /// to open views from.
    #[cfg(all(feature = "process", unix))]
    fn process_runner(
        &self,
        host: &Host,
        kind: Kind,
        report: &mut CheckReport,
    ) -> Option<crate::process::Processes> {
        if kind != Kind::Process || !self.processes {
            return None;
        }
        match crate::process::ProcessRunner::new() {
            Ok(runner) => {
                let runner = runner.with_restart(crate::process::Restart::never());
                let processes = runner.processes();
                host.add_runner(runner);
                Some(processes)
            }
            Err(e) => {
                report.problem("load", format!("no runner for its process: {e}"));
                None
            }
        }
    }

    /// Opens view `view` of process extension `id` and applies its first
    /// frame: how many elements it made.
    #[cfg(all(feature = "remote", unix))]
    fn render_remote(
        &self,
        processes: &crate::process::Processes,
        id: &str,
        view: &str,
    ) -> Result<u32, String> {
        use crate::process::State;
        let deadline = std::time::Instant::now()
            .checked_add(self.frame_within)
            .unwrap_or_else(std::time::Instant::now);
        let process = processes.get(id).ok_or("its process is not running")?;
        loop {
            match process.state() {
                State::Running { .. } => break,
                State::Exited { why, .. } => return Err(format!("its process exited: {why}")),
                State::Stopped => return Err("its process stopped".to_owned()),
                State::Starting if std::time::Instant::now() > deadline => {
                    return Err("its process did not start in time".to_owned());
                }
                State::Starting => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        let mut session = process.open(view)?;
        let frame = loop {
            match session.frames.try_recv() {
                Ok(frame) => break frame,
                Err(futures::channel::mpsc::TryRecvError::Closed) => {
                    return Err(format!("view {view} ended before it drew anything"));
                }
                Err(_) if std::time::Instant::now() > deadline => {
                    return Err(format!(
                        "view {view} sent no frame within {:?}",
                        self.frame_within
                    ));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        };
        let (events, _) = futures::channel::mpsc::unbounded();
        let mut doc = crate::remote::RemoteDocument::new(events);
        doc.apply(frame)
            .map_err(|why| format!("view {view} did not render: {why}"))?;
        let elements = crate::remote::element_count(doc.document());
        if elements == 0 {
            return Err(format!("view {view} drew nothing"));
        }
        Ok(elements)
    }
}

/// The package in `folder`, or why not.
fn read(folder: &Path, report: &mut CheckReport) -> Option<Package> {
    match Package::read(folder) {
        Ok(package) => Some(package),
        Err(PackageError::Read { path, why }) => {
            let file = relative(folder, &path);
            let message = format!("{file} cannot be read: {why}");
            report.problem("files", message).file = Some(file);
            None
        }
        Err(PackageError::Manifest(error)) => {
            manifest_problem(&error, report);
            None
        }
    }
}

/// `path` relative to `folder` when it is inside it.
fn relative(folder: &Path, path: &str) -> String {
    Path::new(path)
        .strip_prefix(folder)
        .map_or_else(|_| path.to_owned(), |p| p.display().to_string())
}

/// A refused manifest, with the field it is about.
fn manifest_problem(error: &ManifestError, report: &mut CheckReport) {
    let (field, line, message) = manifest_fields(error);
    let p = report.problem("manifest", message);
    p.file = Some(MANIFEST.to_owned());
    p.field = field;
    p.line = line;
}

/// What a refused manifest is about: the field, the line (TOML's), and
/// the message on one line.
pub(crate) fn manifest_fields(error: &ManifestError) -> (Option<String>, Option<u32>, String) {
    let (field, line) = match error {
        ManifestError::Parse(text) => (toml_field(text), toml_line(text)),
        ManifestError::BadId(_) => (Some("id".to_owned()), None),
        ManifestError::Api { .. } => (Some("api".to_owned()), None),
        ManifestError::ActionNamespace { action, .. } => {
            (Some(format!("actions[{action}].id")), None)
        }
        ManifestError::SettingNamespace(key) => {
            (Some(format!("sections.settings[{key}].key")), None)
        }
        ManifestError::NoRun(id) | ManifestError::ScriptKind(id) => {
            (Some(format!("actions[{id}].script")), None)
        }
        ManifestError::BadPath(path) => (Some(path.clone()), None),
        ManifestError::BadSetting { key, .. } => (Some(format!("sections.settings[{key}]")), None),
        ManifestError::Duplicate(id) => (Some(id.clone()), None),
        ManifestError::BadPlane { overlay, .. } => {
            (Some(format!("overlays[{overlay}].plane")), None)
        }
        ManifestError::BadToken { token, .. } => (Some(format!("tokens[{token}]")), None),
        // `opener kf`, `hook startup`: what the contribution was.
        ManifestError::Contribution(refused) => (Some(refused.what.clone()), None),
    };
    let message = match error {
        // TOML's own text carries a drawing of the line; the message is
        // its last line.
        ManifestError::Parse(text) => text
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('|'))
            .unwrap_or(text)
            .trim()
            .to_owned(),
        other => other.to_string(),
    };
    (field, line, message)
}

/// `TOML parse error at line 12, column 10` → 12.
fn toml_line(text: &str) -> Option<u32> {
    let rest = text.split("at line ").nth(1)?;
    rest.split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

/// The field a TOML error names: `unknown field `x``, `missing field `y``,
/// or the key on the line it drew.
fn toml_field(text: &str) -> Option<String> {
    for lead in ["unknown field `", "missing field `", "unknown variant `"] {
        if let Some(rest) = text.split(lead).nth(1) {
            if lead == "unknown variant `" {
                break;
            }
            return rest.split('`').next().map(str::to_owned);
        }
    }
    // The drawn line: `12 | effect = "sometimes"`.
    text.lines()
        .find_map(|l| l.split_once(" | ").map(|(_, code)| code))
        .and_then(|code| code.split_once('=').map(|(key, _)| key.trim().to_owned()))
        .filter(|key| !key.is_empty())
}

/// Every Luau file the manifest names, compiled.
fn scripts(package: &Package, report: &mut CheckReport) {
    let manifest: &Manifest = &package.manifest;
    let named = manifest
        .actions
        .iter()
        .filter_map(|a| {
            a.script
                .clone()
                .map(|s| (s, format!("actions[{}].script", a.id)))
        })
        .chain(
            manifest
                .overlays
                .iter()
                .map(|o| (o.script.clone(), format!("overlays[{}].script", o.id))),
        );
    for (path, field) in named {
        match package.text(&path) {
            Ok(source) => {
                if let Err(e) = architect_scripting::luau::Runtime::check(&source) {
                    let p = report.problem("script", e.message.clone());
                    p.file = Some(path);
                    p.line = e.line;
                }
            }
            Err(e) => {
                let p = report.problem("files", e.to_string());
                p.file = Some(path);
                p.field = Some(field);
            }
        }
    }
}

/// A load that failed, with the field it is about.
fn load_problem(error: &LoadError, report: &mut CheckReport) {
    let field = match error {
        LoadError::Action { id, .. } | LoadError::ActionTaken(id) => Some(format!("actions[{id}]")),
        LoadError::ViewKind { view, .. } => Some(format!("views[{view}].kind")),
        LoadError::Sections(_) => Some("sections".to_owned()),
        LoadError::NoRunner(_) => Some("kind".to_owned()),
        LoadError::Start(_) => Some("entry".to_owned()),
        LoadError::Package(_) => None,
    };
    report.problem("load", error.to_string()).field = field;
}

/// The folder a check names: absolute, as the report says it.
#[must_use]
pub fn folder(path: &str) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| PathBuf::from(path))
}
