//! The `A2UI_*` actions: a surface rendered, patched, read and pressed
//! from anywhere the Action List reaches — MCP (a tool's result), the
//! command line (`<app> actions run A2UI_RENDER …`), a script.
//!
//! | id | effect | does |
//! |---|---|---|
//! | `A2UI_RENDER` | mutating | applies A2UI messages; a surface it creates replaces one of the same id, and the app's [`VIEW`] opens (or comes forward) to show it |
//! | `A2UI_PATCH` | mutating | applies updates to surfaces that exist (no `createSurface`) |
//! | `A2UI_SURFACE` | read-only | one surface (its components and data model), or every surface's id |
//! | `A2UI_PRESS` | mutating | presses a component, as the person would: its action runs as the surface's author and a write waits for the person's yes |
//! | `A2UI_CATALOG` | read-only | the catalog: its id, components, JSON Schema and the system prompt |
//!
//! Whoever renders a surface is its author: its events run as them.

use std::sync::Arc;

use a2ui_base::protocol::server_to_client::{
    A2uiMessage, A2uiPayload, DeleteSurfaceData, DeleteSurfacePayload,
};
use architect::action::{
    ActionCaller, ActionError, ActionFuture, ActionInfo, ActionList, CONFIRM_TTL,
    CallerAsyncHandler, Effect,
};
use serde_json::{Value, json};

use crate::catalog::{self, CATALOG_ID, VERSION};
use crate::events::{self, Outcome};
use crate::store::{self, Applied, Surfaces};

pub const RENDER: &str = "A2UI_RENDER";
pub const PATCH: &str = "A2UI_PATCH";
pub const SURFACE: &str = "A2UI_SURFACE";
pub const PRESS: &str = "A2UI_PRESS";
pub const CATALOG: &str = "A2UI_CATALOG";

/// The workspace view that shows the surfaces (feature `ui`).
pub const VIEW: &str = "a2ui";

fn info(id: &str, name: &str, description: &str, effect: Effect, params: &Value) -> ActionInfo {
    ActionInfo {
        id: id.to_owned(),
        display_name: name.to_owned(),
        description: description.to_owned(),
        category: "Agent UI".to_owned(),
        group: "A2UI".to_owned(),
        shortcut: String::new(),
        effect,
        params_schema: Some(params.to_string()),
        result_schema: None,
        enabled: true,
        disabled_reason: None,
        checked: None,
    }
}

fn messages_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "messages": {
                "description": "A2UI v1.0 messages: an array of them, or JSONL text. Components must be in the catalog (A2UI_CATALOG).",
                "oneOf": [
                    { "type": "array", "items": { "type": "object" } },
                    { "type": "string" },
                ],
            },
        },
        "required": ["messages"],
        "additionalProperties": false,
    })
}

fn bad(id: &str, message: impl Into<String>) -> ActionError {
    ActionError::BadArgs {
        id: id.to_owned(),
        message: message.into(),
        expected: None,
    }
}

fn args_of(id: &str, args: &[u8]) -> Result<Value, ActionError> {
    if args.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!({}));
    }
    let value: Value = serde_json::from_slice(args).map_err(|e| bad(id, e.to_string()))?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(bad(id, "arguments are an object"))
    }
}

fn messages_of(id: &str, args: &Value) -> Result<Vec<A2uiMessage>, ActionError> {
    match args.get("messages") {
        Some(Value::String(text)) => store::parse(text).map_err(|e| bad(id, e.to_string())),
        Some(Value::Array(items)) => items
            .iter()
            .cloned()
            .map(store::message)
            .collect::<Result<_, _>>()
            .map_err(|e| bad(id, e.to_string())),
        _ => Err(bad(
            id,
            "messages: an array of A2UI messages, or JSONL text",
        )),
    }
}

fn answer(surfaces: &Surfaces, applied: &Applied) -> Value {
    let outside: Vec<Value> = applied
        .surfaces
        .iter()
        .filter_map(|id| surfaces.get(id))
        .flat_map(|s| {
            let sid = s.id.clone();
            s.outside_catalog()
                .into_iter()
                .map(move |(id, c)| json!({ "surfaceId": sid, "id": id, "component": c }))
        })
        .collect();
    let mut touched = applied.surfaces.clone();
    touched.dedup();
    json!({
        "surfaces": touched,
        "components": applied.components,
        "writes": applied.writes,
        "deleted": applied.deleted,
        "outsideCatalog": outside,
        "view": VIEW,
    })
}

type Work = dyn Fn(&ActionCaller, Value) -> Result<Value, ActionError> + Send + Sync;

