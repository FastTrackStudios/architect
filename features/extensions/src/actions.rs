//! Extensions on the Action List (feature `host`): find, install and
//! remove them from ⌘K, a key, or a model over MCP — on every platform,
//! with no page of their own needed.
//!
//! | action | what it does |
//! |---|---|
//! | `EXT_SHARE_NEEDS` / `EXT_SHARE_NEEDS_SET` | what a share link's guests run, and choosing it: `{folder, code}` (`+ extensions`) |
//! | `EXT_INSTALLS_PUBLISH` / `EXT_INSTALLS_APPLY` | the signed-in person's installs and grants, to and from the open vault, signed by their devices (feature `person-sync`) |
//! | `EXT_LIST` | the extensions there are: each one's id, name, version, kind, which copy runs, what it adds, its `platforms` and why it cannot run here (`not_here`) |
//! | `EXT_NEEDS` | what opening a vault or project folder takes: what is ready, to install, to offer, missing (with why) and refused |
//! | `EXT_SEARCH` | searches every registry: `{query}` → the newest of each that matches |
//! | `EXT_INSTALL` | installs one: `{id, version?}` (a semver range), loaded at once |
//! | `EXT_UNINSTALL` | removes one: `{id}`; a built-in comes back |
//! | `EXT_RELOAD` | reloads every extension whose files changed |
//! | `EXT_CHECK` | loads a folder once on its own and renders its views once: `ok`, or the exact error (`crate::check`) |
//! | `EXT_CLONE` | copies a built-in or installed one into the development folder under a new id (`crate::clone`) |
//!
//! Where the app keeps registries (`Extensions::with_registries`), also
//! `EXT_REGISTRIES`, `EXT_REGISTRY_ADD`, `EXT_REGISTRY_REMOVE`,
//! `EXT_REGISTRY_REFRESH`, `EXT_DEV`, `EXT_DEV_END` and
//! `EXT_DEV_INSTALLS` (`crate::registries`).

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
static NEEDS: ActionMeta = meta(
    "EXT_NEEDS",
    "What a vault or project needs",
    "What opening the vault or project folder `folder` takes: the extensions its \
     .architect/extensions.toml names and the ones it carries in .architect/extensions/, as \
     ready, install (required: one consent for all), offer (optional), missing (each with why) \
     and refused (carried archives that are not signed by a key trusted here). Installs nothing.",
);
static SHARE_NEEDS: ActionMeta = meta(
    "EXT_SHARE_NEEDS",
    "What a share link's guests run",
    "The extensions share link `code` of the vault at `folder` runs for its guests \
     (_architect/shares/<code>.extensions.toml): each id, version range, required or not, from \
     the linked core or a signed archive the link carries (its file and sha256), and the hosts \
     the owner lets it fetch.",
);
static SHARE_NEEDS_SET: ActionMeta = meta(
    "EXT_SHARE_NEEDS_SET",
    "Choose a share link's extensions",
    "Makes `extensions` (each `{id, required?, hosts?}`, running here) what share link `code` of \
     the vault at `folder` runs for its guests: a built-in from the linked core, an installed one \
     as its signed archive copied into the link (sha256 pinned). A development copy or an \
     unsigned install is refused: guests take signed packages only. `hosts` must be ones its \
     manifest asks for. Empty `extensions` clears it.",
);
#[cfg(feature = "person-sync")]
static INSTALLS_PUBLISH: ActionMeta = meta(
    "EXT_INSTALLS_PUBLISH",
    "Share my installs with my devices",
    "Writes the signed-in person's record in the open vault, signed by this device: each \
     extension installed here from a signed archive, its pin, signer and grants. Only that \
     person may ask; it is also done at once after each install, removal or revoke.",
);
#[cfg(feature = "person-sync")]
static INSTALLS_APPLY: ActionMeta = meta(
    "EXT_INSTALLS_APPLY",
    "Install what I installed on my other devices",
    "Applies here the signed-in person's record in the open vault, when it is signed by this \
     device or one of theirs it trusts: installs, removals and revokes, with their grants and \
     no prompt. Only that person may ask.",
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
     whether it loaded. A linked built-in whose manifest has a `[source]` (and no `as`) is cloned \
     from source: its crate written to the development folder and built as a process that keeps \
     its id and overrides the built-in, rebuilt on save (`build` says how the build is). A process \
     extension with actions and no `[source]` is refused (its program answers them by id).",
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

/// `EXT_NEEDS`'s arguments.
#[derive(Debug, Facet)]
pub struct NeedsArgs {
    /// The vault's or project's folder, absolute.
    pub folder: String,
}

/// `EXT_SHARE_NEEDS`'s arguments.
#[derive(Debug, Facet)]
pub struct ShareNeedsArgs {
    /// The vault's folder, absolute.
    pub folder: String,
    /// The link's code.
    pub code: String,
}

/// `EXT_SHARE_NEEDS_SET`'s arguments.
#[derive(Debug, Facet)]
pub struct ShareNeedsSetArgs {
    pub folder: String,
    pub code: String,
    pub extensions: Vec<crate::share_pick::LinkPick>,
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
    /// Take `version` even when it is older than the one installed or
    /// outside its pin (a person asked for it by name).
    pub explicit: Option<bool>,
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
        list.register_with(&NEEDS, Effect::ReadOnly, move |_: &ActionCaller, a: NeedsArgs| {
            let n = me.open_vault(std::path::Path::new(&a.folder));
            let listing = |l: &crate::index::Listing| {
                json!({ "id": l.id, "name": l.name, "version": l.version.to_string(), "kind": l.kind })
            };
            Ok::<_, String>(RawJson::from(
                json!({
                    "ready": n.plan.ready,
                    "install": n.plan.install.iter().map(listing).collect::<Vec<_>>(),
                    "offer": n.plan.offer.iter().map(listing).collect::<Vec<_>>(),
                    "missing": n.plan.missing.iter().map(|(r, m)| json!({
                        "id": r.id, "name": r.name, "version": r.version,
                        "required": r.required, "why": m.to_string(),
                    })).collect::<Vec<_>>(),
                    "refused": n.refused.iter().map(|(f, why)| json!({ "file": f, "why": why })).collect::<Vec<_>>(),
                    "prompt": n.plan.prompt(),
                    "error": n.error,
                })
                .to_string(),
            ))
        })?;
        let me = self.clone();
        list.register_with(
            &SHARE_NEEDS,
            Effect::ReadOnly,
            move |_: &ActionCaller, a: ShareNeedsArgs| {
                let needs = me.share_needs(std::path::Path::new(&a.folder), &a.code)?;
                Ok::<_, String>(RawJson::from(
                    serde_json::to_string(&needs).map_err(|e| e.to_string())?,
                ))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &SHARE_NEEDS_SET,
            Effect::Mutating,
            move |_: &ActionCaller, a: ShareNeedsSetArgs| {
                let needs =
                    me.set_share_needs(std::path::Path::new(&a.folder), &a.code, &a.extensions)?;
                Ok::<_, String>(RawJson::from(
                    serde_json::to_string(&needs).map_err(|e| e.to_string())?,
                ))
            },
        )?;
        #[cfg(feature = "person-sync")]
        self.install_person_actions(&list)?;
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
                    .install_with_in(
                        &a.id,
                        &req,
                        grant,
                        &by,
                        a.pin == Some(true),
                        &[],
                        a.explicit == Some(true),
                    )
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
                        "build": me.build_of(&cloned.id),
                        "events": events,
                    })
                    .to_string(),
                ))
            },
        )?;
        self.install_manage_actions()?;
        self.install_registry_actions()?;
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
                    // Where it runs (#578): its `platforms` (empty: every
                    // one), and why not here when it cannot.
                    "platforms": m.platforms,
                    "not_here": self.host().why_not_here(&m.id),
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

