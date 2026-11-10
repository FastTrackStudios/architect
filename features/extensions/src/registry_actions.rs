//! The registries' and development installs' actions (#493), on the list
//! where the app keeps registries ([`Extensions::with_registries`]).
//!
//! | action | what it does |
//! |---|---|
//! | `EXT_REGISTRIES` | every registry (the app's own first): its name, git URL or folder, publishers' keys, whether it is the app's; and whether development installs are on |
//! | `EXT_REGISTRY_ADD` | adds `{name, git \| folder, keys?: [{name, key}]}` (or replaces the one of that name), kept in the app's settings, then fetches its index; a caller from outside needs a person's yes to add keys |
//! | `EXT_REGISTRY_REMOVE` | removes `{name}` (not the app's own) |
//! | `EXT_REGISTRY_REFRESH` | pulls every registry's index (`{name?}`: one): the ones that failed, and why |
//! | `EXT_DEV` | loads `{folder}` as a development copy, unsigned, reloaded on save, kept across restarts; only with development installs on |
//! | `EXT_DEV_END` | ends the development copy in `{folder}` |
//! | `EXT_DEV_INSTALLS` | turns development installs `{on}` (a debug build has them on regardless); a caller from outside needs a person's yes to turn them on |

use std::path::{Path, PathBuf};

use architect::action::{ActionCaller, ActionError, ActionMeta, Effect, RawJson};
use facet::Facet;
use serde_json::{Value, json};

use crate::actions::event_json;
use crate::extensions::Extensions;
use crate::registries::{Publisher, Registries, Registry};

