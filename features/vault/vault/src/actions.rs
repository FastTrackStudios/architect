//! The vault's actions (feature `actions`).
//!
//! For a person in ⌘K and an agent over the control socket or MCP, over
//! one open folder of markdown ([`OpenVault`]):
//!
//! | action | does |
//! |---|---|
//! | `VAULT_STATUS` | the folder, how many notes, bases and attachments it holds, the bases that failed to parse and the links that resolve to nothing |
//! | `VAULT_LIST {folder?}` | its notes (path, title, folder), in path order, under `folder` when given |
//! | `VAULT_READ {path}` | one note: its properties (frontmatter), body, tags, the links out of it (and where each resolves) and the notes that link to it |
//! | `VAULT_SEARCH {query, limit?}` | the notes whose title or text holds `query` (any case), each with its first matching line |
//! | `VAULT_QUERY_BASE {path, view?}` | runs a `.base` (one view or all): each view's columns and rows |
//! | `VAULT_CREATE {path, body, properties?}` | writes a new note (refused when the path is taken) |
//! | `VAULT_SET_PROPERTY {path, key, value}` | sets one property; `value` is JSON (`3`, `["a","b"]`, `true`) or else text |
//! | `VAULT_REMOVE_PROPERTY {path, key}` | removes one property |
//! | `VAULT_APPEND {path, text}` | adds text to the end of a note |
//! | `VAULT_RENAME {from, to}` | renames or moves a note, rewriting the links that named it |
//! | `VAULT_DELETE {path}` | deletes a note (destructive: an agent's call waits for a person's yes) |
//! | `VAULT_RELOAD` | reads the folder again (after edits made outside the app) |
//!
//! Every write goes to disk through the vault's own write path
//! (`vault.write.atomic`) and leaves the rest of the file as written.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect, RawJson};
use facet::Facet;
use serde_json::{Map, Value, json};
use vault_live::refs::Ref;
use vault_obsidian::obsidian_parse::{FrontmatterEntry, parse_frontmatter};
use vault_obsidian::{LinkIndex, SelfWriteGuard, Vault, VaultPage, mutate};

mod edit;
pub use edit::{Changes, Renamed, Watch, folders_in};
/// A change to a note's tags ([`OpenVault::edit_tags`], [`OpenVault::edit_tags_everywhere`]).
pub use vault_obsidian::tag_edit::TagEdit;

/// The action ids, to name them from code.
pub mod ids {
    pub const STATUS: &str = "VAULT_STATUS";
    pub const LIST: &str = "VAULT_LIST";
    pub const READ: &str = "VAULT_READ";
    pub const SEARCH: &str = "VAULT_SEARCH";
    pub const QUERY_BASE: &str = "VAULT_QUERY_BASE";
    pub const CREATE: &str = "VAULT_CREATE";
    pub const SET_PROPERTY: &str = "VAULT_SET_PROPERTY";
    pub const REMOVE_PROPERTY: &str = "VAULT_REMOVE_PROPERTY";
    pub const APPEND: &str = "VAULT_APPEND";
    pub const DELETE: &str = "VAULT_DELETE";
    pub const RENAME: &str = "VAULT_RENAME";
    pub const RELOAD: &str = "VAULT_RELOAD";
}

/// One folder of markdown, open as a vault: the parsed snapshot the
/// actions read and write. Cheap to clone; clones share the vault.
#[derive(Clone)]
pub struct OpenVault {
    vault: Arc<RwLock<Vault>>,
    guard: Arc<SelfWriteGuard>,
    /// Its folders as last read ([`Self::refresh`] says which came and
    /// went).
    folders: Arc<RwLock<BTreeSet<String>>>,
}

impl std::fmt::Debug for OpenVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenVault")
            .field("root", &self.root())
            .finish_non_exhaustive()
    }
}

