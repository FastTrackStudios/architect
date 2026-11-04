//! The Action List's face of windows, points and services (feature
//! `host`; `crate::interop`):
//!
//! | action | what it does |
//! |---|---|
//! | `EXT_SERVICE_CALL` | `{service, method, args?}`: a service call as the extension that runs it (a page's way; others use the host service's `service`) |
//! | `EXT_POINTS` | every point, its owner and items, and the items left out and why |
//! | `EXT_WINDOWS` | every extension window, and which are open with what state |
//! | `EXT_WINDOW_CLOSE` | closes `{extension, id}` |

use std::sync::Arc;

use architect::action::{
    ActionCaller, ActionError, ActionFuture, ActionInfo, CallerAsyncHandler, Effect, Principal,
};
use serde_json::{Value, json};

use crate::extensions::Extensions;
use crate::hostcall::HostRequest;

fn info(id: &str, name: &str, effect: Effect, description: &str, params: &str) -> ActionInfo {
    ActionInfo {
        id: id.to_owned(),
        display_name: name.to_owned(),
        description: description.to_owned(),
        category: "Extensions".to_owned(),
        group: "Extensions".to_owned(),
        shortcut: String::new(),
        effect,
        params_schema: (!params.is_empty()).then(|| params.to_owned()),
        result_schema: None,
        enabled: true,
        disabled_reason: None,
        checked: None,
    }
}

fn args_of(id: &str, args: &[u8]) -> Result<Value, ActionError> {
    if args.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(args).map_err(|e| ActionError::BadArgs {
        id: id.to_owned(),
        message: e.to_string(),
        expected: None,
    })
}

fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn answer(value: Value) -> ActionFuture {
    Box::pin(async move { Ok(value.to_string().into_bytes()) })
}

impl Extensions {
    /// Puts `EXT_SERVICE_CALL`, `EXT_POINTS`, `EXT_WINDOWS` and
    /// `EXT_WINDOW_CLOSE` on the host's list (with
    /// `install_contribution_actions`).
    ///
    /// # Errors
    ///
    /// One of their ids is taken.
    pub fn install_interop_actions(&self) -> Result<(), ActionError> {
        let list = self.host().list().clone();
        let host = self.host().clone();
        let call: CallerAsyncHandler = Arc::new(move |caller: &ActionCaller, args: &[u8]| {
            const ID: &str = "EXT_SERVICE_CALL";
            let a = args_of(ID, args)?;
            let Some(Principal::Extension { id, on_behalf_of }) = caller.principal().cloned()
            else {
                return Err(ActionError::Denied {
                    id: ID.to_owned(),
                    reason: "only an extension calls a service (its [requires])".to_owned(),
                });
            };
            let api = host.api(&id).acting_for(on_behalf_of.map(|b| *b));
            let request = HostRequest::Service {
                service: text(&a, "service"),
                method: text(&a, "method"),
                args: a.get("args").cloned().unwrap_or(Value::Null),
            };
            let out: ActionFuture = Box::pin(async move {
                api.call(request)
                    .await
                    .map(|v| v.to_string().into_bytes())
                    .map_err(|e| ActionError::Failed {
                        id: ID.to_owned(),
                        message: e.to_string(),
                    })
            });
            Ok(out)
        });
        list.register_forwarded_with(
            &info(
                "EXT_SERVICE_CALL",
                "Call an extension's service",
                Effect::Mutating,
                "Calls method `method` of service `service` with `args`, as the extension running it \
                 (its page): the provider of a version its [requires] meets answers, with its action \
                 <PREFIX><METHOD>, within its capabilities. Anyone but an extension is refused.",
                r#"{"type":"object","properties":{"service":{"type":"string"},"method":{"type":"string"},"args":{}},"required":["service","method"]}"#,
            ),
            call,
        )?;
        let host = self.host().clone();
        list.register_forwarded_with(
            &info(
                "EXT_POINTS",
                "Contribution points",
                Effect::ReadOnly,
                "Every contribution point ([[points]]): its owner, schema and the items it has \
                 (who gave each); the items left out because they do not meet its schema, and why; \
                 items given for points nobody declares.",
                "",
            ),
            Arc::new(move |_: &ActionCaller, _: &[u8]| {
                let points = host.points();
                let all: Vec<Value> = points
                    .all()
                    .into_iter()
                    .map(|d| {
                        json!({
                            "id": d.decl.id,
                            "owner": d.owner,
                            "title": d.decl.title,
                            "schema": d.schema,
                            "deliver": d.decl.deliver,
                            "items": points.items(&d.decl.id),
                        })
                    })
                    .collect();
                Ok(answer(json!({
                    "points": all,
                    "rejected": points.rejected(),
                    "waiting": points.waiting(),
                })))
            }),
        )?;
        let host = self.host().clone();
        list.register_forwarded_with(
            &info(
                "EXT_WINDOWS",
                "Extension windows",
                Effect::ReadOnly,
                "Every extension's windows ([[windows]]): its view, kind (sheet or window), the \
                 action that opens it, and whether it is open, with its state.",
                "",
            ),
            Arc::new(move |_: &ActionCaller, _: &[u8]| {
                let open = host.open_windows();
                let all: Vec<Value> = host
                    .window_decls()
                    .into_iter()
                    .map(|(ext, w)| {
                        let now = open
                            .iter()
                            .find(|o| o.extension == ext && o.window.id == w.id);
                        json!({
                            "extension": ext,
                            "id": w.id,
                            "view": w.view,
                            "title": w.title,
                            "kind": w.kind.name(),
                            "opens": w.opens,
                            "open": now.is_some(),
                            "state": now.map(|o| o.state.clone()),
                        })
                    })
                    .collect();
                Ok(answer(json!({ "windows": all })))
            }),
        )?;
        let host = self.host().clone();
        list.register_forwarded_with(
            &info(
                "EXT_WINDOW_CLOSE",
                "Close an extension window",
                Effect::Mutating,
                "Closes extension `extension`'s window `id`: whether it was open.",
                r#"{"type":"object","properties":{"extension":{"type":"string"},"id":{"type":"string"}},"required":["extension","id"]}"#,
            ),
            Arc::new(move |_: &ActionCaller, args: &[u8]| {
                let a = args_of("EXT_WINDOW_CLOSE", args)?;
                let closed = host.close_window(&text(&a, "extension"), &text(&a, "id"));
                Ok(answer(json!({ "closed": closed })))
            }),
        )?;
        Ok(())
    }
}
