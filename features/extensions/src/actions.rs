//! Extensions on the Action List (feature `host`): find, install and
//! remove them from ⌘K, a key, or a model over MCP — on every platform,
//! with no page of their own needed.
//!
//! | action | what it does |
//! |---|---|
//! | `EXT_LIST` | the extensions there are: each one's id, name, version, kind, which copy runs and what it adds |
//! | `EXT_SEARCH` | searches every registry: `{query}` → the newest of each that matches |
//! | `EXT_INSTALL` | installs one: `{id, version?}` (a semver range), loaded at once |
//! | `EXT_UNINSTALL` | removes one: `{id}`; a built-in comes back |
//! | `EXT_RELOAD` | reloads every extension whose files changed |
//! | `EXT_CHECK` | loads a folder once on its own and renders its views once: `ok`, or the exact error (`crate::check`) |
//! | `EXT_CLONE` | copies a built-in or installed one into the development folder under a new id (`crate::clone`) |

use architect::action::{ActionCaller, ActionError, ActionMeta, Effect, RawJson};
use facet::Facet;
use semver::VersionReq;
use serde_json::{Value, json};

use crate::extensions::Extensions;
use crate::host::Event;

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

static LIST: ActionMeta = meta(
    "EXT_LIST",
    "List extensions",
    "Every extension there is — built in, installed, in development — with its version, its \
     kind, which copy runs, and the actions it adds.",
);
static SEARCH: ActionMeta = meta(
    "EXT_SEARCH",
    "Search for extensions",
    "Searches every registry (and ReaPack repository) for `query`: the newest of each that \
     matches, with where it is listed.",
);
static INSTALL: ActionMeta = meta(
    "EXT_INSTALL",
    "Install an extension",
    "Installs extension `id` — the newest version, or the newest `version` (a semver range like \
     ^1.2) takes — and loads it at once, without a restart; `pin` keeps it there. When the app asks \
     for consent and it asks for capabilities not granted, it is not installed: the answer is \
     needs_consent with what it asks (EXT_PREVIEW); a person installs it with `grant` (device, \
     org), which a caller from outside needs their yes for (confirmation_required).",
);
static UNINSTALL: ActionMeta = meta(
    "EXT_UNINSTALL",
    "Remove an extension",
    "Removes installed extension `id`: its actions and settings go with it; a built-in copy \
     comes back.",
);
static RELOAD: ActionMeta = meta(
    "EXT_RELOAD",
    "Reload changed extensions",
    "Reloads every loaded extension whose files changed; one that fails keeps its last good copy.",
);

static CHECK: ActionMeta = meta(
    "EXT_CHECK",
    "Check an extension",
    "Loads the extension in `folder` once, on its own (its manifest, its Luau scripts or its program, \
     its actions and view kinds), renders each of its views once, and answers `ok` or every problem: \
     the stage, the file, the manifest field and line, the script's line, the view that did not \
     render. The app's own copy is not touched. Run it after every change to an extension.",
);
static CLONE: ActionMeta = meta(
    "EXT_CLONE",
    "Clone an extension",
    "Copies extension `id` (built in or installed: what the app ships stays as it is) into the \
     development folder as `as` (`<id>-copy` unless given; its actions and settings move to that \
     namespace) and runs the copy, which reloads when its files change. Answers its id, folder and \
     whether it loaded. A process extension with actions is refused (its program answers them by id).",
);

/// `EXT_CHECK`'s arguments.
#[derive(Debug, Facet)]
pub struct CheckArgs {
    /// The extension's folder (its `extension.toml` in it).
    pub folder: String,
}

/// `EXT_CLONE`'s arguments.
#[derive(Debug, Facet)]
pub struct CloneArgs {
    /// The extension to copy.
    pub id: String,
    /// The copy's id; `<id>-copy` unless given.
    #[facet(default, rename = "as")]
    pub as_id: Option<String>,
}

/// `EXT_SEARCH`'s arguments.
#[derive(Debug, Facet)]
pub struct SearchArgs {
    pub query: String,
}

/// `EXT_INSTALL`'s arguments.
#[derive(Debug, Facet)]
pub struct InstallArgs {
    pub id: String,
    /// A semver range (`^1.2`); the newest when none.
    pub version: Option<String>,
    /// Where a person grants what it asks: `device`, `org` (the current
    /// one), `org:<id>`. Without it, one that asks for more than was
    /// granted is answered `needs_consent` and not installed.
    pub grant: Option<String>,
    /// Keep it at `version` (or the version installed) from now on.
    pub pin: Option<bool>,
}

/// `EXT_UNINSTALL`'s arguments.
#[derive(Debug, Facet)]
pub struct UninstallArgs {
    pub id: String,
}

/// An event as JSON: `{loaded|unloaded|failed: id, …}`.
#[must_use]
pub fn event_json(event: &Event) -> Value {
    match event {
        Event::Loaded(c) => json!({
            "loaded": c.id,
            "version": c.version.to_string(),
            "reloaded": c.reloaded,
            "added": c.added,
            "removed": c.removed,
            "took_ms": u64::try_from(c.took.as_millis()).unwrap_or(u64::MAX),
        }),
        Event::Unloaded { id } => json!({ "unloaded": id }),
        Event::Failed { id, error } => json!({ "failed": id, "error": error.to_string() }),
    }
}

