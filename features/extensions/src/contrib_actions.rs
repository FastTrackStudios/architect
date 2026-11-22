//! What extensions add to the interface, on the Action List (feature
//! `host`):
//!
//! read it, open files and links with it, write runtime state, raise an
//! event, open a view, turn an extension on and off — from ⌘K, a key, a
//! script or a model over MCP. Put on the list with
//! [`Extensions::install_contribution_actions`].
//!
//! | action | what it does |
//! |---|---|
//! | `EXT_CONTRIBUTIONS` | every loaded extension's contributions now (declared and added at run time), who fills each slot, and the runtime state |
//! | `EXT_OPEN` | opens `{path}`, `{url}` or `{doc_kind, path?}` with the first opener that claims it |
//! | `EXT_VIEW_OPEN` | opens `{extension, view, state?, placement?}` in the window |
//! | `EXT_VIEW_CLOSE` | takes `{extension, view}` away when it shows as a popup or overlay |
//! | `EXT_STATE_SET` | writes runtime state as `{source, key, kind?, text?, icon?, target?, seq?, ttl_ms?}` |
//! | `EXT_STATE_CLEAR` | clears `{source, key?}`'s state |
//! | `EXT_EMIT` | raises app event `{event, detail?}`: the hooks on it run |
//! | `EXT_ENABLE` / `EXT_DISABLE` | turns `{id}` on or off: loaded or unloaded at once, everything it adds with it |
//! | `EXT_EMBEDS` | every inline embed and what it claims, or `{claim}`'s (`fence:kf`): which extension draws it, and how |
//! | `EXT_PROJECT_TYPES` | every project type loaded extensions add (`[[project_types]]`), or `{id}`'s: its properties, template, folders, views and owner |
//! | `EXT_LANGUAGES` | the editor's language registry (#512): the app's languages and every loaded extension's `[[languages]]`, or the one `{id}`, `{fence}` or `{path}` names: its file types, fences, grammar, highlights query, comment token, LSP server command and who registered it (`from`) |
//! | `EXT_FILE_TYPES` | the file types that are vault notes: `md` and every declared one (`[[file_types]]`, `vault_proto::note_types`): its extension, name, icon, the `type` it reads as and who declared it (`from`) |
//! | `EXT_CAPTURE_TARGETS` | every capture target loaded extensions add (`[[capture_targets]]`): its chips, tokens, action and owner |
//! | `DOC_UNDO` / `DOC_REDO` | takes back (or makes again) the last undo step of document `{path?}` (none: the active pane's, else the current document's; its pane's state is the path an opener opened): `{step, text}`, `null` when there is none. ⌘Z and ⇧⌘Z in every view an opener shows a file in |
//! | `EXT_SERVICES` | every service loaded extensions provide: its providers, the one chosen for it and the one in use |
//! | `EXT_SERVICE_USE` | chooses `{service, provider}`'s provider (`provider` null: the host's choice, the newest) |
//! | `EXT_PALETTE` | the palette slot (#381): which extension's view draws each overlay (`[[palette_views]]`), the keys extensions bind (`[[keys]]`) and the palette's sources (`[[palette_sources]]`) |

use architect::action::{
    ActionCaller, ActionError, ActionMeta, Effect, Invocation, InvocationSource, RawJson,
};
use facet::Facet;
use serde_json::{Value, json};

use crate::actions::event_json;
use crate::contrib::{StateKind, StateWrite};
use crate::extensions::Extensions;
use crate::host::{Opening, now_ms};
use crate::manifest::ViewPlacement;

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

