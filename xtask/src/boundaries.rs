//! `cargo xtask boundaries` — the crate-layout ADR, enforced.
//!
//! `docs/content/architecture/layout.md` says a feature is one crate whose
//! roles are cargo features: the default set is the L0 contract and stays
//! wasm-clean, `ui` is L4 and never reaches an engine, storage or host,
//! and only `apps/*` turns a transport host on. Nothing in cargo stops a
//! one-line `features = ["server"]` from breaking all three, so this does.
//!
//! The layer table lives in `boundaries.toml` at the repo root. Each row
//! names a package and the feature set to resolve it with, so a layer can
//! be a whole crate (an unfolded `-proto` sibling) or a cargo feature of
//! one (`crdt` with `ui`). Every row is resolved on its own with
//! `cargo tree -p <pkg>` (cargo's per-package feature resolution, which a
//! workspace-wide `cargo metadata` resolve cannot give), and the rules
//! below run over those graphs.
//!
//! The rules are pure functions over [`Facts`], so the tests seed one
//! violation per rule without invoking cargo.
//!
//! Counts that a repo cannot fix in one PR are ratcheted: `[baseline]`
//! holds the allowed count per rule, a count above it fails, and a count
//! below it fails too until the baseline is lowered, so the number only
//! ever goes down.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

/// The layers of the ADR's "Roles are cargo features" table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
pub enum Layer {
    /// Contract: entities, wire types, service traits. Wasm-clean.
    L0,
    /// Engine.
    L1,
    /// Storage.
    L2,
    /// Host / transport glue.
    L3,
    /// UI: may reach only L0 and `architect[client]`.
    L4,
}

/// One row of the layer table: a package resolved with a feature set.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    /// The cargo package name.
    pub package: String,
    /// Extra features to enable on top of (or instead of) the defaults.
    #[serde(default)]
    pub features: Vec<String>,
    /// Resolve with `--no-default-features`.
    #[serde(default, rename = "no-default-features")]
    pub no_default_features: bool,
    /// The layer this package + feature set belongs to.
    pub layer: Layer,
    /// Why the row exists, for sibling crates awaiting fold-in.
    #[serde(default)]
    pub note: Option<String>,
}

impl Row {
    fn label(&self) -> String {
        let mut label = self.package.clone();
        if self.no_default_features {
            label.push_str(" --no-default-features");
        }
        if !self.features.is_empty() {
            label.push_str(" --features ");
            label.push_str(&self.features.join(","));
        }
        label
    }
}

/// What the L0 / L4 rows must not reach, and what makes a transport host.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Forbidden {
    /// Packages that are L1-L3 (engine, storage, host): `sea-orm`, `axum`, ...
    #[serde(default)]
    pub engine: Vec<String>,
    /// Packages an L0 contract must not reach on top of `engine`: `dioxus`, ...
    #[serde(default)]
    pub ui: Vec<String>,
    /// `tokio` features that mean a runtime (`rt`, `rt-multi-thread`).
    #[serde(default, rename = "tokio-runtime")]
    pub tokio_runtime: Vec<String>,
    /// `architect` features that turn a crate into a transport host.
    #[serde(default, rename = "transport-host")]
    pub transport_host: Vec<String>,
    /// `architect` features an L4 row must not enable (server-side ones).
    #[serde(default, rename = "architect-server")]
    pub architect_server: Vec<String>,
}

/// `boundaries.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Table {
    /// Target the L0 rows are resolved for (the wasm-clean check).
    #[serde(default = "default_contract_target", rename = "contract-target")]
    pub contract_target: String,
    /// Workspace-relative directories whose members may be transport hosts.
    #[serde(default = "default_host_dirs", rename = "host-dirs")]
    pub host_dirs: Vec<String>,
    /// Workspace-relative directories exempt from the "every feature crate
    /// has a row" completeness check (proc-macros, test crates, tooling).
    #[serde(default, rename = "unmapped-ok")]
    pub unmapped_ok: Vec<String>,
    /// Package names whose lockfile sources must agree (one tag per lock).
    #[serde(default, rename = "single-source")]
    pub single_source: Vec<String>,
    /// The forbidden sets.
    #[serde(default)]
    pub forbidden: Forbidden,
    /// Allowed violation counts per rule; absent means zero.
    #[serde(default)]
    pub baseline: BTreeMap<String, usize>,
    /// The layer table.
    #[serde(default, rename = "row")]
    pub rows: Vec<Row>,
    /// Source idioms to count and ratchet (hand routes, bespoke tools, ...).
    #[serde(default, rename = "idiom")]
    pub idioms: Vec<Idiom>,
}

