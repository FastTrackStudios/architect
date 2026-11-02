//! `cargo xtask fleet-deps`: the fleet's feature union, kept in
//! `workspace-hack/Cargo.toml` between the `FLEET SECTION` markers.
//!
//! ## Why this exists
//!
//! kache shares a compiled crate between repos only when the crate, its
//! features and every crate under it are identical. Cargo unifies features
//! per workspace, so a crate the fleet's repos all build (tokio, Dioxus,
//! Blitz, reqwest, …) used to resolve with different features in architect
//! and in each consumer, and one difference rebuilt everything above it.
//!
//! hakari (`.config/hakari.toml`) fixes that inside one workspace: every
//! member depends on `architect-workspace-hack`, which turns on the union of
//! the features its members use. It sees one workspace only. This command
//! adds the rest of the fleet: it resolves each consumer's graph (Task,
//! studio, keyflow, frame) the way their CI builds it, and writes the union
//! for every crate architect's graph shares with them into the section
//! below hakari's. Consumers depend on architect, so the hack crate is in
//! their graph too and its features unify into theirs: their own extra
//! features become a no-op, and a crate builds the same in every repo.
//!
//! A build here is `cargo build --workspace --all-targets` (dev-dependencies),
//! with default features and with all features, as hakari resolves, per
//! platform with guppy, the resolver hakari uses. The hack crate is left out of each repo's own
//! resolution, so the union is of what the repos need, not of an older
//! union. wasm is measured but not written: the hack line is off on wasm
//! (`GATE`), because a whole-workspace union there would put the native
//! server stack in every web build.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use guppy::MetadataCommand;
use guppy::graph::cargo::{CargoOptions, CargoResolverVersion, CargoSet};
use guppy::graph::feature::StandardFeatures;
use guppy::graph::{DependencyDirection, ExternalSource, GitReq, PackageGraph, PackageMetadata};
use guppy::platform::{Platform, TargetFeatures};

/// The hakari-managed crate's package name (`.config/hakari.toml`).
pub const HACK: &str = "architect-workspace-hack";

/// Where the fleet section lives.
const HACK_MANIFEST: &str = "workspace-hack/Cargo.toml";

const BEGIN: &str = "### BEGIN FLEET SECTION";
const END: &str = "### END FLEET SECTION";
const HAKARI_END: &str = "### END HAKARI SECTION";

/// The table every member's hack line lives in. Off on wasm (see the module
/// docs), and off under `--cfg no_workspace_hack`, which `xtask boundaries`
/// sets so it checks the dependencies a crate declares rather than the
/// union.
pub const GATE: &str =
    r#"[target.'cfg(not(any(target_family = "wasm", no_workspace_hack)))'.dependencies]"#;

/// One build the fleet runs: a target and the host that builds it.
struct Build {
    target: &'static str,
    host: &'static str,
}

/// Linux servers, desktops and CI; the macOS desktop (Apple Silicon and
/// Intel, both built on Apple Silicon); the iPhone apps; the web apps.
const BUILDS: &[Build] = &[
    Build {
        target: "x86_64-unknown-linux-gnu",
        host: "x86_64-unknown-linux-gnu",
    },
    Build {
        target: "aarch64-apple-darwin",
        host: "aarch64-apple-darwin",
    },
    Build {
        target: "x86_64-apple-darwin",
        host: "aarch64-apple-darwin",
    },
    Build {
        target: "aarch64-apple-ios",
        host: "aarch64-apple-darwin",
    },
    Build {
        target: "wasm32-unknown-unknown",
        host: "x86_64-unknown-linux-gnu",
    },
];