impl Extensions {
    /// Puts the `EXT_*` actions on the host's list.
    ///
    /// # Errors
    ///
    /// An id is taken.
    pub fn install_actions(&self) -> Result<(), ActionError> {
        let list = self.host().list().clone();
        let me = self.clone();
        list.register_with(&LIST, Effect::ReadOnly, move |_: &ActionCaller, (): ()| {
            Ok::<_, String>(RawJson::from(me.listing().to_string()))
        })?;
        let me = self.clone();
        list.register_with(
            &SEARCH,
            Effect::ReadOnly,
            move |_: &ActionCaller, a: SearchArgs| {
                let hits: Vec<Value> = me
                    .search(&a.query)
                    .into_iter()
                    .map(|f| {
                        let l = f.hit.listing;
                        json!({
                            "id": l.id,
                            "name": l.name,
                            "version": l.version.to_string(),
                            "kind": l.kind,
                            "description": l.description,
                            "registry": f.source,
                        })
                    })
                    .collect();
                Ok::<_, String>(RawJson::from(json!({ "found": hits }).to_string()))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &INSTALL,
            Effect::Mutating,
            move |caller: &ActionCaller, a: InstallArgs| {
                let req = match a.version.as_deref() {
                    None | Some("") => VersionReq::STAR,
                    Some(r) => VersionReq::parse(r).map_err(|e| format!("version {r}: {e}"))?,
                };
                let org = me.host().grants().and_then(|g| g.org());
                let grant = a
                    .grant
                    .as_deref()
                    .map(|g| crate::consent::Scope::parse(g, org.as_deref()))
                    .transpose()?;
                let by = caller.principal().map_or_else(
                    || "person".to_owned(),
                    architect::action::Principal::describe,
                );
                let installed = me
                    .install_with(&a.id, &req, grant, &by, a.pin == Some(true))
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>(RawJson::from(Self::installed_json(&installed).to_string()))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &UNINSTALL,
            Effect::Destructive,
            move |_: &ActionCaller, a: UninstallArgs| {
                let removed = me.uninstall(&a.id).map_err(|e| e.to_string())?;
                Ok::<_, String>(RawJson::from(json!({ "removed": removed }).to_string()))
            },
        )?;
        let checker = self.host().clone();
        list.register_async(&CHECK, Effect::ReadOnly, move |a: CheckArgs| {
            let checker = checker.checker();
            let (tx, rx) = futures::channel::oneshot::channel();
            let spawned = std::thread::Builder::new()
                .name("ext-check".to_owned())
                .spawn(move || {
                    let report = checker.check(&crate::check::folder(&a.folder));
                    let _ = tx.send(report);
                });
            async move {
                spawned.map_err(|e| format!("no thread to check on: {e}"))?;
                rx.await.map_err(|_| "the check stopped".to_owned())
            }
        })?;
        let me = self.clone();
        list.register_with(
            &CLONE,
            Effect::Mutating,
            move |_: &ActionCaller, a: CloneArgs| {
                let cloned = me.clone_to_dev(&a.id, a.as_id.as_deref())?;
                let events: Vec<Value> = cloned.events.iter().map(event_json).collect();
                Ok::<_, String>(RawJson::from(
                    json!({
                        "id": cloned.id,
                        "folder": cloned.folder.display().to_string(),
                        "loaded": me.host().get(&cloned.id).is_some(),
                        "events": events,
                    })
                    .to_string(),
                ))
            },
        )?;
        self.install_manage_actions()?;
        let me = self.clone();
        list.register_with(
            &RELOAD,
            Effect::Mutating,
            move |_: &ActionCaller, (): ()| {
                let events: Vec<Value> = me.reload_changed().iter().map(event_json).collect();
                Ok::<_, String>(RawJson::from(json!({ "events": events }).to_string()))
            },
        )?;
        Ok(())
    }

    fn listing(&self) -> Value {
        let loaded = self.host().loaded();
        let extensions: Vec<Value> = self
            .chosen()
            .into_iter()
            .map(|c| {
                let m = &c.runs.package.manifest;
                let actions = loaded
                    .iter()
                    .find(|l| l.manifest.id == m.id)
                    .map(|l| l.actions.clone())
                    .unwrap_or_default();
                json!({
                    "id": m.id,
                    "name": m.name,
                    "version": m.version.to_string(),
                    "kind": m.kind.name(),
                    "runs": c.describe(),
                    "loaded": !actions.is_empty() || loaded.iter().any(|l| l.manifest.id == m.id),
                    "actions": actions,
                    "settings": m.sections.iter().map(|s| s.id.clone()).collect::<Vec<_>>(),
                    "views": m.views.iter().map(|v| v.id.clone()).collect::<Vec<_>>(),
                    "overlays": m.overlays.iter().map(|o| o.id.clone()).collect::<Vec<_>>(),
                })
            })
            .collect();
        json!({ "extensions": extensions })
    }
}
