//! `cargo xtask feature new <name>` — a new feature crate from the
//! zero-ceremony templates.
//!
//! Writes `features/<name>/<name>-proto` with an entity, a service and
//! its error, and `features/<name>/<name>-memory` with an in-memory
//! backend that implements both, so `cargo test -p <name>-memory` passes
//! before anyone has written a line. The Cargo manifests carry the
//! proto-crate profile (`vox` on by default, `server` optional) so nobody
//! has to remember it.

use std::fs;
use std::path::{Path, PathBuf};

/// Create the two crates. Refuses to overwrite: an existing feature
/// directory is a sign the name is taken, not an invitation.
pub fn feature_new(repo: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        || !name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
    {
        return Err(format!(
            "feature names are lowercase kebab-case crate names (`inventory`, `tour-dates`), got {name:?}"
        ));
    }
    let root = repo.join("features").join(name);
    if root.exists() {
        return Err(format!("{} already exists", root.display()));
    }
    let snake = name.replace('-', "_");
    let pascal = to_pascal(&snake);
    let entity = pascal.clone();
    let entity_snake = snake.clone();
    let service = format!("{pascal}Service");

    let mut written = Vec::new();
    let mut write = |path: PathBuf, contents: String| -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        fs::write(&path, contents).map_err(|e| format!("{}: {e}", path.display()))?;
        written.push(path);
        Ok(())
    };

    let proto = root.join(format!("{name}-proto"));
    write(
        proto.join("Cargo.toml"),
        format!(
            r#"[package]
name = "{name}-proto"
version.workspace = true
edition.workspace = true
publish = false
description = "The {name} contract: entities, services, errors — every wire is generated from here."

[dependencies]
architect.workspace = true
chrono.workspace = true
facet.workspace = true
thiserror.workspace = true
uuid.workspace = true
vox = {{ workspace = true, optional = true }}
sea-orm = {{ workspace = true, optional = true }}

# The proto-crate profile. `vox` is the RPC face (on by default); `server`
# adds the SeaORM storage and the migration. That is the whole contract.
[features]
default = ["vox"]
vox = ["dep:vox", "architect/vox"]
server = ["dep:sea-orm", "architect/server"]

[lints]
workspace = true
"#
        ),
    )?;
    write(
        proto.join("src").join("lib.rs"),
        format!(
            r#"//! The `{name}` contract. Three declarations; everything else — the vox
//! service, the HTTP routes, both clients, the storage, the migration —
//! is generated.

use uuid::Uuid;

/// The entity. `NoteCreate`/`Update`/`List`, the `{entity}Repo` trait,
/// its clients and its storage all come from this.
#[architect::entity(table_name = "{entity_snake}s", repo, events)]
pub struct {entity} {{
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[architect(filterable, sortable)]
    pub name: String,
}}

/// How this feature fails. `HttpError` says how each case looks over
/// HTTP; `From<TransportError>` lets the generated clients implement the
/// service trait themselves.
#[architect::wire]
#[derive(Eq, thiserror::Error)]
pub enum {pascal}Error {{
    #[error("not found")]
    NotFound,
    #[error("unreachable: {{0}}")]
    Unreachable(String),
}}

impl architect::http::HttpError for {pascal}Error {{
    fn status(&self) -> u16 {{
        match self {{
            Self::NotFound => 404,
            Self::Unreachable(_) => 503,
        }}
    }}
    fn code(&self) -> &'static str {{
        match self {{
            Self::NotFound => "not_found",
            Self::Unreachable(_) => "unreachable",
        }}
    }}
}}

impl From<architect::TransportError> for {pascal}Error {{
    fn from(e: architect::TransportError) -> Self {{
        Self::Unreachable(e.to_string())
    }}
}}

/// The service: `POST /{name}/<method-kebab>` over HTTP, the same
/// methods over vox, and clients for both that implement this trait.
#[architect::service]
pub trait {service} {{
    /// Look one up by name.
    async fn find(&self, name: String) -> Result<{entity}, {pascal}Error>;
}}