/// A source idiom the repo is moving away from, counted line by line.
///
/// Rule 1 of the architect way says a capability is described once, by a
/// trait; every hand-written `.route(` is a second description. The count
/// can't go to zero in one PR, so it is ratcheted like the layer rules.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Idiom {
    /// Baseline key, e.g. `hand-routes`.
    pub name: String,
    /// Substring that marks one occurrence on a line.
    pub pattern: String,
    /// Workspace-relative directories to scan for `*.rs`.
    #[serde(default = "default_idiom_dirs")]
    pub dirs: Vec<String>,
    /// Workspace-relative paths (files or directories) that are exempt.
    #[serde(default)]
    pub allow: Vec<String>,
    /// What to do instead, printed with each hit.
    #[serde(default)]
    pub instead: Option<String>,
}

fn default_idiom_dirs() -> Vec<String> {
    vec![
        "apps".to_owned(),
        "crates".to_owned(),
        "features".to_owned(),
    ]
}

fn default_contract_target() -> String {
    "wasm32-unknown-unknown".to_owned()
}

fn default_host_dirs() -> Vec<String> {
    vec!["apps".to_owned()]
}

/// The rules. `id()` is what `[baseline]` and the report use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rule {
    /// An L0 row reaches an engine/storage/host/UI package or a tokio runtime.
    ContractClean,
    /// An L4 row reaches an engine/storage/host package or a server-side
    /// `architect` feature.
    UiReach,
    /// A member outside the host dirs enables a transport host by default.
    TransportHost,
    /// A member lacks `[lints] workspace = true`.
    Lints,
    /// The lockfile has more than one source for a single-source package.
    SingleSource,
    /// A feature crate has no row in the layer table.
    Unmapped,
    /// A counted source idiom; ratcheted per `[[idiom]]` name, not here.
    Idiom,
}

impl Rule {
    /// Every rule, in report order.
    pub const ALL: [Self; 6] = [
        Self::ContractClean,
        Self::UiReach,
        Self::TransportHost,
        Self::Lints,
        Self::SingleSource,
        Self::Unmapped,
    ];

    /// The stable id used in `[baseline]`.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::ContractClean => "contract-clean",
            Self::UiReach => "ui-reach",
            Self::TransportHost => "transport-host",
            Self::Lints => "lints",
            Self::SingleSource => "single-source",
            Self::Unmapped => "unmapped",
            Self::Idiom => "idiom",
        }
    }
}

/// One broken rule.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Violation {
    /// Which rule.
    pub rule: Rule,
    /// What broke it, for a human.
    pub detail: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.rule.id(), self.detail)
    }
}

/// A resolved package: its name and the features cargo turned on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// Package name.
    pub name: String,
    /// Enabled features.
    pub features: BTreeSet<String>,
}

/// A workspace member as the rules see it.
#[derive(Debug, Clone)]
pub struct Member {
    /// Package name.
    pub name: String,
    /// Manifest directory, relative to the workspace root, `/`-separated.
    pub dir: String,
    /// Whether the manifest has `[lints] workspace = true`.
    pub workspace_lints: bool,
    /// Whether it is a proc-macro crate.
    pub proc_macro: bool,
}

/// Everything the rules look at, gathered once.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    /// Workspace members.
    pub members: Vec<Member>,
    /// The resolved graph of each table row, by row index.
    pub row_graphs: Vec<Vec<Resolved>>,
    /// The resolved default graph of each member outside the host dirs.
    pub default_graphs: BTreeMap<String, Vec<Resolved>>,
    /// `(package, source)` pairs from `Cargo.lock`.
    pub lock_sources: Vec<(String, String)>,
    /// `path:line` hits per idiom name.
    pub idiom_hits: BTreeMap<String, Vec<String>>,
}