/// The cfg a section is written under. hakari writes its own tables by
/// triple (`[target.x86_64-unknown-linux-gnu.dependencies]`); a second
/// table with the same header is a TOML error, so these spell the same
/// platforms as cfgs. `None`: measured, never written.
fn cfg_of(triple: &str) -> Option<&'static str> {
    match triple {
        "x86_64-unknown-linux-gnu" => {
            Some(r#"cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))"#)
        }
        "aarch64-apple-darwin" => Some(r#"cfg(all(target_arch = "aarch64", target_os = "macos"))"#),
        "x86_64-apple-darwin" => Some(r#"cfg(all(target_arch = "x86_64", target_os = "macos"))"#),
        "aarch64-apple-ios" => Some(r#"cfg(all(target_arch = "aarch64", target_os = "ios"))"#),
        _ => None,
    }
}

/// A table of the union: crates built for a target, or for a host (build
/// scripts, proc macros, and what they use).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    Target(&'static str),
    Host(&'static str),
}

impl Section {
    fn header(self) -> Option<String> {
        match self {
            Self::Target(t) => cfg_of(t).map(|c| format!("[target.'{c}'.dependencies]")),
            Self::Host(h) => cfg_of(h).map(|c| format!("[target.'{c}'.build-dependencies]")),
        }
    }

    fn label(self) -> String {
        match self {
            Self::Target(t) => format!("{t} target"),
            Self::Host(h) => format!("{h} host"),
        }
    }
}

/// A third-party package, as the lockfile names it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pkg {
    pub name: String,
    pub version: String,
    /// The lockfile source; for git, with the commit it resolved to.
    pub source: String,
}

type Features = BTreeSet<String>;
/// What one repo builds: per section, every third-party package and the
/// features it is built with.
pub type Resolution = BTreeMap<Section, BTreeMap<Pkg, Features>>;

fn is_hack(name: &str) -> bool {
    name == HACK || name.ends_with("workspace-hack")
}

/// Crates.io or git, and not one of this repo's own crates (a consumer
/// sees those as git dependencies; they are never shared, see the
/// skill's "Fleet toolchain").
fn third_party(pkg: &PackageMetadata<'_>) -> Option<Pkg> {
    if pkg.in_workspace() || is_hack(pkg.name()) {
        return None;
    }
    let source = pkg.source().external_source()?;
    if source.contains("FastTrackStudios/architect") {
        return None;
    }
    Some(Pkg {
        name: pkg.name().to_owned(),
        version: pkg.version().to_string(),
        source: source.to_owned(),
    })
}

/// Load a workspace's package graph (`cargo metadata`). `locked`: fail
/// rather than touch its `Cargo.lock`.
pub fn load(root: &Path, locked: bool) -> Result<PackageGraph, String> {
    let mut cmd = MetadataCommand::new();
    cmd.current_dir(root);
    if locked {
        cmd.other_options(["--locked".to_owned()]);
    }
    PackageGraph::from_command(&mut cmd).map_err(|e| format!("{}: {e}", root.display()))
}

/// How much of the hack crate a resolution sees.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Hack {
    /// None of it: what the repo's members need.
    Omit,
    /// All of it, but a package counts only when something other than the
    /// hack links to it: the repo's graph with the union's features on,
    /// without the union's own stale entries.
    Follow,
}

/// Resolve `graph` for every fleet build.
pub fn resolve(graph: &PackageGraph, hack: Hack) -> Result<Resolution, String> {
    let hack_ids: Vec<_> = graph
        .packages()
        .filter(|p| is_hack(p.name()))
        .map(|p| p.id())
        .collect();
    let mut out = Resolution::new();
    // Default features and all features, as hakari resolves each repo: its
    // section holds what any member's features turn on (Task's `wasm-split`
    // reaches dioxus-core), so the fleet's union has to as well.
    for features in [StandardFeatures::Default, StandardFeatures::All] {
        for build in BUILDS {
            let platform = |triple: &'static str| {
                Platform::new(triple, TargetFeatures::Unknown).map_err(|e| format!("{triple}: {e}"))
            };
            let mut opts = CargoOptions::new();
            opts.set_resolver(CargoResolverVersion::V2)
                .set_include_dev(true)
                .set_target_platform(platform(build.target)?)
                .set_host_platform(platform(build.host)?);
            if hack == Hack::Omit {
                opts.add_omitted_packages(hack_ids.iter().copied());
            }
            let initials = graph
                .resolve_workspace()
                .filter(DependencyDirection::Forward, |p| !is_hack(p.name()))
                .to_feature_set(features);
            let set = initials
                .into_cargo_set(&opts)
                .map_err(|e| format!("resolving for {}: {e}", build.target))?;
            collect(&set, Section::Target(build.target), hack, &mut out, false);
            // A wasm build's host side is the Linux host's, already measured
            // by the native build; the hack is off for wasm builds anyway.
            if cfg_of(build.target).is_some() {
                collect(&set, Section::Host(build.host), hack, &mut out, true);
            }
        }
    }
    Ok(out)
}