static CONTRIBUTIONS: ActionMeta = meta(
    "EXT_CONTRIBUTIONS",
    "What extensions add",
    "Every loaded extension's contributions to the interface — openers, document kinds, tabs, \
     status items, menu and context-menu entries, action-bar buttons, hooks, slots — as declared \
     and as added at run time; who fills each shell slot; and the runtime state (status text, \
     badges, metadata) with its source, sequence and expiry.",
);
static OPEN: ActionMeta = meta(
    "EXT_OPEN",
    "Open with an extension",
    "Opens a file (`path`), a link (`url`) or a document of a kind (`doc_kind`, with its `path`) \
     with the first extension opener that claims it: in its view, or with its action. Answers \
     which extension and opener took it; nothing claiming it is an error naming what was asked.",
);
static VIEW_OPEN: ActionMeta = meta(
    "EXT_VIEW_OPEN",
    "Open an extension's view",
    "Opens extension `extension`'s view `view` in the window, showing `state` when given, placed \
     as its manifest says unless `placement` (tab, split, overlay, popup, zoomed, float) does.",
);
static VIEW_CLOSE: ActionMeta = meta(
    "EXT_VIEW_CLOSE",
    "Close an extension's popup",
    "Takes extension `extension`'s view `view` away when it shows over the window (a popup or an \
     overlay): what a notification centre does after its \"go to\". One not showing is no error.",
);
static STATE_SET: ActionMeta = meta(
    "EXT_STATE_SET",
    "Set runtime state",
    "Writes runtime state owned by `source` (`agent:<name>`, `app`; an extension writes its own \
     through its guest or `ext`): a status item's text (`kind` status, `key` its id), a badge or \
     metadata on a `target`. A `seq` at or below the last kept for the key is dropped as stale; \
     `ttl_ms` makes it lapse. Text is one line of at most 80 characters.",
);
static STATE_CLEAR: ActionMeta = meta(
    "EXT_STATE_CLEAR",
    "Clear runtime state",
    "Clears `source`'s runtime state: one `key`, or all of it. Answers how many went.",
);
static EMIT: ActionMeta = meta(
    "EXT_EMIT",
    "Raise an app event",
    "Raises app event `event` (startup, workspace.changed, doc.opened, doc.saved, doc.created, \
     doc.renamed, doc.deleted, app.focused, app.blurred, theme.changed, selection.changed, \
     changed:<topic>, app:<name>) with `detail` (text, or JSON for a document event's \
     `{path}`): every loaded extension's hooks on it run. Answers how many started.",
);
static EMBEDS: ActionMeta = meta(
    "EXT_EMBEDS",
    "Inline embeds",
    "Every loaded extension's inline embeds and what each claims (fence languages, note types, \
     embed target types); with `claim` (`fence:kf`, `note_type:song`, `embed_type:setlist`), the \
     one that draws it: its extension, view or page, and how tall it may grow. None claiming it \
     answers `null`.",
);
static PROJECT_TYPES: ActionMeta = meta(
    "EXT_PROJECT_TYPES",
    "Project types",
    "The types of architect's `project` kind that loaded extensions add (`[[project_types]]`): \
     each one's id, name, icon, the properties it adds to `project`, its `project.md` template's \
     text, the folders a new project gets, its views and the extension it is from. With `id`, \
     that one (`null` when no loaded extension adds it).",
);
static LANGUAGES: ActionMeta = meta(
    "EXT_LANGUAGES",
    "Languages",
    "The editor's language registry: the app's own languages and the ones loaded extensions add \
     (`[[languages]]`). Each has its id, name, file extensions, fence names, grammar (`linked` or \
     `linked:<name>`), highlights query, comment token, its LSP server's command and args \
     when it has one (EDITOR_SERVERS shows the ones running), and `from` (`app` or the \
     extension). \
     With `id`, `fence` (a code fence's name) or `path` (a file's), the one language it names \
     (`null` when none is registered).",
);
static FILE_TYPES: ActionMeta = meta(
    "EXT_FILE_TYPES",
    "Note file types",
    "The file types that are the vault's notes: `md`, and each one loaded extensions declare \
     (`[[file_types]]`) with its extension, name, icon, the `type` its files read as when their \
     frontmatter has none, and the extension it is from (`from`). A type stays a note type \
     after its extension is turned off (`registered`: in the vault's registry).",
);
static CAPTURE_TARGETS: ActionMeta = meta(
    "EXT_CAPTURE_TARGETS",
    "Capture targets",
    "What a capture can become that loaded extensions add (`[[capture_targets]]`): each one's \
     id, name, icon, description, the action that makes one from a capture, the chips it \
     offers, the quick-add tokens it adds and the extension it is from (`from`). Architect's \
     capture (CAPTURE_TARGETS) offers them after its own note.",
);
static PALETTE: ActionMeta = meta(
    "EXT_PALETTE",
    "The palette slot",
    "What the loaded extensions put in the palette (#381): `views`, which extension's view draws \
     each overlay of the shell's slot (`[[palette_views]]`: `palette` is ⌘K, `quick-open` ⌘O; \
     none, and it does not show); `keys`, the keys they bind to their actions (`[[keys]]`); \
     `sources`, the sources of the palette's rows (`[[palette_sources]]`). Each with the \
     extension it is from (`from`).",
);
static ENABLE: ActionMeta = meta(
    "EXT_ENABLE",
    "Turn an extension on",
    "Turns extension `id` back on: the copy that should run loads at once, everything it adds \
     with it.",
);
static DISABLE: ActionMeta = meta(
    "EXT_DISABLE",
    "Turn an extension off",
    "Turns extension `id` off: unloaded at once — its actions, settings, themes, tabs, status \
     items, menu entries, slots and runtime state go, its views close or say it is gone — and \
     left off until it is turned on.",
);