impl Violation {
    /// The `[baseline]` key: the rule id, or the idiom's name.
    #[must_use]
    pub fn key(&self) -> &str {
        match self.rule {
            Rule::Idiom => self.detail.split(": ").next().unwrap_or(""),
            rule => rule.id(),
        }
    }
}

/// 1-based numbers of the lines in `text` containing `pattern`, skipping
/// `//` comment lines.
fn idiom_lines(text: &str, pattern: &str) -> Vec<usize> {
    text.lines()
        .zip(1_usize..)
        .filter(|(line, _)| !line.trim_start().starts_with("//") && line.contains(pattern))
        .map(|(_, n)| n)
        .collect()
}

fn reached<'a>(graph: &'a [Resolved], names: &'a [String]) -> impl Iterator<Item = &'a Resolved> {
    graph.iter().filter(move |r| names.contains(&r.name))
}

fn features_hit<'a>(
    graph: &'a [Resolved],
    package: &'a str,
    features: &'a [String],
) -> impl Iterator<Item = String> + 'a {
    graph
        .iter()
        .filter(move |r| r.name == package)
        .flat_map(move |r| {
            features
                .iter()
                .filter(|f| r.features.contains(*f))
                .map(move |f| format!("{package}/{f}"))
        })
}

fn in_dirs(dir: &str, dirs: &[String]) -> bool {
    dirs.iter().any(|d| {
        let d = d.trim_end_matches('/');
        dir == d || dir.starts_with(&format!("{d}/"))
    })
}

fn sorted_unique(mut found: Vec<String>) -> Vec<String> {
    found.sort();
    found.dedup();
    found
}

/// Run every rule over `facts`.
#[must_use]
pub fn check(table: &Table, facts: &Facts) -> Vec<Violation> {
    let mut out = Vec::new();
    let forbidden = &table.forbidden;

    for (row, graph) in table.rows.iter().zip(&facts.row_graphs) {
        let mut found: Vec<String> = reached(graph, &forbidden.engine)
            .map(|r| r.name.clone())
            .collect();
        match row.layer {
            Layer::L0 => {
                found.extend(reached(graph, &forbidden.ui).map(|r| r.name.clone()));
                found.extend(features_hit(graph, "tokio", &forbidden.tokio_runtime));
                found.extend(features_hit(graph, "architect", &forbidden.transport_host));
                found.extend(features_hit(
                    graph,
                    "architect",
                    &forbidden.architect_server,
                ));
                for name in sorted_unique(found) {
                    out.push(Violation {
                        rule: Rule::ContractClean,
                        detail: format!(
                            "{} (L0, {}) reaches {name}",
                            row.label(),
                            table.contract_target
                        ),
                    });
                }
            }
            Layer::L4 => {
                found.extend(features_hit(graph, "architect", &forbidden.transport_host));
                found.extend(features_hit(
                    graph,
                    "architect",
                    &forbidden.architect_server,
                ));
                for name in sorted_unique(found) {
                    out.push(Violation {
                        rule: Rule::UiReach,
                        detail: format!("{} (L4) reaches {name}", row.label()),
                    });
                }
            }
            Layer::L1 | Layer::L2 | Layer::L3 => {}
        }
    }

    for (package, graph) in &facts.default_graphs {
        let hosts =
            sorted_unique(features_hit(graph, "architect", &forbidden.transport_host).collect());
        for feature in hosts {
            out.push(Violation {
                rule: Rule::TransportHost,
                detail: format!(
                    "{package} is outside {} and enables {feature} by default",
                    table.host_dirs.join(", ")
                ),
            });
        }
    }

    for member in &facts.members {
        if !member.workspace_lints {
            out.push(Violation {
                rule: Rule::Lints,
                detail: format!(
                    "{} ({}) has no `[lints] workspace = true`",
                    member.name, member.dir
                ),
            });
        }
    }

    let mut sources: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (name, source) in &facts.lock_sources {
        if table
            .single_source
            .iter()
            .any(|pattern| name_matches(pattern, name))
        {
            sources
                .entry(name.as_str())
                .or_default()
                .insert(source_without_commit(source));
        }
    }
    let distinct: BTreeSet<&str> = sources.values().flatten().copied().collect();
    if distinct.len() > 1 {
        out.push(Violation {
            rule: Rule::SingleSource,
            detail: format!(
                "Cargo.lock has {} sources for architect crates: {}",
                distinct.len(),
                distinct.into_iter().collect::<Vec<_>>().join(" | ")
            ),
        });
    }

    let mapped: BTreeSet<&str> = table.rows.iter().map(|r| r.package.as_str()).collect();
    for member in &facts.members {
        let candidate = member.dir.starts_with("features/") && !member.proc_macro;
        if candidate
            && !mapped.contains(member.name.as_str())
            && !in_dirs(&member.dir, &table.unmapped_ok)
        {
            out.push(Violation {
                rule: Rule::Unmapped,
                detail: format!(
                    "{} ({}) has no row in boundaries.toml",
                    member.name, member.dir
                ),
            });
        }
    }

    for idiom in &table.idioms {
        for at in facts.idiom_hits.get(&idiom.name).into_iter().flatten() {
            let instead = idiom
                .instead
                .as_deref()
                .map(|i| format!(" (instead: {i})"))
                .unwrap_or_default();
            out.push(Violation {
                rule: Rule::Idiom,
                detail: format!("{}: {at}{instead}", idiom.name),
            });
        }
    }

    out.sort();
    out
}