fn collect(set: &CargoSet<'_>, section: Section, hack: Hack, out: &mut Resolution, host: bool) {
    let (features, links) = if host {
        (set.host_features(), set.host_links().collect::<Vec<_>>())
    } else {
        (
            set.target_features(),
            set.target_links().collect::<Vec<_>>(),
        )
    };
    let linked: BTreeSet<_> = links
        .iter()
        .filter(|l| !is_hack(l.from().name()))
        .map(|l| l.to().id())
        .collect();
    let table = out.entry(section).or_default();
    for list in features.packages_with_features(DependencyDirection::Forward) {
        let pkg = list.package();
        if hack == Hack::Follow && !linked.contains(pkg.id()) {
            continue;
        }
        if let Some(key) = third_party(pkg) {
            table
                .entry(key)
                .or_default()
                .extend(list.named_features().map(str::to_owned));
        }
    }
}

/// The union: for every package architect builds that a consumer builds
/// too, the features of every repo that builds it, where the repos do not
/// already agree.
pub fn union(own: &Resolution, follow: &Resolution, others: &[(String, Resolution)]) -> Resolution {
    let mut out = Resolution::new();
    for (&section, packages) in follow {
        if section.header().is_none() {
            continue;
        }
        for pkg in packages.keys() {
            let theirs: Vec<&Features> = others
                .iter()
                .filter_map(|(_, r)| r.get(&section).and_then(|t| t.get(pkg)))
                .collect();
            if theirs.is_empty() {
                continue;
            }
            let mine = own.get(&section).and_then(|t| t.get(pkg));
            let all: Features = theirs
                .iter()
                .copied()
                .chain(mine)
                .flatten()
                .cloned()
                .collect();
            let agree = mine.is_some_and(|m| *m == all) && theirs.iter().all(|t| **t == all);
            if !agree {
                out.entry(section).or_default().insert(pkg.clone(), all);
            }
        }
    }
    out
}

/// A package's dependency spec, without its key: its source and features.
fn spec(pkg: &Pkg, features: &Features) -> Option<String> {
    let mut parts = Vec::new();
    match ExternalSource::new(&pkg.source)? {
        ExternalSource::Registry(_) | ExternalSource::Sparse(_) => {
            if !pkg.source.contains("crates.io") {
                return None;
            }
            parts.push(format!("version = \"{}\"", exact_req(&pkg.version)));
        }
        ExternalSource::Git {
            repository, req, ..
        } => {
            parts.push(format!("git = \"{repository}\""));
            match req {
                GitReq::Branch(b) => parts.push(format!("branch = \"{b}\"")),
                GitReq::Tag(t) => parts.push(format!("tag = \"{t}\"")),
                GitReq::Rev(r) => parts.push(format!("rev = \"{r}\"")),
                GitReq::Default => {}
                _ => return None,
            }
        }
        _ => return None,
    }
    let default = features.contains("default");
    if !default {
        parts.push("default-features = false".to_owned());
    }
    let named: Vec<String> = features
        .iter()
        .filter(|f| *f != "default")
        .map(|f| format!("\"{f}\""))
        .collect();
    if !named.is_empty() {
        parts.push(format!("features = [{}]", named.join(", ")));
    }
    Some(parts.join(", "))
}

