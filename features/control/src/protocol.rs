//! What a caller asks an app's Action List and what it is told: one shape
//! for every face.
//!
//! A model over MCP (`architect-mcp`) and a person or script on the
//! command line (this crate's socket and CLI) ask the same five things —
//! [`Method::TOOLS`]: `actions`, `describe`, `run`, `batch`, `watch` — with
//! the same parameters ([`Method::input_schema`]), parsed by one function
//! ([`Request::parse`]) and answered by one function
//! ([`crate::Surface::answer`]). An [`Answer`] is then only *rendered*: as
//! an MCP tool result's text, or as a JSON line on the socket. Neither
//! face can drift from the other, because neither has a table of its own.
//!
//! The socket adds two methods MCP has as a resource and a handshake:
//! `schema` (the whole set as JSON Schema; MCP's `actions://schema`) and
//! `hello` (the app, its version, the protocol).
//!
//! **Failures** carry a [`Code`]: a stable, machine-read name for what
//! went wrong (`bad_args`, `denied`, …), mapped one to one from the
//! Action List's typed errors ([`Failure::from_action`]) and to the CLI's
//! exit codes ([`Code::exit`]).

use std::time::Duration;

use architect::action::{ActionError, Effect};
use serde_json::{Value, json};

/// The protocol's name and revision, as `hello` gives it.
pub const PROTOCOL: &str = "architect-control/1";

/// The most steps one `batch` runs; a longer one is refused whole.
pub const MAX_BATCH_STEPS: usize = 64;

/// The longest a `watch` waits for something to happen.
pub const MAX_WAIT: Duration = Duration::from_secs(30);

/// The most events one `watch` returns.
pub const MAX_EVENTS: usize = 500;

/// How many events a `watch` returns when it is not told.
pub const DEFAULT_EVENTS: usize = 100;

/// What a caller can ask.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    /// The list, filtered by text, category or effect.
    Actions,
    /// One action in full: what it takes and gives, as JSON Schema.
    Describe,
    /// Run one action with its arguments.
    Run,
    /// Run several in order, stopping at the first failure.
    Batch,
    /// What happened since a point, waiting a while for the next.
    Watch,
    /// Every action the caller may run as one JSON Schema document.
    Schema,
    /// The app, its version, the protocol and where its events are.
    Hello,
}

impl Method {
    /// Every method, in the order `schema` lists them.
    pub const ALL: [Self; 7] = [
        Self::Actions,
        Self::Describe,
        Self::Run,
        Self::Batch,
        Self::Watch,
        Self::Schema,
        Self::Hello,
    ];

    /// The methods MCP offers as its generic tools, in its order. The rest
    /// it has as a resource (`schema`) and its own handshake (`hello`).
    pub const TOOLS: [Self; 5] = [
        Self::Actions,
        Self::Describe,
        Self::Run,
        Self::Batch,
        Self::Watch,
    ];

