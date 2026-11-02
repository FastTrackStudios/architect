//! [`Surface`]: an app's Action List as the one thing every face asks.

use std::time::Duration;

use architect::action::{ActionCaller, ActionEvent, ActionInfo, ActionList, EventKind, Principal};
use serde_json::{Value, json};

use crate::protocol::{
    Answer, Batch, Code, DEFAULT_EVENTS, Failure, Filter, MAX_BATCH_STEPS, MAX_EVENTS, MAX_WAIT,
    Method, PROTOCOL, Request, Step, StepResult, Watch, effect_name,
};

/// Who a call is made as when its caller names nobody.
static ANONYMOUS: Principal = Principal::Anonymous;

/// An app's Action List, its name and version, and who a caller that
/// names nobody is: what MCP's server and the control socket both answer
/// from.
///
/// Every call is made as a [`Principal`] the list's authorizer checks —
/// the caller's own, or the surface's ([`Self::with_principal`],
/// `Anonymous` unless set) when it names none. A caller from outside is
/// never the trusted app: [`ActionCaller::app`] given to [`Self::answer`]
/// runs as the surface's principal too.
#[derive(Clone, Debug)]
pub struct Surface {
    name: String,
    version: String,
    list: ActionList,
    principal: Principal,
}

impl Surface {
    /// `list`, introduced as `name` `version`.
    #[must_use]
    pub fn new(name: impl Into<String>, version: impl Into<String>, list: ActionList) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            list,
            principal: Principal::Anonymous,
        }
    }

    /// Who a caller that names nobody is, as the list's authorizer sees
    /// it.
    #[must_use]
    pub fn with_principal(mut self, principal: Principal) -> Self {
        self.principal = principal;
        self
    }

    /// The app's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The app's version.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The list it answers from.
    #[must_use]
    pub const fn list(&self) -> &ActionList {
        &self.list
    }

    /// Who a caller that names nobody is.
    #[must_use]
    pub const fn principal(&self) -> &Principal {
        &self.principal
    }

    /// `caller`, made as the surface's principal when it names none.
    #[must_use]
    pub fn caller(&self, caller: &ActionCaller) -> ActionCaller {
        match caller.principal() {
            Some(_) => caller.clone(),
            None => caller.clone().with_principal(self.principal.clone()),
        }
    }

    /// Answer `request` for `caller` (made as the surface's principal
    /// when it names none). A `watch` with a wait blocks the calling
    /// thread while it waits.
    pub async fn answer(&self, caller: &ActionCaller, request: &Request) -> Answer {
        let caller = self.caller(caller);
        let who = caller.principal().unwrap_or(&ANONYMOUS);
        match request {
            Request::Actions(filter) => Answer::Done(self.actions(who, filter)),
            Request::Describe { id } => self.list.info_for(who, id).map_or_else(
                || {
                    Answer::Failed(Failure::new(
                        Code::Unknown,
                        "no such action — find it with `actions`",
                    ))
                },
                |info| Answer::Done(detail(&info)),
            ),
            Request::Run { id, args, ticket } => match self
                .run_one(&with_ticket(&caller, ticket.as_deref()), id, args.as_ref())
                .await
            {
                Ok(value) => Answer::Done(value),
                Err(failure) => Answer::Failed(failure),
            },
            Request::Batch {
                steps,
                stop_on_error,
            } => Answer::Batch(self.batch(&caller, steps, *stop_on_error).await),
            Request::Watch(watch) => Answer::Done(self.watch(*watch)),
            Request::Schema => Answer::Done(self.schema(who)),
            Request::Hello => Answer::Done(self.hello(who)),
        }
    }

    /// The actions `who` may run, each as `actions` lists it: what MCP's
    /// `actions://list` resource gives.
    #[must_use]
    pub fn listing(&self, who: &Principal) -> Vec<Value> {
        self.list.infos_for(who).iter().map(summary).collect()
    }

    fn actions(&self, who: &Principal, filter: &Filter) -> Value {
        let text = filter.text.as_deref().map(str::to_lowercase);
        let category = filter.category.as_deref().map(str::to_lowercase);
        let effect = filter.effect.as_deref().map(str::to_lowercase);
        let found: Vec<Value> = self
            .list
            .infos_for(who)
            .iter()
            .filter(|i| {
                text.as_deref().is_none_or(|t| {
                    [
                        &i.id,
                        &i.display_name,
                        &i.description,
                        &i.category,
                        &i.group,
                    ]
                    .iter()
                    .any(|f| f.to_lowercase().contains(t))
                })
            })
            .filter(|i| {
                category
                    .as_deref()
                    .is_none_or(|c| i.category.to_lowercase() == c)
            })
            .filter(|i| effect.as_deref().is_none_or(|e| effect_name(i.effect) == e))
            .map(summary)
            .collect();
        json!({ "count": found.len(), "actions": found })
    }

    /// Run one action as `caller`: its JSON result, or what went wrong.
    ///
    /// # Errors
    ///
    /// The Action List's refusal or the action's failure, as a
    /// [`Failure`].
    pub async fn run_one(
        &self,
        caller: &ActionCaller,
        id: &str,
        args: Option<&Value>,
    ) -> Result<Value, Failure> {
        let bytes = match args {
            None | Some(Value::Null) => Vec::new(),
            Some(v) => v.to_string().into_bytes(),
        };
        let out = self
            .list
            .run_json_async_with(caller, id, &bytes)
            .await
            .map_err(|e| Failure::from_action(&e))?;
        Ok(serde_json::from_slice(&out).unwrap_or(Value::Null))
    }

    /// Each step runs as `caller`, through the same door as `run` — a
    /// step is authorized on its own. Not atomic: what ran stays done.
    async fn batch(&self, caller: &ActionCaller, steps: &[Step], stop: bool) -> Batch {
        if steps.len() > MAX_BATCH_STEPS {
            return Batch {
                of: steps.len(),
                results: Vec::new(),
                refused: Some(Failure::new(
                    Code::BadRequest,
                    format!(
                        "{} steps; a batch runs at most {MAX_BATCH_STEPS} — split it",
                        steps.len()
                    ),
                )),
            };
        }
        let mut results = Vec::with_capacity(steps.len());
        for (i, step) in steps.iter().enumerate() {
            let caller = with_ticket(caller, step.ticket.as_deref());
            let outcome = self.run_one(&caller, &step.id, step.args.as_ref()).await;
            let failed = outcome.is_err();
            results.push(StepResult {
                step: i,
                id: step.id.clone(),
                outcome,
            });
            if failed && stop {
                break;
            }
        }
        Batch {
            of: steps.len(),
            results,
            refused: None,
        }
    }

    /// The events after `since` (from now when it is `None`), and the
    /// `next` to ask with. `lost` says some were missed — the app
    /// restarted since (`since` is past its latest) or the list no longer
    /// keeps them — so a watcher reads the state again rather than trust
    /// what it holds: events are invalidation, not a replay.
    fn watch(&self, watch: Watch) -> Value {
        let latest = self.list.latest();
        let since = watch.since.unwrap_or(latest);
        if since > latest {
            return json!({ "next": latest, "lost": true, "events": [] });
        }
        let max = watch
            .max
            .and_then(|m| usize::try_from(m).ok())
            .unwrap_or(DEFAULT_EVENTS)
            .min(MAX_EVENTS);
        let wait = watch
            .wait_ms
            .map_or(Duration::ZERO, Duration::from_millis)
            .min(MAX_WAIT);
        let (events, next) = self.list.events_since(since, max, wait);
        // Sequence numbers run on by one: a first event further on than
        // the next after `since` means the ones between are gone.
        let lost = events
            .first()
            .is_some_and(|e| e.seq > since.saturating_add(1));
        json!({
            "next": next,
            "lost": lost,
            "events": events.iter().map(event).collect::<Vec<_>>(),
        })
    }

    /// Every action `who` may run as one JSON Schema document: a run's
    /// `{id, args}` for each, its effect and what it gives back.
    #[must_use]
    pub fn schema(&self, who: &Principal) -> Value {
        let actions: Vec<Value> = self
            .list
            .infos_for(who)
            .iter()
            .map(|info| {
                let mut properties = serde_json::Map::new();
                properties.insert("id".to_owned(), json!({ "const": info.id }));
                let mut required = vec![Value::String("id".to_owned())];
                if let Some(takes) = schema_of(info.params_schema.as_ref()) {
                    properties.insert("args".to_owned(), takes);
                    required.push(Value::String("args".to_owned()));
                }
                json!({
                    "title": info.id,
                    "description": describe_text(info),
                    "type": "object",
                    "required": required,
                    "properties": properties,
                    "additionalProperties": false,
                    "x-effect": effect_name(info.effect),
                    "x-gives": schema_of(info.result_schema.as_ref()),
                })
            })
            .collect();
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": format!("{}'s Action List", self.name),
            "description": "A run of one action: its id and the arguments it takes.",
            "oneOf": actions,
        })
    }

    /// The protocol's own methods as JSON Schema: what `api schema`
    /// prints, MCP's `tools/list` in another form.
    #[must_use]
    pub fn api_schema(&self) -> Value {
        let methods: Vec<Value> = Method::ALL
            .iter()
            .map(|m| {
                json!({
                    "title": m.name(),
                    "description": m.description(),
                    "type": "object",
                    "required": ["id", "method"],
                    "properties": {
                        "id": {},
                        "method": {"const": m.name()},
                        "params": m.input_schema(),
                    },
                    "x-read-only": m.read_only(),
                })
            })
            .collect();
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": PROTOCOL,
            "description": format!(
                "{}'s control socket: one JSON request a line, `{{id, method, params}}`, \
                 answered `{{id, result}}` or `{{id, error: {{code, error, …}}}}`.",
                self.name
            ),
            "oneOf": methods,
        })
    }

    fn hello(&self, who: &Principal) -> Value {
        json!({
            "name": self.name,
            "version": self.version,
            "protocol": PROTOCOL,
            "methods": Method::ALL.iter().map(|m| m.name()).collect::<Vec<_>>(),
            "actions": self.list.infos_for(who).len(),
            "latest": self.list.latest(),
        })
    }
}