/// `architect-*` matches `architect-ui`; anything else matches itself.
fn name_matches(pattern: &str, name: &str) -> bool {
    pattern
        .strip_suffix('*')
        .map_or(pattern == name, |prefix| name.starts_with(prefix))
}

/// `git+https://…?tag=v1#abc` and `git+https://…?tag=v1#def` are the same
/// pin; only the part before `#` names what the manifest asked for.
fn source_without_commit(source: &str) -> &str {
    source.split('#').next().unwrap_or(source)
}

/// How a rule's count compares with its baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ratchet {
    /// At the baseline.
    Held,
    /// Above it: new violations.
    Worse { allowed: usize, found: usize },
    /// Below it: lower the baseline to lock the gain in.
    Better { allowed: usize, found: usize },
}

/// Every `[baseline]` key the table can use: the rule ids, then the idioms.
#[must_use]
pub fn keys(table: &Table) -> Vec<String> {
    Rule::ALL
        .iter()
        .map(|r| r.id().to_owned())
        .chain(table.idioms.iter().map(|i| i.name.clone()))
        .collect()
}

/// Compare per-key counts with `[baseline]`.
#[must_use]
pub fn ratchet(table: &Table, violations: &[Violation]) -> Vec<(String, Ratchet)> {
    keys(table)
        .into_iter()
        .map(|key| {
            let found = violations.iter().filter(|v| v.key() == key).count();
            let allowed = table.baseline.get(&key).copied().unwrap_or(0);
            let verdict = match found.cmp(&allowed) {
                std::cmp::Ordering::Equal => Ratchet::Held,
                std::cmp::Ordering::Greater => Ratchet::Worse { allowed, found },
                std::cmp::Ordering::Less => Ratchet::Better { allowed, found },
            };
            (key, verdict)
        })
        .collect()
}

/// Parse one line of `cargo tree --prefix none -f '{p}|{f}'`.
///
/// `{p}` is `name vX.Y.Z (source)` and may end in ` (*)` for a repeat.
fn parse_tree_line(line: &str) -> Option<Resolved> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let (package, features) = line.split_once('|').unwrap_or((line, ""));
    let name = package.split_whitespace().next()?.to_owned();
    let features = features
        .trim()
        .trim_end_matches("(*)")
        .split(',')
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(str::to_owned)
        .collect();
    Some(Resolved { name, features })
}

fn parse_tree(output: &str) -> Vec<Resolved> {
    output.lines().filter_map(parse_tree_line).collect()
}

