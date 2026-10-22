//! An app's Action List as a Model Context Protocol server.
//!
//! The Action List (`architect::action::ActionList`) is every command an
//! app has, run by id with JSON in and out — what its keys, menus and CLI
//! already run. This puts a model on the same list: it can see what the
//! app can do, read what each action takes, run actions, run several as
//! one step, and watch what happens in the app while it works.
//!
//! Generic tools, not one per action — an app has hundreds, and a model
//! is better served by five tools it can always see and a list it can
//! search:
//!
//! | tool       | what it does                                               |
//! |------------|------------------------------------------------------------|
//! | `actions`  | the list, filtered by text, category or effect             |
//! | `describe` | one action: what it does, takes (JSON Schema) and gives    |
//! | `run`      | run one action with its arguments                          |
//! | `batch`    | run several in order, stopping at the first failure        |
//! |            | (at most [`MAX_BATCH_STEPS`]; not atomic)                  |
//! | `watch`    | what happened since a point — runs and state changes —     |
//! |            | waiting a while for the next when nothing has yet          |
//!
//! The protocol is one function, [`Server::handle`]: a JSON-RPC message
//! in, the reply (if it wants one) out. [`serve_stdio`] runs it over
//! stdin/stdout; behind the `http` feature, [`http::router`] serves it as
//! MCP's Streamable HTTP transport, for a running app on localhost.
//!
//! What a model is told is held to a size ([`MAX_TEXT`]): an action that
//! answers with a song's every hit should page, and when one does not,
//! the answer is cut and says so rather than flooding the context.
//!
//! Every call is made as a [`Principal`] — the server's own
//! ([`Server::with_principal`], `Anonymous` unless set) or one a transport
//! resolved per request ([`Server::handle_as`]). When the list has an
//! authorizer (`ActionList::set_authorizer`), that principal sees only the
//! actions it may run (`actions`, `describe`, the `actions://list`
//! resource) and is refused the rest, on `run` and on every `batch` step.
//! A disabled action is listed with `enabled: false` and why.

use std::time::Duration;

use architect::action::{
    ActionError, ActionEvent, ActionInfo, ActionList, Effect, EventKind, Principal,
};
use serde_json::{Value, json};

#[cfg(feature = "http")]
pub mod http;

/// The protocol revision this server speaks when a client asks for one
/// it does not know.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// The revisions it answers in, when a client asks for one of them.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// The most text one tool result carries before it is cut.
pub const MAX_TEXT: usize = 100_000;

/// The longest a `watch` waits for something to happen.
pub const MAX_WAIT: Duration = Duration::from_secs(30);

/// The most events one `watch` returns.
pub const MAX_EVENTS: usize = 500;

/// The most steps one `batch` runs; a longer one is refused whole.
pub const MAX_BATCH_STEPS: usize = 64;

/// A JSON-RPC error code: the method is not one this server has.
const METHOD_NOT_FOUND: i64 = -32_601;
/// A JSON-RPC error code: the parameters are wrong.
const INVALID_PARAMS: i64 = -32_602;
/// A JSON-RPC error code: the message is not JSON-RPC.
const INVALID_REQUEST: i64 = -32_600;
/// A JSON-RPC error code: the message is not JSON.
const PARSE_ERROR: i64 = -32_700;

/// The server: an app's name, what it tells a model on connecting, and
/// its Action List.
#[derive(Clone, Debug)]
pub struct Server {
    name: String,
    version: String,
    instructions: String,
    list: ActionList,
    principal: Principal,
}