static SERVICES: ActionMeta = meta(
    "EXT_SERVICES",
    "Services and their providers",
    "Every service loaded extensions provide (`[provides]`): each one's providers with their \
     versions, the provider chosen for it (EXT_SERVICE_USE) and the one a call goes to now \
     (`in_use`: the chosen one while it is loaded, else the newest, the first by id on a tie).",
);
static SERVICE_USE: ActionMeta = meta(
    "EXT_SERVICE_USE",
    "Choose a service's provider",
    "Chooses extension `provider` for service `service` when several provide it: every call \
     that extension meets goes to it while it is loaded. `provider` null gives the choice back \
     to the host (the newest).",
);

/// `EXT_SERVICE_USE`'s arguments.
#[derive(Debug, Facet)]
pub struct ServiceUseArgs {
    pub service: String,
    pub provider: Option<String>,
}

/// `EXT_OPEN`'s arguments: one of `path`, `url`, `doc_kind`.
#[derive(Debug, Facet)]
pub struct OpenArgs {
    pub path: Option<String>,
    pub url: Option<String>,
    pub doc_kind: Option<String>,
}

/// `EXT_VIEW_OPEN`'s arguments.
#[derive(Debug, Facet)]
pub struct ViewOpenArgs {
    pub extension: String,
    pub view: String,
    pub state: Option<String>,
    /// `tab`, `split`, `overlay`, `popup`, `zoomed` or `float`.
    pub placement: Option<String>,
}

/// `EXT_VIEW_CLOSE`'s arguments.
#[derive(Debug, Facet)]
pub struct ViewCloseArgs {
    pub extension: String,
    pub view: String,
}

/// `EXT_STATE_SET`'s arguments.
#[derive(Debug, Facet)]
pub struct StateArgs {
    pub source: String,
    pub key: String,
    /// `status` (the default), `badge` or `meta`.
    pub kind: Option<String>,
    pub text: Option<String>,
    pub icon: Option<String>,
    pub target: Option<String>,
    pub seq: Option<u64>,
    pub ttl_ms: Option<u64>,
}

/// `EXT_STATE_CLEAR`'s arguments.
#[derive(Debug, Facet)]
pub struct ClearArgs {
    pub source: String,
    pub key: Option<String>,
}

