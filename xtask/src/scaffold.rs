//! `cargo xtask feature new <name>` — a new feature, in the one-crate
//! layout (`docs/content/architecture/layout.md`, STA-257).
//!
//! Writes ONE crate at `features/<name>/`:
//!
//! - `src/lib.rs`: the contract (an entity, a service, its error). The
//!   default feature set, wasm-clean.
//! - `src/memory.rs`: the in-memory backend, behind the `memory` flag.
//! - `src/ui.rs`: an empty module behind the `ui` flag.
//! - `spec/service.md`: the service's rules, for tracey.
//! - `tests/memory.rs`: the scenario, in process and over vox. It passes
//!   before anyone has written a line.
//!
//! Then it adds the crate to the workspace `members` and to
//! `[workspace.dependencies]` with `default-features = false` (cargo
//! cannot turn defaults off on a workspace-inherited dep, so the entry
//! has to). There are no `-proto` / `-memory` / `-ui` sibling crates.

use std::fs;
use std::path::{Path, PathBuf};

/// Create the crate and register it in `repo/Cargo.toml`. Refuses to
/// overwrite: an existing feature directory is a sign the name is
/// taken, not an invitation.
pub fn feature_new(repo: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    check_name(name)?;
    let manifest = repo.join("Cargo.toml");
    let workspace = fs::read_to_string(&manifest).map_err(|e| {
        format!(
            "{}: {e} (run this from the workspace root)",
            manifest.display()
        )
    })?;
    let root = repo.join("features").join(name);
    if root.exists() {
        return Err(format!("{} already exists", root.display()));
    }
    let registered = register(&workspace, name)?;

    let mut written = Vec::new();
    for (relative, contents) in files(name) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        fs::write(&path, contents).map_err(|e| format!("{}: {e}", path.display()))?;
        written.push(path);
    }
    fs::write(&manifest, registered).map_err(|e| format!("{}: {e}", manifest.display()))?;
    format_rust(&written);
    written.push(manifest);
    Ok(written)
}

/// Run rustfmt over the generated sources, so the templates do not have
/// to guess where a long type name wraps. Best effort: a missing rustfmt
/// leaves correct, merely unformatted code.
fn format_rust(paths: &[PathBuf]) {
    let sources: Vec<&PathBuf> = paths
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .collect();
    let rustfmt = std::env::var_os("RUSTFMT").unwrap_or_else(|| "rustfmt".into());
    let formatted = std::process::Command::new(rustfmt)
        .args(["--edition", "2024"])
        .args(&sources)
        .status()
        .is_ok_and(|s| s.success());
    if !formatted {
        eprintln!("note: rustfmt did not run; `cargo fmt` the new crate");
    }
}

/// Feature names are lowercase kebab-case crate names.
fn check_name(name: &str) -> Result<(), String> {
    let valid = name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.ends_with('-')
        && !name.contains("--");
    if valid {
        Ok(())
    } else {
        Err(format!(
            "feature names are lowercase kebab-case crate names (`inventory`, `tour-dates`), got {name:?}"
        ))
    }
}

/// The workspace manifest with `features/<name>` added to `members` and
/// `<name>` added to `[workspace.dependencies]`. Edits the text, so the
/// manifest's comments and ordering survive.
fn register(workspace: &str, name: &str) -> Result<String, String> {
    let member = format!("features/{name}");
    let dep_key = format!("{name} = ");
    if workspace.contains(&format!("\"{member}\"")) {
        return Err(format!("{member} is already a workspace member"));
    }
    if workspace
        .lines()
        .any(|l| l.trim_start().starts_with(&dep_key))
    {
        return Err(format!("`{name}` is already a workspace dependency"));
    }
    let with_member = add_member(workspace, &member)?;
    add_workspace_dep(
        &with_member,
        &format!("{name} = {{ path = \"{member}\", default-features = false }}"),
    )
}