/// The requirement the hack writes: the version this workspace locks, as a
/// caret (`1.1.10` is `^1.1.10`), the way hakari's `exact-versions` writes
/// it. A consumer locked lower moves up to it: a looser `1` would leave a
/// consumer at `1.1.9`, where a feature `1.1.10` added (flate2
/// `runtime_detection`) does not exist and nothing resolves.
fn exact_req(version: &str) -> String {
    version.split('+').next().unwrap_or(version).to_owned()
}

/// `version`'s release line, for a dependency key's suffix
/// (`crypto-common-0_2`): `1`, `0.4`, `0.0.3`; a prerelease as itself.
fn semver_req(version: &str) -> String {
    let core = version.split('+').next().unwrap_or(version);
    // A prerelease matches only a requirement that names one.
    if core.contains('-') {
        return core.to_owned();
    }
    let mut it = core.split('.');
    let (major, minor, patch) = (
        it.next().unwrap_or("0"),
        it.next().unwrap_or("0"),
        it.next().unwrap_or("0"),
    );
    if major != "0" {
        major.to_owned()
    } else if minor != "0" {
        format!("0.{minor}")
    } else {
        format!("0.0.{patch}")
    }
}

/// What names a dependency inside the hack crate: its package and its
/// requirement (a version, or a git source). Cargo wants one key per
/// package across the whole manifest, hakari's tables included.
type Ident = (String, String);

fn ident_of_pkg(pkg: &Pkg) -> Ident {
    let req = match ExternalSource::new(&pkg.source) {
        Some(ExternalSource::Git {
            repository, req, ..
        }) => format!("git {repository} {req:?}"),
        _ => exact_req(&pkg.version),
    };
    (pkg.name.clone(), req)
}

fn ident_of_spec(key: &str, spec: &toml::Table) -> Ident {
    let get = |k: &str| spec.get(k).and_then(toml::Value::as_str);
    let name = get("package").unwrap_or(key).to_owned();
    let req = get("git").map_or_else(
        || get("version").unwrap_or_default().to_owned(),
        |repository| {
            let req = match (get("tag"), get("rev"), get("branch")) {
                (Some(t), _, _) => GitReq::Tag(t),
                (_, Some(r), _) => GitReq::Rev(r),
                (_, _, Some(b)) => GitReq::Branch(b),
                _ => GitReq::Default,
            };
            format!("git {repository} {req:?}")
        },
    );
    (name, req)
}

/// Every `(key, spec)` of a manifest fragment's dependency tables, by
/// table header.
fn tables(text: &str) -> Result<BTreeMap<String, Vec<(String, toml::Table)>>, String> {
    let doc: toml::Table = text.parse().map_err(|e| format!("{HACK_MANIFEST}: {e}"))?;
    let mut out: BTreeMap<String, Vec<_>> = BTreeMap::new();
    let mut add = |header: String, deps: Option<&toml::Value>| {
        let list = out.entry(header).or_default();
        for (key, spec) in deps.and_then(toml::Value::as_table).into_iter().flatten() {
            list.push((key.clone(), spec.as_table().cloned().unwrap_or_default()));
        }
    };
    for kind in ["dependencies", "build-dependencies"] {
        if doc.contains_key(kind) {
            add(format!("[{kind}]"), doc.get(kind));
        }
    }
    let targets = doc.get("target").and_then(toml::Value::as_table);
    for (cfg, kinds) in targets.into_iter().flatten() {
        for (kind, deps) in kinds.as_table().into_iter().flatten() {
            add(format!("[target.'{cfg}'.{kind}]"), Some(deps));
        }
    }
    Ok(out)
}

/// The text between two marker lines.
fn between<'a>(text: &'a str, begin: &str, end: &str) -> Option<&'a str> {
    let (_, rest) = text.split_once(begin)?;
    rest.split_once(end).map(|(inner, _)| inner)
}