/// `EXT_EMIT`'s arguments.
#[derive(Debug, Facet)]
pub struct EmitArgs {
    pub event: String,
    pub detail: Option<String>,
}

/// `EXT_EMBEDS`' arguments.
#[derive(Debug, Default, Facet)]
pub struct EmbedsArgs {
    /// `fence:kf`, `note_type:song`, `embed_type:setlist`.
    pub claim: Option<String>,
}

/// `EXT_PROJECT_TYPES`' arguments.
#[derive(Debug, Default, Facet)]
pub struct ProjectTypesArgs {
    pub id: Option<String>,
}

/// `EXT_LANGUAGES`' arguments: none for every language, or one way of
/// naming one.
#[derive(Debug, Default, Facet)]
pub struct LanguagesArgs {
    pub id: Option<String>,
    /// A code fence's name (`rs`).
    pub fence: Option<String>,
    /// A file's path or name (`src/main.rs`).
    pub path: Option<String>,
}

/// `DOC_UNDO` / `DOC_REDO`'s arguments.
#[derive(Debug, Default, Facet)]
pub struct DocStepArgs {
    /// The document; none: the active pane's (else the current
    /// document's), by its pane's state.
    pub path: Option<String>,
}

pub(crate) static DOC_UNDO: ActionMeta = ActionMeta {
    id: "DOC_UNDO",
    trait_name: "Documents",
    method_name: "undo",
    display_name: "Undo",
    description: "Takes back the last undo step of document `path` (a document's path, as `read_document` names it): its own history — a CRDT document's every step, the person's and each extension's; a vault note's edits made through the documents. Edits made since by others stay. Without `path`, the document shown in the active pane (else the current document), as an opener opened it. Answers `{step, text}` (the step's name and the text after), or null when there is nothing to undo.",
    category: "Edit",
    group: "Edit",
    shortcut: "",
    toggleable: false,
    undo: false,
};

