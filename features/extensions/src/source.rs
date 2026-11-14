//! A linked built-in's development copy, built from its source (#272).
//!
//! A built-in linked into the app (`crate::linked`) whose manifest says
//! where its crate is (`[source]`, [`SourceDecl`]) is cloned
//! (`EXT_CLONE`, [`Extensions::clone_to_dev`]) by writing that crate to
//! the development folder and building its process binary there
//! (`cargo build --bin …`, the binary running `Guest::extension(..).serve()`).
//! The copy keeps the built-in's id: it replaces it, so its program
//! answers the ids the built-in had. It overrides the built-in live:
//! "built in, overridden by dev (building… / failed: …)", with the way
//! back (`end_dev`, "Use the built-in").
//!
//! The crate is the folder: `Cargo.toml` (its workspace's inheritance
//! resolved, its path dependencies made absolute, the workspace's patches,
//! profiles and lockfile with it), its sources, its `extension.toml`, and
//! the program a good build put at its `[entry] path`. Every poll
//! ([`Extensions::reload_changed`]) fingerprints its sources; on a change
//! it is rebuilt on a thread of its own, and when the build is good the
//! program is swapped in (the process restarted on it). A build that fails
//! keeps the last good program running, and says why: in
//! [`Extensions::problems`] (`APP_STATUS`), in `EXT_LOGS` (source `build`)
//! and on the extensions page.
//!
//! Where the crate comes from: the app's own copy of the repository when
//! it has one ([`Extensions::with_vendored`]: a build from a checkout, or
//! sources shipped with the app), else a clone of `git` at `rev` under
//! `<dev root>/.sources`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

// `std`'s natively; a browser's clock on wasm, where `std`'s panics.
use web_time::Instant;

use architect::action::Problem;
use serde::Serialize;

use crate::catalog::{Chosen, Origin};
use crate::clone::Cloned;
use crate::extensions::Extensions;
use crate::host::{Event, LoadError};
use crate::logs::{Level, now_ms};
use crate::manifest::{MANIFEST, Manifest, SourceDecl};
use crate::package::Package;
use crate::watch;

/// Where a development copy's build is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BuildState {
    /// `cargo build` runs (since, ms since the epoch).
    Building { since_ms: u64 },
    /// The last build was good, and its program is the one that runs.
    Built { at_ms: u64, took_ms: u64 },
    /// The last build failed: its first error, and the whole output's
    /// error lines. The last good program, if any, runs on.
    Failed {
        at_ms: u64,
        error: String,
        file: Option<String>,
    },
}

/// One development copy built from source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BuildStatus {
    /// Its id (the built-in's).
    pub id: String,
    /// Its crate's folder.
    pub folder: PathBuf,
    pub state: BuildState,
    /// Whether a good build's program is there to run.
    pub good: bool,
}

impl BuildStatus {
    /// What the extensions page adds to "built in, overridden by …".
    #[must_use]
    pub fn says(&self) -> String {
        match (&self.state, self.good) {
            (BuildState::Building { .. }, true) => {
                "building… (the last good build runs)".to_owned()
            }
            (BuildState::Building { .. }, false) => {
                "building… (the built-in runs until it builds)".to_owned()
            }
            (BuildState::Built { took_ms, .. }, _) => format!("built in {took_ms} ms"),
            (BuildState::Failed { error, .. }, true) => {
                format!("failed: {error} (the last good build runs)")
            }
            (BuildState::Failed { error, .. }, false) => {
                format!("failed: {error} (the built-in runs)")
            }
        }
    }
}

/// What a build thread hands back.
struct Outcome {
    took: Duration,
    result: Result<(), Failure>,
}

#[derive(Clone)]
struct Failure {
    error: String,
    file: Option<String>,
    lines: Vec<String>,
}

struct Crate {
    id: String,
    bin: String,
    entry: String,
    print: u64,
    state: BuildState,
    good: bool,
    /// The build that runs, until it reports.
    running: Option<Receiver<Outcome>>,
    /// Its sources changed while it built: build again after.
    again: bool,
    /// When the change it builds was seen.
    seen: Instant,
}

#[derive(Default)]
struct State {
    vendored: BTreeMap<String, PathBuf>,
    target: Option<PathBuf>,
    crates: BTreeMap<PathBuf, Crate>,
}