/// The keys hakari's section gives its dependencies.
fn hakari_keys(manifest: &str) -> Result<BTreeMap<Ident, String>, String> {
    let section = between(manifest, "### BEGIN HAKARI SECTION", HAKARI_END).unwrap_or_default();
    Ok(tables(section)?
        .into_values()
        .flatten()
        .map(|(key, spec)| (ident_of_spec(&key, &spec), key))
        .collect())
}

/// The section's text, markers included. `taken`: the keys hakari already
/// gives packages, reused for the same package.
pub fn render(block: &Resolution, sources: &[String], taken: &BTreeMap<Ident, String>) -> String {
    let mut keys = taken.clone();
    let mut used: BTreeSet<String> = keys.values().cloned().collect();
    for pkg in block.values().flat_map(BTreeMap::keys) {
        let ident = ident_of_pkg(pkg);
        if keys.contains_key(&ident) {
            continue;
        }
        let key = unique_key(pkg, &used);
        used.insert(key.clone());
        keys.insert(ident, key);
    }
    let mut out = String::new();
    out.push_str(BEGIN);
    out.push('\n');
    out.push_str(
        "# The fleet's feature union (`cargo xtask fleet-deps`, `xtask/src/fleet_deps.rs`):\n\
         # every crate architect builds that a consumer builds too, with the features\n\
         # of every repo that builds it. Regenerate it; do not edit it by hand.\n",
    );
    let _ = writeln!(out, "# From: {}.", sources.join(", "));
    for (section, packages) in block {
        let Some(header) = section.header() else {
            continue;
        };
        let mut lines = Vec::new();
        for (pkg, features) in packages {
            let Some(spec) = spec(pkg, features) else {
                continue;
            };
            let Some(key) = keys.get(&ident_of_pkg(pkg)) else {
                continue;
            };
            let rename = if *key == pkg.name {
                String::new()
            } else {
                format!("package = \"{}\", ", pkg.name)
            };
            lines.push(format!("{key} = {{ {rename}{spec} }}"));
        }
        if lines.is_empty() {
            continue;
        }
        let _ = write!(out, "\n{header}\n{}\n", lines.join("\n"));
    }
    out.push_str(END);
    out.push('\n');
    out
}

fn unique_key(pkg: &Pkg, used: &BTreeSet<String>) -> String {
    let suffix = match ExternalSource::new(&pkg.source) {
        Some(ExternalSource::Git { .. }) => "git".to_owned(),
        _ => semver_req(&pkg.version).replace(['.', '-'], "_"),
    };
    let mut key = pkg.name.clone();
    if used.contains(&key) {
        key = format!("{}-{suffix}", pkg.name);
    }

    if !used.contains(&key) {
        return key;
    }
    (2_u32..10_000)
        .map(|n| format!("{key}-{n}"))
        .find(|k| !used.contains(k))
        .unwrap_or(key)
}

/// Put `section` into the hack manifest, replacing the old one, or right
/// after hakari's section the first time.
pub fn splice(manifest: &str, section: &str) -> Result<String, String> {
    let rest_of = |tail: &'_ str| tail.strip_prefix('\n').unwrap_or(tail).to_owned();
    if let Some((head, rest)) = manifest.split_once(BEGIN) {
        if let Some((_, tail)) = rest.split_once(END) {
            return Ok(format!("{head}{section}{}", rest_of(tail)));
        }
    }
    let (head, tail) = manifest
        .split_once(HAKARI_END)
        .ok_or_else(|| format!("{HACK_MANIFEST}: no `{HAKARI_END}` marker"))?;
    Ok(format!("{head}{HAKARI_END}\n\n{section}{}", rest_of(tail)))
}

/// The fleet section's entries, by table header.
pub fn entries(manifest: &str) -> Result<BTreeMap<String, Vec<(String, toml::Table)>>, String> {
    between(manifest, BEGIN, END).map_or_else(|| Ok(BTreeMap::new()), tables)
}