pub(crate) static DOC_REDO: ActionMeta = ActionMeta {
    id: "DOC_REDO",
    trait_name: "Documents",
    method_name: "redo",
    display_name: "Redo",
    description: "Makes the step of document `path` undone last (`DOC_UNDO`) again; without `path`, the active pane's document. Answers `{step, text}`, or null when there is nothing to redo.",
    category: "Edit",
    group: "Edit",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// The document a pane-less `DOC_UNDO` acts on: the active pane's state
/// (else the current document's), the path its opener gave it.
fn shown_document(list: &architect::action::ActionList) -> Result<String, String> {
    let layout: Value = list
        .run_json("WORKSPACE_LAYOUT", b"null")
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or("no workspace to find the document in: give `path`")?;
    let pane = ["active", "current_document"]
        .iter()
        .filter_map(|k| layout.get(*k).and_then(Value::as_str))
        .find(|p| !p.is_empty())
        .ok_or("no pane is active: give `path`")?;
    let info: Value = list
        .run_json(
            "WORKSPACE_PANE_GET",
            json!({ "pane": pane }).to_string().as_bytes(),
        )
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or("the active pane is gone: give `path`")?;
    let state = info
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or_default();
    crate::documents::checked_path(state)
        .map(str::to_owned)
        .map_err(|_| format!("pane {pane} shows no document: give `path`"))
}

/// `EXT_ENABLE` / `EXT_DISABLE`'s arguments.
#[derive(Debug, Facet)]
pub struct IdArgs {
    pub id: String,
}

/// A placement by its name.
///
/// # Errors
///
/// It is not one.
pub fn placement(name: &str) -> Result<ViewPlacement, String> {
    Ok(match name {
        "tab" => ViewPlacement::Tab,
        "split" => ViewPlacement::Split,
        "overlay" => ViewPlacement::Overlay,
        "popup" => ViewPlacement::Popup,
        "zoomed" => ViewPlacement::Zoomed,
        "float" => ViewPlacement::Float,
        other => {
            return Err(format!(
                "`placement` is tab, split, overlay, popup, zoomed or float, not {other:?}"
            ));
        }
    })
}

/// A source the Action List may write as: not an extension's.
fn outside_source(source: &str) -> Result<(), String> {
    if source.starts_with("ext:") {
        return Err(format!(
            "{source} is an extension's: it writes its own state (its guest's set_state, a \
             script's ext.state)"
        ));
    }
    Ok(())
}

/// The invocation a door's call carries, or one from `source`.
fn invoked(caller: &ActionCaller, source: InvocationSource) -> Invocation {
    caller
        .invocation()
        .cloned()
        .unwrap_or_else(|| Invocation::new(source))
}

impl Extensions {
    /// `DOC_UNDO` and `DOC_REDO`: a document's own undo, for a view that
    /// is not a text editor (a chart), its menus, ⌘K and MCP.
    fn install_document_actions(&self) -> Result<(), ActionError> {
        let list = self.host().list().clone();
        for (meta, undo) in [(&DOC_UNDO, true), (&DOC_REDO, false)] {
            let me = self.clone();
            let shown = list.clone();
            list.register_with(
                meta,
                Effect::Mutating,
                move |caller: &ActionCaller, a: DocStepArgs| {
                    let host = me.host();
                    let docs = host.documents().ok_or("documents: this app gives none")?;
                    let path = match a.path.filter(|p| !p.is_empty()) {
                        Some(p) => crate::documents::checked_path(&p)?.to_owned(),
                        None => shown_document(&shown)?,
                    };
                    let by = caller.principal().cloned().unwrap_or_else(|| {
                        architect::action::Principal::Service {
                            name: "app".to_owned(),
                        }
                    });
                    let done = if undo {
                        docs.undo(&path, &by)
                    } else {
                        docs.redo(&path, &by)
                    }
                    .map_err(|e| e.to_string())?;
                    let answer = match done {
                        Some(d) => {
                            host.raise_doc(&crate::events::DocEvent::Saved { path });
                            json!({ "step": d.step, "text": d.text })
                        }
                        None => Value::Null,
                    };
                    Ok::<_, String>(RawJson::from(answer.to_string()))
                },
            )?;
        }
        Ok(())
    }

    /// Puts the contribution actions (`EXT_CONTRIBUTIONS`, `EXT_OPEN`,
    /// `EXT_VIEW_OPEN`, `EXT_STATE_SET`, `EXT_STATE_CLEAR`, `EXT_EMIT`,
    /// `EXT_ENABLE`, `EXT_DISABLE`, `EXT_EMBEDS`, `EXT_PROJECT_TYPES`,
    /// `EXT_LANGUAGES`, `EXT_FILE_TYPES`, `EXT_CAPTURE_TARGETS`) on the
    /// host's list.
    ///
    /// # Errors
    ///
    /// An id is taken.
    #[allow(clippy::too_many_lines)]
    pub fn install_contribution_actions(&self) -> Result<(), ActionError> {
        self.install_interop_actions()?;
        self.install_document_actions()?;
        self.install_toolbar_actions()?;
        let list = self.host().list().clone();
        let me = self.clone();
        list.register_with(
            &CONTRIBUTIONS,
            Effect::ReadOnly,
            move |_: &ActionCaller, (): ()| {
                Ok::<_, String>(RawJson::from(me.contributions_json().to_string()))
            },
        )?;
        let me = self.clone();
        list.register_async_with(
            &OPEN,
            Effect::Mutating,
            move |caller: &ActionCaller, a: OpenArgs| {
                let (me, caller) = (me.clone(), caller.clone());
                async move {
                    let what = match (a.path, a.url, a.doc_kind) {
                        (Some(path), None, None) => Opening::Path(path),
                        (None, Some(url), None) => Opening::Url(url),
                        (path, None, Some(kind)) => Opening::DocKind { kind, path },
                        _ => {
                            return Err(
                                "give one of `path`, `url` or `doc_kind` (with its `path`)"
                                    .to_owned(),
                            );
                        }
                    };
                    let answer = me
                        .host()
                        .open(what, invoked(&caller, InvocationSource::App))
                        .await?;
                    Ok(RawJson::from(answer.to_string()))
                }
            },
        )?;
        let me = self.clone();
        list.register_with(
            &VIEW_OPEN,
            Effect::Mutating,
            move |caller: &ActionCaller, a: ViewOpenArgs| {
                let placement = a.placement.as_deref().map(placement).transpose()?;
                let opened = me.host().open_view(
                    &a.extension,
                    &a.view,
                    a.state,
                    placement,
                    invoked(caller, InvocationSource::App),
                )?;
                Ok::<_, String>(RawJson::from(json!({ "opened": opened }).to_string()))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &VIEW_CLOSE,
            Effect::Mutating,
            move |_: &ActionCaller, a: ViewCloseArgs| {
                me.host().close_view(&a.extension, &a.view)?;
                Ok::<_, String>(RawJson::from(json!({ "closed": true }).to_string()))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &STATE_SET,
            Effect::Mutating,
            move |_: &ActionCaller, a: StateArgs| {
                outside_source(&a.source)?;
                let kind = match a.kind.as_deref() {
                    None | Some("status") => StateKind::Status,
                    Some("badge") => StateKind::Badge,
                    Some("meta") => StateKind::Meta,
                    Some(other) => {
                        return Err(format!("`kind` is status, badge or meta, not {other:?}"));
                    }
                };
                let applied = me
                    .host()
                    .contributions()
                    .set_state(
                        StateWrite {
                            source: a.source,
                            key: a.key,
                            kind,
                            text: a.text.unwrap_or_default(),
                            icon: a.icon.unwrap_or_default(),
                            target: a.target.unwrap_or_default(),
                            seq: a.seq.unwrap_or_default(),
                            ttl_ms: a.ttl_ms.unwrap_or_default(),
                        },
                        now_ms(),
                    )
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>(RawJson::from(json!({ "applied": applied }).to_string()))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &STATE_CLEAR,
            Effect::Mutating,
            move |_: &ActionCaller, a: ClearArgs| {
                outside_source(&a.source)?;
                let cleared = me
                    .host()
                    .contributions()
                    .clear_state(&a.source, a.key.as_deref());
                Ok::<_, String>(RawJson::from(json!({ "cleared": cleared }).to_string()))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &EMIT,
            Effect::Mutating,
            move |_: &ActionCaller, a: EmitArgs| {
                if !crate::contrib::known_event(&a.event) {
                    return Err(format!(
                        "{:?} is not an event ({}, changed:<topic>, app:<name>)",
                        a.event,
                        crate::contrib::EVENTS.join(", ")
                    ));
                }
                // A JSON object or list is the hooks' detail as it is
                // (`{"path": …}` for a document event); else, text.
                let text = a.detail.unwrap_or_default();
                let detail = serde_json::from_str::<Value>(&text)
                    .ok()
                    .filter(|v| v.is_object() || v.is_array())
                    .unwrap_or(Value::String(text));
                let started = me.host().emit_value(&a.event, &detail);
                Ok::<_, String>(RawJson::from(json!({ "started": started }).to_string()))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &EMBEDS,
            Effect::ReadOnly,
            move |_: &ActionCaller, a: EmbedsArgs| {
                let embeds = me.host().embeds();
                let Some(text) = a.claim else {
                    return Ok::<_, String>(RawJson::from(embeds_json(&me).to_string()));
                };
                let claim = crate::contrib::Claim::parse(&text).ok_or_else(|| {
                    format!("{text:?} is not a claim (fence:<lang>, note_type:<type>, embed_type:<type>)")
                })?;
                let answer = embeds.resolve(&claim).map_or(Value::Null, |spec| {
                    json!({
                        "claim": claim.to_string(),
                        "extension": spec.extension,
                        "embed": spec.embed.id,
                        "view": spec.embed.view,
                        "page": spec.page,
                        "kind": format!("{:?}", spec.kind).to_lowercase(),
                        "max_height": spec.max_height,
                    })
                });
                Ok(RawJson::from(answer.to_string()))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &LANGUAGES,
            Effect::ReadOnly,
            move |_: &ActionCaller, a: LanguagesArgs| {
                let languages = me.host().languages();
                let one = |r: crate::languages::Registered| {
                    let mut v = serde_json::to_value(&r.language).unwrap_or(Value::Null);
                    v.as_object_mut()
                        .map(|map| map.insert("from".to_owned(), json!(r.owner)));
                    v
                };
                let named = if let Some(id) = &a.id {
                    Some(languages.get(id))
                } else if let Some(fence) = &a.fence {
                    Some(languages.for_fence(fence))
                } else {
                    a.path.as_ref().map(|path| languages.for_path(path))
                };
                let answer = named.map_or_else(
                    || Value::Array(languages.all().into_iter().map(one).collect()),
                    |found| found.map_or(Value::Null, one),
                );
                Ok::<_, String>(RawJson::from(answer.to_string()))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &PROJECT_TYPES,
            Effect::ReadOnly,
            move |_: &ActionCaller, a: ProjectTypesArgs| {
                let host = me.host();
                let reg = host.contributions();
                let one = |t: crate::contrib::By<crate::contrib::ProjectTypeDecl>| {
                    let mut v = serde_json::to_value(&t.item).unwrap_or(Value::Null);
                    v.as_object_mut()
                        .map(|map| map.insert("from".to_owned(), json!(t.owner)));
                    v
                };
                let answer = a.id.map_or_else(
                    || Value::Array(reg.project_types().into_iter().map(one).collect()),
                    |id| reg.project_type(&id).map_or(Value::Null, one),
                );
                Ok::<_, String>(RawJson::from(answer.to_string()))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &FILE_TYPES,
            Effect::ReadOnly,
            move |_: &ActionCaller, (): ()| {
                let declared = me.host().contributions().file_types();
                let registered = vault_proto::note_types::declared();
                let mut all = vec![json!({
                    "extension": vault_proto::note_types::MARKDOWN,
                    "name": "Note",
                    "icon": "file-text",
                    "note_type": Value::Null,
                    "from": "app",
                    "registered": true,
                })];
                all.extend(declared.into_iter().map(|t| {
                    let mut v = serde_json::to_value(&t.item).unwrap_or(Value::Null);
                    if let Some(map) = v.as_object_mut() {
                        map.insert("from".to_owned(), json!(t.owner));
                        map.insert(
                            "registered".to_owned(),
                            json!(registered.iter().any(|r| r.extension == t.item.extension)),
                        );
                    }
                    v
                }));
                Ok::<_, String>(RawJson::from(Value::Array(all).to_string()))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &CAPTURE_TARGETS,
            Effect::ReadOnly,
            move |_: &ActionCaller, (): ()| {
                let host = me.host();
                let all: Vec<Value> = host
                    .contributions()
                    .capture_targets()
                    .into_iter()
                    .map(|t| {
                        let mut v = serde_json::to_value(&t.item).unwrap_or(Value::Null);
                        v.as_object_mut()
                            .map(|map| map.insert("from".to_owned(), json!(t.owner)));
                        v
                    })
                    .collect();
                Ok::<_, String>(RawJson::from(Value::Array(all).to_string()))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &PALETTE,
            Effect::ReadOnly,
            move |_: &ActionCaller, (): ()| {
                fn from<T: serde::Serialize>(by: &crate::contrib::By<T>) -> Value {
                    let mut v = serde_json::to_value(&by.item).unwrap_or(Value::Null);
                    v.as_object_mut()
                        .map(|map| map.insert("from".to_owned(), json!(by.owner)));
                    v
                }
                let reg = me.host().contributions();
                let answer = json!({
                    "views": reg.palette_views().iter().map(from).collect::<Vec<_>>(),
                    "keys": reg.key_bindings().iter().map(from).collect::<Vec<_>>(),
                    "sources": reg.palette_sources().iter().map(from).collect::<Vec<_>>(),
                });
                Ok::<_, String>(RawJson::from(answer.to_string()))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &SERVICES,
            Effect::ReadOnly,
            move |_: &ActionCaller, (): ()| {
                Ok::<_, String>(RawJson::from(services_json(&me.host()).to_string()))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &SERVICE_USE,
            Effect::Mutating,
            move |_: &ActionCaller, a: ServiceUseArgs| {
                let host = me.host();
                if let Some(id) = &a.provider
                    && host
                        .get(id)
                        .is_some_and(|l| !l.manifest.provides.contains_key(&a.service))
                {
                    return Err(format!("{id} does not provide {}", a.service));
                }
                host.use_provider(&a.service, a.provider.as_deref());
                Ok::<_, String>(RawJson::from(services_json(&host).to_string()))
            },
        )?;
        for (meta, on) in [(&ENABLE, true), (&DISABLE, false)] {
            let me = self.clone();
            list.register_with(
                meta,
                Effect::Mutating,
                move |_: &ActionCaller, a: IdArgs| {
                    let known =
                        me.chosen().iter().any(|c| c.runs.id == a.id) || me.is_disabled(&a.id);
                    if !known {
                        return Err(format!("there is no extension {}", a.id));
                    }
                    let events = if on {
                        me.enable(&a.id)
                    } else {
                        me.disable(&a.id)
                    };
                    let events: Vec<Value> = events.iter().map(event_json).collect();
                    Ok::<_, String>(RawJson::from(
                        json!({ "id": a.id, "on": on, "events": events }).to_string(),
                    ))
                },
            )?;
        }
        Ok(())
    }

    /// `EXT_CONTRIBUTIONS`'s answer.
    fn contributions_json(&self) -> Value {
        let reg = self.host().contributions();
        let owners: Vec<Value> = reg
            .owners()
            .into_iter()
            .map(|o| {
                json!({
                    "extension": o.owner,
                    "declared": o.declared,
                    "runtime": o.runtime,
                    "keys": o.now().keys(),
                })
            })
            .collect();
        let slots: serde_json::Map<String, Value> = reg
            .slots()
            .into_iter()
            .map(|(slot, (owner, view))| (slot, json!({ "extension": owner, "view": view })))
            .collect();
        json!({
            "revision": reg.revision(),
            "extensions": owners,
            "slots": slots,
            "embeds": embeds_json(self),
            "state": reg.state(now_ms()),
        })
    }
}

/// `EXT_SERVICES`' answer.
fn services_json(host: &crate::host::Host) -> Value {
    let services: Vec<Value> = host
        .service_providers()
        .into_iter()
        .map(|s| {
            json!({
                "service": s.service,
                "providers": s.providers.iter()
                    .map(|(id, v)| json!({ "id": id, "version": v.to_string() }))
                    .collect::<Vec<_>>(),
                "chosen": s.chosen,
                "in_use": s.in_use,
            })
        })
        .collect();
    json!({ "services": services })
}

/// Every inline embed, with its owner and claims.
fn embeds_json(extensions: &Extensions) -> Value {
    let embeds: Vec<Value> = extensions
        .host()
        .contributions()
        .embeds()
        .into_iter()
        .map(|by| {
            json!({
                "extension": by.owner,
                "id": by.item.id,
                "claims": by.item.claims.all().iter().map(ToString::to_string).collect::<Vec<_>>(),
            })
        })
        .collect();
    Value::Array(embeds)
}