/// Whether `caller` may act for `person`'s installs: the app itself (its
/// own code, a key), or that person signed in; never another account, a
/// guest or a peer.
#[cfg(feature = "person-sync")]
fn as_person(caller: &ActionCaller, person: &str) -> Result<(), String> {
    match caller.principal() {
        None => Ok(()),
        Some(architect::action::Principal::User { user_id }) if user_id == person => Ok(()),
        Some(_) => Err(format!("only {person} may ask for their installs")),
    }
}

#[cfg(feature = "person-sync")]
impl Extensions {
    /// `EXT_INSTALLS_PUBLISH` and `EXT_INSTALLS_APPLY`, for the person set
    /// with [`Self::sync_person`].
    fn install_person_actions(
        &self,
        list: &architect::action::ActionList,
    ) -> Result<(), ActionError> {
        let me = self.clone();
        list.register_with(
            &INSTALLS_PUBLISH,
            Effect::Mutating,
            move |caller: &ActionCaller, (): ()| {
                let sync = me
                    .person_sync()
                    .ok_or("no one is signed in with a vault open")?;
                as_person(caller, &sync.person)?;
                let record = me.publish_installs(&sync, None)?;
                let ids: Vec<&str> = record
                    .installs
                    .iter()
                    .filter(|i| !i.removed)
                    .map(|i| i.id.as_str())
                    .collect();
                Ok::<_, String>(RawJson::from(json!({ "published": ids }).to_string()))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &INSTALLS_APPLY,
            Effect::Mutating,
            move |caller: &ActionCaller, (): ()| {
                let sync = me
                    .person_sync()
                    .ok_or("no one is signed in with a vault open")?;
                as_person(caller, &sync.person)?;
                let out = me.apply_installs(&sync).map_err(|e| e.to_string())?;
                let rows: Vec<Value> = out
                    .into_iter()
                    .map(|r| match r {
                        Ok(a) => json!({ "applied": format!("{a:?}") }),
                        Err((id, e)) => json!({ "id": id, "error": e.to_string() }),
                    })
                    .collect();
                Ok::<_, String>(RawJson::from(json!({ "applied": rows }).to_string()))
            },
        )?;
        Ok(())
    }
}