// The trait-prefixed mount names (`{service}Service`, `{snake}_service_layer`).
pub use prelude::*;
"#
        ),
    )?;

    let memory = root.join(format!("{name}-memory"));
    write(
        memory.join("Cargo.toml"),
        format!(
            r#"[package]
name = "{name}-memory"
version.workspace = true
edition.workspace = true
publish = false
description = "In-memory {name} backend — tests, demos, offline."

[dependencies]
architect = {{ workspace = true, features = ["vox", "local", "http-client"] }}
{name}-proto = {{ path = "../{name}-proto" }}
uuid.workspace = true

[dev-dependencies]
tokio.workspace = true

[lints]
workspace = true
"#
        ),
    )?;
    write(
        memory.join("src").join("lib.rs"),
        format!(
            r#"//! An in-memory `{name}` backend: one struct implementing the repo and
//! the service, and one `Services` impl naming its bundle.

use std::sync::{{Arc, Mutex}};

use architect::{{Filter, Page, RepoError, Sort}};
use {snake}_proto::*;
use uuid::Uuid;

#[derive(Clone, Default)]
pub struct Memory {{
    rows: Arc<Mutex<Vec<{entity}>>>,
}}

impl {service} for Memory {{
    async fn find(&self, name: String) -> Result<{entity}, {pascal}Error> {{
        architect::lock(&self.rows)
            .iter()
            .find(|r| r.name == name)
            .cloned()
            .ok_or({pascal}Error::NotFound)
    }}
}}

impl {entity}Repo for Memory {{
    async fn get(&self, id: Uuid) -> Result<{entity}, RepoError> {{
        architect::lock(&self.rows)
            .iter()
            .find(|r| r.id == id)
            .cloned()
            .ok_or(RepoError::NotFound)
    }}
    async fn list(&self, page: Page, _sort: Option<Sort>, _filter: Option<Filter>) -> Result<{entity}List, RepoError> {{
        let items = architect::lock(&self.rows).clone();
        let total = u32::try_from(items.len()).unwrap_or(u32::MAX);
        Ok({entity}List {{ items, total, page }})
    }}
    async fn create(&self, input: {entity}Create) -> Result<{entity}, RepoError> {{
        let row = {entity} {{ id: Uuid::new_v4(), name: input.name }};
        architect::lock(&self.rows).push(row.clone());
        Ok(row)
    }}
    async fn update(&self, id: Uuid, input: {entity}Update) -> Result<{entity}, RepoError> {{
        let mut rows = architect::lock(&self.rows);
        let row = rows.iter_mut().find(|r| r.id == id).ok_or(RepoError::NotFound)?;
        if let Some(name) = input.name {{
            row.name = name;
        }}
        Ok(row.clone())
    }}
    async fn delete(&self, id: Uuid) -> Result<(), RepoError> {{
        let mut rows = architect::lock(&self.rows);
        let before = rows.len();
        rows.retain(|r| r.id != id);
        if rows.len() == before {{
            return Err(RepoError::NotFound);
        }}
        Ok(())
    }}
}}

impl architect::Services for Memory {{
    fn layers() -> impl architect::Layer<Self> {{
        architect::layers![{service}Service, {entity}RepoLayer]
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;
    use architect::{{LocalServer, Scope, Services as _}};

    /// The scenario is generic over the trait: the in-process backend and
    /// the vox client both satisfy it.
    async fn finds_what_it_created(s: &(impl {service} + {entity}Repo)) {{
        let made = s.create({entity}Create {{ name: "one".into() }}).await.unwrap();
        assert_eq!(s.find("one".into()).await.unwrap().id, made.id);
        assert_eq!(s.find("two".into()).await.unwrap_err(), {pascal}Error::NotFound);
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
        let service: {service}Client = local.establish().await.unwrap();
        let repo: {entity}RepoClient = local.establish().await.unwrap();
        let made = repo.create({entity}Create {{ name: "one".into() }}).await.unwrap();
        assert_eq!(service.find("one".into()).await.unwrap().id, made.id);
        scope.close().await;
    }}
}}
"#
        ),
    )?;
    Ok(written)
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