/// Entries that no longer name a package this workspace builds there.
pub fn stale(manifest: &str, follow: &Resolution) -> Result<Vec<String>, String> {
    let sections: BTreeMap<String, Section> = follow
        .keys()
        .filter_map(|s| s.header().map(|h| (h, *s)))
        .collect();
    let mut out = Vec::new();
    for (header, deps) in entries(manifest)? {
        let Some(section) = sections.get(&header) else {
            out.push(format!("{header}: not a fleet build's table"));
            continue;
        };
        let built = &follow[section];
        for (key, spec) in deps {
            let name = spec
                .get("package")
                .and_then(toml::Value::as_str)
                .unwrap_or(&key);
            let found = built.keys().any(|pkg| {
                pkg.name == name
                    && spec
                        .get("version")
                        .and_then(toml::Value::as_str)
                        .map_or_else(
                            || {
                                spec.get("git")
                                    .and_then(toml::Value::as_str)
                                    .is_some_and(|git| pkg.source.contains(git))
                            },
                            |req| exact_req(&pkg.version) == req,
                        )
            });
            if !found {
                out.push(format!("{}: {key} is no longer built", section.label()));
            }
        }
    }
    Ok(out)
}

/// Members whose hack line is outside [`GATE`].
pub fn ungated(root: &Path, locked: bool) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    for manifest in member_manifests(root, locked)? {
        let text = std::fs::read_to_string(&manifest)
            .map_err(|e| format!("{}: {e}", manifest.display()))?;
        if hack_line(&text).is_some_and(|(_, table)| table != GATE) {
            out.push(manifest);
        }
    }
    Ok(out)
}

/// Move every member's hack line (where `cargo hakari manage-deps` puts it,
/// under `[dependencies]`) into [`GATE`]. Returns the manifests changed.
pub fn gate(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut changed = Vec::new();
    for manifest in ungated(root, false)? {
        let text = std::fs::read_to_string(&manifest)
            .map_err(|e| format!("{}: {e}", manifest.display()))?;
        let Some(gated) = gate_text(&text) else {
            continue;
        };
        std::fs::write(&manifest, gated).map_err(|e| format!("{}: {e}", manifest.display()))?;
        changed.push(manifest);
    }
    Ok(changed)
}

/// The hack dependency line and the table it sits in. Any key ending in
/// `workspace-hack`, so the gate also works in a consumer, whose own
/// hakari crate is `workspace-hack`.
fn hack_line(text: &str) -> Option<(usize, &str)> {
    let mut table = "";
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            table = trimmed;
        } else if trimmed
            .split(['=', ' ', '.'])
            .next()
            .is_some_and(|key| !key.is_empty() && is_hack(key))
        {
            return Some((i, table));
        }
    }
    None
}

fn gate_text(text: &str) -> Option<String> {
    let (at, _) = hack_line(text)?;
    let mut lines: Vec<&str> = text.lines().collect();
    let line = lines.remove(at);
    let mut out: Vec<String> = lines.iter().map(|l| (*l).to_owned()).collect();
    if let Some(header) = out.iter().position(|l| l.trim() == GATE) {
        out.insert(header.saturating_add(1), line.to_owned());
    } else {
        while out.last().is_some_and(|l| l.trim().is_empty()) {
            out.pop();
        }
        out.push(String::new());
        out.push(GATE.to_owned());
        out.push(line.to_owned());
    }
    let mut joined = out.join("\n");
    joined.push('\n');
    Some(joined)
}

fn member_manifests(root: &Path, locked: bool) -> Result<Vec<PathBuf>, String> {
    let graph = load(root, locked)?;
    Ok(graph
        .workspace()
        .iter()
        .filter(|p| !is_hack(p.name()))
        .map(|p| p.manifest_path().as_std_path().to_path_buf())
        .collect())
}