    /// Its name on the wire, and MCP's tool name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Actions => "actions",
            Self::Describe => "describe",
            Self::Run => "run",
            Self::Batch => "batch",
            Self::Watch => "watch",
            Self::Schema => "schema",
            Self::Hello => "hello",
        }
    }

    /// The method called `name`.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    /// The parameters it takes; any other is refused.
    #[must_use]
    pub const fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Actions => &["filter", "category", "effect"],
            Self::Describe => &["id"],
            Self::Run => &["id", "args"],
            Self::Batch => &["steps", "stop_on_error"],
            Self::Watch => &["since", "max", "wait_ms"],
            Self::Schema | Self::Hello => &[],
        }
    }

    /// Whether it only reads. `run` and `batch` run whatever they are
    /// given, so they are not.
    #[must_use]
    pub const fn read_only(self) -> bool {
        !matches!(self, Self::Run | Self::Batch)
    }

    /// What it does, as a model or `--help` is told.
    #[must_use]
    pub fn description(self) -> String {
        match self {
            Self::Actions => "List the app's actions — everything it can do — by id, with what each does, its category, its effect (read-only, mutating, destructive) and whether it takes arguments. Filter by text (matches id, name, description, category, group), category, or effect.".to_owned(),
            Self::Describe => "One action in full: what it does, the JSON Schema of the arguments it takes and of what it gives back.".to_owned(),
            Self::Run => "Run an action. `args` is what it takes (see `describe`); leave it out for an action that takes nothing. Gives back the action's result, or what went wrong and what it takes.".to_owned(),
            Self::Batch => format!("Run several actions in order — each `{{id, args}}` — stopping at the first that fails unless `stop_on_error` is false. At most {MAX_BATCH_STEPS} steps. Not atomic: steps that ran stay done when a later one fails, and nothing is rolled back. Gives back each step's result."),
            Self::Watch => "What has happened in the app since `since` (leave it out for 'from now'): actions run, and state that changed (a topic such as tracks or project, and which). Pass back the `next` it returns to keep following. With `wait_ms`, waits up to that long (30 s at most) for something when nothing has happened yet. `lost` says events were missed (the app restarted or the watcher fell behind): read the state again.".to_owned(),
            Self::Schema => "Every action the caller may run, as one JSON Schema document: each a `{id, args}` shape, its effect and what it gives back.".to_owned(),
            Self::Hello => "The app's name and version, the protocol, how many actions the caller may run, and the latest event's sequence number.".to_owned(),
        }
    }

    /// The JSON Schema of its parameters: MCP's `inputSchema`, and the
    /// socket's `params`.
    #[must_use]
    pub fn input_schema(self) -> Value {
        match self {
            Self::Actions => json!({"type": "object", "properties": {
                "filter": {"type": "string", "description": "Text to look for."},
                "category": {"type": "string"},
                "effect": {"type": "string", "enum": ["read-only", "mutating", "destructive"]},
            }}),
            Self::Describe => json!({"type": "object", "required": ["id"], "properties": {
                "id": {"type": "string", "description": "The action's id, from `actions`."},
            }}),
            Self::Run => json!({"type": "object", "required": ["id"], "properties": {
                "id": {"type": "string"},
                "args": {"type": "object", "description": "The action's arguments, as its schema says."},
            }}),
            Self::Batch => json!({"type": "object", "required": ["steps"], "properties": {
                "steps": {"type": "array", "maxItems": MAX_BATCH_STEPS, "items": {"type": "object", "required": ["id"], "properties": {
                    "id": {"type": "string"},
                    "args": {"type": "object"},
                }}},
                "stop_on_error": {"type": "boolean"},
            }}),
            Self::Watch => json!({"type": "object", "properties": {
                "since": {"type": "integer", "minimum": 0},
                "max": {"type": "integer", "minimum": 1},
                "wait_ms": {"type": "integer", "minimum": 0},
            }}),
            Self::Schema | Self::Hello => json!({"type": "object", "properties": {}}),
        }
    }
}

/// What `actions` is filtered by. Text matches the id, name,
/// description, category and group; category and effect match whole
/// (case aside).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    pub text: Option<String>,
    pub category: Option<String>,
    /// `read-only`, `mutating` or `destructive` ([`effect_name`]).
    pub effect: Option<String>,
}

/// One step of a `batch`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub id: String,
    pub args: Option<Value>,
}

/// Where a `watch` starts, how much it gives and how long it waits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Watch {
    /// After this event (`None`: from now).
    pub since: Option<u64>,
    /// At most this many ([`DEFAULT_EVENTS`] unless set; [`MAX_EVENTS`]
    /// at most).
    pub max: Option<u64>,
    /// Waiting up to this long for one when there is none yet
    /// ([`MAX_WAIT`] at most).
    pub wait_ms: Option<u64>,
}

/// One thing asked of an app's Action List.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Actions(Filter),
    Describe {
        id: String,
    },
    /// `args` `None` (or `null`) for an action that takes nothing.
    Run {
        id: String,
        args: Option<Value>,
    },
    Batch {
        steps: Vec<Step>,
        stop_on_error: bool,
    },
    Watch(Watch),
    Schema,
    Hello,
}