impl Server {
    /// A server for `list`, introducing itself as `name` `version`.
    #[must_use]
    pub fn new(name: impl Into<String>, version: impl Into<String>, list: ActionList) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            instructions: String::new(),
            list,
            principal: Principal::Anonymous,
        }
    }

    /// Who the model is, as the list's authorizer sees it, for every
    /// message [`Self::handle`] answers. `Anonymous` unless set.
    #[must_use]
    pub fn with_principal(mut self, principal: Principal) -> Self {
        self.principal = principal;
        self
    }

    /// What a model is told when it connects, before the generic part:
    /// what the app is, how its ids are named, where to start.
    #[must_use]
    pub fn with_instructions(mut self, text: impl Into<String>) -> Self {
        self.instructions = text.into();
        self
    }

    /// The list it serves.
    #[must_use]
    pub const fn list(&self) -> &ActionList {
        &self.list
    }

    /// One JSON-RPC message in, its reply out — `None` for a
    /// notification, which wants none — as the server's principal.
    #[must_use]
    pub fn handle(&self, message: &str) -> Option<String> {
        self.handle_as(&self.principal, message)
    }

    /// One JSON-RPC message in, its reply out, as `who` — for a transport
    /// that knows who is on the other end of each request.
    #[must_use]
    pub fn handle_as(&self, who: &Principal, message: &str) -> Option<String> {
        let request: Value = match serde_json::from_str(message) {
            Ok(v) => v,
            Err(e) => {
                return Some(error_reply(&Value::Null, PARSE_ERROR, &e.to_string()).to_string());
            }
        };
        let id = request.get("id").cloned();
        let Some(method) = request.get("method").and_then(Value::as_str) else {
            return Some(
                error_reply(&id.unwrap_or(Value::Null), INVALID_REQUEST, "no method").to_string(),
            );
        };
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        // A notification (no id) is told nothing back.
        let id = id?;
        let reply = match self.dispatch(who, method, &params) {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => error_reply(&id, code, &message),
        };
        Some(reply.to_string())
    }

    fn dispatch(
        &self,
        who: &Principal,
        method: &str,
        params: &Value,
    ) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => Ok(self.initialize(who, params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => self.call(who, params),
            "resources/list" => Ok(json!({ "resources": [{
                "uri": "actions://list",
                "name": "Action List",
                "description": "Every action the app has: id, what it does, its effect, whether it takes arguments.",
                "mimeType": "application/json",
            }]})),
            "resources/read" => {
                let uri = params
                    .get("uri")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if uri != "actions://list" {
                    return Err((INVALID_PARAMS, format!("no resource {uri}")));
                }
                let all: Vec<Value> = self.list.infos_for(who).iter().map(summary).collect();
                Ok(json!({ "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": Value::Array(all).to_string(),
                }]}))
            }
            _ => Err((METHOD_NOT_FOUND, format!("no method {method}"))),
        }
    }

    fn initialize(&self, who: &Principal, params: &Value) -> Value {
        let asked = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or(PROTOCOL_VERSION);
        let version = if PROTOCOL_VERSIONS.contains(&asked) {
            asked
        } else {
            PROTOCOL_VERSION
        };
        let mut instructions = self.instructions.clone();
        if !instructions.is_empty() {
            instructions.push_str("\n\n");
        }
        instructions.push_str(&format!(
            "Everything {} can do is an action in its Action List ({} of them) — the same \
             actions its keys and menus run. Find one with `actions` (filter by text), read what \
             it takes with `describe`, run it with `run`, or several in order with `batch` \
             (at most {} steps, not atomic: steps that ran stay done when a later one fails). \
             An action listed `enabled: false` says why it cannot run now. Read-only actions are safe to run freely; mutating ones change the open project; \
             destructive ones may not be undoable — say what you are about to do first. \
             `watch` tells you what has happened since a point (pass back the `next` it \
             returns), and waits for the next change when you ask it to.",
            self.name,
            self.list.infos_for(who).len(),
            MAX_BATCH_STEPS,
        ));
        json!({
            "protocolVersion": version,
            "capabilities": {
                "tools": { "listChanged": false },
                "resources": { "listChanged": false, "subscribe": false },
            },
            "serverInfo": { "name": self.name, "version": self.version },
            "instructions": instructions,
        })
    }

    fn call(&self, who: &Principal, params: &Value) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((INVALID_PARAMS, "no tool name".to_owned()))?;
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let Some(spec) = tools_spec().iter().find(|t| t.name == name) else {
            return Err((INVALID_PARAMS, format!("no tool {name}")));
        };
        // An argument the tool does not take is a mistake to say, not to
        // ignore: the caller meant something by it.
        if let Some(object) = args.as_object() {
            let unknown: Vec<&String> = object
                .keys()
                .filter(|k| !spec.keys.contains(&k.as_str()))
                .collect();
            if !unknown.is_empty() {
                return Err((
                    INVALID_PARAMS,
                    format!("{name} takes {:?}, not {unknown:?}", spec.keys),
                ));
            }
        }
        Ok(match name {
            "actions" => text_result(&self.actions(who, &args), false),
            "describe" => match args
                .get("id")
                .and_then(Value::as_str)
                .and_then(|id| self.list.info_for(who, id))
            {
                Some(info) => text_result(&detail(&info), false),
                None => text_result(
                    &json!({"error": "no such action — find it with `actions`"}),
                    true,
                ),
            },
            "run" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or_default();
                match self.run_one(who, id, args.get("args")) {
                    Ok(value) => text_result(&value, false),
                    Err(error) => text_result(&error, true),
                }
            }
            "batch" => {
                let (value, failed) = self.batch(who, &args);
                text_result(&value, failed)
            }
            "watch" => text_result(&self.watch(&args), false),
            _ => return Err((INVALID_PARAMS, format!("no tool {name}"))),
        })
    }

    fn actions(&self, who: &Principal, args: &Value) -> Value {
        let text = args
            .get("filter")
            .and_then(Value::as_str)
            .map(str::to_lowercase);
        let category = args
            .get("category")
            .and_then(Value::as_str)
            .map(str::to_lowercase);
        let effect = args
            .get("effect")
            .and_then(Value::as_str)
            .map(str::to_lowercase);
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

    /// Run one action, its JSON result or what went wrong.
    fn run_one(&self, who: &Principal, id: &str, args: Option<&Value>) -> Result<Value, Value> {
        let bytes = match args {
            None | Some(Value::Null) => Vec::new(),
            Some(v) => v.to_string().into_bytes(),
        };
        match self.list.run_json_as(who, id, &bytes) {
            Ok(out) => Ok(serde_json::from_slice(&out).unwrap_or(Value::Null)),
            Err(ActionError::Unknown(id) | ActionError::Duplicate(id)) => Err(json!({
                "error": format!("no action {id} — find it with `actions`"),
            })),
            Err(ActionError::BadArgs {
                message, expected, ..
            }) => Err(json!({
                "error": format!("bad arguments: {message}"),
                "takes": expected.and_then(|s| serde_json::from_str::<Value>(&s).ok()),
            })),
            Err(ActionError::Failed { message, .. }) => Err(json!({ "error": message })),
            Err(ActionError::Denied { reason, .. }) => Err(json!({
                "error": format!("not allowed: {reason}"),
            })),
            Err(ActionError::Disabled { reason, .. }) => Err(json!({
                "error": format!("disabled: {reason}"),
                "disabled": true,
            })),
        }
    }

    /// Each step runs as `who`, through the same door as `run` — a step
    /// is authorized on its own. Not atomic: what ran stays done.
    fn batch(&self, who: &Principal, args: &Value) -> (Value, bool) {
        let stop = args
            .get("stop_on_error")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let steps = args
            .get("steps")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if steps.len() > MAX_BATCH_STEPS {
            return (
                json!({
                    "error": format!(
                        "{} steps; a batch runs at most {MAX_BATCH_STEPS} — split it",
                        steps.len()
                    ),
                    "ran": 0,
                    "of": steps.len(),
                }),
                true,
            );
        }
        let mut results = Vec::with_capacity(steps.len());
        let mut failed = false;
        for (i, step) in steps.iter().enumerate() {
            let id = step.get("id").and_then(Value::as_str).unwrap_or_default();
            match self.run_one(who, id, step.get("args")) {
                Ok(value) => {
                    results.push(json!({"step": i, "id": id, "ok": true, "result": value}))
                }
                Err(error) => {
                    failed = true;
                    results.push(json!({"step": i, "id": id, "ok": false, "error": error}));
                    if stop {
                        break;
                    }
                }
            }
        }
        (
            json!({ "ran": results.len(), "of": steps.len(), "results": results }),
            failed,
        )
    }

    fn watch(&self, args: &Value) -> Value {
        let since = args
            .get("since")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| self.list.latest());
        let max = args
            .get("max")
            .and_then(Value::as_u64)
            .and_then(|m| usize::try_from(m).ok())
            .unwrap_or(100)
            .min(MAX_EVENTS);
        let wait = args
            .get("wait_ms")
            .and_then(Value::as_u64)
            .map_or(Duration::ZERO, Duration::from_millis)
            .min(MAX_WAIT);
        let (events, next) = self.list.events_since(since, max, wait);
        json!({
            "next": next,
            "events": events.iter().map(event).collect::<Vec<_>>(),
        })
    }
}