/// Insert `"<member>",` as the last entry of `[workspace] members`.
fn add_member(workspace: &str, member: &str) -> Result<String, String> {
    let mut out = Vec::new();
    let mut in_members = false;
    let mut done = false;
    for line in workspace.lines() {
        let trimmed = line.trim();
        if !done && !in_members && trimmed.starts_with("members") && trimmed.contains('[') {
            let open = trimmed.split_once('[').map_or("", |(_, rest)| rest);
            if let Some(inline) = open.strip_suffix(']') {
                // `members = []` or `members = ["a", "b"]` on one line.
                out.push("members = [".to_string());
                for entry in inline.split(',').map(str::trim).filter(|e| !e.is_empty()) {
                    out.push(format!("    {entry},"));
                }
                out.push(format!("    \"{member}\","));
                out.push("]".to_string());
                done = true;
                continue;
            }
            in_members = true;
        } else if in_members && trimmed.starts_with(']') {
            out.push(format!("    \"{member}\","));
            in_members = false;
            done = true;
        }
        out.push(line.to_string());
    }
    if done {
        Ok(join_lines(&out, workspace))
    } else {
        Err("no `members = [...]` list in the workspace Cargo.toml".to_string())
    }
}

/// Append `entry` to the end of `[workspace.dependencies]`, above any
/// blank or comment lines that introduce the next table.
fn add_workspace_dep(workspace: &str, entry: &str) -> Result<String, String> {
    let lines: Vec<&str> = workspace.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.trim() == "[workspace.dependencies]")
        .ok_or("no [workspace.dependencies] table in the workspace Cargo.toml")?;
    let body = start.saturating_add(1);
    let next_table = lines
        .iter()
        .skip(body)
        .position(|l| l.trim_start().starts_with('['))
        .map_or(lines.len(), |offset| body.saturating_add(offset));
    let mut at = next_table;
    while at > body
        && lines
            .get(at.saturating_sub(1))
            .is_some_and(|l| l.trim().is_empty() || l.trim_start().starts_with('#'))
    {
        at = at.saturating_sub(1);
    }
    let mut out: Vec<String> = lines.iter().map(ToString::to_string).collect();
    out.insert(at, entry.to_string());
    Ok(join_lines(&out, workspace))
}