impl Request {
    /// `method` with `params`, as either face receives them.
    ///
    /// As lenient as MCP has always been about what is missing — a `run`
    /// with no id runs nothing and says so, a `batch` with no steps runs
    /// none — and strict about what is extra: a parameter the method does
    /// not take is a mistake to say, not to ignore, since the caller meant
    /// something by it.
    ///
    /// # Errors
    ///
    /// [`Code::NoMethod`] for a method there is none of;
    /// [`Code::BadRequest`] for a parameter it does not take.
    pub fn parse(method: &str, params: &Value) -> Result<Self, Failure> {
        let Some(m) = Method::named(method) else {
            return Err(Failure::new(Code::NoMethod, format!("no method {method}")));
        };
        if let Some(object) = params.as_object() {
            let unknown: Vec<&String> = object
                .keys()
                .filter(|k| !m.keys().contains(&k.as_str()))
                .collect();
            if !unknown.is_empty() {
                return Err(Failure::new(
                    Code::BadRequest,
                    format!("{method} takes {:?}, not {unknown:?}", m.keys()),
                ));
            }
        }
        let text = |key: &str| params.get(key).and_then(Value::as_str).map(str::to_owned);
        let number = |key: &str| params.get(key).and_then(Value::as_u64);
        Ok(match m {
            Method::Actions => Self::Actions(Filter {
                text: text("filter"),
                category: text("category"),
                effect: text("effect"),
            }),
            Method::Describe => Self::Describe {
                id: text("id").unwrap_or_default(),
            },
            Method::Run => Self::Run {
                id: text("id").unwrap_or_default(),
                args: params.get("args").cloned(),
            },
            Method::Batch => Self::Batch {
                steps: params
                    .get("steps")
                    .and_then(Value::as_array)
                    .map(|steps| {
                        steps
                            .iter()
                            .map(|s| Step {
                                id: s
                                    .get("id")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_owned(),
                                args: s.get("args").cloned(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                stop_on_error: params
                    .get("stop_on_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            },
            Method::Watch => Self::Watch(Watch {
                since: number("since"),
                max: number("max"),
                wait_ms: number("wait_ms"),
            }),
            Method::Schema => Self::Schema,
            Method::Hello => Self::Hello,
        })
    }

    /// Which method it is.
    #[must_use]
    pub const fn method(&self) -> Method {
        match self {
            Self::Actions(_) => Method::Actions,
            Self::Describe { .. } => Method::Describe,
            Self::Run { .. } => Method::Run,
            Self::Batch { .. } => Method::Batch,
            Self::Watch(_) => Method::Watch,
            Self::Schema => Method::Schema,
            Self::Hello => Method::Hello,
        }
    }

    /// Its parameters, as [`Self::parse`] reads them back: what a client
    /// sends.
    #[must_use]
    pub fn params(&self) -> Value {
        let mut out = serde_json::Map::new();
        let mut put = |key: &str, value: Option<Value>| {
            if let Some(value) = value {
                out.insert(key.to_owned(), value);
            }
        };
        match self {
            Self::Actions(f) => {
                put("filter", f.text.clone().map(Value::String));
                put("category", f.category.clone().map(Value::String));
                put("effect", f.effect.clone().map(Value::String));
            }
            Self::Describe { id } => put("id", Some(Value::String(id.clone()))),
            Self::Run { id, args } => {
                put("id", Some(Value::String(id.clone())));
                put("args", args.clone());
            }
            Self::Batch {
                steps,
                stop_on_error,
            } => {
                let steps = steps
                    .iter()
                    .map(|s| {
                        let mut step = json!({ "id": s.id });
                        if let (Some(args), Some(object)) = (&s.args, step.as_object_mut()) {
                            object.insert("args".to_owned(), args.clone());
                        }
                        step
                    })
                    .collect();
                put("steps", Some(Value::Array(steps)));
                put("stop_on_error", Some(Value::Bool(*stop_on_error)));
            }
            Self::Watch(w) => {
                put("since", w.since.map(Value::from));
                put("max", w.max.map(Value::from));
                put("wait_ms", w.wait_ms.map(Value::from));
            }
            Self::Schema | Self::Hello => {}
        }
        Value::Object(out)
    }
}

/// What went wrong, by name: stable, so a script can branch on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Code {
    /// No action has this id (for this caller).
    Unknown,
    /// The arguments are not what the action takes.
    BadArgs,
    /// The action ran and said no, or panicked.
    Failed,
    /// The caller may not run it.
    Denied,
    /// It cannot run right now, and says why.
    Disabled,
    /// Two actions share an id.
    Duplicate,
    /// It needs a person's yes first: the answer carries the ticket to
    /// retry with once they have given it (#237).
    ConfirmationRequired,
    /// The request is not one the method takes (a parameter it has no
    /// use for, a step too many).
    BadRequest,
    /// There is no such method.
    NoMethod,
    /// No app answered: nothing serves that socket.
    Unavailable,
    /// The answer could not be read: not JSON, not this protocol.
    Protocol,
}

impl Code {
    /// Every code, in [`Self::exit`] order.
    pub const ALL: [Self; 11] = [
        Self::Failed,
        Self::BadRequest,
        Self::NoMethod,
        Self::Unknown,
        Self::BadArgs,
        Self::Denied,
        Self::Disabled,
        Self::ConfirmationRequired,
        Self::Duplicate,
        Self::Unavailable,
        Self::Protocol,
    ];

    /// Its name on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::BadArgs => "bad_args",
            Self::Failed => "failed",
            Self::Denied => "denied",
            Self::Disabled => "disabled",
            Self::Duplicate => "duplicate",
            Self::ConfirmationRequired => "confirmation_required",
            Self::BadRequest => "bad_request",
            Self::NoMethod => "no_method",
            Self::Unavailable => "unavailable",
            Self::Protocol => "protocol",
        }
    }

    /// The code called `name`.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == name)
    }

    /// The CLI's exit code for it. `0` is success; `2` is also what a
    /// command line that does not parse exits with.
    #[must_use]
    pub const fn exit(self) -> u8 {
        match self {
            Self::Failed => 1,
            Self::BadRequest | Self::NoMethod => 2,
            Self::Unknown => 3,
            Self::BadArgs => 4,
            Self::Denied => 5,
            Self::Disabled => 6,
            Self::ConfirmationRequired => 7,
            Self::Duplicate => 8,
            // sysexits: EX_UNAVAILABLE, EX_PROTOCOL.
            Self::Unavailable => 69,
            Self::Protocol => 76,
        }
    }
}

impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What went wrong, as either face tells it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub code: Code,
    /// What went wrong, in words (`bad arguments: …`, `not allowed: …`).
    pub error: String,
    /// For [`Code::BadArgs`]: the JSON Schema of what the action takes.
    pub takes: Option<Value>,
    /// For [`Code::ConfirmationRequired`]: the ticket to retry with once a
    /// person has approved the call, as the app gave it.
    pub ticket: Option<Value>,
    /// `takes` as the action's schema text, for a plain-text answer.
    takes_text: Option<String>,
}

impl Failure {
    /// A failure with no schema and no ticket.
    #[must_use]
    pub fn new(code: Code, error: impl Into<String>) -> Self {
        Self {
            code,
            error: error.into(),
            takes: None,
            ticket: None,
            takes_text: None,
        }
    }

    /// The Action List's typed error as a failure: one variant, one code.
    #[must_use]
    pub fn from_action(error: &ActionError) -> Self {
        match error {
            ActionError::Unknown(id) => Self::new(
                Code::Unknown,
                format!("no action {id} — find it with `actions`"),
            ),
            ActionError::Duplicate(id) => Self::new(
                Code::Duplicate,
                format!("{id} is registered more than once"),
            ),
            ActionError::BadArgs {
                message, expected, ..
            } => Self {
                takes: expected
                    .as_deref()
                    .and_then(|s| serde_json::from_str::<Value>(s).ok()),
                takes_text: expected.clone(),
                ..Self::new(Code::BadArgs, format!("bad arguments: {message}"))
            },
            ActionError::Failed { message, .. } => Self::new(Code::Failed, message.clone()),
            ActionError::Denied { reason, .. } => {
                Self::new(Code::Denied, format!("not allowed: {reason}"))
            }
            ActionError::Disabled { reason, .. } => {
                Self::new(Code::Disabled, format!("disabled: {reason}"))
            }
        }
    }

    /// As a JSON object: `code` and `error`, with `takes` for bad
    /// arguments, `disabled: true` for a disabled action and `ticket` for
    /// one that needs a person's yes.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut out = json!({ "code": self.code.as_str(), "error": self.error });
        if let Some(object) = out.as_object_mut() {
            match self.code {
                Code::BadArgs => {
                    object.insert(
                        "takes".to_owned(),
                        self.takes.clone().unwrap_or(Value::Null),
                    );
                }
                Code::Disabled => {
                    object.insert("disabled".to_owned(), Value::Bool(true));
                }
                Code::ConfirmationRequired => {
                    object.insert(
                        "ticket".to_owned(),
                        self.ticket.clone().unwrap_or(Value::Null),
                    );
                }
                _ => {}
            }
        }
        out
    }

    /// As plain text: the error, and for bad arguments the schema they
    /// should match.
    #[must_use]
    pub fn to_text(&self) -> String {
        match (&self.takes_text, &self.takes) {
            (Some(schema), _) => format!("{}. It takes: {schema}", self.error),
            (None, Some(schema)) => format!("{}. It takes: {schema}", self.error),
            (None, None) => self.error.clone(),
        }
    }

    /// Read back from [`Self::to_json`]: what a client was told. Anything
    /// without a code it knows is [`Code::Protocol`].
    #[must_use]
    pub fn from_json(value: &Value) -> Self {
        let code = value
            .get("code")
            .and_then(Value::as_str)
            .and_then(Code::named)
            .unwrap_or(Code::Protocol);
        let error = value
            .get("error")
            .and_then(Value::as_str)
            .map_or_else(|| value.to_string(), str::to_owned);
        Self {
            takes: value.get("takes").filter(|v| !v.is_null()).cloned(),
            ticket: value.get("ticket").filter(|v| !v.is_null()).cloned(),
            ..Self::new(code, error)
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.error)
    }
}

impl std::error::Error for Failure {}

/// One step of a batch, as it went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepResult {
    pub step: usize,
    pub id: String,
    pub outcome: Result<Value, Failure>,
}

