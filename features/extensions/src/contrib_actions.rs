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
//! | `EXT_STATE_SET` | writes runtime state as `{source, key, kind?, text?, icon?, target?, seq?, ttl_ms?}` |
//! | `EXT_STATE_CLEAR` | clears `{source, key?}`'s state |
//! | `EXT_EMIT` | raises app event `{event, detail?}`: the hooks on it run |
//! | `EXT_ENABLE` / `EXT_DISABLE` | turns `{id}` on or off: loaded or unloaded at once, everything it adds with it |
//! | `EXT_EMBEDS` | every inline embed and what it claims, or `{claim}`'s (`fence:kf`): which extension draws it, and how |
//! | `EXT_PROJECT_TYPES` | every project type loaded extensions add (`[[project_types]]`), or `{id}`'s: its properties, template, folders, views and owner |

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
    /// Puts the contribution actions (`EXT_CONTRIBUTIONS`, `EXT_OPEN`,
    /// `EXT_VIEW_OPEN`, `EXT_STATE_SET`, `EXT_STATE_CLEAR`, `EXT_EMIT`,
    /// `EXT_ENABLE`, `EXT_DISABLE`, `EXT_EMBEDS`, `EXT_PROJECT_TYPES`) on
    /// the host's list.
    ///
    /// # Errors
    ///
    /// An id is taken.
    #[allow(clippy::too_many_lines)]
    pub fn install_contribution_actions(&self) -> Result<(), ActionError> {
        self.install_interop_actions()?;
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
            &PROJECT_TYPES,
            Effect::ReadOnly,
            move |_: &ActionCaller, a: ProjectTypesArgs| {
                let host = me.host();
                let reg = host.contributions();
                let one = |t: crate::contrib::By<crate::contrib::ProjectTypeDecl>| {
                    let mut v = serde_json::to_value(&t.item).unwrap_or(Value::Null);
                    if let Some(map) = v.as_object_mut() {
                        map.insert("from".to_owned(), json!(t.owner));
                    }
                    v
                };
                let answer = match a.id {
                    Some(id) => reg.project_type(&id).map_or(Value::Null, one),
                    None => Value::Array(reg.project_types().into_iter().map(one).collect()),
                };
                Ok::<_, String>(RawJson::from(answer.to_string()))
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