/// Serve `server` over stdin/stdout, one JSON-RPC message a line, until
/// stdin closes.
///
/// # Errors
///
/// Reading stdin or writing stdout failed.
pub fn serve_stdio(server: &Server) -> std::io::Result<()> {
    use std::io::{BufRead as _, Write as _};
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = server.handle(&line) {
            stdout.write_all(reply.as_bytes())?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

/// A tool: its name, the arguments it takes, and how it is described.
struct ToolSpec {
    name: &'static str,
    keys: &'static [&'static str],
    read_only: bool,
}

const fn tools_spec() -> &'static [ToolSpec] {
    &[
        ToolSpec {
            name: "actions",
            keys: &["filter", "category", "effect"],
            read_only: true,
        },
        ToolSpec {
            name: "describe",
            keys: &["id"],
            read_only: true,
        },
        ToolSpec {
            name: "run",
            keys: &["id", "args"],
            read_only: false,
        },
        ToolSpec {
            name: "batch",
            keys: &["steps", "stop_on_error"],
            read_only: false,
        },
        ToolSpec {
            name: "watch",
            keys: &["since", "max", "wait_ms"],
            read_only: true,
        },
    ]
}

/// The tools, as `tools/list` gives them.
fn tools() -> Value {
    let annotate = |name: &str| {
        let read_only = tools_spec().iter().any(|t| t.name == name && t.read_only);
        json!({ "readOnlyHint": read_only, "destructiveHint": !read_only, "openWorldHint": false })
    };
    json!([
        {
            "name": "actions",
            "description": "List the app's actions — everything it can do — by id, with what each does, its category, its effect (read-only, mutating, destructive) and whether it takes arguments. Filter by text (matches id, name, description, category, group), category, or effect.",
            "inputSchema": {"type": "object", "properties": {
                "filter": {"type": "string", "description": "Text to look for."},
                "category": {"type": "string"},
                "effect": {"type": "string", "enum": ["read-only", "mutating", "destructive"]},
            }},
            "annotations": annotate("actions"),
        },
        {
            "name": "describe",
            "description": "One action in full: what it does, the JSON Schema of the arguments it takes and of what it gives back.",
            "inputSchema": {"type": "object", "required": ["id"], "properties": {
                "id": {"type": "string", "description": "The action's id, from `actions`."},
            }},
            "annotations": annotate("describe"),
        },
        {
            "name": "run",
            "description": "Run an action. `args` is what it takes (see `describe`); leave it out for an action that takes nothing. Gives back the action's result, or what went wrong and what it takes.",
            "inputSchema": {"type": "object", "required": ["id"], "properties": {
                "id": {"type": "string"},
                "args": {"type": "object", "description": "The action's arguments, as its schema says."},
            }},
            "annotations": annotate("run"),
        },
        {
            "name": "batch",
            "description": format!("Run several actions in order — each `{{id, args}}` — stopping at the first that fails unless `stop_on_error` is false. At most {MAX_BATCH_STEPS} steps. Not atomic: steps that ran stay done when a later one fails, and nothing is rolled back. Gives back each step's result."),
            "inputSchema": {"type": "object", "required": ["steps"], "properties": {
                "steps": {"type": "array", "maxItems": MAX_BATCH_STEPS, "items": {"type": "object", "required": ["id"], "properties": {
                    "id": {"type": "string"},
                    "args": {"type": "object"},
                }}},
                "stop_on_error": {"type": "boolean"},
            }},
            "annotations": annotate("batch"),
        },
        {
            "name": "watch",
            "description": "What has happened in the app since `since` (leave it out for 'from now'): actions run, and state that changed (a topic such as tracks or project, and which). Pass back the `next` it returns to keep following. With `wait_ms`, waits up to that long (30 s at most) for something when nothing has happened yet.",
            "inputSchema": {"type": "object", "properties": {
                "since": {"type": "integer", "minimum": 0},
                "max": {"type": "integer", "minimum": 1},
                "wait_ms": {"type": "integer", "minimum": 0},
            }},
            "annotations": annotate("watch"),
        },
    ])
}