/// The development copies built from source, and where sources are.
/// Cheap to clone.
#[derive(Clone, Default)]
pub struct Builds {
    inner: Arc<Mutex<State>>,
}

impl std::fmt::Debug for Builds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Builds").finish_non_exhaustive()
    }
}

impl Builds {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether `dir` is a crate built here.
    pub(crate) fn has(&self, dir: &Path) -> bool {
        self.lock().crates.contains_key(dir)
    }

    /// Whether `dir` is a crate with no good build yet: not to load.
    pub(crate) fn pending(&self, dir: &Path) -> bool {
        self.lock().crates.get(dir).is_some_and(|c| !c.good)
    }

    /// `dir` is no longer built (its development session ended).
    pub(crate) fn forget(&self, dir: &Path) {
        self.lock().crates.remove(dir);
    }

    fn statuses(&self) -> Vec<BuildStatus> {
        self.lock()
            .crates
            .iter()
            .map(|(folder, c)| BuildStatus {
                id: c.id.clone(),
                folder: folder.clone(),
                state: c.state.clone(),
                good: c.good,
            })
            .collect()
    }
}

impl Extensions {
    /// This app has the sources of repository `git` at `root` (a checkout
    /// it was built from, or sources shipped with it): a built-in whose
    /// `[source]` is in `git` is cloned from there, not fetched.
    #[must_use]
    pub fn with_vendored(self, git: &str, root: impl Into<PathBuf>) -> Self {
        self.builds()
            .lock()
            .vendored
            .insert(normal_git(git), root.into());
        self
    }

    /// Where development copies built from source build
    /// (`CARGO_TARGET_DIR`): `<dev root>/.target` unless given. One for
    /// all of them, so what they share is built once.
    #[must_use]
    pub fn with_build_target(self, dir: impl Into<PathBuf>) -> Self {
        self.builds().lock().target = Some(dir.into());
        self
    }

    /// Every development copy built from source, and its build.
    #[must_use]
    pub fn builds_now(&self) -> Vec<BuildStatus> {
        self.builds().statuses()
    }

    /// The build of `id`'s development copy, when it is built from source.
    #[must_use]
    pub fn build_of(&self, id: &str) -> Option<BuildStatus> {
        self.builds_now().into_iter().find(|b| b.id == id)
    }

    /// What the extensions page says of `chosen`: [`Chosen::describe`],
    /// and a development copy's build when it is built from source
    /// (`built in (0.1.0), overridden by 0.1.0 (in development): failed: …`).
    #[must_use]
    pub fn describe(&self, chosen: &Chosen<Package>) -> String {
        let said = chosen.describe();
        let build = (chosen.runs.origin == Origin::Dev)
            .then(|| chosen.runs.package.dir())
            .flatten()
            .and_then(|dir| self.builds_now().into_iter().find(|b| b.folder == dir));
        match build {
            Some(b) => format!("{said}: {}", b.says()),
            None => said,
        }
    }

    /// `from` (a built-in with a `[source]`) written to the development
    /// folder as a crate, under its own id, and built: its development
    /// copy, which overrides it once the first build is good. A folder
    /// already there (an earlier clone) is built as it is.
    pub(crate) fn clone_source(&self, from: &Manifest) -> Result<Cloned, String> {
        let source = from
            .source
            .as_ref()
            .ok_or_else(|| format!("{} says nowhere its source is ([source])", from.id))?;
        let root = self
            .dev_root()
            .ok_or("this app has no development folder for copies")?;
        let folder = root.join(&from.id);
        if !folder.join("Cargo.toml").is_file() {
            if folder.exists() {
                return Err(format!(
                    "{} exists and is not its crate: move it away",
                    folder.display()
                ));
            }
            let checkout = self.checkout(source, &root)?;
            write_crate(&checkout, source, &folder).inspect_err(|_| {
                let _ = std::fs::remove_dir_all(&folder);
            })?;
        }
        let events = self.dev_crate(&folder)?;
        Ok(Cloned {
            id: from.id.clone(),
            folder,
            events,
        })
    }