impl OpenVault {
    /// Reads the folder at `root` (its notes, bases and attachments,
    /// honouring `.obsidian/app.json`'s ignore filters).
    ///
    /// # Errors
    ///
    /// The folder cannot be read.
    pub fn open(root: &Path) -> Result<Self, vault_obsidian::LoadError> {
        Ok(Self {
            vault: Arc::new(RwLock::new(Vault::open(root)?)),
            guard: Arc::new(SelfWriteGuard::new()),
            folders: Arc::new(RwLock::new(edit::folders_in(root))),
        })
    }

    /// The folder.
    #[must_use]
    pub fn root(&self) -> PathBuf {
        self.read().root.clone()
    }

    /// The write guard the actions mark their own writes with, for a
    /// watcher to tell them from edits made outside the app.
    #[must_use]
    pub fn guard(&self) -> &SelfWriteGuard {
        &self.guard
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Vault> {
        self.vault
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Vault> {
        self.vault
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The notes with every word of `query` in their title or text, as
    /// the vault's full-text search finds them
    /// (`vault_live::sync::search_pages`): title matches first, at most
    /// `limit`.
    #[must_use]
    pub fn search_text(&self, query: &str, limit: usize) -> Vec<vault_proto::TextHit> {
        self.with(|v| {
            vault_live::sync::search_pages(
                v.pages
                    .iter()
                    .map(|p| (p.raw.as_str(), p.rel_path.as_str(), title(p))),
                query,
                limit,
            )
        })
    }

    /// Runs `f` over the vault as it is now.
    pub fn with<R>(&self, f: impl FnOnce(&Vault) -> R) -> R {
        f(&self.read())
    }

    /// Runs `f` over the vault to change it, with the write guard its
    /// writes to disk are marked with (what `bases` saves a `.base`
    /// through).
    pub fn update<R>(&self, f: impl FnOnce(&mut Vault, &SelfWriteGuard) -> R) -> R {
        f(&mut self.write(), &self.guard)
    }

    /// Reads the folder again.
    ///
    /// # Errors
    ///
    /// The folder cannot be read; the vault is left as it was.
    pub fn reload(&self) -> Result<(), vault_obsidian::LoadError> {
        let root = self.root();
        let fresh = Vault::open(&root)?;
        *self.write() = fresh;
        Ok(())
    }
}

const fn meta(id: &'static str, name: &'static str, description: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "Vault",
        method_name: id,
        display_name: name,
        description,
        category: "Vault",
        group: "Vault",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

static STATUS: ActionMeta = meta(
    ids::STATUS,
    "Vault status",
    "The vault's folder and how many notes, bases and attachments it holds, with the bases \
     that failed to parse (path and reason) and the links that resolve to no note.",
);
static LIST: ActionMeta = meta(
    ids::LIST,
    "List notes",
    "The vault's notes in path order (path, title, folder); only those under `folder` when \
     it is given.",
);
static READ: ActionMeta = meta(
    ids::READ,
    "Read a note",
    "One note by its vault path: its properties (frontmatter, as JSON), its body, its tags \
     (frontmatter and inline #tags), the links out of it with the note each resolves to, and \
     the notes that link to it.",
);
static SEARCH: ActionMeta = meta(
    ids::SEARCH,
    "Search notes",
    "The notes whose title or text holds `query` (any case), each with its first matching \
     line; at most `limit` (default 20).",
);
static QUERY_BASE: ActionMeta = meta(
    ids::QUERY_BASE,
    "Run a base",
    "Runs a `.base` file in the vault over its notes: one view by name, or every view. Each \
     view's columns and its rows, grouped as the view groups them.",
);
static CREATE: ActionMeta = meta(
    ids::CREATE,
    "New note",
    "Writes a new note at `path` (a vault path ending in .md) with `body` and, optionally, \
     `properties` (a JSON object, written as frontmatter). Refused when the path is taken.",
);
static SET_PROPERTY: ActionMeta = meta(
    ids::SET_PROPERTY,
    "Set a property",
    "Sets one property of a note's frontmatter. `value` is read as JSON (3, true, \
     [\"a\",\"b\"], {\"k\":1}) and, when it is not JSON, as text. The rest of the file is left \
     as written.",
);
static REMOVE_PROPERTY: ActionMeta = meta(
    ids::REMOVE_PROPERTY,
    "Remove a property",
    "Removes one property from a note's frontmatter.",
);
static APPEND: ActionMeta = meta(
    ids::APPEND,
    "Append to a note",
    "Adds `text` to the end of a note, on a line of its own.",
);
static DELETE: ActionMeta = meta(ids::DELETE, "Delete a note", "Deletes a note from disk.");
static RENAME: ActionMeta = meta(
    ids::RENAME,
    "Rename a note",
    "Renames or moves the note at `from` to `to` (a vault path ending in .md) and rewrites every \
wikilink that named it, in every note, to name it there. Answers the notes whose links changed.",
);
static RELOAD: ActionMeta = meta(
    ids::RELOAD,
    "Reload the vault",
    "Reads the vault's folder again, for edits made outside the app.",
);

#[derive(Facet)]
struct ListArgs {
    /// Only notes under this folder (a vault path).
    #[facet(default)]
    folder: Option<String>,
}

#[derive(Facet)]
struct PathArgs {
    /// A note's vault path (`Notes/Welcome.md`).
    path: String,
}

#[derive(Facet)]
struct RenameArgs {
    /// The note's vault path now.
    from: String,
    /// Its vault path after (`Notes/New name.md`; another folder moves it).
    to: String,
}

#[derive(Facet)]
struct SearchArgs {
    query: String,
    #[facet(default)]
    limit: Option<u32>,
}

#[derive(Facet)]
struct BaseArgs {
    /// The `.base` file's vault path.
    path: String,
    /// One view by name (any case); every view when left out.
    #[facet(default)]
    view: Option<String>,
}

#[derive(Facet)]
struct CreateArgs {
    path: String,
    #[facet(default)]
    body: Option<String>,
    /// A JSON object, written as the note's frontmatter.
    #[facet(default)]
    properties: Option<String>,
}

#[derive(Facet)]
struct SetPropertyArgs {
    path: String,
    key: String,
    value: String,
}

#[derive(Facet)]
struct KeyArgs {
    path: String,
    key: String,
}

#[derive(Facet)]
struct AppendArgs {
    path: String,
    text: String,
}

fn raw(v: &Value) -> RawJson {
    RawJson::from(v.to_string())
}

/// A note's frontmatter as one JSON object.
fn properties(page: &VaultPage) -> Map<String, Value> {
    page.parsed
        .frontmatter
        .iter()
        .map(|e| (e.key.clone(), e.value.clone()))
        .collect()
}

/// Its `title` property, else its file name.
fn title(page: &VaultPage) -> String {
    page.parsed
        .frontmatter
        .iter()
        .find(|e| e.key == "title")
        .and_then(|e| e.value.as_str())
        .map_or_else(|| page.basename.clone(), str::to_owned)
}

/// Its tags: the frontmatter's `tags` (a list or a comma/space string)
/// and its inline `#tags`, without the `#`, once each, sorted.
#[must_use]
pub fn page_tags(page: &VaultPage) -> Vec<String> {
    let mut tags = BTreeSet::new();
    for e in &page.parsed.frontmatter {
        if e.key != "tags" && e.key != "tag" {
            continue;
        }
        let mut add = |s: &str| {
            let t = s.trim().trim_start_matches('#');
            if !t.is_empty() {
                tags.insert(t.to_owned());
            }
        };
        match &e.value {
            Value::String(s) => s.split([',', ' ']).for_each(&mut add),
            Value::Array(a) => a.iter().filter_map(Value::as_str).for_each(&mut add),
            _ => {}
        }
    }
    for block in &page.parsed.blocks {
        for r in &block.refs {
            if let Ref::Tag(t) = r {
                tags.insert(t.path.join("/"));
            }
        }
    }
    tags.into_iter().collect()
}

fn status(vault: &Vault) -> Value {
    let links = LinkIndex::build(vault);
    let bad_bases: Vec<Value> = vault
        .bases
        .iter()
        .filter_map(|b| {
            b.parsed
                .as_ref()
                .err()
                .map(|e| json!({ "path": b.rel_path, "error": e }))
        })
        .collect();
    let unresolved: Vec<Value> = links
        .unresolved()
        .iter()
        .map(|u| json!({ "from": u.source, "link": u.linkpath }))
        .collect();
    json!({
        "root": vault.root,
        "notes": vault.pages.len(),
        "bases": vault.bases.len(),
        "attachments": vault.attachments.len(),
        "unparsed_bases": bad_bases,
        "unresolved_links": unresolved,
    })
}

fn list_notes(vault: &Vault, folder: Option<&str>) -> Value {
    let folder = folder.map(|f| f.trim_matches('/'));
    let mut notes: Vec<&VaultPage> = vault
        .pages
        .iter()
        .filter(|p| {
            folder.is_none_or(|f| {
                f.is_empty() || p.folder == f || p.folder.starts_with(&format!("{f}/"))
            })
        })
        .collect();
    notes.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Value::Array(
        notes
            .into_iter()
            .map(|p| json!({ "path": p.rel_path, "title": title(p), "folder": p.folder }))
            .collect(),
    )
}

/// One note as `VAULT_READ` answers it.
///
/// # Errors
///
/// No note at `path`.
pub fn read_note(vault: &Vault, path: &str) -> Result<Value, String> {
    let page = vault
        .page(path)
        .ok_or_else(|| format!("{path}: no such note"))?;
    let links = LinkIndex::build(vault);
    let (_, body_at) = parse_frontmatter(&page.raw);
    let out: Vec<Value> = links
        .outgoing(path)
        .iter()
        .map(|l| json!({ "link": l.linkpath, "resolves_to": l.resolved, "alias": l.alias }))
        .collect();
    Ok(json!({
        "path": page.rel_path,
        "title": title(page),
        "properties": Value::Object(properties(page)),
        "tags": page_tags(page),
        "links": out,
        "backlinks": links.backlinks(path),
        "body": page.raw.get(body_at..).unwrap_or_default(),
    }))
}

fn search(vault: &Vault, query: &str, limit: usize) -> Value {
    let needle = query.to_lowercase();
    let mut hits: Vec<Value> = Vec::new();
    let mut pages: Vec<&VaultPage> = vault.pages.iter().collect();
    pages.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    for p in pages {
        if hits.len() >= limit {
            break;
        }
        let in_title = title(p).to_lowercase().contains(&needle);
        let line = p
            .raw
            .lines()
            .find(|l| l.to_lowercase().contains(&needle))
            .map(str::trim);
        if in_title || line.is_some() {
            hits.push(json!({ "path": p.rel_path, "title": title(p), "line": line }));
        }
    }
    Value::Array(hits)
}

fn query_base(vault: &Vault, path: &str, view: Option<&str>) -> Result<Value, String> {
    let base = match vault_obsidian::find_base(vault, path) {
        None => return Err(format!("{path}: no such base")),
        Some(Err(e)) => return Err(format!("{path}: {e}")),
        Some(Ok(b)) => b,
    };
    let rows = vault_obsidian::vault_to_rows(vault);
    let tables = vault_obsidian::run_base(base, &rows, view).map_err(|e| e.to_string())?;
    serde_json::to_value(tables).map_err(|e| e.to_string())
}

/// `value` as JSON, else as text.
fn value_of(value: &str) -> Value {
    serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_owned()))
}

fn entries_of(properties: Option<&str>) -> Result<Vec<FrontmatterEntry>, String> {
    let Some(text) = properties.filter(|s| !s.trim().is_empty()) else {
        return Ok(Vec::new());
    };
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(map)) => Ok(map
            .into_iter()
            .map(|(key, value)| FrontmatterEntry { key, value })
            .collect()),
        Ok(_) => Err("properties: not a JSON object".into()),
        Err(e) => Err(format!("properties: {e}")),
    }
}