/// Parse `[[package]]` name/source pairs out of a `Cargo.lock`.
fn parse_lock(text: &str) -> Result<Vec<(String, String)>, String> {
    #[derive(Deserialize)]
    struct Lock {
        #[serde(default)]
        package: Vec<LockPackage>,
    }
    #[derive(Deserialize)]
    struct LockPackage {
        name: String,
        source: Option<String>,
    }
    let lock: Lock = toml::from_str(text).map_err(|e| format!("Cargo.lock: {e}"))?;
    Ok(lock
        .package
        .into_iter()
        .map(|p| (p.name, p.source.unwrap_or_else(|| "path".to_owned())))
        .collect())
}

fn has_workspace_lints(manifest: &str) -> Result<bool, String> {
    let value: toml::Value = toml::from_str(manifest).map_err(|e| e.to_string())?;
    Ok(value
        .get("lints")
        .and_then(|l| l.get("workspace"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false))
}

fn cargo() -> Command {
    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned()))
}

fn run(cmd: &mut Command, what: &str) -> Result<String, String> {
    let output = cmd.output().map_err(|e| format!("{what}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "{what} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| format!("{what}: {e}"))
}

fn tree(
    root: &Path,
    package: &str,
    row: Option<&Row>,
    target: Option<&str>,
) -> Result<Vec<Resolved>, String> {
    let mut cmd = cargo();
    cmd.current_dir(root).args([
        "tree", "--locked", "-p", package, "-e", "normal", "--prefix", "none", "-f", "{p}|{f}",
    ]);
    if let Some(target) = target {
        cmd.args(["--target", target]);
    }
    if let Some(row) = row {
        if row.no_default_features {
            cmd.arg("--no-default-features");
        }
        if !row.features.is_empty() {
            cmd.args(["--features", &row.features.join(",")]);
        }
    }
    run(&mut cmd, &format!("cargo tree -p {package}")).map(|out| parse_tree(&out))
}

fn members(root: &Path) -> Result<Vec<Member>, String> {
    let mut cmd = cargo();
    cmd.current_dir(root)
        .args(["metadata", "--format-version", "1", "--no-deps", "--locked"]);
    let text = run(&mut cmd, "cargo metadata")?;
    let meta: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("cargo metadata: {e}"))?;
    let root_str = meta
        .get("workspace_root")
        .and_then(serde_json::Value::as_str)
        .ok_or("cargo metadata: no workspace_root")?
        .to_owned();
    let packages = meta
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .ok_or("cargo metadata: no packages")?;
    let mut out = Vec::new();
    for package in packages {
        let name = package
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or("cargo metadata: package without name")?;
        let manifest = PathBuf::from(
            package
                .get("manifest_path")
                .and_then(serde_json::Value::as_str)
                .ok_or("cargo metadata: package without manifest_path")?,
        );
        let dir = manifest
            .parent()
            .and_then(|d| d.strip_prefix(&root_str).ok())
            .map(|d| d.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let proc_macro = package
            .get("targets")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|targets| {
                targets.iter().any(|t| {
                    t.get("kind")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|k| k.iter().any(|k| k == "proc-macro"))
                })
            });
        let text = std::fs::read_to_string(&manifest)
            .map_err(|e| format!("{}: {e}", manifest.display()))?;
        let workspace_lints =
            has_workspace_lints(&text).map_err(|e| format!("{}: {e}", manifest.display()))?;
        out.push(Member {
            name: name.to_owned(),
            dir,
            workspace_lints,
            proc_macro,
        });
    }
    Ok(out)
}

fn gather(root: &Path, table: &Table) -> Result<Facts, String> {
    let members = members(root)?;
    let mut row_graphs = Vec::with_capacity(table.rows.len());
    for row in &table.rows {
        let target = (row.layer == Layer::L0).then_some(table.contract_target.as_str());
        row_graphs.push(tree(root, &row.package, Some(row), target)?);
    }
    let mut default_graphs = BTreeMap::new();
    for member in &members {
        if !in_dirs(&member.dir, &table.host_dirs) && !member.proc_macro {
            default_graphs.insert(member.name.clone(), tree(root, &member.name, None, None)?);
        }
    }
    let lock =
        std::fs::read_to_string(root.join("Cargo.lock")).map_err(|e| format!("Cargo.lock: {e}"))?;
    let mut idiom_hits = BTreeMap::new();
    for idiom in &table.idioms {
        idiom_hits.insert(idiom.name.clone(), scan_idiom(root, idiom)?);
    }
    Ok(Facts {
        members,
        row_graphs,
        default_graphs,
        lock_sources: parse_lock(&lock)?,
        idiom_hits,
    })
}