/// A tool's result: `value` as JSON text, cut to [`MAX_TEXT`].
fn text_result(value: &Value, is_error: bool) -> Value {
    let mut text = value.to_string();
    if text.len() > MAX_TEXT {
        let mut cut = MAX_TEXT;
        while !text.is_char_boundary(cut) {
            cut = cut.saturating_sub(1);
        }
        text.truncate(cut);
        text.push_str(&format!(
            "… [cut at {MAX_TEXT} bytes — ask for less: a filter, a range, a limit]"
        ));
    }
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

fn error_reply(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

const fn effect_name(effect: Effect) -> &'static str {
    match effect {
        Effect::ReadOnly => "read-only",
        Effect::Mutating => "mutating",
        Effect::Destructive => "destructive",
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
    })
}

/// An action in full, its schemas as JSON.
fn detail(info: &ActionInfo) -> Value {
    let schema = |s: &Option<String>| {
        s.as_deref()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
    };
    json!({
        "id": info.id,
        "name": info.display_name,
        "description": info.description,
        "category": info.category,
        "group": info.group,
        "effect": effect_name(info.effect),
        "shortcut": (!info.shortcut.is_empty()).then_some(&info.shortcut),
        "takes": schema(&info.params_schema),
        "gives": schema(&info.result_schema),
        "enabled": info.enabled,
        "disabled_reason": info.disabled_reason,
    })
}