/// `<dir name>@<short sha>` of a checkout, for the section's header.
pub fn describe(repo: &Path) -> String {
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    };
    // The repo's name from its remote, so a worktree reads as its repo.
    let name = git(&["remote", "get-url", "origin"])
        .and_then(|url| {
            let last = url
                .trim_end_matches('/')
                .rsplit(['/', ':'])
                .next()?
                .to_owned();
            Some(last.trim_end_matches(".git").to_owned())
        })
        .unwrap_or_else(|| repo.display().to_string());
    let sha = git(&["rev-parse", "--short=10", "HEAD"]).unwrap_or_else(|| "?".to_owned());
    format!("{name}@{sha}")
}

/// How each consumer differs from architect, per section: for the
/// measurement in the PR and the skill.
pub fn report(own: &Resolution, others: &[(String, Resolution)]) -> String {
    let mut out = String::new();
    for (name, theirs) in others {
        let _ = writeln!(out, "== {name}");
        for (section, mine) in own {
            let Some(their) = theirs.get(section) else {
                continue;
            };
            let shared: Vec<_> = mine.keys().filter(|p| their.contains_key(*p)).collect();
            let differ: Vec<_> = shared.iter().filter(|p| mine[**p] != their[**p]).collect();
            let names_mine: BTreeMap<&str, &Pkg> =
                mine.keys().map(|p| (p.name.as_str(), p)).collect();
            let skew = their
                .keys()
                .filter(|p| {
                    names_mine.get(p.name.as_str()).is_some_and(|m| *m != *p)
                        && !mine.contains_key(*p)
                })
                .count();
            let _ = writeln!(
                out,
                "  {}: {} shared, {} with different features, {} same name at another version or source",
                section.label(),
                shared.len(),
                differ.len(),
                skew
            );
            for pkg in differ {
                let (a, b) = (&mine[*pkg], &their[*pkg]);
                let only_a: Vec<_> = a.difference(b).cloned().collect();
                let only_b: Vec<_> = b.difference(a).cloned().collect();
                let _ = writeln!(
                    out,
                    "    {} {}: architect-only {only_a:?}, {name}-only {only_b:?}",
                    pkg.name, pkg.version
                );
            }
        }
    }
    out
}