fn join_lines(lines: &[String], original: &str) -> String {
    let mut text = lines.join("\n");
    if original.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// Every file of the new crate, relative to `features/<name>/`.
fn files(name: &str) -> Vec<(&'static str, String)> {
    let snake = name.replace('-', "_");
    let pascal = to_pascal(&snake);
    let service = format!("{pascal}Service");
    vec![
        ("Cargo.toml", cargo_toml(name)),
        ("src/lib.rs", lib_rs(name, &snake, &pascal, &service)),
        ("src/memory.rs", memory_rs(name, &pascal, &service)),
        ("src/ui.rs", ui_rs(name)),
        ("spec/service.md", spec_md(name, &service)),
        ("tests/memory.rs", test_rs(name, &snake, &pascal, &service)),
    ]
}

fn cargo_toml(name: &str) -> String {
    format!(
        r#"[package]
name = "{name}"
version.workspace = true
edition.workspace = true
publish = false
description = "The {name} feature: its contract, with backends and UI as cargo features."

# One crate per feature (docs/content/architecture/layout.md). The
# default feature set is the contract and stays wasm-clean: no tokio
# runtime, sea-orm, axum, iroh or dioxus without a flag.
[dependencies]
architect.workspace = true
facet.workspace = true
thiserror.workspace = true
uuid.workspace = true
vox = {{ workspace = true, optional = true }}

[features]
default = []
# L0: the vox client and server stubs for the service.
vox = ["dep:vox", "architect/vox"]
# L1/L2: the in-memory backend (`{name}::memory::Memory`).
memory = []
# L4: Dioxus components. Add `dep:dioxus` and `architect/client` with the
# first component.
ui = []

# The scenario test needs the backend and the vox face; a dev-dependency
# on this crate turns them on for test builds only, so `cargo test -p
# {name}` runs it while the library's defaults stay the contract.
[dev-dependencies]
{name} = {{ path = ".", features = ["memory", "vox"] }}
architect = {{ workspace = true, features = ["local"] }}
tokio.workspace = true

[lints]
workspace = true
"#
    )
}

fn lib_rs(name: &str, snake: &str, pascal: &str, service: &str) -> String {
    format!(
        r#"//! The `{name}` contract. Three declarations; everything else (the vox
//! service, the HTTP routes, both clients, the storage, the migration)
//! is generated from them.
//!
//! This file is the default feature set: it compiles for
//! `wasm32-unknown-unknown` with no features. Backends and UI are cargo
//! features of this crate (`memory`, `ui`), not sibling crates.

// Without `vox` the service trait is a plain `async fn` trait, which
// rustc flags on wasm; the generated clients add the bounds they need.
#![allow(async_fn_in_trait)]

use uuid::Uuid;

/// The entity. `{pascal}Create` / `Update` / `List`, the `{pascal}Repo`
/// trait, its clients and its storage all come from this.
#[architect::entity(table_name = "{snake}s", repo, events)]
#[derive(Eq)]
pub struct {pascal} {{
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[architect(filterable, sortable)]
    pub name: String,
}}

/// How this feature fails. The status, the code and the transport
/// variant (what lets the generated clients implement the service trait)
/// are written by the attribute.
#[architect::error]
#[derive(Eq)]
pub enum {pascal}Error {{
    #[error("not found")]
    NotFound,
}}

/// The service: `POST /{name}/<method-kebab>` over HTTP, the same
/// methods over vox, and clients for both that implement this trait.
#[architect::service]
pub trait {service} {{
    /// Look one up by name.
    async fn find(&self, name: String) -> Result<{pascal}, {pascal}Error>;
}}

// The trait-prefixed mount names (`{service}Service`, ...).
pub use prelude::*;

/// The in-memory backend: tests, demos, offline.
#[cfg(feature = "memory")]
pub mod memory;

/// Dioxus components for this feature.
#[cfg(feature = "ui")]
pub mod ui;
"#
    )
}

fn memory_rs(name: &str, pascal: &str, service: &str) -> String {
    format!(
        r#"//! An in-memory `{name}` backend: one struct implementing the repo and
//! the service, and (with `vox`) one `Services` impl naming its bundle.

use std::sync::{{Arc, Mutex}};

use architect::{{Filter, Page, RepoError, Sort}};
use uuid::Uuid;

use crate::{{
    {pascal}, {pascal}Create, {pascal}Error, {pascal}List, {pascal}Repo, {pascal}Update, {service},
}};

#[derive(Clone, Default)]
pub struct Memory {{
    rows: Arc<Mutex<Vec<{pascal}>>>,
}}

impl {service} for Memory {{
    // r[impl {name}.service.find]
    async fn find(&self, name: String) -> Result<{pascal}, {pascal}Error> {{
        architect::lock(&self.rows)
            .iter()
            .find(|r| r.name == name)
            .cloned()
            .ok_or({pascal}Error::NotFound)
    }}
}}

impl {pascal}Repo for Memory {{
    async fn get(&self, id: Uuid) -> Result<{pascal}, RepoError> {{
        architect::lock(&self.rows)
            .iter()
            .find(|r| r.id == id)
            .cloned()
            .ok_or(RepoError::NotFound)
    }}

    async fn list(
        &self,
        page: Page,
        _sort: Option<Sort>,
        _filter: Option<Filter>,
    ) -> Result<{pascal}List, RepoError> {{
        let items = architect::lock(&self.rows).clone();
        let total = u32::try_from(items.len()).unwrap_or(u32::MAX);
        Ok({pascal}List {{ items, total, page }})
    }}

    async fn create(&self, input: {pascal}Create) -> Result<{pascal}, RepoError> {{
        let row = {pascal} {{
            id: Uuid::new_v4(),
            name: input.name,
        }};
        architect::lock(&self.rows).push(row.clone());
        Ok(row)
    }}

    async fn update(&self, id: Uuid, input: {pascal}Update) -> Result<{pascal}, RepoError> {{
        let mut rows = architect::lock(&self.rows);
        let row = rows
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or(RepoError::NotFound)?;
        if let Some(name) = input.name {{
            row.name = name;
        }}
        let updated = row.clone();
        drop(rows);
        Ok(updated)
    }}

    async fn delete(&self, id: Uuid) -> Result<(), RepoError> {{
        let mut rows = architect::lock(&self.rows);
        let before = rows.len();
        rows.retain(|r| r.id != id);
        let removed = rows.len() != before;
        drop(rows);
        if removed {{ Ok(()) }} else {{ Err(RepoError::NotFound) }}
    }}
}}

#[cfg(feature = "vox")]
impl architect::Services for Memory {{
    fn layers() -> impl architect::Layer<Self> {{
        architect::layers![crate::{service}Service, crate::{pascal}RepoLayer]
    }}
}}
"#
    )
}

fn ui_rs(name: &str) -> String {
    format!(
        r"//! `{name}` components. Empty until the first one lands; it depends on
//! the contract and `architect[client]` only, never on a backend.
"
    )
}

fn spec_md(name: &str, service: &str) -> String {
    format!(
        r"# {name}: `{service}`

Rules for the `{name}` contract. tracey links each rule to the code
that implements it (`// r[impl ...]`) and the test that verifies it
(`// r[verify ...]`); see `docs/content/architecture/specs.md`.

r[{name}.service.find]
`find` MUST return the row whose `name` equals the argument, and
`NotFound` when there is none.
"
    )
}

fn test_rs(name: &str, snake: &str, pascal: &str, service: &str) -> String {
    format!(
        r#"//! The scenario, against the in-memory backend: in process and over vox.
//! It is generic over the traits, so every wire runs the same checks.

#![allow(clippy::expect_used)]

use architect::{{LocalServer, Scope, Services as _}};
use {snake}::memory::Memory;
use {snake}::{{
    {pascal}Create, {pascal}Error, {pascal}Repo, {pascal}RepoClient, {service}, {service}Client,
}};

async fn finds_what_it_created(s: &(impl {service} + {pascal}Repo)) {{
    let made = s
        .create({pascal}Create {{ name: "one".into() }})
        .await
        .expect("create");
    // r[verify {name}.service.find]
    assert_eq!(s.find("one".into()).await.expect("find").id, made.id);
    assert_eq!(
        s.find("two".into()).await.expect_err("missing"),
        {pascal}Error::NotFound
    );
}}

#[tokio::test]
async fn in_process() {{
    finds_what_it_created(&Memory::default()).await;
}}

#[tokio::test]
async fn over_vox() {{
    let scope = Scope::new();
    let local = LocalServer::serve(Memory::default().into_router(), scope.clone());
    // Two typed clients over one backend; each implements its trait.
    let service: {service}Client = local.establish().await.expect("service client");
    let repo: {pascal}RepoClient = local.establish().await.expect("repo client");
    let made = repo
        .create({pascal}Create {{ name: "one".into() }})
        .await
        .expect("create");
    assert_eq!(service.find("one".into()).await.expect("find").id, made.id);
    scope.close().await;
}}
"#
    )
}

fn to_pascal(snake: &str) -> String {
    snake
        .split('_')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut c = s.chars();
            c.next()
                .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                .unwrap_or_default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKSPACE: &str = r#"[workspace]
resolver = "2"
members = [
    "crates/architect",
    # ── repo tooling ──
    "xtask",
]

[workspace.dependencies]
architect = { path = "crates/architect" }
uuid = "1"

# ── Patches ──
[patch.crates-io]
"#;

    #[test]
    fn registers_member_and_dependency() {
        let out = register(WORKSPACE, "tour-dates").expect("register");
        assert!(out.contains("    \"xtask\",\n    \"features/tour-dates\",\n]"));
        assert!(out.contains(
            "uuid = \"1\"\ntour-dates = { path = \"features/tour-dates\", default-features = false }\n\n# ── Patches ──"
        ));
        assert!(
            register(&out, "tour-dates").is_err(),
            "a second run refuses"
        );
    }

    #[test]
    fn registers_into_an_inline_members_list() {
        let out = add_member("[workspace]\nmembers = []\n", "features/x").expect("member");
        assert_eq!(out, "[workspace]\nmembers = [\n    \"features/x\",\n]\n");
    }

    #[test]
    fn rejects_bad_names() {
        for bad in ["", "Inventory", "1st", "tour_dates", "tour-", "a--b"] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
        assert!(check_name("tour-dates2").is_ok());
    }

    #[test]
    fn one_crate_no_siblings() {
        let files = files("tour-dates");
        assert!(files.iter().all(|(path, _)| !path.contains("proto")));
        let (_, manifest) = files
            .iter()
            .find(|(path, _)| *path == "Cargo.toml")
            .expect("manifest");
        assert!(manifest.contains("name = \"tour-dates\""));
        assert!(manifest.contains("default = []"));
    }
}