const fn meta(id: &'static str, name: &'static str, description: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "Extensions",
        method_name: id,
        display_name: name,
        description,
        category: "Extensions",
        group: "Extensions",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

static REGISTRIES: ActionMeta = meta(
    "EXT_REGISTRIES",
    "List extension registries",
    "Every registry the app searches and installs from (its own first, then the ones a person \
     added): name, git URL or folder, the publishers whose keys it trusts, whether it is the \
     app's own; and whether development installs are on.",
);
static ADD: ActionMeta = meta(
    "EXT_REGISTRY_ADD",
    "Add an extension registry",
    "Adds registry `name` from a git repository (`git`, a URL) or a folder (`folder`), with the \
     publishers' keys its archives may be signed by (`keys`: [{name, key}], each key the base64 \
     line of a minisign .pub), kept in the app's settings, and fetches its index. One of that \
     name is replaced. Adding keys from outside needs a person's yes (confirmation_required).",
);
static REMOVE: ActionMeta = meta(
    "EXT_REGISTRY_REMOVE",
    "Remove an extension registry",
    "Removes registry `name` (one a person added; the app's own stay). What was installed from \
     it stays installed.",
);
static REFRESH: ActionMeta = meta(
    "EXT_REGISTRY_REFRESH",
    "Refresh extension registries",
    "Pulls every registry's index (or registry `name`'s): answers the ones that failed, and why.",
);
static DEV: ActionMeta = meta(
    "EXT_DEV",
    "Load an extension folder for development",
    "Loads the package in `folder` (its extension.toml and, for a process extension, its \
     program) as a development copy: unsigned, it shadows every other copy of its id, reloads \
     when its files change, and is loaded again on the next start. Only with development \
     installs on (EXT_DEV_INSTALLS, Settings → Extensions, or a debug build).",
);
static DEV_END: ActionMeta = meta(
    "EXT_DEV_END",
    "End an extension's development copy",
    "Ends the development copy loaded from `folder` (its files stay); another copy of its id \
     runs again.",
);
static DEV_INSTALLS: ActionMeta = meta(
    "EXT_DEV_INSTALLS",
    "Turn development installs on or off",
    "Development installs (`on`): whether EXT_DEV may load an unsigned folder. A debug build has \
     them on regardless. Turning them on from outside needs a person's yes.",
);

/// `EXT_REGISTRY_ADD`'s arguments.
#[derive(Debug, Facet)]
pub struct AddArgs {
    pub name: String,
    /// A git repository's URL.
    pub git: Option<String>,
    /// A folder with an `index.toml`.
    pub folder: Option<String>,
    /// The publishers whose archives it may list.
    pub keys: Option<Vec<Publisher>>,
}

/// `EXT_REGISTRY_REMOVE`'s arguments.
#[derive(Debug, Facet)]
pub struct NameArgs {
    pub name: String,
}

/// `EXT_REGISTRY_REFRESH`'s arguments.
#[derive(Debug, Facet)]
pub struct RefreshArgs {
    /// One registry; every one when none.
    pub name: Option<String>,
}

/// `EXT_DEV`'s and `EXT_DEV_END`'s arguments.
#[derive(Debug, Facet)]
pub struct FolderArgs {
    /// The package's folder (its `extension.toml` in it).
    pub folder: String,
}

/// `EXT_DEV_INSTALLS`'s arguments.
#[derive(Debug, Facet)]
pub struct OnArgs {
    pub on: bool,
}

fn raw(v: &Value) -> RawJson {
    RawJson::from(v.to_string())
}

/// The registries and the switch as JSON.
fn listing(r: &Registries) -> Value {
    let rows: Vec<Value> = r
        .list()
        .into_iter()
        .map(|l| {
            json!({
                "name": l.registry.name,
                "git": l.registry.git,
                "folder": l.registry.folder,
                "keys": l.registry.keys.iter().map(|k| json!({ "name": k.name, "key": k.key })).collect::<Vec<_>>(),
                "builtin": l.builtin,
            })
        })
        .collect();
    json!({
        "registries": rows,
        "dev_installs": r.dev_installs(),
        "dev_setting": r.dev_setting(),
        "dev": r.dev_folders().iter().map(|d| d.display().to_string()).collect::<Vec<_>>(),
    })
}

/// A package folder, absolute, with a manifest in it.
fn package_folder(folder: &str) -> Result<PathBuf, String> {
    let dir = Path::new(folder);
    let dir = std::fs::canonicalize(dir).map_err(|e| format!("{folder}: {e}"))?;
    if dir.join(crate::manifest::MANIFEST).is_file() {
        Ok(dir)
    } else {
        Err(format!(
            "{}: no {} in it",
            dir.display(),
            crate::manifest::MANIFEST
        ))
    }
}

/// A registry that could not be refreshed.
#[derive(Debug, Facet)]
pub struct Failed {
    pub registry: String,
    pub error: String,
}

/// `EXT_REGISTRY_REFRESH`'s (and `EXT_REGISTRY_ADD`'s) answer.
#[derive(Debug, Facet)]
pub struct Refreshed {
    /// Every registry asked for was refreshed.
    pub ok: bool,
    pub failed: Vec<Failed>,
}

/// Runs `refresh` on a thread of its own and answers its failures.
fn refreshed(
    me: Extensions,
    name: Option<String>,
) -> impl std::future::Future<Output = Result<Refreshed, String>> {
    let (tx, rx) = futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("ext-registry-refresh".to_owned())
        .spawn(move || {
            let failed = me.refresh_registries(name.as_deref());
            let _ = tx.send(failed);
        });
    async move {
        spawned.map_err(|e| format!("no thread to refresh on: {e}"))?;
        let failed = rx.await.map_err(|_| "the refresh stopped".to_owned())?;
        let failed: Vec<Failed> = failed
            .into_iter()
            .map(|(registry, error)| Failed { registry, error })
            .collect();
        Ok(Refreshed {
            ok: failed.is_empty(),
            failed,
        })
    }
}

impl Extensions {
    /// Pulls the index of every registry (or `name`'s): the ones that
    /// failed, and why.
    #[must_use]
    pub fn refresh_registries(&self, name: Option<&str>) -> Vec<(String, String)> {
        let failed: Vec<(String, String)> = self
            .refresh()
            .into_iter()
            .filter(|(n, _)| name.is_none_or(|want| want == n))
            .map(|(n, e)| (n, e.to_string()))
            .collect();
        self.host()
            .list()
            .changed("extensions", "registries refreshed");
        failed
    }

    /// Loads `folder` as a development copy, when development installs
    /// are on, and keeps it for the next start.
    ///
    /// # Errors
    ///
    /// Development installs are off, or the folder has no manifest.
    pub fn dev_install(&self, folder: &str) -> Result<Value, String> {
        let registries = self
            .registries()
            .ok_or("this app keeps no development installs")?;
        if !registries.dev_installs() {
            return Err("development installs are off: turn them on in Settings → Extensions \
                        (EXT_DEV_INSTALLS {\"on\": true}) or use a debug build"
                .to_owned());
        }
        let dir = package_folder(folder)?;
        let id = std::fs::read_to_string(dir.join(crate::manifest::MANIFEST))
            .ok()
            .and_then(|t| crate::manifest::Manifest::parse(&t).ok())
            .map(|m| m.id);
        let events = self.dev(dir.clone());
        registries.keep_dev(&dir, true)?;
        let loaded = id.as_deref().is_some_and(|id| self.host().get(id).is_some());
        Ok(json!({
            "id": id,
            "folder": dir.display().to_string(),
            "loaded": loaded,
            "events": events.iter().map(event_json).collect::<Vec<_>>(),
        }))
    }

    /// Puts `EXT_REGISTRIES`, `EXT_REGISTRY_*` and `EXT_DEV*` on the
    /// list, when the app keeps registries.
    pub(crate) fn install_registry_actions(&self) -> Result<(), ActionError> {
        let Some(registries) = self.registries() else {
            return Ok(());
        };
        let list = self.host().list().clone();
        let r = registries.clone();
        list.register_with(&REGISTRIES, Effect::ReadOnly, move |_: &ActionCaller, (): ()| {
            Ok::<_, String>(raw(&listing(&r)))
        })?;
        let me = self.clone();
        let r = registries.clone();
        list.register_async(&ADD, Effect::Mutating, move |a: AddArgs| {
            let registry = Registry {
                name: a.name.trim().to_owned(),
                git: a.git.filter(|g| !g.trim().is_empty()),
                folder: a.folder.filter(|f| !f.trim().is_empty()),
                keys: a.keys.unwrap_or_default(),
            };
            let name = registry.name.clone();
            let added = r.add(registry);
            let me = me.clone();
            async move {
                added?;
                refreshed(me, Some(name)).await
            }
        })?;
        let r = registries.clone();
        list.register_with(&REMOVE, Effect::Mutating, move |_: &ActionCaller, a: NameArgs| {
            let removed = r.remove(&a.name)?;
            Ok::<_, String>(raw(&json!({ "removed": removed })))
        })?;
        let me = self.clone();
        list.register_async(&REFRESH, Effect::Mutating, move |a: RefreshArgs| {
            refreshed(me.clone(), a.name)
        })?;
        let me = self.clone();
        list.register_with(&DEV, Effect::Mutating, move |_: &ActionCaller, a: FolderArgs| {
            me.dev_install(&a.folder).map(|v| raw(&v))
        })?;
        let me = self.clone();
        let r = registries.clone();
        list.register_with(&DEV_END, Effect::Mutating, move |_: &ActionCaller, a: FolderArgs| {
            let dir = std::fs::canonicalize(&a.folder).unwrap_or_else(|_| PathBuf::from(&a.folder));
            let events = me.end_dev(&dir);
            r.keep_dev(&dir, false)?;
            Ok::<_, String>(raw(&json!({
                "folder": dir.display().to_string(),
                "events": events.iter().map(event_json).collect::<Vec<_>>(),
            })))
        })?;
        let r = registries;
        list.register_with(
            &DEV_INSTALLS,
            Effect::Mutating,
            move |_: &ActionCaller, a: OnArgs| {
                r.set_dev_installs(a.on)?;
                Ok::<_, String>(raw(&json!({ "dev_installs": r.dev_installs(), "dev_setting": a.on })))
            },
        )?;
        Ok(())
    }
}