fn event(e: &ActionEvent) -> Value {
    match &e.kind {
        EventKind::Ran { id, ok } => json!({"seq": e.seq, "ran": id, "ok": ok}),
        EventKind::Changed { topic, detail } => {
            json!({"seq": e.seq, "changed": topic, "which": detail})
        }
        EventKind::Registered { id } => json!({"seq": e.seq, "registered": id}),
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_BATCH_STEPS, Server};
    use architect::action::{ActionList, ActionMeta, Effect, Principal};
    use architect_permissions::{Action, Decision, PermissionEngine, Resource};
    use facet::Facet;
    use serde_json::{Value, json};

    static HITS: ActionMeta = ActionMeta {
        id: "DRUMS_HITS",
        trait_name: "Drums",
        method_name: "hits",
        display_name: "Drum hits",
        description: "Every hit of a drum",
        category: "Drums",
        group: "Analysis",
        shortcut: "",
        toggleable: false,
        undo: false,
    };

    #[derive(Facet)]
    struct Query {
        track: String,
    }

    fn server() -> Server {
        let list = ActionList::new();
        list.register(&HITS, Effect::ReadOnly, |q: Query| {
            if q.track == "none" {
                return Err("no such track".to_owned());
            }
            Ok(vec![0.5_f64, 1.0])
        })
        .expect("registered");
        Server::new("Test", "0", list)
    }

    fn call(server: &Server, name: &str, arguments: &Value) -> Value {
        let request = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": name, "arguments": arguments}});
        let reply: Value =
            serde_json::from_str(&server.handle(&request.to_string()).expect("a reply"))
                .expect("json");
        reply
    }

    fn text(reply: &Value) -> Value {
        let text = reply["result"]["content"][0]["text"]
            .as_str()
            .expect("text");
        serde_json::from_str(text).expect("json text")
    }

    /// The handshake answers in the asked revision and tells the model
    /// what the app is; a notification gets nothing back.
    #[test]
    fn it_introduces_itself() {
        let s = server().with_instructions("A test app.");
        let reply: Value = serde_json::from_str(
            &s.handle(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#)
                .expect("a reply"),
        )
        .expect("json");
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
        assert!(
            reply["result"]["instructions"]
                .as_str()
                .is_some_and(|t| t.starts_with("A test app."))
        );
        assert!(
            s.handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .is_none()
        );
    }

    /// Found by text, described with its schema, run with its arguments;
    /// a refusal says why and what it takes; a failure is an error result;
    /// a stray argument to a tool is refused naming what it takes; the run
    /// is something a watcher sees.
    #[test]
    fn a_model_finds_runs_and_watches_actions() {
        let s = server();
        let found = text(&call(&s, "actions", &json!({"filter": "hit"})));
        assert_eq!(found["count"], 1);
        assert_eq!(found["actions"][0]["effect"], "read-only");

        let described = text(&call(&s, "describe", &json!({"id": "DRUMS_HITS"})));
        assert_eq!(described["takes"]["required"], json!(["track"]));

        let start = s.list().latest();
        let ran = call(
            &s,
            "run",
            &json!({"id": "DRUMS_HITS", "args": {"track": "{A}"}}),
        );
        assert_eq!(ran["result"]["isError"], false);
        assert_eq!(text(&ran), json!([0.5, 1]));

        let bad = call(
            &s,
            "run",
            &json!({"id": "DRUMS_HITS", "args": {"trak": "{A}"}}),
        );
        assert_eq!(bad["result"]["isError"], true);
        assert_eq!(text(&bad)["takes"]["required"], json!(["track"]));

        let failed = call(
            &s,
            "run",
            &json!({"id": "DRUMS_HITS", "args": {"track": "none"}}),
        );
        assert_eq!(text(&failed)["error"], "no such track");

        let stray = call(&s, "run", &json!({"id": "DRUMS_HITS", "arg": {}}));
        assert_eq!(stray["error"]["code"], -32_602);

        let seen = text(&call(&s, "watch", &json!({"since": start})));
        let ran_events: Vec<&Value> = seen["events"]
            .as_array()
            .expect("events")
            .iter()
            .filter(|e| e["ran"] == "DRUMS_HITS")
            .collect();
        assert_eq!(ran_events.len(), 3, "{seen}");
    }

    /// A batch runs in order and stops at the first failure.
    #[test]
    fn a_batch_stops_at_the_first_failure() {
        let s = server();
        let reply = call(
            &s,
            "batch",
            &json!({"steps": [
                {"id": "DRUMS_HITS", "args": {"track": "a"}},
                {"id": "DRUMS_HITS", "args": {"track": "none"}},
                {"id": "DRUMS_HITS", "args": {"track": "b"}},
            ]}),
        );
        let out = text(&reply);
        assert_eq!(out["ran"], 2);
        assert_eq!(reply["result"]["isError"], true);
    }

    static DELETE: ActionMeta = ActionMeta {
        id: "PROJECT_DELETE",
        trait_name: "Project",
        method_name: "delete",
        display_name: "Delete project",
        description: "Delete the open project",
        category: "Project",
        group: "File",
        shortcut: "",
        toggleable: false,
        undo: false,
    };

    /// A user may run what `action/DRUMS_HITS` + `run` allows; a service
    /// anything.
    struct UsersRunDrums;
    impl PermissionEngine for UsersRunDrums {
        fn check(&self, who: &Principal, what: &Resource, action: &Action) -> Decision {
            let drums = what.as_str() == "action/DRUMS_HITS" && action.as_str() == Action::RUN;
            if drums || matches!(who, Principal::Service { .. }) {
                Decision::Allow
            } else {
                Decision::deny("users run drums only")
            }
        }
    }

    /// A server whose list lets a user run `DRUMS_HITS` and nothing else,
    /// speaking for a user.
    fn guarded() -> Server {
        let s = server();
        s.list()
            .register(&DELETE, Effect::Destructive, |(): ()| Ok(()))
            .expect("registered");
        s.list().set_authorizer(UsersRunDrums);
        s.with_principal(Principal::User {
            user_id: "model".into(),
        })
    }

    /// (a) A disabled action is listed and described as such, with why,
    /// and running it says why not.
    #[test]
    fn a_disabled_action_is_listed_with_why() {
        let s = server();
        s.list()
            .set_enabled("DRUMS_HITS", || Err("no project is open".to_owned()))
            .expect("set");
        let found = text(&call(&s, "actions", &json!({})));
        assert_eq!(found["actions"][0]["enabled"], false);
        assert_eq!(found["actions"][0]["disabled_reason"], "no project is open");
        let described = text(&call(&s, "describe", &json!({"id": "DRUMS_HITS"})));
        assert_eq!(described["disabled_reason"], "no project is open");
        let ran = call(&s, "run", &json!({"id": "DRUMS_HITS", "args": {"track": "a"}}));
        assert_eq!(ran["result"]["isError"], true);
        assert_eq!(text(&ran)["error"], "disabled: no project is open");
    }

    /// (b) + (g) The model sees only what it may run, is refused the rest
    /// on `run` and on a `batch` step (which stops there), and a
    /// transport's per-request principal sees what that one may.
    #[test]
    fn the_model_sees_and_runs_only_what_it_may() {
        let s = guarded();
        let found = text(&call(&s, "actions", &json!({})));
        assert_eq!(found["count"], 1);
        assert_eq!(found["actions"][0]["id"], "DRUMS_HITS");
        let hidden = call(&s, "describe", &json!({"id": "PROJECT_DELETE"}));
        assert_eq!(hidden["result"]["isError"], true);

        let refused = call(&s, "run", &json!({"id": "PROJECT_DELETE"}));
        assert_eq!(refused["result"]["isError"], true);
        assert!(
            text(&refused)["error"]
                .as_str()
                .is_some_and(|e| e.starts_with("not allowed"))
        );

        let start = s.list().latest();
        let batch = call(
            &s,
            "batch",
            &json!({"steps": [
                {"id": "DRUMS_HITS", "args": {"track": "a"}},
                {"id": "PROJECT_DELETE"},
                {"id": "DRUMS_HITS", "args": {"track": "b"}},
            ]}),
        );
        let out = text(&batch);
        assert_eq!(out["ran"], 2);
        assert_eq!(out["results"][1]["ok"], false);
        let (events, _) = s.list().events_since(start, 10, std::time::Duration::ZERO);
        assert_eq!(events.len(), 1, "only the allowed step ran: {events:?}");

        let listing = json!({"jsonrpc": "2.0", "id": 1, "method": "resources/read",
            "params": {"uri": "actions://list"}});
        let anyone = Principal::Service { name: "app".into() };
        let as_model: Value =
            serde_json::from_str(&s.handle(&listing.to_string()).expect("reply")).expect("json");
        let as_app: Value = serde_json::from_str(
            &s.handle_as(&anyone, &listing.to_string()).expect("reply"),
        )
        .expect("json");
        let count = |v: &Value| -> usize {
            serde_json::from_str::<Value>(v["result"]["contents"][0]["text"].as_str().expect("text"))
                .expect("json")
                .as_array()
                .map_or(0, Vec::len)
        };
        assert_eq!(count(&as_model), 1);
        assert_eq!(count(&as_app), 2);
    }

    /// (f) A batch longer than the cap is refused whole: nothing runs.
    #[test]
    fn a_batch_over_the_cap_runs_nothing() {
        let s = server();
        let start = s.list().latest();
        let steps: Vec<Value> = (0..=MAX_BATCH_STEPS)
            .map(|_| json!({"id": "DRUMS_HITS", "args": {"track": "a"}}))
            .collect();
        let reply = call(&s, "batch", &json!({ "steps": steps }));
        assert_eq!(reply["result"]["isError"], true);
        assert_eq!(text(&reply)["ran"], 0);
        assert_eq!(s.list().latest(), start, "nothing ran");
        let tools: Value = serde_json::from_str(
            &s.handle(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
                .expect("reply"),
        )
        .expect("json");
        let batch = tools["result"]["tools"]
            .as_array()
            .and_then(|t| t.iter().find(|t| t["name"] == "batch"))
            .expect("batch tool");
        assert!(
            batch["description"]
                .as_str()
                .is_some_and(|d| d.contains("Not atomic"))
        );
    }
}