    /// Runs the crate in `dir` (an extension's, with its `extension.toml`)
    /// as a development copy built from source: built now, and again when
    /// its sources change. It overrides the other copies of its id once a
    /// build is good.
    ///
    /// # Errors
    ///
    /// Its manifest does not read, or names no program (`[entry] path`).
    pub fn dev_crate(&self, dir: &Path) -> Result<Vec<Event>, String> {
        let manifest = Package::read(dir).map_err(|e| e.to_string())?.manifest;
        if manifest.entry.path.is_empty() {
            return Err(format!(
                "{}: its manifest names no program ([entry] path)",
                manifest.id
            ));
        }
        let bin = manifest
            .source
            .as_ref()
            .map(|s| s.bin.clone())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| {
                Path::new(&manifest.entry.path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
        let good = dir.join(&manifest.entry.path).is_file();
        {
            let mut state = self.builds().lock();
            let target = self.build_target(&state);
            state.crates.insert(
                dir.to_path_buf(),
                Crate {
                    id: manifest.id.clone(),
                    bin,
                    entry: manifest.entry.path.clone(),
                    print: source_print(dir, &manifest.entry.path),
                    state: BuildState::Building { since_ms: now_ms() },
                    good,
                    running: None,
                    again: false,
                    seen: Instant::now(),
                },
            );
            if let Some(c) = state.crates.get_mut(dir) {
                start(dir, c, &target);
            }
        }
        self.log_build(
            &manifest.id,
            Level::Info,
            "building its crate (cargo build)",
        );
        Ok(self.dev(dir.to_path_buf()))
    }

    fn build_target(&self, state: &State) -> PathBuf {
        state.target.clone().unwrap_or_else(|| {
            self.dev_root()
                .unwrap_or_else(std::env::temp_dir)
                .join(".target")
        })
    }

    fn log_build(&self, id: &str, level: Level, text: impl Into<String>) {
        self.host().logs().push(id, level, "build", text);
    }

    /// Where `source`'s repository is here: the app's own copy, or a
    /// clone of it under `<root>/.sources`.
    fn checkout(&self, source: &SourceDecl, root: &Path) -> Result<PathBuf, String> {
        if let Some(dir) = self.builds().lock().vendored.get(&normal_git(&source.git)) {
            return Ok(dir.clone());
        }
        if source.git.is_empty() {
            return Err("its [source] names no repository (git)".to_owned());
        }
        let rev = if source.rev.is_empty() {
            "main"
        } else {
            &source.rev
        };
        let name: String = format!("{}@{rev}", normal_git(&source.git))
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let dir = root.join(".sources").join(name);
        if !dir.join(".git").exists() {
            run(Command::new("git")
                .args(["clone", "--quiet", &source.git])
                .arg(&dir))?;
            run(Command::new("git").arg("-C").arg(&dir).args([
                "-c",
                "advice.detachedHead=false",
                "checkout",
                "--quiet",
                rev,
            ]))?;
        }
        Ok(dir)
    }

    /// Builds what changed and swaps in what built: what happened. Called
    /// by [`Self::reload_changed`].
    pub(crate) fn rebuild_changed(&self) -> Vec<Event> {
        let mut swap = Vec::new();
        {
            let mut state = self.builds().lock();
            let target = self.build_target(&state);
            for (dir, c) in &mut state.crates {
                let print = source_print(dir, &c.entry);
                let changed = print != c.print;
                if changed {
                    c.print = print;
                    if c.running.is_some() {
                        c.again = true;
                    } else {
                        c.seen = Instant::now();
                        start(dir, c, &target);
                    }
                }
                let Some(running) = &c.running else { continue };
                let outcome = match running.try_recv() {
                    Ok(o) => o,
                    Err(TryRecvError::Empty) => continue,
                    Err(TryRecvError::Disconnected) => Outcome {
                        took: Duration::ZERO,
                        result: Err(Failure {
                            error: "the build stopped".to_owned(),
                            file: None,
                            lines: Vec::new(),
                        }),
                    },
                };
                c.running = None;
                swap.push((dir.clone(), c.id.clone(), c.seen.elapsed(), outcome));
                if std::mem::take(&mut c.again) {
                    c.seen = Instant::now();
                    start(dir, c, &target);
                }
            }
        }
        let mut events = Vec::new();
        for (dir, id, since_change, outcome) in swap {
            events.extend(self.finished(&dir, &id, since_change, outcome));
        }
        events
    }

    /// A build of `dir` reported: its program swapped in when good.
    fn finished(&self, dir: &Path, id: &str, since: Duration, outcome: Outcome) -> Vec<Event> {
        let took = u64::try_from(outcome.took.as_millis()).unwrap_or(u64::MAX);
        let set = |state: BuildState, good: bool| {
            if let Some(c) = self.builds().lock().crates.get_mut(dir) {
                // A build started since (its sources changed again) keeps
                // saying it builds.
                if c.running.is_none() {
                    c.state = state;
                }
                c.good |= good;
            }
        };
        if let Err(failure) = outcome.result {
            for line in &failure.lines {
                self.log_build(id, Level::Error, line.clone());
            }
            self.log_build(
                id,
                Level::Error,
                format!("build failed after {took} ms; the last good build runs on"),
            );
            set(
                BuildState::Failed {
                    at_ms: now_ms(),
                    error: failure.error,
                    file: failure.file,
                },
                false,
            );
            return Vec::new();
        }
        set(
            BuildState::Built {
                at_ms: now_ms(),
                took_ms: took,
            },
            true,
        );
        let loaded_here = self
            .host()
            .get(id)
            .is_some_and(|l| l.dir.as_deref() == Some(dir));
        let events = if loaded_here {
            vec![match Package::read(dir)
                .map_err(LoadError::Package)
                .and_then(|p| self.host().load(&p, Origin::Dev))
            {
                Ok(change) => Event::Loaded(change),
                Err(error) => Event::Failed {
                    id: id.to_owned(),
                    error,
                },
            }]
        } else {
            self.sync()
        };
        let swapped = since.as_millis();
        self.log_build(
            id,
            Level::Info,
            format!("built in {took} ms; swapped in {swapped} ms after the change"),
        );
        tracing::info!(
            ext.id = id,
            ext.build_ms = took,
            ext.swap_ms = u64::try_from(swapped).unwrap_or(u64::MAX),
            "ext.rebuilt"
        );
        events
    }

    /// Each failed build, as a problem.
    pub(crate) fn build_problems(&self) -> Vec<Problem> {
        self.builds_now()
            .into_iter()
            .filter_map(|b| match b.state {
                BuildState::Failed { error, file, .. } => {
                    let kept = if b.good {
                        "the last good build runs"
                    } else {
                        "the built-in runs"
                    };
                    let p = Problem::new(
                        "extension",
                        format!("{}: its build failed: {error} ({kept})", b.id),
                    );
                    Some(match file {
                        Some(f) => p.file(f),
                        None => p,
                    })
                }
                _ => None,
            })
            .collect()
    }
}

/// Starts building `c` (in `dir`) on a thread of its own.
fn start(dir: &Path, c: &mut Crate, target: &Path) {
    let (to, from) = channel();
    let (dir, target, bin, entry) = (
        dir.to_path_buf(),
        target.to_path_buf(),
        c.bin.clone(),
        c.entry.clone(),
    );
    c.state = BuildState::Building { since_ms: now_ms() };
    c.running = Some(from);
    // A browser has no threads (or cargo): the spawn fails and is logged.
    let spawned = std::thread::Builder::new()
        .name(format!("ext-build-{}", c.id))
        .spawn(move || {
            let started = Instant::now();
            let result = build(&dir, &target, &bin, &entry);
            let _ = to.send(Outcome {
                took: started.elapsed(),
                result,
            });
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "ext.build_thread");
    }
}

/// `cargo build --bin bin` in `dir`, its program put at `entry` (swapped
/// whole, so a running copy is not written into).
fn build(dir: &Path, target: &Path, bin: &str, entry: &str) -> Result<(), Failure> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let out = Command::new(cargo)
        .current_dir(dir)
        .args([
            "build",
            "--bin",
            bin,
            "--message-format",
            "short",
            "--color",
            "never",
        ])
        .env("CARGO_TARGET_DIR", target)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Failure {
            error: format!("cargo could not run: {e}"),
            file: None,
            lines: Vec::new(),
        })?;
    if !out.status.success() {
        return Err(failure(dir, &String::from_utf8_lossy(&out.stderr)));
    }
    let built = target
        .join("debug")
        .join(format!("{bin}{}", std::env::consts::EXE_SUFFIX));
    let to = dir.join(entry);
    let fail = |e: std::io::Error| Failure {
        error: format!("its program could not be put at {entry}: {e}"),
        file: None,
        lines: Vec::new(),
    };
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(fail)?;
    }
    let next = to.with_file_name(format!(
        ".{}.next",
        to.file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default()
    ));
    std::fs::copy(&built, &next).map_err(fail)?;
    std::fs::rename(&next, &to).map_err(fail)
}