/// What a `batch` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Batch {
    /// How many steps it was given.
    pub of: usize,
    /// Each step that ran (or was refused), in order.
    pub results: Vec<StepResult>,
    /// Why none ran: it was over [`MAX_BATCH_STEPS`].
    pub refused: Option<Failure>,
}

impl Batch {
    /// The first failure: the refusal, or the first step that failed.
    #[must_use]
    pub fn failure(&self) -> Option<&Failure> {
        self.refused
            .as_ref()
            .or_else(|| self.results.iter().find_map(|r| r.outcome.as_ref().err()))
    }

    /// As JSON: `ran`, `of` and each step's `{step, id, ok, result |
    /// error}`, with `code` (the first failure's) when one failed. A step's
    /// error is its [`Failure::to_json`], or its [`Failure::to_text`] when
    /// `text_errors`.
    #[must_use]
    pub fn to_json(&self, text_errors: bool) -> Value {
        if let Some(refused) = &self.refused {
            return json!({
                "code": refused.code.as_str(),
                "error": refused.error,
                "ran": 0,
                "of": self.of,
            });
        }
        let results: Vec<Value> = self
            .results
            .iter()
            .map(|r| match &r.outcome {
                Ok(value) => json!({"step": r.step, "id": r.id, "ok": true, "result": value}),
                Err(failure) => {
                    let error = if text_errors {
                        Value::String(failure.to_text())
                    } else {
                        failure.to_json()
                    };
                    json!({"step": r.step, "id": r.id, "ok": false, "error": error})
                }
            })
            .collect();
        let mut out = json!({ "ran": results.len(), "of": self.of, "results": results });
        if let (Some(failure), Some(object)) = (self.failure(), out.as_object_mut()) {
            object.insert(
                "code".to_owned(),
                Value::String(failure.code.as_str().to_owned()),
            );
        }
        out
    }
}