/// Every `*.rs` file under `dir`, skipping `target/` and dot-directories.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    for entry in entries {
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if name != "target" && !name.starts_with('.') {
                rust_files(&path, out)?;
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

fn scan_idiom(root: &Path, idiom: &Idiom) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    for dir in &idiom.dirs {
        rust_files(&root.join(dir), &mut files)?;
    }
    files.sort();
    let mut hits = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(root)
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        if in_dirs(&rel, &idiom.allow) {
            continue;
        }
        let text =
            std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        hits.extend(
            idiom_lines(&text, &idiom.pattern)
                .into_iter()
                .map(|n| format!("{rel}:{n}")),
        );
    }
    Ok(hits)
}

/// Load the table at `config`.
pub fn load(config: &Path) -> Result<Table, String> {
    let text = std::fs::read_to_string(config).map_err(|e| format!("{}: {e}", config.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", config.display()))
}

/// `cargo xtask boundaries`: check, print, ratchet.
pub fn run_cli(root: &Path, config: &Path, verbose: bool) -> Result<(), String> {
    let table = load(config)?;
    let facts = gather(root, &table)?;
    let violations = check(&table, &facts);
    if verbose {
        for row in &table.rows {
            let note = row.note.as_deref().unwrap_or("");
            println!("row   {:?} {} {note}", row.layer, row.label());
        }
    }
    for v in &violations {
        println!("{v}");
    }
    let mut failed = Vec::new();
    for (key, verdict) in ratchet(&table, &violations) {
        match verdict {
            Ratchet::Held => {
                if verbose {
                    println!("ok    {key}");
                }
            }
            Ratchet::Worse { allowed, found } => {
                println!("FAIL  {key}: {found} violations, baseline {allowed}");
                failed.push(key);
            }
            Ratchet::Better { allowed, found } => {
                println!(
                    "FAIL  {key}: {found} violations, baseline {allowed}: lower [baseline] {key} to {found}"
                );
                failed.push(key);
            }
        }
    }
    println!(
        "boundaries: {} rows, {} members, {} violations",
        table.rows.len(),
        facts.members.len(),
        violations.len()
    );
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("boundaries off baseline: {}", failed.join(", ")))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const TABLE: &str = r#"
        single-source = ["architect", "auth*"]
        unmapped-ok = ["features/auth/tests"]

        [forbidden]
        engine = ["sea-orm", "axum", "iroh"]
        ui = ["dioxus"]
        tokio-runtime = ["rt", "rt-multi-thread"]
        transport-host = ["server", "server-axum", "host", "iroh"]
        architect-server = ["server-seaorm", "dispatch-tokio"]

        [[row]]
        package = "greeter"
        layer = "L0"

        [[row]]
        package = "greeter"
        features = ["ui"]
        layer = "L4"

        [[idiom]]
        name = "hand-routes"
        pattern = ".route("
        instead = "a #[architect::service] trait"
    "#;

    fn table() -> Table {
        toml::from_str(TABLE).unwrap()
    }

    fn pkg(name: &str, features: &[&str]) -> Resolved {
        Resolved {
            name: name.to_owned(),
            features: features.iter().map(|f| (*f).to_owned()).collect(),
        }
    }

    fn member(name: &str, dir: &str) -> Member {
        Member {
            name: name.to_owned(),
            dir: dir.to_owned(),
            workspace_lints: true,
            proc_macro: false,
        }
    }

    /// A clean world: every rule passes.
    fn clean() -> Facts {
        Facts {
            members: vec![
                member("greeter", "features/greeter"),
                member("app-server", "apps/app/server"),
            ],
            row_graphs: vec![
                vec![
                    pkg("greeter", &[]),
                    pkg("architect", &["vox"]),
                    pkg("tokio", &["sync"]),
                ],
                vec![
                    pkg("greeter", &["ui"]),
                    pkg("architect", &["vox", "client"]),
                    pkg("dioxus", &[]),
                ],
            ],
            default_graphs: BTreeMap::from([(
                "greeter".to_owned(),
                vec![pkg("greeter", &[]), pkg("architect", &["vox"])],
            )]),
            lock_sources: vec![
                ("architect".to_owned(), "path".to_owned()),
                ("auth".to_owned(), "path".to_owned()),
            ],
            idiom_hits: BTreeMap::from([("hand-routes".to_owned(), Vec::new())]),
        }
    }

    fn rules(v: &[Violation]) -> Vec<Rule> {
        v.iter().map(|v| v.rule).collect()
    }

    #[test]
    fn a_clean_workspace_passes_every_rule() {
        let v = check(&table(), &clean());
        assert!(v.is_empty(), "{v:?}");
        assert!(
            ratchet(&table(), &v)
                .iter()
                .all(|(_, r)| *r == Ratchet::Held)
        );
    }

    #[test]
    fn contract_reaching_storage_or_a_runtime_is_caught() {
        let mut facts = clean();
        facts.row_graphs[0].push(pkg("sea-orm", &[]));
        facts.row_graphs[0].push(pkg("tokio", &["rt-multi-thread"]));
        facts.row_graphs[0].push(pkg("dioxus", &[]));
        let v = check(&table(), &facts);
        assert_eq!(rules(&v), vec![Rule::ContractClean; 3], "{v:?}");
        assert!(
            v.iter()
                .any(|v| v.detail.ends_with("reaches tokio/rt-multi-thread"))
        );
    }

    #[test]
    fn ui_reaching_an_engine_or_server_architect_is_caught() {
        let mut facts = clean();
        facts.row_graphs[1].push(pkg("axum", &[]));
        facts.row_graphs[1][1]
            .features
            .insert("server-seaorm".to_owned());
        let v = check(&table(), &facts);
        assert_eq!(rules(&v), vec![Rule::UiReach; 2], "{v:?}");
    }

    #[test]
    fn ui_may_use_dioxus_and_the_client() {
        let v = check(&table(), &clean());
        assert!(!rules(&v).contains(&Rule::UiReach));
    }

    #[test]
    fn a_transport_host_outside_apps_is_caught() {
        let mut facts = clean();
        facts
            .default_graphs
            .get_mut("greeter")
            .unwrap()
            .push(pkg("architect", &["server-axum"]));
        let v = check(&table(), &facts);
        assert_eq!(rules(&v), vec![Rule::TransportHost], "{v:?}");
    }

    #[test]
    fn a_member_without_workspace_lints_is_caught() {
        let mut facts = clean();
        facts.members[0].workspace_lints = false;
        assert_eq!(rules(&check(&table(), &facts)), vec![Rule::Lints]);
    }

    #[test]
    fn two_architect_pins_in_one_lock_are_caught() {
        let mut facts = clean();
        facts.lock_sources = vec![
            (
                "architect".to_owned(),
                "git+https://github.com/FastTrackStudios/architect?tag=v0.9.6#aaa".to_owned(),
            ),
            (
                "auth".to_owned(),
                "git+https://github.com/FastTrackStudios/architect?branch=action-list#bbb"
                    .to_owned(),
            ),
            (
                "serde".to_owned(),
                "registry+https://github.com/rust-lang/crates.io-index".to_owned(),
            ),
        ];
        assert_eq!(rules(&check(&table(), &facts)), vec![Rule::SingleSource]);
    }

    #[test]
    fn one_tag_resolved_at_one_commit_is_one_source() {
        let mut facts = clean();
        let pin = "git+https://github.com/FastTrackStudios/architect?tag=v0.9.6#aaa";
        facts.lock_sources = vec![
            ("architect".to_owned(), pin.to_owned()),
            ("auth".to_owned(), pin.to_owned()),
        ];
        assert!(check(&table(), &facts).is_empty());
    }

    #[test]
    fn a_feature_crate_with_no_row_is_caught() {
        let mut facts = clean();
        facts
            .members
            .push(member("auth-proto", "features/auth/auth-proto"));
        facts
            .members
            .push(member("auth-tests", "features/auth/tests/native"));
        facts.members.push(Member {
            proc_macro: true,
            ..member("crdt-derive", "features/crdt/crdt-derive")
        });
        let v = check(&table(), &facts);
        assert_eq!(rules(&v), vec![Rule::Unmapped], "{v:?}");
        assert!(v[0].detail.starts_with("auth-proto"));
    }

    #[test]
    fn the_ratchet_fails_both_ways() {
        let mut t = table();
        t.baseline.insert("lints".to_owned(), 1);
        let mut facts = clean();
        // Below baseline: must be lowered.
        assert_eq!(
            ratchet(&t, &check(&t, &facts))[3],
            (
                "lints".to_owned(),
                Ratchet::Better {
                    allowed: 1,
                    found: 0
                }
            )
        );
        facts.members[0].workspace_lints = false;
        assert_eq!(
            ratchet(&t, &check(&t, &facts))[3],
            ("lints".to_owned(), Ratchet::Held)
        );
        facts.members[1].workspace_lints = false;
        assert_eq!(
            ratchet(&t, &check(&t, &facts))[3],
            (
                "lints".to_owned(),
                Ratchet::Worse {
                    allowed: 1,
                    found: 2
                }
            )
        );
    }

    #[test]
    fn a_hand_route_is_counted_and_ratcheted_by_name() {
        let mut t = table();
        let mut facts = clean();
        facts.idiom_hits.insert(
            "hand-routes".to_owned(),
            vec![
                "apps/server/src/lib.rs:12".to_owned(),
                "apps/server/src/lib.rs:13".to_owned(),
            ],
        );
        let v = check(&t, &facts);
        assert_eq!(rules(&v), vec![Rule::Idiom; 2], "{v:?}");
        assert_eq!(v[0].key(), "hand-routes");
        assert!(
            v[0].detail
                .contains("instead: a #[architect::service] trait")
        );
        let verdict = |t: &Table| {
            ratchet(t, &v)
                .into_iter()
                .find(|(k, _)| k == "hand-routes")
                .unwrap()
                .1
        };
        assert_eq!(
            verdict(&t),
            Ratchet::Worse {
                allowed: 0,
                found: 2
            }
        );
        t.baseline.insert("hand-routes".to_owned(), 2);
        assert_eq!(verdict(&t), Ratchet::Held);
    }

    #[test]
    fn idiom_lines_skip_comments() {
        let text = "fn a() {\n    r.route(\"/x\", get(x))\n    // .route(\"/old\")\n    .route(\"/y\", post(y));\n}\n";
        assert_eq!(idiom_lines(text, ".route("), vec![2, 4]);
    }

    #[test]
    fn cargo_tree_lines_parse() {
        let out = "greeter v0.1.0 (/x/examples/greeter)|default\n\
                   tokio v1.47.1|bytes,default,rt,sync\n\
                   architect v0.1.0 (/x/crates/architect)|vox (*)\n\
                   facet v0.50.0|\n";
        let g = parse_tree(out);
        assert_eq!(g.len(), 4);
        assert_eq!(g[1], pkg("tokio", &["bytes", "default", "rt", "sync"]));
        assert_eq!(g[2], pkg("architect", &["vox"]));
        assert_eq!(g[3], pkg("facet", &[]));
    }

    #[test]
    fn lockfile_and_manifest_parse() {
        let lock = "version = 4\n[[package]]\nname = \"a\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"b\"\nversion = \"1.0.0\"\nsource = \"registry+x\"\n";
        assert_eq!(
            parse_lock(lock).unwrap(),
            vec![
                ("a".to_owned(), "path".to_owned()),
                ("b".to_owned(), "registry+x".to_owned())
            ]
        );
        assert!(has_workspace_lints("[package]\nname='a'\n[lints]\nworkspace = true\n").unwrap());
        assert!(!has_workspace_lints("[package]\nname='a'\n").unwrap());
    }

    #[test]
    fn the_repo_table_parses_and_covers_every_rule_id() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../boundaries.toml");
        let t = load(&path).unwrap();
        assert!(!t.rows.is_empty());
        let ids = keys(&t);
        for key in t.baseline.keys() {
            assert!(ids.contains(key), "unknown baseline rule {key}");
        }
    }
}