/// A failed build's stderr: its first error (and file), and its error
/// lines.
fn failure(dir: &Path, stderr: &str) -> Failure {
    let lines: Vec<String> = stderr
        .lines()
        .filter(|l| l.contains("error"))
        .map(str::to_owned)
        .collect();
    let first = lines
        .iter()
        .find(|l| l.contains(": error"))
        .or_else(|| lines.first())
        .cloned()
        .unwrap_or_else(|| stderr.lines().last().unwrap_or("cargo failed").to_owned());
    // `src/lib.rs:12:5: error[E0425]: …`
    let (file, error) = match first.split_once(": error") {
        Some((at, rest)) if !at.contains(' ') => (
            at.split(':')
                .next()
                .map(|f| dir.join(f).display().to_string()),
            format!("{at}: error{rest}"),
        ),
        _ => (None, first),
    };
    Failure { error, file, lines }
}

fn run(command: &mut Command) -> Result<(), String> {
    let out = command
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// `git` without a scheme's trailing noise, for comparing.
fn normal_git(git: &str) -> String {
    git.trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_owned()
}

/// A crate's sources' fingerprint: what a build reads, not what it
/// writes (its program, its lockfile, `target/`).
fn source_print(dir: &Path, entry: &str) -> u64 {
    watch::fingerprint_except(dir, &|rel: &Path| {
        rel == Path::new(entry)
            || rel == Path::new("Cargo.lock")
            || rel.components().any(|c| {
                let c = c.as_os_str().to_string_lossy();
                c == "target" || c.starts_with('.')
            })
    })
}

/// The crate at `source.path` in `checkout`, written to `to` as a crate
/// of its own: its workspace's inheritance resolved, paths absolute, the
/// workspace's lockfile, patches, profiles and cargo config with it.
///
/// # Errors
///
/// It is not there, or does not read, or a file could not be written.
pub fn write_crate(checkout: &Path, source: &SourceDecl, to: &Path) -> Result<(), String> {
    let from = checkout.join(&source.path);
    let text = std::fs::read_to_string(from.join("Cargo.toml"))
        .map_err(|e| format!("{}: {e}", from.join("Cargo.toml").display()))?;
    let root = workspace_root(&from, checkout);
    let ws: toml::Table = match &root {
        Some(r) => toml::from_str(
            &std::fs::read_to_string(r.join("Cargo.toml")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?,
        None => toml::Table::new(),
    };
    let manifest = standalone(&text, &from, root.as_deref(), &ws)?;
    let copied = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(to)?;
        copy_tree(&from, to)?;
        std::fs::write(to.join("Cargo.toml"), manifest)?;
        if let Some(r) = &root {
            for f in ["Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml"] {
                if r.join(f).is_file() {
                    if let Some(parent) = to.join(f).parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::copy(r.join(f), to.join(f))?;
                }
            }
        }
        Ok(())
    })();
    copied.map_err(|e| format!("writing its crate to {}: {e}", to.display()))?;
    // Its manifest must still be one.
    Manifest::parse(
        &std::fs::read_to_string(to.join(MANIFEST))
            .map_err(|e| format!("its crate has no {MANIFEST}: {e}"))?,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// The workspace `from` is a member of: the nearest folder up to
/// `checkout` whose `Cargo.toml` has a `[workspace]`.
fn workspace_root(from: &Path, checkout: &Path) -> Option<PathBuf> {
    let mut at = from.parent();
    while let Some(dir) = at {
        let has = std::fs::read_to_string(dir.join("Cargo.toml"))
            .ok()
            .and_then(|t| toml::from_str::<toml::Table>(&t).ok())
            .is_some_and(|t| t.contains_key("workspace"));
        if has {
            return Some(dir.to_path_buf());
        }
        if dir == checkout {
            break;
        }
        at = dir.parent();
    }
    None
}

/// Crate manifest `text` (at `from`) as a workspace of its own.
fn standalone(
    text: &str,
    from: &Path,
    root: Option<&Path>,
    ws: &toml::Table,
) -> Result<String, String> {
    let mut table: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
    let w = ws.get("workspace").and_then(toml::Value::as_table);
    let get = |key: &str| w.and_then(|w| w.get(key)).and_then(toml::Value::as_table);
    if let Some(toml::Value::Table(package)) = table.get_mut("package") {
        for (key, value) in package.iter_mut() {
            if inherits(value) {
                if let Some(v) = get("package").and_then(|p| p.get(key)) {
                    *value = v.clone();
                }
            }
        }
    }
    let deps = get("dependencies");
    let resolve = |table: &mut toml::Table| {
        for (name, dep) in table.iter_mut() {
            *dep = resolved(name, dep, from, root, deps);
        }
    };
    for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(toml::Value::Table(t)) = table.get_mut(key) {
            resolve(t);
        }
    }
    if let Some(toml::Value::Table(targets)) = table.get_mut("target") {
        for (_, target) in targets.iter_mut() {
            for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
                if let Some(toml::Value::Table(t)) = target.get_mut(key) {
                    resolve(t);
                }
            }
        }
    }
    if table.get("lints").is_some_and(inherits) {
        match w.and_then(|w| w.get("lints")) {
            Some(lints) => {
                table.insert("lints".to_owned(), lints.clone());
            }
            None => {
                table.remove("lints");
            }
        }
    }
    table.insert(
        "workspace".to_owned(),
        toml::Value::Table(toml::Table::new()),
    );
    for key in ["patch", "profile"] {
        if let Some(v) = ws.get(key) {
            let mut v = v.clone();
            if key == "patch" {
                if let (toml::Value::Table(sources), Some(root)) = (&mut v, root) {
                    for (_, patches) in sources.iter_mut() {
                        if let toml::Value::Table(patches) = patches {
                            for (_, dep) in patches.iter_mut() {
                                absolute(dep, root);
                            }
                        }
                    }
                }
            }
            table.insert(key.to_owned(), v);
        }
    }
    let body = toml::to_string(&table).map_err(|e| e.to_string())?;
    Ok(format!(
        "# Written by EXT_CLONE from {} (#272): its workspace's inheritance resolved.\n{body}",
        from.display()
    ))
}

/// `{ workspace = true }`.
fn inherits(value: &toml::Value) -> bool {
    value
        .as_table()
        .and_then(|t| t.get("workspace"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false)
}

/// Dependency `name` as a crate of its own says it: the workspace's
/// entry under its own features and `optional`, or its own with its path
/// absolute.
fn resolved(
    name: &str,
    dep: &toml::Value,
    from: &Path,
    root: Option<&Path>,
    deps: Option<&toml::Table>,
) -> toml::Value {
    if !inherits(dep) {
        let mut dep = dep.clone();
        absolute(&mut dep, from);
        return dep;
    }
    let mut out = match deps.and_then(|d| d.get(name)) {
        Some(toml::Value::String(version)) => {
            let mut t = toml::Table::new();
            t.insert("version".to_owned(), toml::Value::String(version.clone()));
            t
        }
        Some(toml::Value::Table(t)) => t.clone(),
        _ => toml::Table::new(),
    };
    if let Some(own) = dep.as_table() {
        for (key, value) in own {
            match key.as_str() {
                "workspace" => {}
                "features" => {
                    let mut features: Vec<toml::Value> = out
                        .get("features")
                        .and_then(toml::Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    for f in value.as_array().into_iter().flatten() {
                        if !features.contains(f) {
                            features.push(f.clone());
                        }
                    }
                    out.insert("features".to_owned(), toml::Value::Array(features));
                }
                _ => {
                    out.insert(key.clone(), value.clone());
                }
            }
        }
    }
    let mut out = toml::Value::Table(out);
    if let Some(root) = root {
        absolute(&mut out, root);
    }
    out
}

/// A dependency's `path`, made absolute against `base`.
fn absolute(dep: &mut toml::Value, base: &Path) {
    if let Some(toml::Value::String(path)) = dep.as_table_mut().and_then(|t| t.get_mut("path")) {
        if Path::new(path.as_str()).is_relative() {
            let joined = base.join(path.as_str());
            *path = std::fs::canonicalize(&joined)
                .unwrap_or(joined)
                .display()
                .to_string();
        }
    }
}

/// `from`'s files into `to`: not `target/`, nor hidden ones.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        let text = name.to_string_lossy();
        if text == "target" || text.starts_with('.') {
            continue;
        }
        let (src, dst) = (entry.path(), to.join(&name));
        if entry.file_type()?.is_dir() || src.is_dir() {
            std::fs::create_dir_all(&dst)?;
            copy_tree(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{SourceDecl, failure, source_print, write_crate};
    use std::path::Path;

    #[test]
    fn a_member_crate_is_written_as_a_workspace_of_its_own() {
        let repo = tempfile::tempdir().unwrap();
        let r = repo.path();
        std::fs::create_dir_all(r.join("apps/x/src")).unwrap();
        std::fs::create_dir_all(r.join("lib")).unwrap();
        std::fs::write(
            r.join("Cargo.toml"),
            r#"[workspace]
members = ["apps/x", "lib"]
[workspace.package]
version = "0.3.0"
edition = "2024"
[workspace.dependencies]
lib = { path = "lib", features = ["a"] }
serde = "1"
[workspace.lints.rust]
unsafe_code = "forbid"
[patch.crates-io]
foo = { path = "vendor/foo" }
[profile.dev]
debug = 1
"#,
        )
        .unwrap();
        std::fs::write(r.join("Cargo.lock"), "# lock\n").unwrap();
        std::fs::write(
            r.join("apps/x/Cargo.toml"),
            r#"[package]
name = "x"
version.workspace = true
edition.workspace = true
[dependencies]
lib = { workspace = true, features = ["b"] }
serde.workspace = true
near = { path = "../../lib" }
[lints]
workspace = true
"#,
        )
        .unwrap();
        std::fs::write(r.join("apps/x/src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(
            r.join("apps/x/extension.toml"),
            "id = \"x\"\nname = \"X\"\nversion = \"0.3.0\"\nkind = \"process\"\n[entry]\npath = \"x\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(r.join("apps/x/target")).unwrap();

        let out = tempfile::tempdir().unwrap();
        let to = out.path().join("x");
        let source = SourceDecl {
            path: "apps/x".into(),
            ..SourceDecl::default()
        };
        write_crate(r, &source, &to).unwrap();
        let written: toml::Table =
            toml::from_str(&std::fs::read_to_string(to.join("Cargo.toml")).unwrap()).unwrap();
        assert_eq!(written["package"]["version"].as_str(), Some("0.3.0"));
        let lib = &written["dependencies"]["lib"];
        let lib_path = std::fs::canonicalize(r.join("lib")).unwrap();
        assert_eq!(lib["path"].as_str(), Some(lib_path.to_str().unwrap()));
        assert_eq!(lib["features"].as_array().unwrap().len(), 2);
        assert_eq!(
            written["dependencies"]["serde"]["version"].as_str(),
            Some("1")
        );
        assert_eq!(
            written["dependencies"]["near"]["path"].as_str(),
            Some(lib_path.to_str().unwrap())
        );
        assert_eq!(
            written["lints"]["rust"]["unsafe_code"].as_str(),
            Some("forbid")
        );
        assert!(written.contains_key("workspace"));
        assert!(
            written["patch"]["crates-io"]["foo"]["path"]
                .as_str()
                .unwrap()
                .starts_with(r.to_str().unwrap())
        );
        assert_eq!(written["profile"]["dev"]["debug"].as_integer(), Some(1));
        assert!(to.join("Cargo.lock").is_file());
        assert!(to.join("extension.toml").is_file());
        assert!(!to.join("target").exists(), "not its build output");

        // What a build writes is not a change to build again.
        let print = source_print(&to, "x");
        std::fs::write(to.join("x"), "a program").unwrap();
        std::fs::write(to.join("Cargo.lock"), "# pruned\n").unwrap();
        assert_eq!(source_print(&to, "x"), print);
        std::fs::write(to.join("src/main.rs"), "fn main() { }").unwrap();
        assert_ne!(source_print(&to, "x"), print);
    }

    #[test]
    fn a_failed_build_says_its_first_error_and_where() {
        let f = failure(
            Path::new("/dev/x"),
            "   Compiling x v0.1.0\nsrc/lib.rs:12:5: error[E0425]: cannot find value `y`\nerror: could not compile `x`\n",
        );
        assert_eq!(f.file.as_deref(), Some("/dev/x/src/lib.rs"));
        assert!(f.error.starts_with("src/lib.rs:12:5: error[E0425]"));
        assert_eq!(f.lines.len(), 2);
    }
}