/// What a request was answered with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The method's result.
    Done(Value),
    /// It did not do what was asked, and why.
    Failed(Failure),
    /// A batch, which may have failed part way.
    Batch(Batch),
}

impl Answer {
    /// Whether it is a failure (a batch with a failed step is).
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.failure().is_some()
    }

    /// The failure, if it is one: a batch's first.
    #[must_use]
    pub fn failure(&self) -> Option<&Failure> {
        match self {
            Self::Done(_) => None,
            Self::Failed(failure) => Some(failure),
            Self::Batch(batch) => batch.failure(),
        }
    }

    /// As JSON: a result as it is, a failure as [`Failure::to_json`], a
    /// batch as [`Batch::to_json`]. What the socket sends and what MCP's
    /// tool result says, byte for byte.
    #[must_use]
    pub fn to_json(&self) -> Value {
        match self {
            Self::Done(value) => value.clone(),
            Self::Failed(failure) => failure.to_json(),
            Self::Batch(batch) => batch.to_json(false),
        }
    }

    /// As the socket sends it: `{"id", "result"}`, or `{"id", "error"}`
    /// for a failure.
    #[must_use]
    pub fn to_line(&self, id: &Value) -> Value {
        Reply::from(self).to_line(id)
    }
}

/// An answer as a client reads it off the socket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// `result`: what was asked for.
    Ok(Value),
    /// `error`: a failure's JSON (`code`, `error`, …), or a failed batch's
    /// report.
    Err(Value),
}

impl From<&Answer> for Reply {
    fn from(answer: &Answer) -> Self {
        if answer.is_error() {
            Self::Err(answer.to_json())
        } else {
            Self::Ok(answer.to_json())
        }
    }
}

impl Reply {
    /// The reply line for request `id`.
    #[must_use]
    pub fn to_line(&self, id: &Value) -> Value {
        match self {
            Self::Ok(value) => json!({ "id": id, "result": value }),
            Self::Err(value) => json!({ "id": id, "error": value }),
        }
    }