/// `caller`, carrying `ticket` when the call brought one: a person's
/// approval of this exact call (`ActionCaller::with_ticket`).
#[must_use]
pub fn with_ticket(caller: &ActionCaller, ticket: Option<&str>) -> ActionCaller {
    ticket.map_or_else(
        || caller.clone(),
        |ticket| caller.clone().with_ticket(ticket),
    )
}

/// A schema text as JSON.
fn schema_of(text: Option<&String>) -> Option<Value> {
    text.and_then(|s| serde_json::from_str::<Value>(s).ok())
}

/// What an action does, or its name when it says nothing more.
fn describe_text(info: &ActionInfo) -> String {
    if info.description.is_empty() {
        info.display_name.clone()
    } else {
        info.description.clone()
    }
}

/// An action as the list shows it: enough to choose one.
fn summary(info: &ActionInfo) -> Value {
    json!({
        "id": info.id,
        "name": info.display_name,
        "description": info.description,
        "category": info.category,
        "effect": effect_name(info.effect),
        "takes_args": info.params_schema.is_some(),
        "shortcut": (!info.shortcut.is_empty()).then_some(&info.shortcut),
        "enabled": info.enabled,
        "disabled_reason": info.disabled_reason,
        "checked": info.checked,
    })
}

/// An action in full, its schemas as JSON.
fn detail(info: &ActionInfo) -> Value {
    json!({
        "id": info.id,
        "name": info.display_name,
        "description": info.description,
        "category": info.category,
        "group": info.group,
        "effect": effect_name(info.effect),
        "shortcut": (!info.shortcut.is_empty()).then_some(&info.shortcut),
        "takes": schema_of(info.params_schema.as_ref()),
        "gives": schema_of(info.result_schema.as_ref()),
        "enabled": info.enabled,
        "disabled_reason": info.disabled_reason,
        "checked": info.checked,
    })
}

fn event(e: &ActionEvent) -> Value {
    match &e.kind {
        EventKind::Ran { id, ok } => json!({"seq": e.seq, "ran": id, "ok": ok}),
        EventKind::Changed { topic, detail } => {
            json!({"seq": e.seq, "changed": topic, "which": detail})
        }
        EventKind::Registered { id } => json!({"seq": e.seq, "registered": id}),
        EventKind::Unregistered { id } => json!({"seq": e.seq, "unregistered": id}),
    }
}
