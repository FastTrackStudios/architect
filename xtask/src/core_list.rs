//! `cargo xtask core`: Architect's core extensions list and the standard
//! distribution built from it (#631, `docs/content/guide/core-extensions.md`).
//!
//! The list (`extensions/core-extensions.toml`) names each extension the
//! standard release includes, by reference to its own repo: id, publisher,
//! source, exact version (tag and commit), the publisher's signature over
//! that release, and its form per platform (`linked` or `packaged`).
//! `dist/standard` is the distribution crate that builds it, outside the
//! workspace, its `Cargo.toml` and `src/list.rs` generated from the list.
//!
//! | command | network | what it does |
//! | --- | --- | --- |
//! | `check` | no | the list's rules; `dist/standard` is what `sync` writes; its lock holds one Architect and each linked pin |
//! | `sync` | yes (resolves) | writes `dist/standard`'s `Cargo.toml`, `src/list.rs` and lock from the list and the root `Cargo.toml` |
//! | `verify` | yes | each tag still names its `rev`; each release's sums are signed by its publisher with the pinned comment; each packaged archive is in them |
//! | `fetch` | yes | each packaged folder for a platform, checked (sha256, signed sums, `BUILT_FROM`), into `<dir>/core/<id>/` |
//! | `graph` | yes (resolves) | resolves `dist/standard` as cargo builds it and fails on a second Architect, a Task crate or a moved pin |

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use sha2::Digest as _;

/// The list, from the repo root.
pub const LIST: &str = "extensions/core-extensions.toml";

/// The standard distribution crate, from the repo root.
pub const STANDARD: &str = "dist/standard";

/// The generated Rust in [`STANDARD`].
const LIST_RS: &str = "src/list.rs";

/// The crate's package and binary names.
const PACKAGE: &str = "architect-standard";
const BIN: &str = "architect-desktop";

/// Every spelling of architect's git URL a linked crate's graph may use:
/// each is patched back to this checkout.
pub const ARCHITECT_GIT: [&str; 2] = [
    "https://github.com/FastTrackStudios/architect",
    "https://www.github.com/FastTrackStudios/architect",
];

/// Task's crates pin an old Architect (v0.10.0); none may be linked.
const TASK_GIT: &str = "github.com/FastTrackStudios/task";

/// What `[core.signature] key = "org"` means.
const ORG: &str = "org";

/// `cargo xtask core …`.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum Action {
    /// The list's rules, `dist/standard` up to date with it, and its lock
    /// holding one Architect and each linked pin. Offline; CI's fast check.
    Check,
    /// Write `dist/standard` (`Cargo.toml`, `src/list.rs`, its lock) from
    /// the list and the root `Cargo.toml`. Resolves (fetches git sources).
    Sync,
    /// Check every pin against its source: the tag still names `rev`, the
    /// release's sums are signed by the publisher with the pinned comment,
    /// and each packaged archive is listed in them with its `sha256`.
    Verify,
    /// Fetch every packaged folder for `platform`, checked, into
    /// `<into>/core/<id>/` (beside the binary the build put in `<into>`).
    Fetch {
        /// The platform whose packaged extensions to fetch.
        #[arg(long, value_enum, default_value = "node")]
        platform: Platform,
        /// Where the binary is; the folders go in its `core/`.
        #[arg(long)]
        into: PathBuf,
    },
    /// Resolve `dist/standard` as cargo builds it and fail on any second
    /// Architect, any Task crate or a linked crate off its pin.
    Graph,
}

/// Where a form runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    /// The app's window.
    Desktop,
    /// The headless node (`serve`): share links and Sites.
    Node,
    /// The iPhone and iPad app (not built by `dist/standard` yet).
    Ios,
    /// The hosted web app (not built by `dist/standard` yet).
    Web,
}

impl Platform {
    const fn name(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Node => "node",
            Self::Ios => "ios",
            Self::Web => "web",
        }
    }

    /// `dist/standard` builds it today.
    const fn built(self) -> bool {
        matches!(self, Self::Desktop | Self::Node)
    }
}

/// `extensions/core-extensions.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct List {
    #[serde(default)]
    pub core: Vec<Entry>,
}

/// One core extension.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub publisher: String,
    pub source: String,
    pub version: String,
    pub tag: String,
    pub rev: String,
    pub signature: Signature,
    pub linked: Option<Linked>,
    pub packaged: Option<Packaged>,
}

/// The publisher's minisign signature over the release's sums.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signature {
    pub key: String,
    pub sums: String,
    pub minisig: String,
    pub comment: String,
}

/// The linked form: compiled in from its crate.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Linked {
    #[serde(rename = "crate")]
    pub krate: String,
    #[serde(default)]
    pub features: Vec<String>,
    pub platforms: Vec<Platform>,
    pub extension: String,
    pub blocks: Option<String>,
    #[serde(default)]
    pub patch: BTreeMap<String, Vec<String>>,
}

/// The packaged form: a file its release carries.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Packaged {
    pub platforms: Vec<Platform>,
    pub archive: String,
    pub sha256: String,
    pub folder: Option<String>,
    pub island: Option<IslandViews>,
}

/// A packaged folder that is a share-page island.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IslandViews {
    pub progress: Option<String>,
    pub transport: Option<String>,
    pub lyrics: Option<String>,
    pub stylesheet: Option<String>,
}

impl Entry {
    /// The minisign public key its signature is checked with.
    fn key(&self) -> &str {
        if self.signature.key == ORG {
            architect_extension_archive::ORG_KEY
        } else {
            &self.signature.key
        }
    }

    /// Its packaged folder, if it ships one `dist/standard` builds.
    fn folder(&self) -> Option<(&Packaged, &str)> {
        let p = self.packaged.as_ref()?;
        let folder = p.folder.as_deref()?;
        p.platforms.iter().any(|x| x.built()).then_some((p, folder))
    }
}

// ── reading and the rules ───────────────────────────────────────────────

/// Parses `text` as the list and applies its rules.
pub fn parse(text: &str) -> Result<List, String> {
    let list: List = toml::from_str(text).map_err(|e| format!("{LIST}: {e}"))?;
    let problems = problems(&list);
    if problems.is_empty() {
        Ok(list)
    } else {
        Err(format!(
            "{LIST}:\n{}",
            problems
                .iter()
                .map(|p| format!("  - {p}"))
                .collect::<Vec<_>>()
                .join("\n")
        ))
    }
}