    /// A reply line read back: its id and the reply.
    ///
    /// # Errors
    ///
    /// [`Code::Protocol`] when it has neither `result` nor `error`.
    pub fn from_line(line: &Value) -> Result<(Value, Self), Failure> {
        let id = line.get("id").cloned().unwrap_or(Value::Null);
        if let Some(error) = line.get("error") {
            return Ok((id, Self::Err(error.clone())));
        }
        line.get("result").map_or_else(
            || Err(Failure::new(Code::Protocol, format!("not a reply: {line}"))),
            |result| Ok((id, Self::Ok(result.clone()))),
        )
    }

    /// What it carries, success or not.
    #[must_use]
    pub const fn value(&self) -> &Value {
        match self {
            Self::Ok(value) | Self::Err(value) => value,
        }
    }

    /// The failure it is, if it is one.
    #[must_use]
    pub fn failure(&self) -> Option<Failure> {
        match self {
            Self::Ok(_) => None,
            Self::Err(value) => Some(Failure::from_json(value)),
        }
    }
}

/// An effect as both faces name it.
#[must_use]
pub const fn effect_name(effect: Effect) -> &'static str {
    match effect {
        Effect::ReadOnly => "read-only",
        Effect::Mutating => "mutating",
        Effect::Destructive => "destructive",
    }
}

#[cfg(test)]
mod tests {
    use super::{Answer, Code, Failure, Method, Reply, Request, Step};
    use serde_json::json;

    /// What a client sends is what the server reads back.
    #[test]
    fn params_round_trip() {
        let requests = [
            Request::parse("actions", &json!({"filter": "drum", "effect": "read-only"})),
            Request::parse("describe", &json!({"id": "A_B"})),
            Request::parse("run", &json!({"id": "A_B", "args": {"x": 1}})),
            Request::parse(
                "batch",
                &json!({"steps": [{"id": "A_B"}, {"id": "C_D", "args": 2}]}),
            ),
            Request::parse("watch", &json!({"since": 3, "wait_ms": 10})),
            Request::parse("schema", &json!({})),
            Request::parse("hello", &json!(null)),
        ];
        for request in requests {
            let request = request.expect("parses");
            let again = Request::parse(request.method().name(), &request.params());
            assert_eq!(again, Ok(request));
        }
        assert_eq!(
            Request::parse("batch", &json!({"steps": [{"id": "X_Y"}]})),
            Ok(Request::Batch {
                steps: vec![Step {
                    id: "X_Y".into(),
                    args: None
                }],
                stop_on_error: true
            })
        );
    }

    /// A stray parameter and an unknown method are refused, each with its
    /// code.
    #[test]
    fn extras_and_unknown_methods_are_refused() {
        let stray = Request::parse("run", &json!({"id": "A_B", "arg": {}})).expect_err("stray");
        assert_eq!(stray.code, Code::BadRequest);
        assert!(stray.error.contains("arg"), "{stray}");
        let none = Request::parse("nope", &json!({})).expect_err("no method");
        assert_eq!(none.code, Code::NoMethod);
        assert!(Method::TOOLS.iter().all(|m| Method::ALL.contains(m)));
    }

    /// Every code has its own name and its own exit code, none of them 0.
    #[test]
    fn codes_are_distinct() {
        for (i, a) in Code::ALL.iter().enumerate() {
            assert_eq!(Code::named(a.as_str()), Some(*a));
            assert_ne!(a.exit(), 0);
            for b in Code::ALL.iter().skip(i + 1) {
                assert_ne!(a.as_str(), b.as_str());
                let shared_usage = [Code::BadRequest, Code::NoMethod];
                if !(shared_usage.contains(a) && shared_usage.contains(b)) {
                    assert_ne!(a.exit(), b.exit(), "{a} and {b}");
                }
            }
        }
    }

    /// A failure read off the wire is the failure sent, ticket included.
    #[test]
    fn failures_round_trip() {
        let sent = Failure {
            ticket: Some(json!({"id": "t1", "expires_in_s": 60})),
            ..Failure::new(Code::ConfirmationRequired, "a person must approve this")
        };
        let line = Answer::Failed(sent.clone()).to_line(&json!(7));
        let (id, reply) = Reply::from_line(&line).expect("a reply");
        assert_eq!(id, json!(7));
        assert_eq!(reply.failure(), Some(sent));
    }
}