/// `cargo xtask fleet-deps`.
pub fn run(root: &Path, repos: &[PathBuf], check: bool, print_report: bool) -> Result<(), String> {
    let manifest_path = root.join(HACK_MANIFEST);
    if check {
        let mut problems: Vec<String> = ungated(root, true)?
            .into_iter()
            .map(|m| {
                format!(
                    "{}: the workspace-hack line is not under {GATE}",
                    m.display()
                )
            })
            .collect();
        let graph = load(root, true)?;
        let follow = resolve(&graph, Hack::Follow)?;
        let manifest = read(&manifest_path)?;
        problems.extend(stale(&manifest, &follow)?);
        if problems.is_empty() {
            println!("fleet-deps: every hack line gated, every fleet entry current");
            return Ok(());
        }
        for p in &problems {
            eprintln!("fleet-deps: {p}");
        }
        return Err(format!(
            "{} problem(s); run `cargo xtask fleet-deps` (with --repo for each consumer to refresh the union)",
            problems.len()
        ));
    }

    let sources: Vec<String> = repos.iter().map(|r| describe(r)).collect();
    // The old section stays until the first round replaces it: emptying it
    // would let cargo drop its crates and add them back at their newest
    // versions, past what the fleet locks. Every round re-renders the whole
    // section from the repos' own resolutions, so a stale entry never
    // survives. (A section cargo cannot resolve at all: delete it by hand.)
    for m in gate(root)? {
        println!(
            "fleet-deps: gated the workspace-hack line in {}",
            m.display()
        );
    }
    if repos.is_empty() {
        return Ok(());
    }
    let mut others = Vec::new();
    for (repo, name) in repos.iter().zip(&sources) {
        let graph = load(repo, true)?;
        others.push((name.clone(), resolve(&graph, Hack::Omit)?));
    }
    // Turning a feature on can bring in crates (Dioxus `mobile` brings
    // wry), whose own features need the union too: repeat until the
    // section stops changing.
    for round in 1..=8 {
        let graph = load(root, false)?;
        let own = resolve(&graph, Hack::Omit)?;
        if round == 1 && print_report {
            print!("{}", report(&own, &others));
        }
        let follow = resolve(&graph, Hack::Follow)?;
        let block = union(&own, &follow, &others);
        let manifest = read(&manifest_path)?;
        let next = splice(
            &manifest,
            &render(&block, &sources, &hakari_keys(&manifest)?),
        )?;
        if next == manifest {
            let n: usize = block.values().map(BTreeMap::len).sum();
            println!("fleet-deps: {n} entries, stable after {round} round(s)");
            return Ok(());
        }
        std::fs::write(&manifest_path, next).map_err(|e| format!("{HACK_MANIFEST}: {e}"))?;
    }
    Err("fleet-deps: the section did not settle in 8 rounds".to_owned())
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requirements_pick_the_release_line() {
        assert_eq!(semver_req("1.48.0"), "1");
        assert_eq!(semver_req("0.4.12"), "0.4");
        assert_eq!(semver_req("0.0.3"), "0.0.3");
        assert_eq!(semver_req("0.50.0-rc.7"), "0.50.0-rc.7");
    }

    #[test]
    fn gating_moves_the_line_into_the_gate_table() {
        let text = "[package]\nname = \"a\"\n\n[dependencies]\nserde = \"1\"\narchitect-workspace-hack = { version = \"0.1\", path = \"../workspace-hack\" }\n\n[lints]\nworkspace = true\n";
        let gated = gate_text(text).unwrap();
        assert!(gated.ends_with(&format!("{GATE}\narchitect-workspace-hack = {{ version = \"0.1\", path = \"../workspace-hack\" }}\n")));
        assert!(gated.contains("[dependencies]\nserde = \"1\"\n\n[lints]"));
        assert_eq!(hack_line(&gated).map(|(_, t)| t), Some(GATE));
    }

    #[test]
    fn splice_goes_after_hakari_then_replaces_itself() {
        let hack = "[package]\n### BEGIN HAKARI SECTION\n[dependencies]\n### END HAKARI SECTION\n";
        let first = splice(hack, &format!("{BEGIN}\nA\n{END}\n")).unwrap();
        assert!(first.ends_with(&format!("{HAKARI_END}\n\n{BEGIN}\nA\n{END}\n")));
        let second = splice(&first, &format!("{BEGIN}\nB\n{END}\n")).unwrap();
        assert_eq!(second, first.replace("\nA\n", "\nB\n"));
    }

    #[test]
    fn union_skips_what_every_repo_agrees_on() {
        let pkg = |n: &str| Pkg {
            name: n.into(),
            version: "1.0.0".into(),
            source: "registry+https://github.com/rust-lang/crates.io-index".into(),
        };
        let set = |f: &[&str]| f.iter().map(|s| (*s).to_owned()).collect::<Features>();
        let section = Section::Target("x86_64-unknown-linux-gnu");
        let mut own = Resolution::new();
        own.entry(section)
            .or_default()
            .extend([(pkg("tokio"), set(&["rt"])), (pkg("serde"), set(&["std"]))]);
        let mut task = Resolution::new();
        task.entry(section).or_default().extend([
            (pkg("tokio"), set(&["rt", "tracing"])),
            (pkg("serde"), set(&["std"])),
        ]);
        let block = union(&own, &own, &[("task".into(), task)]);
        assert_eq!(block[&section].len(), 1);
        assert_eq!(block[&section][&pkg("tokio")], set(&["rt", "tracing"]));
        let text = render(&block, &["task@abc".into()], &BTreeMap::new());
        assert!(text.contains(
            "tokio = { version = \"1.0.0\", default-features = false, features = [\"rt\", \"tracing\"] }"
        ));
    }
}