/// Puts the `VAULT_*` actions on `list`, over `vault`.
///
/// # Errors
///
/// An id is taken.
pub fn register(list: &ActionList, vault: &OpenVault) -> Result<(), ActionError> {
    let v = vault.clone();
    list.register_with(
        &STATUS,
        Effect::ReadOnly,
        move |_: &ActionCaller, (): ()| Ok::<_, String>(raw(&v.with(status))),
    )?;
    let v = vault.clone();
    list.register_with(
        &LIST,
        Effect::ReadOnly,
        move |_: &ActionCaller, a: ListArgs| {
            Ok::<_, String>(raw(&v.with(|vault| list_notes(vault, a.folder.as_deref()))))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &READ,
        Effect::ReadOnly,
        move |_: &ActionCaller, a: PathArgs| {
            v.with(|vault| read_note(vault, &a.path)).map(|j| raw(&j))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &SEARCH,
        Effect::ReadOnly,
        move |_: &ActionCaller, a: SearchArgs| {
            let limit = a
                .limit
                .map_or(20, |n| usize::try_from(n).unwrap_or(usize::MAX));
            Ok::<_, String>(raw(&v.with(|vault| search(vault, &a.query, limit))))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &QUERY_BASE,
        Effect::ReadOnly,
        move |_: &ActionCaller, a: BaseArgs| {
            v.with(|vault| query_base(vault, &a.path, a.view.as_deref()))
                .map(|j| raw(&j))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &CREATE,
        Effect::Mutating,
        move |_: &ActionCaller, a: CreateArgs| {
            let fm = entries_of(a.properties.as_deref())?;
            mutate::create_page(
                &mut v.write(),
                &a.path,
                &fm,
                a.body.as_deref().unwrap_or_default(),
                &v.guard,
            )
            .map_err(|e| e.to_string())?;
            Ok::<_, String>(raw(&json!({ "path": a.path })))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &SET_PROPERTY,
        Effect::Mutating,
        move |_: &ActionCaller, a: SetPropertyArgs| {
            let value = value_of(&a.value);
            mutate::set_property(&mut v.write(), &a.path, &a.key, value.clone(), &v.guard)
                .map_err(|e| e.to_string())?;
            Ok::<_, String>(raw(
                &json!({ "path": a.path, "key": a.key, "value": value }),
            ))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &REMOVE_PROPERTY,
        Effect::Mutating,
        move |_: &ActionCaller, a: KeyArgs| {
            mutate::remove_property(&mut v.write(), &a.path, &a.key, &v.guard)
                .map_err(|e| e.to_string())?;
            Ok::<_, String>(raw(&json!({ "path": a.path, "key": a.key })))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &APPEND,
        Effect::Mutating,
        move |_: &ActionCaller, a: AppendArgs| {
            mutate::append_to_page(&mut v.write(), &a.path, &a.text, false, &v.guard)
                .map_err(|e| e.to_string())?;
            Ok::<_, String>(raw(&json!({ "path": a.path })))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &DELETE,
        Effect::Destructive,
        move |_: &ActionCaller, a: PathArgs| {
            mutate::delete_page(&mut v.write(), &a.path, &v.guard).map_err(|e| e.to_string())?;
            Ok::<_, String>(raw(&json!({ "path": a.path })))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &RENAME,
        Effect::Mutating,
        move |_: &ActionCaller, a: RenameArgs| {
            let r = v.rename(&a.from, &a.to)?;
            Ok::<_, String>(raw(
                &json!({ "from": r.from, "to": r.to, "relinked": r.relinked }),
            ))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &RELOAD,
        Effect::Mutating,
        move |_: &ActionCaller, (): ()| {
            v.reload().map_err(|e| e.to_string())?;
            Ok::<_, String>(raw(&v.with(status)))
        },
    )?;
    Ok(())
}