/// A handler doing `work` at once, its answer as JSON.
fn sync(id: &'static str, work: Arc<Work>) -> CallerAsyncHandler {
    Arc::new(move |caller: &ActionCaller, args: &[u8]| {
        let args = args_of(id, args)?;
        let out = work(caller, args)?;
        let run: ActionFuture =
            Box::pin(async move { Ok(serde_json::to_vec(&out).unwrap_or_default()) });
        Ok(run)
    })
}

/// The workspace's actions a render shows its surface with (#234).
const LAYOUT: &str = "WORKSPACE_LAYOUT";
const OPEN: &str = "WORKSPACE_OPEN";

/// Opens the [`VIEW`] (or brings it forward) through the workspace's
/// actions, as the app: beside the active pane, or in the right dock when
/// none is. False when the app has no workspace on its list now (the
/// lab's is the Vault's) or it would not.
async fn show(list: &ActionList) -> bool {
    let app = ActionCaller::app();
    let Ok(layout) = list.run_json_async_with(&app, LAYOUT, b"{}").await else {
        return false;
    };
    let active = serde_json::from_slice::<Value>(&layout)
        .ok()
        .and_then(|l| l.get("active").and_then(Value::as_str).map(str::to_owned))
        .filter(|a| !a.is_empty() && a != VIEW);
    let at = active.map_or_else(
        || json!({ "dock": { "side": "right" } }),
        |pane| json!({ "split": { "pane": pane, "side": "right" } }),
    );
    // Focused: open, or open already and brought forward.
    let args = json!({ "view": VIEW, "at": at, "focus": true });
    list.run_json_async_with(&app, OPEN, args.to_string().as_bytes())
        .await
        .is_ok()
}

fn render(list: &ActionList, surfaces: &Surfaces) -> CallerAsyncHandler {
    let (list, surfaces) = (list.clone(), surfaces.clone());
    Arc::new(move |caller: &ActionCaller, args: &[u8]| {
        let args = args_of(RENDER, args)?;
        let (mut out, created) = apply_render(&surfaces, caller, &args)?;
        let list = list.clone();
        let run: ActionFuture = Box::pin(async move {
            out["shown"] = json!(created && show(&list).await);
            Ok(serde_json::to_vec(&out).unwrap_or_default())
        });
        Ok(run)
    })
}

/// Applies a render's messages; the answer, and whether it created a
/// surface.
fn apply_render(
    surfaces: &Surfaces,
    caller: &ActionCaller,
    args: &Value,
) -> Result<(Value, bool), ActionError> {
    let mut messages = messages_of(RENDER, args)?;
    // A surface rendered again is replaced: deleted first, in the same
    // step.
    let existing = surfaces.ids();
    let mut replace = Vec::new();
    let mut created = false;
    for m in &messages {
        if let A2uiPayload::CreateSurface(p) = &m.payload {
            created = true;
            if existing.contains(&p.create_surface.surface_id) {
                replace.push(A2uiMessage {
                    version: VERSION.to_owned(),
                    payload: A2uiPayload::DeleteSurface(DeleteSurfacePayload {
                        delete_surface: DeleteSurfaceData {
                            surface_id: p.create_surface.surface_id.clone(),
                        },
                    }),
                });
            }
        }
    }
    let replaced = replace.len();
    replace.append(&mut messages);
    let mut applied =
        surfaces
            .apply(&replace, caller.principal())
            .map_err(|e| ActionError::Failed {
                id: RENDER.to_owned(),
                message: e.to_string(),
            })?;
    applied.deleted.drain(..replaced.min(applied.deleted.len()));
    Ok((answer(surfaces, &applied), created))
}

fn patch(surfaces: &Surfaces) -> CallerAsyncHandler {
    let surfaces = surfaces.clone();
    sync(
        PATCH,
        Arc::new(move |caller, args| {
            let messages = messages_of(PATCH, &args)?;
            if messages
                .iter()
                .any(|m| matches!(m.payload, A2uiPayload::CreateSurface(_)))
            {
                return Err(bad(PATCH, "createSurface: render it with A2UI_RENDER"));
            }
            let applied =
                surfaces
                    .apply(&messages, caller.principal())
                    .map_err(|e| ActionError::Failed {
                        id: PATCH.to_owned(),
                        message: e.to_string(),
                    })?;
            Ok(answer(&surfaces, &applied))
        }),
    )
}

fn surface(surfaces: &Surfaces) -> CallerAsyncHandler {
    let surfaces = surfaces.clone();
    sync(
        SURFACE,
        Arc::new(move |_, args| {
            args.get("surfaceId").and_then(Value::as_str).map_or_else(
                || Ok(json!({ "surfaces": surfaces.ids() })),
                |id| {
                    surfaces
                        .get(id)
                        .map(|s| s.to_json())
                        .ok_or_else(|| ActionError::Failed {
                            id: SURFACE.to_owned(),
                            message: format!("no surface “{id}”"),
                        })
                },
            )
        }),
    )
}

