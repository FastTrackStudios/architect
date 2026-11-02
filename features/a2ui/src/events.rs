//! A surface's events become Action List calls.
//!
//! A Button's `{"event": {"name": "TASKS_ADD", "context": {…}}}` runs the
//! action `TASKS_ADD` with the context resolved against the surface's data
//! model as its arguments. The Action List is the only command path: there
//! is no other way for a surface to change anything outside itself.
//!
//! The call is made as whoever rendered the surface (its author, an agent
//! over MCP), through the list's authorizer, and
//! [`ActionCaller::confirming_writes`]: the person pressed the button, but
//! the agent wrote what it runs, so anything that is not read-only waits
//! for the person's yes in the confirmation sheet
//! (`architect_commands::ConfirmSheet`). [`Outcome::Waiting`] carries the
//! ticket; [`answered`] tells when the person answered, and the same call
//! runs again carrying it. A surface the app rendered itself runs as the
//! app.

use std::collections::HashMap;

use a2ui_base::catalog::basic_functions::build_basic_functions;
use a2ui_base::catalog::function_api::FunctionImplementation;
use a2ui_base::model::data_context::DataContext;
use a2ui_base::model::data_model::DataModel;
use a2ui_base::protocol::common_types::Action;
use architect::action::{
    ActionCaller, ActionError, ActionEvent, ActionList, CONFIRM_TOPIC, EventKind,
};
use serde_json::{Map, Value};

use crate::catalog;
use crate::store::{Surface, Surfaces};

/// What pressing a component did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The action ran; its answer.
    Ran(Value),
    /// It waits for the person's yes: `ticket`, for `id`.
    Waiting { id: String, ticket: String },
    /// It did not run, and why.
    Refused(String),
}

/// The caller `surface`'s events run as.
#[must_use]
pub fn caller(surface: &Surface) -> ActionCaller {
    surface
        .author
        .clone()
        .map_or_else(ActionCaller::app, |who| {
            ActionCaller::new(who).confirming_writes()
        })
}

/// An event's action id and its arguments, resolved at `base` (a template
/// item's path, or `""`).
///
/// # Errors
///
/// No such component, it is outside the catalog, it has no event, or its
/// action is a local function (not run from a surface here).
pub fn resolve(
    surface: &Surface,
    component: &str,
    base: &str,
) -> Result<(String, Value, Option<String>), String> {
    let model = surface.model();
    let components = model.components.borrow();
    let c = components
        .get(component)
        .ok_or_else(|| format!("no component “{component}”"))?;
    if catalog::component(&c.component_type).is_none() {
        return Err(catalog::refusal(&c.component_type));
    }
    match c.action() {
        Some(Action::Event { event }) => {
            let data = DataModel::from_value(surface.data.clone());
            let functions = functions();
            let root = DataContext::new(&data, &functions);
            let cx = if base.is_empty() {
                root
            } else {
                root.nested(base.trim_start_matches('/'))
            };
            let mut args = Map::new();
            let mut keys: Vec<&String> = event.context.keys().collect();
            keys.sort();
            for key in keys {
                if let Some(v) = event.context.get(key) {
                    args.insert(key.clone(), cx.resolve_dynamic_value(v));
                }
            }
            // The catalog (and A2UI) spell them `wantResponse` and
            // `responsePath`; `a2ui-base` reads only `want_response` and
            // `response_path`.
            let raw = c.get_raw("action").and_then(|a| a.get("event"));
            let field = |camel: &str| raw.and_then(|e| e.get(camel));
            let wants =
                event.want_response || field("wantResponse").and_then(Value::as_bool) == Some(true);
            let respond = wants
                .then(|| {
                    event.response_path.clone().or_else(|| {
                        field("responsePath")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                })
                .flatten();
            Ok((event.name, Value::Object(args), respond))
        }
        Some(Action::FunctionCall { function_call }) => Err(format!(
            "“{}” is a local function; a surface here runs only Action List actions",
            function_call.call
        )),
        None => Err(format!("“{component}” has no action")),
    }
}

/// `a2ui-base`'s client functions (formatting, checks), for bindings.
#[must_use]
pub fn functions() -> HashMap<String, Box<dyn FunctionImplementation>> {
    build_basic_functions()
        .into_iter()
        .map(|f| (f.name().to_owned(), f))
        .collect()
}

/// Runs the action of `component` on surface `surface` (at template path
/// `base`), carrying `ticket` when the person approved one.
pub async fn press(
    list: &ActionList,
    surfaces: &Surfaces,
    (surface, component, base): (&str, &str, &str),
    ticket: Option<&str>,
) -> Outcome {
    let span = tracing::info_span!(
        "a2ui.event",
        a2ui.surface = surface,
        a2ui.component = component,
        a2ui.action = tracing::field::Empty,
        a2ui.outcome = tracing::field::Empty,
    );
    let Some(found) = surfaces.get(surface) else {
        span.record("a2ui.outcome", "no_surface");
        return Outcome::Refused(format!("no surface “{surface}”"));
    };
    let (id, args, respond) = match resolve(&found, component, base) {
        Ok(r) => r,
        Err(why) => {
            span.record("a2ui.outcome", "refused");
            return Outcome::Refused(why);
        }
    };
    span.record("a2ui.action", id.as_str());
    let mut who = caller(&found);
    if let Some(t) = ticket {
        who = who.with_ticket(t);
    }
    let bytes = serde_json::to_vec(&args).unwrap_or_default();
    let outcome = match list.run_json_async_with(&who, &id, &bytes).await {
        Ok(out) => {
            let value = serde_json::from_slice(&out).unwrap_or(Value::Null);
            if let Some(path) = respond {
                // The answer where the surface asked for it: best effort.
                let _ = surfaces.set_data(surface, &path, Some(&value));
            }
            Outcome::Ran(value)
        }
        Err(ActionError::ConfirmationRequired { id, ticket, .. }) => {
            Outcome::Waiting { id, ticket }
        }
        Err(e) => Outcome::Refused(e.to_string()),
    };
    span.record(
        "a2ui.outcome",
        match &outcome {
            Outcome::Ran(_) => "ran",
            Outcome::Waiting { .. } => "waiting",
            Outcome::Refused(_) => "refused",
        },
    );
    outcome
}

/// Whether the person answered `ticket` in `events`: `Some(true)` for
/// yes, `Some(false)` for no.
#[must_use]
pub fn answered(events: &[ActionEvent], ticket: &str) -> Option<bool> {
    events.iter().rev().find_map(|e| match &e.kind {
        EventKind::Changed { topic, detail } if topic == CONFIRM_TOPIC => {
            let (t, state) = detail.split_once(' ')?;
            (t == ticket).then_some(state)?;
            match state {
                "approved" => Some(true),
                "denied" => Some(false),
                _ => None,
            }
        }
        _ => None,
    })
}