fn read_list(root: &Path) -> Result<List, String> {
    let path = root.join(LIST);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&text)
}

fn is_name(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn is_id(s: &str) -> bool {
    !s.is_empty()
        && s.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        })
}

fn is_url(s: &str) -> bool {
    s.strip_prefix("https://")
        .is_some_and(|rest| !rest.is_empty() && !rest.contains(char::is_whitespace))
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_version(s: &str) -> bool {
    let (core, pre) = s.split_once('-').map_or((s, None), |(c, p)| (c, Some(p)));
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        && pre.is_none_or(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        })
}

/// `a::b::C`: a Rust path of at least two segments.
fn is_path(s: &str) -> bool {
    let segments: Vec<&str> = s.split("::").collect();
    segments.len() >= 2
        && segments.iter().all(|seg| {
            let mut chars = seg.chars();
            chars
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

/// A minisign public key's base64 line: `RW` and 54 more characters.
fn is_key(s: &str) -> bool {
    s.len() == 56
        && s.starts_with("RW")
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
}

/// A relative path inside an archive: no root, no `..`.
fn is_inner(s: &str) -> bool {
    !s.is_empty()
        && !s.contains('\\')
        && Path::new(s)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

fn platforms_problems(what: &str, platforms: &[Platform], out: &mut Vec<String>) {
    if platforms.is_empty() {
        out.push(format!("{what}: `platforms` names none"));
    }
    let set: BTreeSet<_> = platforms.iter().collect();
    if set.len() != platforms.len() {
        out.push(format!("{what}: `platforms` names one twice"));
    }
}

/// Every rule the list breaks.
#[must_use]
pub fn problems(list: &List) -> Vec<String> {
    let mut out = Vec::new();
    let mut ids = BTreeSet::new();
    let mut crates: BTreeMap<&str, &str> = BTreeMap::new();
    for e in &list.core {
        let id = &e.id;
        if !is_id(id) {
            out.push(format!(
                "`{id}`: an id is lower-case letters, digits, `_` and `-`, in `.`-separated parts"
            ));
        }
        if !ids.insert(id.as_str()) {
            out.push(format!("`{id}` is listed twice"));
        }
        if e.publisher.trim().is_empty() {
            out.push(format!("`{id}`: `publisher` is empty"));
        }
        for (field, url) in [
            ("source", &e.source),
            ("signature.sums", &e.signature.sums),
            ("signature.minisig", &e.signature.minisig),
        ] {
            if !is_url(url) {
                out.push(format!("`{id}`: `{field}` is not an https URL: {url}"));
            }
        }
        if !is_version(&e.version) {
            out.push(format!(
                "`{id}`: `version` is not X.Y.Z[-pre]: {}",
                e.version
            ));
        }
        if e.tag.is_empty() || !e.tag.ends_with(&e.version) {
            out.push(format!(
                "`{id}`: `tag` ({}) does not name `version` ({})",
                e.tag, e.version
            ));
        }
        if !is_hex(&e.rev, 40) {
            out.push(format!(
                "`{id}`: `rev` is not a full commit id (40 lower-case hex): {}",
                e.rev
            ));
        }
        if e.signature.key != ORG && !is_key(&e.signature.key) {
            out.push(format!(
                "`{id}`: `signature.key` is neither `org` nor a minisign public key"
            ));
        }
        if !e.signature.comment.split_whitespace().any(|w| w == e.tag) {
            out.push(format!(
                "`{id}`: `signature.comment` does not name the tag {}",
                e.tag
            ));
        }
        if e.linked.is_none() && e.packaged.is_none() {
            out.push(format!(
                "`{id}`: no form: give `linked`, `packaged` or both"
            ));
        }
        if let Some(l) = &e.linked {
            linked_problems(e, l, &mut crates, &mut out);
        }
        if let Some(p) = &e.packaged {
            packaged_problems(id, p, &mut out);
        }
        if let (Some(l), Some(p)) = (&e.linked, &e.packaged) {
            for platform in &l.platforms {
                if p.platforms.contains(platform) {
                    out.push(format!(
                        "`{id}`: {} is both linked and packaged: one form per platform",
                        platform.name()
                    ));
                }
            }
        }
    }
    out
}

fn linked_problems<'a>(
    e: &'a Entry,
    l: &'a Linked,
    crates: &mut BTreeMap<&'a str, &'a str>,
    out: &mut Vec<String>,
) {
    let id = &e.id;
    platforms_problems(&format!("`{id}` linked"), &l.platforms, out);
    if !is_name(&l.krate) {
        out.push(format!(
            "`{id}`: `linked.crate` is not a crate name: {}",
            l.krate
        ));
    }
    if let Some(other) = crates.insert(&l.krate, id) {
        out.push(format!(
            "`{id}`: crate `{}` is already linked by `{other}`",
            l.krate
        ));
    }
    if let Some(f) = l.features.iter().find(|f| !is_name(f)) {
        out.push(format!(
            "`{id}`: `linked.features` names `{f}`, not a feature name"
        ));
    }
    if !is_path(&l.extension) {
        out.push(format!(
            "`{id}`: `linked.extension` is not a Rust path: {}",
            l.extension
        ));
    }
    if let Some(b) = &l.blocks
        && !is_path(b)
    {
        out.push(format!("`{id}`: `linked.blocks` is not a Rust path: {b}"));
    }
    for (url, names) in &l.patch {
        if !is_url(url) {
            out.push(format!(
                "`{id}`: `linked.patch` key is not an https URL: {url}"
            ));
        }
        if ARCHITECT_GIT.contains(&url.as_str()) {
            out.push(format!(
                "`{id}`: `linked.patch` may not patch architect: the build patches it to this checkout"
            ));
        }
        if names.is_empty() || names.iter().any(|n| !is_name(n)) {
            out.push(format!(
                "`{id}`: `linked.patch.\"{url}\"` must name crates of {}",
                e.source
            ));
        }
    }
}

fn packaged_problems(id: &str, p: &Packaged, out: &mut Vec<String>) {
    platforms_problems(&format!("`{id}` packaged"), &p.platforms, out);
    if !is_url(&p.archive) {
        out.push(format!(
            "`{id}`: `packaged.archive` is not an https URL: {}",
            p.archive
        ));
    }
    if !is_hex(&p.sha256, 64) {
        out.push(format!(
            "`{id}`: `packaged.sha256` is not 64 lower-case hex"
        ));
    }
    if let Some(folder) = &p.folder {
        if !is_inner(folder) {
            out.push(format!(
                "`{id}`: `packaged.folder` is not a relative path inside the archive: {folder}"
            ));
        }
        if !p.archive.ends_with(".tar.gz") {
            out.push(format!(
                "`{id}`: a `packaged.folder` is taken from a .tar.gz archive"
            ));
        }
    } else {
        if !p.archive.ends_with(".archext") {
            out.push(format!(
                "`{id}`: a packaged archive is a .tar.gz with a `folder`, or an .archext"
            ));
        }
        if p.island.is_some() {
            out.push(format!("`{id}`: an island is a `folder`"));
        }
    }
    if let Some(island) = &p.island {
        for v in [
            &island.progress,
            &island.transport,
            &island.lyrics,
            &island.stylesheet,
        ]
        .into_iter()
        .flatten()
        {
            if v.is_empty() || v.contains(['/', '\\', '"']) {
                out.push(format!(
                    "`{id}`: island view or file `{v}` is not a plain name"
                ));
            }
        }
    }
}

// ── the root it is generated against ───────────────────────────────────

/// What `dist/standard` takes from architect's root: its version, its
/// `[patch]` and `[profile]` tables, its dependency specs, its members.
struct Root {
    version: String,
    patch: toml::Table,
    profile: toml::Table,
    deps: toml::Table,
    /// Every workspace member but xtask: name → its directory, relative to
    /// the repo root.
    members: BTreeMap<String, String>,
}

fn root_of(root: &Path) -> Result<Root, String> {
    let path = root.join("Cargo.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let doc: toml::Table = text
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let table = |v: Option<&toml::Value>| v.and_then(toml::Value::as_table).cloned();
    let workspace = table(doc.get("workspace")).unwrap_or_default();
    let version = table(workspace.get("package"))
        .and_then(|p| p.get("version").and_then(|v| v.as_str()).map(str::to_owned))
        .ok_or("the root Cargo.toml has no workspace.package.version")?;
    Ok(Root {
        version,
        patch: table(doc.get("patch")).unwrap_or_default(),
        profile: table(doc.get("profile")).unwrap_or_default(),
        deps: table(workspace.get("dependencies")).unwrap_or_default(),
        members: members(root)?,
    })
}

/// The workspace's members (`cargo metadata --no-deps`), xtask aside.
fn members(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let out = Command::new(cargo())
        .current_dir(root)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata --no-deps: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let meta: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: {e}"))?;
    let mut members = BTreeMap::new();
    for p in meta
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let field = |k: &str| p.get(k).and_then(serde_json::Value::as_str);
        let (Some(name), Some(manifest)) = (field("name"), field("manifest_path")) else {
            continue;
        };
        if name == "xtask" {
            continue;
        }
        let dir = Path::new(manifest)
            .parent()
            .unwrap_or_else(|| Path::new(""));
        let rel = dir.strip_prefix(root).unwrap_or(dir);
        members.insert(name.to_owned(), slashes(rel));
    }
    Ok(members)
}

fn slashes(p: &Path) -> String {
    p.components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn cargo() -> std::ffi::OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}

// ── generation ─────────────────────────────────────────────────────────

/// A TOML key, bare when it can be.
fn key(k: &str) -> String {
    if is_name(k) {
        k.to_owned()
    } else {
        toml::Value::String(k.to_owned()).to_string()
    }
}

/// `v` on one line (inline tables and arrays), `path`s made relative to
/// `dist/standard` (two levels below the root).
fn inline(v: &toml::Value, rebase: bool) -> String {
    match v {
        toml::Value::Table(t) => {
            let fields: Vec<String> = t
                .iter()
                .map(|(k, v)| {
                    let v = match (k.as_str(), v.as_str()) {
                        ("path", Some(p)) if rebase => {
                            toml::Value::String(format!("../../{p}")).to_string()
                        }
                        _ => inline(v, rebase),
                    };
                    format!("{} = {v}", key(k))
                })
                .collect();
            format!("{{ {} }}", fields.join(", "))
        }
        toml::Value::Array(a) => format!(
            "[{}]",
            a.iter()
                .map(|v| inline(v, rebase))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => other.to_string(),
    }
}

fn quoted(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

/// A root `[workspace.dependencies]` spec, as a dependency line.
fn root_dep(root: &Root, name: &str) -> Result<String, String> {
    let spec = root
        .deps
        .get(name)
        .ok_or_else(|| format!("the root Cargo.toml has no workspace dependency `{name}`"))?;
    Ok(format!("{} = {}", key(name), inline(spec, true)))
}

fn member_path(root: &Root, name: &str) -> Result<String, String> {
    root.members
        .get(name)
        .map(|dir| format!("../../{dir}"))
        .ok_or_else(|| format!("architect has no member `{name}`"))
}

/// Who `e` is, for a comment.
fn about(e: &Entry) -> String {
    format!("{} {}: {} {} ({})", e.id, e.version, e.source, e.tag, e.rev)
}

/// `dist/standard/Cargo.toml`: the list's linked crates by their pins,
/// the root's patches and profiles, and `patched` (architect members by
/// name) patched back to this checkout under both of architect's URLs.
fn manifest(list: &List, root: &Root, patched: &BTreeSet<String>) -> Result<String, String> {
    let mut s = String::new();
    let mut line = |l: &str| {
        s.push_str(l);
        s.push('\n');
    };
    for l in [
        "# @generated by `cargo xtask core sync`: do not edit. It is made from",
        "# extensions/core-extensions.toml (the core extensions) and the root",
        "# Cargo.toml (its [patch] and [profile] tables, its members); change",
        "# those and run `cargo xtask core sync`. `cargo xtask core check` fails",
        "# while this file is out of date (docs/content/guide/core-extensions.md).",
        "#",
        "# The standard distribution of Architect (#631): the Architect app with",
        "# its core extensions. A workspace of its own, outside architect's: every",
        "# architect crate in its graph is patched back to this checkout, under",
        "# both spellings of its URL, so the build holds one Architect.",
        "#",
        "# This crate's own code is MIT OR Apache-2.0; the binary links each core",
        "# extension's crates under their own licences (Studio's: GPL-3.0-or-later).",
        "",
        "[package]",
    ] {
        line(l);
    }
    line(&format!("name = {}", quoted(PACKAGE)));
    line(&format!("version = {}", quoted(&root.version)));
    line("edition = \"2024\"");
    line("license = \"MIT OR Apache-2.0\"");
    line("publish = false");
    line(
        "description = \"The standard distribution of Architect: the Architect app with the core extensions extensions/core-extensions.toml lists, linked from their own repos at the tags it pins, and its packaged ones beside the binary.\"",
    );
    line("");
    line("[workspace]");
    line("");
    line("[[bin]]");
    line(&format!("name = {}", quoted(BIN)));
    line("path = \"src/main.rs\"");
    line("");
    line("[features]");
    line("# The window, as the Architect app has it. Off (`--no-default-features`),");
    line("# the binary is the headless node alone: no Blitz, wgpu or wayland.");
    line("default = [\"gui\"]");
    let desktop_only: Vec<String> = list
        .core
        .iter()
        .filter_map(|e| e.linked.as_ref())
        .filter(|l| {
            l.platforms.contains(&Platform::Desktop) && !l.platforms.contains(&Platform::Node)
        })
        .map(|l| quoted(&format!("dep:{}", l.krate)))
        .collect();
    let mut gui = vec![quoted("architect-desktop/gui")];
    gui.extend(desktop_only);
    line(&format!("gui = [{}]", gui.join(", ")));
    line("# The node draws the fences installed WASI components claim, as the");
    line("# release's node does (`cargo xtask dist node --features node-wasi`).");
    line("node-wasi = [\"architect-desktop/node-wasi\"]");
    line("");
    line("[dependencies]");
    line(&format!(
        "architect-desktop = {{ path = {}, default-features = false, features = [\"vox\"] }}",
        quoted(&member_path(root, "architect-desktop")?)
    ));
    line(&format!(
        "architect-sites = {{ path = {} }}",
        quoted(&member_path(root, "architect-sites")?)
    ));
    for dep in [
        "base64",
        "serde",
        "serde_json",
        "sha2",
        "thiserror",
        "tracing",
    ] {
        line(&root_dep(root, dep)?);
    }
    let linked: Vec<(&Entry, &Linked)> = list
        .core
        .iter()
        .filter_map(|e| e.linked.as_ref().map(|l| (e, l)))
        .filter(|(_, l)| l.platforms.iter().any(|p| p.built()))
        .collect();
    if !linked.is_empty() {
        line("");
        line("# The linked core extensions, each at the tag the list pins.");
    }
    for (e, l) in &linked {
        line(&format!("# {}", about(e)));
        let mut fields = vec![
            format!("git = {}", quoted(&e.source)),
            format!("tag = {}", quoted(&e.tag)),
        ];
        if !l.features.is_empty() {
            fields.push(format!(
                "features = [{}]",
                l.features
                    .iter()
                    .map(|f| quoted(f))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let desktop_only =
            l.platforms.contains(&Platform::Desktop) && !l.platforms.contains(&Platform::Node);
        if desktop_only {
            fields.push("optional = true".to_owned());
        }
        line(&format!("{} = {{ {} }}", key(&l.krate), fields.join(", ")));
    }
    line("");
    line("[dev-dependencies]");
    line("tempfile = \"3\"");

    // The patches: the root's (its forks), each linked extension's own,
    // then architect back to this checkout.
    let mut tables: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (url, table) in &root.patch {
        if ARCHITECT_GIT.contains(&url.as_str()) {
            continue;
        }
        let Some(table) = table.as_table() else {
            continue;
        };
        let entries = tables.entry(url.clone()).or_default();
        for (name, spec) in table {
            entries.insert(name.clone(), inline(spec, true));
        }
    }
    for (e, l) in &linked {
        for (url, names) in &l.patch {
            let entries = tables.entry(url.clone()).or_default();
            for name in names {
                let spec = format!(
                    "{{ git = {}, tag = {} }}",
                    quoted(&e.source),
                    quoted(&e.tag)
                );
                if let Some(old) = entries.insert(name.clone(), spec.clone())
                    && old != spec
                {
                    return Err(format!(
                        "`{}` patches `{name}` from {url}, which another patch already replaces",
                        e.id
                    ));
                }
            }
        }
    }
    for url in ARCHITECT_GIT {
        let entries = tables.entry(url.to_owned()).or_default();
        for name in patched {
            entries.insert(
                name.clone(),
                format!("{{ path = {} }}", quoted(&member_path(root, name)?)),
            );
        }
    }
    // crates-io first, as cargo writes it; the rest by URL.
    let mut order: Vec<&String> = tables.keys().collect();
    order.sort_by_key(|u| (u.as_str() != "crates-io", (*u).clone()));
    for url in order {
        let Some(entries) = tables.get(url) else {
            continue;
        };
        if entries.is_empty() {
            continue;
        }
        line("");
        if ARCHITECT_GIT.contains(&url.as_str()) {
            line("# One Architect: this checkout's, for every architect crate the graph has.");
        }
        if url == "crates-io" {
            line("[patch.crates-io]");
        } else {
            line(&format!("[patch.{}]", quoted(url)));
        }
        for (name, spec) in entries {
            line(&format!("{} = {spec}", key(name)));
        }
    }
    if !root.profile.is_empty() {
        let mut t = toml::Table::new();
        t.insert(
            "profile".to_owned(),
            toml::Value::Table(root.profile.clone()),
        );
        let profiles = toml::to_string(&t).map_err(|e| format!("the root's profiles: {e}"))?;
        line("");
        line("# The root's profiles: a patch and a profile apply only from a workspace root.");
        s.push_str(&profiles);
        if !s.ends_with('\n') {
            s.push('\n');
        }
    }
    Ok(s)
}

/// `dist/standard/src/list.rs`: what the list compiles in.
fn list_rs(list: &List) -> String {
    let mut s = String::from(
        "//! @generated by `cargo xtask core sync` from\n\
         //! `extensions/core-extensions.toml`: edit the list, not this file\n\
         //! (`cargo xtask core check` fails while they differ).\n\n",
    );
    let linked: Vec<(&Entry, &Linked)> = list
        .core
        .iter()
        .filter_map(|e| e.linked.as_ref().map(|l| (e, l)))
        .filter(|(_, l)| l.platforms.iter().any(|p| p.built()))
        .collect();
    let packaged: Vec<(&Entry, &Packaged)> = list
        .core
        .iter()
        .filter_map(|e| e.folder().map(|(p, _)| (e, p)))
        .collect();
    let island = packaged.iter().any(|(_, p)| p.island.is_some());
    s.push_str("use architect_desktop::Distribution;\n\n");
    s.push_str(if island {
        "use crate::packaged::{Island, Packaged};\n\n"
    } else {
        "use crate::packaged::Packaged;\n\n"
    });
    s.push_str(
        "/// `dist` with every linked core extension compiled in (on the node\n\
         /// build, without `gui`, and the desktop one, with it, as each entry's\n\
         /// platforms say).\n",
    );
    if linked.is_empty() {
        s.push_str("pub fn linked(dist: Distribution) -> Distribution {\n    dist\n}\n");
    } else {
        s.push_str(
            "pub fn linked(dist: Distribution) -> Distribution {\n    let mut dist = dist;\n",
        );
        for (e, l) in &linked {
            let desktop = l.platforms.contains(&Platform::Desktop);
            let node = l.platforms.contains(&Platform::Node);
            let cfg = match (desktop, node) {
                (true, false) => "    #[cfg(feature = \"gui\")]\n",
                (false, true) => "    #[cfg(not(feature = \"gui\"))]\n",
                _ => "",
            };
            let _ = writeln!(s, "    // {}", about(e));
            let _ = write!(
                s,
                "{cfg}    {{\n        dist = dist.linked(&{});\n",
                l.extension
            );
            if let Some(blocks) = &l.blocks {
                let _ = writeln!(s, "        dist = dist.block({blocks}());");
            }
            s.push_str("    }\n");
        }
        s.push_str("    dist\n}\n");
    }
    s.push_str(
        "\n/// The packaged core extensions this build uses, each read from\n\
         /// `core/<id>/` beside the binary (`cargo xtask core fetch`).\n",
    );
    if packaged.is_empty() {
        s.push_str("pub const PACKAGED: &[Packaged] = &[];\n");
        return s;
    }
    s.push_str("pub const PACKAGED: &[Packaged] = &[\n");
    let opt = |v: &Option<String>| {
        v.as_ref()
            .map_or_else(|| "None".to_owned(), |v| format!("Some({v:?})"))
    };
    for (e, p) in &packaged {
        let _ = writeln!(s, "    // {}", about(e));
        let _ = write!(
            s,
            "    Packaged {{\n        id: {:?},\n        version: {:?},\n",
            e.id, e.version
        );
        match &p.island {
            Some(i) => {
                let _ = write!(
                    s,
                    "        island: Some(Island {{\n            progress: {},\n            transport: {},\n            lyrics: {},\n            stylesheet: {},\n        }}),\n",
                    opt(&i.progress),
                    opt(&i.transport),
                    opt(&i.lyrics),
                    opt(&i.stylesheet)
                );
            }
            None => s.push_str("        island: None,\n"),
        }
        s.push_str("    },\n");
    }
    s.push_str("];\n");
    s
}

// ── the lock ───────────────────────────────────────────────────────────

/// A `Cargo.lock` package: name, version, source (none: a path).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Locked {
    name: String,
    version: String,
    source: Option<String>,
}

fn read_lock(path: &Path) -> Result<Vec<Locked>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let doc: toml::Table = text
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(doc
        .get("package")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| {
            Some(Locked {
                name: p.get("name")?.as_str()?.to_owned(),
                version: p.get("version")?.as_str()?.to_owned(),
                source: p
                    .get("source")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect())
}

/// The architect members the build has (path packages of the lock).
fn patched_from(lock: &[Locked], root: &Root) -> BTreeSet<String> {
    lock.iter()
        .filter(|p| p.source.is_none() && root.members.contains_key(&p.name))
        .map(|p| p.name.clone())
        .collect()
}

/// `git+<url>?…`: the package's git URL, if it is from git.
fn git_url(source: &str) -> Option<&str> {
    let rest = source.strip_prefix("git+")?;
    Some(rest.split(['?', '#']).next().unwrap_or(rest))
}

/// What breaks "one Architect, no Task, every linked crate on its pin"
/// in a resolved graph.
fn graph_problems(packages: &[Locked], list: &List) -> Vec<String> {
    let mut out = Vec::new();
    let mut second: BTreeSet<String> = BTreeSet::new();
    for p in packages {
        let Some(source) = &p.source else { continue };
        let url = git_url(source).unwrap_or_default();
        if ARCHITECT_GIT
            .iter()
            .any(|a| url.trim_end_matches(".git") == *a)
        {
            second.insert(format!("{} {} ({source})", p.name, p.version));
        }
        if source.contains(TASK_GIT) {
            out.push(format!(
                "{} {} is a Task crate ({source}): Task's crates pin an old Architect; the extension that pulls it must turn that feature off",
                p.name, p.version
            ));
        }
    }
    if !second.is_empty() {
        out.push(format!(
            "a second Architect: {} architect crate(s) from git, not this checkout (run `cargo xtask core sync`):\n    {}",
            second.len(),
            second.into_iter().collect::<Vec<_>>().join("\n    ")
        ));
    }
    for e in &list.core {
        let Some(l) = &e.linked else { continue };
        if !l.platforms.iter().any(|p| p.built()) {
            continue;
        }
        let want = format!("git+{}?tag={}#{}", e.source, e.tag, e.rev);
        let found: Vec<&Locked> = packages.iter().filter(|p| p.name == l.krate).collect();
        if !found
            .iter()
            .any(|p| p.source.as_deref() == Some(want.as_str()))
        {
            out.push(format!(
                "`{}`: crate `{}` is not resolved at its pin {want} (found: {})",
                e.id,
                l.krate,
                found
                    .iter()
                    .map(|p| p.source.clone().unwrap_or_else(|| "a path".to_owned()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    out
}

// ── the commands ───────────────────────────────────────────────────────

/// The repo root (where xtask's `Cargo.toml`'s parent is).
fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

pub fn run_cli(action: &Action) -> Result<(), String> {
    let root = repo();
    match action {
        Action::Check => check(&root),
        Action::Sync => sync(&root),
        Action::Verify => verify(&root),
        Action::Fetch { platform, into } => fetch(&root, *platform, into),
        Action::Graph => graph(&root),
    }
}

fn report(list: &List) {
    for e in &list.core {
        let mut forms = Vec::new();
        if let Some(l) = &e.linked {
            forms.push(format!(
                "linked ({})",
                l.platforms
                    .iter()
                    .map(|p| p.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(p) = &e.packaged {
            forms.push(format!(
                "packaged ({})",
                p.platforms
                    .iter()
                    .map(|p| p.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        println!(
            "core: {} {} by {}: {}",
            e.id,
            e.version,
            e.publisher,
            forms.join("; ")
        );
        let unbuilt: Vec<&str> = e
            .linked
            .iter()
            .flat_map(|l| l.platforms.iter())
            .chain(e.packaged.iter().flat_map(|p| p.platforms.iter()))
            .filter(|p| !p.built())
            .map(|p| p.name())
            .collect();
        if !unbuilt.is_empty() {
            println!(
                "core:   not built by dist/standard yet: {}",
                unbuilt.join(", ")
            );
        }
        if e.packaged.as_ref().is_some_and(|p| p.folder.is_none()) {
            println!("core:   .archext core packages are not bundled yet (#631)");
        }
    }
}

fn check(root: &Path) -> Result<(), String> {
    let list = read_list(root)?;
    report(&list);
    let base = root.join(STANDARD);
    let lock_path = base.join("Cargo.lock");
    let lock = read_lock(&lock_path)?;
    let roots = root_of(root)?;
    let mut problems = Vec::new();
    let want = manifest(&list, &roots, &patched_from(&lock, &roots))?;
    for (file, want) in [("Cargo.toml", want), (LIST_RS, list_rs(&list))] {
        let path = base.join(file);
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        if have != want {
            problems.push(format!(
                "{STANDARD}/{file} is not what the list makes: run `cargo xtask core sync`\n{}",
                first_difference(&have, &want)
            ));
        }
    }
    problems.extend(graph_problems(&lock, &list));
    if problems.is_empty() {
        println!(
            "core: {} entr(y/ies); {STANDARD} up to date; its lock holds one Architect",
            list.core.len()
        );
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

/// `cargo metadata` of `dist/standard`: resolves (and writes) its lock.
fn str_of<'a>(v: &'a serde_json::Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(serde_json::Value::as_str)
}

fn resolve(root: &Path) -> Result<Vec<Locked>, String> {
    let manifest = root.join(STANDARD).join("Cargo.toml");
    let mut cmd = Command::new(cargo());
    cmd.current_dir(root)
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(&manifest);
    let out = cmd.output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata --manifest-path {}: {}",
            manifest.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let meta: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: {e}"))?;
    // The resolved graph's packages (not the lock's leftovers).
    let ids: BTreeSet<&str> = meta
        .get("resolve")
        .and_then(|r| r.get("nodes"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|n| str_of(n, "id"))
        .collect();
    Ok(meta
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|p| str_of(p, "id").is_some_and(|id| ids.contains(id)))
        .filter_map(|p| {
            Some(Locked {
                name: str_of(p, "name")?.to_owned(),
                version: str_of(p, "version")?.to_owned(),
                source: str_of(p, "source").map(str::to_owned),
            })
        })
        .collect())
}

fn write(path: &Path, text: impl AsRef<[u8]>) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn sync(root: &Path) -> Result<(), String> {
    let list = read_list(root)?;
    report(&list);
    let roots = root_of(root)?;
    let base = root.join(STANDARD);
    write(&base.join(LIST_RS), &list_rs(&list))?;
    let lock = base.join("Cargo.lock");
    if !lock.exists() {
        // Start from architect's own resolution, so its crates build with
        // the versions (and the kache keys) its workspace has.
        std::fs::copy(root.join("Cargo.lock"), &lock)
            .map_err(|e| format!("{}: {e}", lock.display()))?;
    }
    // Every member patched, resolved; then only the ones the graph has.
    let all: BTreeSet<String> = roots.members.keys().cloned().collect();
    write(&base.join("Cargo.toml"), &manifest(&list, &roots, &all)?)?;
    resolve(root)?;
    let patched = patched_from(&read_lock(&lock)?, &roots);
    write(
        &base.join("Cargo.toml"),
        &manifest(&list, &roots, &patched)?,
    )?;
    let packages = resolve(root)?;
    let problems = graph_problems(&packages, &list);
    println!(
        "core: wrote {STANDARD} ({} architect crate(s) patched to this checkout)",
        patched.len()
    );
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

/// Where `have` and `want` first part, as two numbered lines: what a
/// check on a machine of its own (a CI runner) needs to say, since it
/// cannot be diffed there by hand.
fn first_difference(have: &str, want: &str) -> String {
    let (mut h, mut w) = (have.lines(), want.lines());
    for n in 1_usize.. {
        match (h.next(), w.next()) {
            (Some(a), Some(b)) if a == b => {}
            (None, None) => return "  (they differ only in line endings)".to_owned(),
            (a, b) => {
                return format!(
                    "  line {n}: committed {:?}\n  line {n}: the list makes {:?}",
                    a.unwrap_or("<end>"),
                    b.unwrap_or("<end>")
                );
            }
        }
    }
    String::new()
}

fn graph(root: &Path) -> Result<(), String> {
    let list = read_list(root)?;
    let packages = resolve(root)?;
    let roots = root_of(root)?;
    let path_architect = packages
        .iter()
        .filter(|p| p.source.is_none() && roots.members.contains_key(&p.name))
        .count();
    let mut sources: BTreeMap<String, usize> = BTreeMap::new();
    for p in &packages {
        let s = p
            .source
            .as_deref()
            .map_or("path", |s| s.split(['?', '#']).next().unwrap_or(s));
        let n = sources.entry(s.to_owned()).or_default();
        *n = n.saturating_add(1);
    }
    println!(
        "core: {} packages; {path_architect} architect crates, all from this checkout",
        packages.len()
    );
    for (s, n) in &sources {
        if !s.starts_with("registry+") {
            println!("core:   {n:>4}  {s}");
        }
    }
    let problems = graph_problems(&packages, &list);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

// ── fetching and verifying ─────────────────────────────────────────────

/// The largest file fetched (the archives are a node and its island).
const MAX_FETCH: u64 = 512 * 1024 * 1024;

fn download(url: &str) -> Result<Vec<u8>, String> {
    let out = Command::new("curl")
        .args(["-fsSL", "--max-time", "600", "--max-filesize"])
        .arg(MAX_FETCH.to_string())
        .arg(url)
        .output()
        .map_err(|e| format!("cannot run curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{url}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(bytes))
}

/// The release's sums, checked: signed by `e`'s publisher with the
/// pinned trusted comment.
fn signed_sums(e: &Entry) -> Result<String, String> {
    let sums = download(&e.signature.sums)?;
    let sig = download(&e.signature.minisig)?;
    let sig = String::from_utf8(sig).map_err(|_| format!("`{}`: its minisig is not text", e.id))?;
    let key = minisign_verify::PublicKey::from_base64(e.key())
        .map_err(|err| format!("`{}`: the key does not decode: {err}", e.id))?;
    let signature = minisign_verify::Signature::decode(&sig)
        .map_err(|err| format!("`{}`: the signature does not decode: {err}", e.id))?;
    key.verify(&sums, &signature, false).map_err(|err| {
        format!(
            "`{}`: {} is not signed by {}'s key: {err}",
            e.id, e.signature.sums, e.publisher
        )
    })?;
    if signature.trusted_comment() != e.signature.comment {
        return Err(format!(
            "`{}`: the signature's trusted comment is `{}`, the list pins `{}`",
            e.id,
            signature.trusted_comment(),
            e.signature.comment
        ));
    }
    String::from_utf8(sums).map_err(|_| format!("`{}`: its sums are not text", e.id))
}

/// The sha256 `sums` gives `name`.
fn listed<'a>(sums: &'a str, name: &str) -> Option<&'a str> {
    sums.lines().find_map(|l| {
        let (hash, file) = l.split_once(char::is_whitespace)?;
        (file.trim_start_matches([' ', '*']) == name).then_some(hash)
    })
}

fn file_name(url: &str) -> &str {
    url.rsplit('/').next().unwrap_or(url)
}

/// The commit `tag` names in `source` (peeled), by `git ls-remote`.
fn tag_rev(source: &str, tag: &str) -> Result<String, String> {
    let plain = format!("refs/tags/{tag}");
    let peeled = format!("{plain}^{{}}");
    let out = Command::new("git")
        .args(["ls-remote", source, &plain, &peeled])
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git ls-remote {source}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let find = |r: &str| {
        text.lines().find_map(|l| {
            let (rev, name) = l.split_once('\t')?;
            (name == r).then(|| rev.to_owned())
        })
    };
    find(&peeled)
        .or_else(|| find(&plain))
        .ok_or_else(|| format!("{source} has no tag {tag}"))
}

fn verify(root: &Path) -> Result<(), String> {
    let list = read_list(root)?;
    let mut problems = Vec::new();
    for e in &list.core {
        match tag_rev(&e.source, &e.tag) {
            Ok(rev) if rev == e.rev => {
                println!("core: {} {}: names {}", e.id, e.tag, e.rev);
            }
            Ok(rev) => problems.push(format!(
                "`{}`: {} names {rev} now, the list pins {}",
                e.id, e.tag, e.rev
            )),
            Err(err) => problems.push(format!("`{}`: {err}", e.id)),
        }
        match signed_sums(e) {
            Ok(sums) => {
                println!(
                    "core: {} {}: sums signed by {} (`{}`)",
                    e.id, e.tag, e.publisher, e.signature.comment
                );
                if let Some(p) = &e.packaged {
                    let name = file_name(&p.archive);
                    match listed(&sums, name) {
                        Some(h) if h == p.sha256 => {
                            println!("core: {} {name}: listed with its sha256", e.id);
                        }
                        Some(h) => problems.push(format!(
                            "`{}`: the signed sums give {name} {h}, the list pins {}",
                            e.id, p.sha256
                        )),
                        None => {
                            problems
                                .push(format!("`{}`: the signed sums do not list {name}", e.id));
                        }
                    }
                }
            }
            Err(err) => problems.push(err),
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

/// The files under `folder/` in the gzipped tar `archive`, by their path
/// inside it. Only regular files and folders; anything else (a link, a
/// device) or a path that leaves the folder is refused.
fn untar(archive: &[u8], folder: &str) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    let gz = flate2::read::GzDecoder::new(archive);
    let mut tar = tar::Archive::new(gz);
    let prefix = Path::new(folder);
    let mut out = Vec::new();
    for entry in tar.entries().map_err(|e| format!("the archive: {e}"))? {
        let mut entry = entry.map_err(|e| format!("the archive: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("the archive: {e}"))?
            .into_owned();
        let Ok(rest) = path.strip_prefix(prefix) else {
            continue;
        };
        if rest.as_os_str().is_empty() {
            continue;
        }
        if !rest.components().all(|c| matches!(c, Component::Normal(_))) {
            return Err(format!("the archive's {} leaves {folder}", path.display()));
        }
        match entry.header().entry_type() {
            tar::EntryType::Directory => {}
            tar::EntryType::Regular => {
                let mut bytes = Vec::new();
                entry
                    .by_ref()
                    .take(MAX_FETCH)
                    .read_to_end(&mut bytes)
                    .map_err(|e| format!("the archive's {}: {e}", path.display()))?;
                out.push((rest.to_path_buf(), bytes));
            }
            other => {
                return Err(format!(
                    "the archive's {} is not a file or a folder ({other:?})",
                    path.display()
                ));
            }
        }
    }
    if out.is_empty() {
        return Err(format!("the archive has nothing under {folder}"));
    }
    Ok(out)
}

fn fetch(root: &Path, platform: Platform, into: &Path) -> Result<(), String> {
    let list = read_list(root)?;
    let core = into.join("core");
    let mut fetched = 0_usize;
    for e in &list.core {
        let Some((p, folder)) = e.folder() else {
            continue;
        };
        if !p.platforms.contains(&platform) {
            continue;
        }
        let archive = download(&p.archive)?;
        let got = sha256_hex(&archive);
        if got != p.sha256 {
            return Err(format!(
                "`{}`: {} is {got}, the list pins {}",
                e.id, p.archive, p.sha256
            ));
        }
        let sums = signed_sums(e)?;
        let name = file_name(&p.archive);
        if listed(&sums, name) != Some(p.sha256.as_str()) {
            return Err(format!(
                "`{}`: the signed sums do not give {name} the pinned sha256",
                e.id
            ));
        }
        let files = untar(&archive, folder)?;
        // `BUILT_FROM`, when the folder has one, names the commit it was
        // built from: it must be the list's.
        if let Some((_, built)) = files.iter().find(|(p, _)| p == Path::new("BUILT_FROM")) {
            let built = String::from_utf8_lossy(built).trim().to_owned();
            if built != e.rev && built != e.tag {
                return Err(format!(
                    "`{}`: its folder was built from {built}, the list pins {}",
                    e.id, e.rev
                ));
            }
        }
        let dir = core.join(&e.id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
        }
        for (rel, bytes) in &files {
            write(&dir.join(rel), bytes)?;
        }
        println!(
            "core: {} {} ({}): {} file(s) into {}",
            e.id,
            e.version,
            platform.name(),
            files.len(),
            dir.display()
        );
        fetched = fetched.saturating_add(1);
    }
    println!(
        "core: fetched {fetched} packaged core extension(s) for {}",
        platform.name()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST_TEXT: &str = include_str!("../../extensions/core-extensions.toml");

    #[test]
    fn the_list_keeps_its_rules() {
        let list = parse(LIST_TEXT).unwrap();
        assert!(list.core.iter().any(|e| e.id == "studio-setlists"));
    }

    fn entry(extra: &str) -> String {
        format!(
            r#"[[core]]
id = "x.y"
publisher = "p"
source = "https://example.com/x"
version = "1.2.3"
tag = "v1.2.3"
rev = "{rev}"
[core.signature]
key = "org"
sums = "https://example.com/SHA256SUMS"
minisig = "https://example.com/SHA256SUMS.minisig"
comment = "x v1.2.3 SHA256SUMS"
{extra}"#,
            rev = "a".repeat(40)
        )
    }

    const LINKED: &str = "[core.linked]\ncrate = \"x\"\nplatforms = [\"desktop\", \"node\"]\nextension = \"x::ext::X\"\n";

    #[test]
    fn an_entry_with_a_form_parses() {
        parse(&entry(LINKED)).unwrap();
    }

    #[test]
    fn the_rules_name_what_is_wrong() {
        let err = parse(&entry("")).unwrap_err();
        assert!(err.contains("no form"), "{err}");
        let both = format!(
            "{LINKED}[core.packaged]\nplatforms = [\"node\"]\narchive = \"https://e.com/a.archext\"\nsha256 = \"{}\"\n",
            "0".repeat(64)
        );
        let err = parse(&entry(&both)).unwrap_err();
        assert!(err.contains("both linked and packaged"), "{err}");
        let err = parse(&entry(LINKED).replace("tag = \"v1.2.3\"", "tag = \"v9\"")).unwrap_err();
        assert!(err.contains("does not name `version`"), "{err}");
        let err = parse(&entry(LINKED).replace("x v1.2.3 SHA256SUMS", "x SHA256SUMS")).unwrap_err();
        assert!(err.contains("does not name the tag"), "{err}");
        let twice = format!("{}{}", entry(LINKED), entry(LINKED));
        assert!(parse(&twice).unwrap_err().contains("listed twice"));
        let unknown = entry(LINKED).replace("[core.linked]", "grants = []\n[core.linked]");
        assert!(parse(&unknown).is_err(), "unknown fields are refused");
    }

    #[test]
    fn a_folder_must_stay_inside_its_archive() {
        let p = format!(
            "[core.packaged]\nplatforms = [\"node\"]\narchive = \"https://e.com/a.tar.gz\"\nsha256 = \"{}\"\nfolder = \"../etc\"\n",
            "0".repeat(64)
        );
        assert!(
            parse(&entry(&p))
                .unwrap_err()
                .contains("not a relative path")
        );
    }

    #[test]
    fn list_rs_links_each_entry_for_its_platforms() {
        let node_only = entry(LINKED).replace("[\"desktop\", \"node\"]", "[\"node\"]")
            + "blocks = \"x::share::blocks\"\n";
        let rs = list_rs(&parse(&node_only).unwrap());
        assert!(rs.contains("#[cfg(not(feature = \"gui\"))]"), "{rs}");
        assert!(rs.contains("dist.linked(&x::ext::X)"), "{rs}");
        assert!(rs.contains("dist.block(x::share::blocks())"), "{rs}");
        assert!(
            rs.contains("pub const PACKAGED: &[Packaged] = &[];"),
            "{rs}"
        );
    }

    #[test]
    fn the_graph_check_finds_a_second_architect_and_task() {
        let list = parse(&entry(LINKED)).unwrap();
        let pin = format!("git+https://example.com/x?tag=v1.2.3#{}", "a".repeat(40));
        let ok = vec![
            Locked {
                name: "x".into(),
                version: "1.2.3".into(),
                source: Some(pin),
            },
            Locked {
                name: "architect".into(),
                version: "0.1.0".into(),
                source: None,
            },
        ];
        assert!(graph_problems(&ok, &list).is_empty());
        let mut bad = ok;
        bad.push(Locked {
            name: "architect".into(),
            version: "0.1.0".into(),
            source: Some(
                "git+https://www.github.com/FastTrackStudios/architect?tag=v0.10.0#abc".into(),
            ),
        });
        bad.push(Locked {
            name: "task-dial".into(),
            version: "1.0.0".into(),
            source: Some("git+https://github.com/FastTrackStudios/task?branch=main#abc".into()),
        });
        let problems = graph_problems(&bad, &list).join("\n");
        assert!(problems.contains("a second Architect"), "{problems}");
        assert!(problems.contains("Task crate"), "{problems}");
    }

    #[test]
    fn sums_are_read_as_sha256sum_writes_them() {
        let sums = "abc  one.tar.gz\ndef *two.tar.gz\n";
        assert_eq!(listed(sums, "one.tar.gz"), Some("abc"));
        assert_eq!(listed(sums, "two.tar.gz"), Some("def"));
        assert_eq!(listed(sums, "three"), None);
    }
}