fn press(list: &ActionList, surfaces: &Surfaces) -> CallerAsyncHandler {
    let (list, surfaces) = (list.clone(), surfaces.clone());
    Arc::new(move |_: &ActionCaller, args: &[u8]| {
        let args = args_of(PRESS, args)?;
        let text = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_owned);
        let surface = text("surfaceId").ok_or_else(|| bad(PRESS, "surfaceId is required"))?;
        let component = text("componentId").ok_or_else(|| bad(PRESS, "componentId is required"))?;
        let base = text("path").unwrap_or_default();
        let ticket = text("ticket");
        let (list, surfaces) = (list.clone(), surfaces.clone());
        let run: ActionFuture = Box::pin(async move {
            match events::press(
                &list,
                &surfaces,
                (&surface, &component, &base),
                ticket.as_deref(),
            )
            .await
            {
                Outcome::Ran(value) => {
                    Ok(serde_json::to_vec(&json!({ "ran": value })).unwrap_or_default())
                }
                Outcome::Waiting { id, ticket } => Err(ActionError::ConfirmationRequired {
                    id,
                    ticket,
                    expires_in: CONFIRM_TTL,
                }),
                Outcome::Refused(message) => Err(ActionError::Failed {
                    id: PRESS.to_owned(),
                    message,
                }),
            }
        });
        Ok(run)
    })
}

fn catalog_handler() -> CallerAsyncHandler {
    sync(
        CATALOG,
        Arc::new(|_, _| {
            Ok(json!({
                "catalogId": CATALOG_ID,
                "version": VERSION,
                "components": catalog::names(),
                "prompt": catalog::prompt(),
                "schema": catalog::schema(),
            }))
        }),
    )
}

/// Puts the `A2UI_*` actions on `list`, over `surfaces`.
///
/// # Errors
///
/// One of the ids is already on the list.
pub fn register(list: &ActionList, surfaces: &Surfaces) -> Result<(), ActionError> {
    let id_schema = json!({
        "type": "object",
        "properties": { "surfaceId": { "type": "string", "description": "Leave it out for every surface's id." } },
        "additionalProperties": false,
    });
    let press_schema = json!({
        "type": "object",
        "properties": {
            "surfaceId": { "type": "string" },
            "componentId": { "type": "string", "description": "A Button (or anything with an action)." },
            "path": { "type": "string", "description": "A template item's data path, when the component is one." },
            "ticket": { "type": "string", "description": "The confirmation ticket a person approved." },
        },
        "required": ["surfaceId", "componentId"],
        "additionalProperties": false,
    });
    list.register_forwarded_with(
        &info(
            RENDER,
            "Render an agent UI",
            "Shows the person a UI: applies A2UI v1.0 messages (createSurface, updateComponents, updateDataModel, deleteSurface), drawn with the app's components. A surface rendered again is replaced. Only the catalog's components are drawn (A2UI_CATALOG has them and a prompt). Answers the surfaces touched and any component outside the catalog. A surface it creates is shown: the app opens its Agent UI view, or brings it forward (`shown`).",
            Effect::Mutating,
            &messages_schema(),
        ),
        render(list, surfaces),
    )?;
    list.register_forwarded_with(
        &info(
            PATCH,
            "Change an agent UI",
            "Changes surfaces already shown, live: updateComponents, updateDataModel (a JSON Pointer upsert; no value deletes), deleteSurface. Not createSurface: that is A2UI_RENDER.",
            Effect::Mutating,
            &messages_schema(),
        ),
        patch(surfaces),
    )?;
    list.register_forwarded_with(
        &info(
            SURFACE,
            "Read an agent UI",
            "One surface's components and data model (what the person entered), or every surface's id.",
            Effect::ReadOnly,
            &id_schema,
        ),
        surface(surfaces),
    )?;
    list.register_forwarded_with(
        &info(
            PRESS,
            "Press in an agent UI",
            "Presses a component, as the person would: its action runs as the surface's author, and one that changes something waits for the person's yes (run again with the ticket once they approve).",
            Effect::Mutating,
            &press_schema,
        ),
        press(list, surfaces),
    )?;
    list.register_forwarded_with(
        &info(
            CATALOG,
            "The agent UI catalog",
            "The components a surface may use: their JSON Schema, and a prompt that describes them to a model.",
            Effect::ReadOnly,
            &json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        catalog_handler(),
    )
}
