//! Parity (feature `parity`): an app's Action List says the same over MCP
//! and over its control socket — the same set, the same answers.
//!
//! Both faces render answers from one `Surface`, so they agree by
//! construction; this checks it end to end, the socket included, for an
//! app's real list. An app's test is one call:
//!
//! ```ignore
//! #[test]
//! fn mcp_and_the_cli_agree() {
//!     let server = my_app::mcp_server(test_doubles());
//!     architect_mcp::parity::Parity::new(&server).check().unwrap();
//! }
//! ```
//!
//! What is compared: the generic tools against the protocol's methods;
//! the listing (`actions`, the `actions://list` and `actions://schema`
//! resources against the socket's `actions` and `schema`); every action's
//! `describe`; a run of every read-only action with no arguments (a run
//! of anything else only when asked, [`Parity::run`]: a check must not
//! delete a project); an unknown action, a stray parameter; and `watch`
//! over all of it. A call that needs a person's approval answers each
//! face with a ticket of its own; tickets are not compared. Each over MCP (`Server::call_tool`) and over a socket
//! served for the occasion, as the server's own principal.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use architect::action::{ActionCaller, Effect};
use architect_control::client::Client;
use architect_control::protocol::{Method, Reply};
use architect_control::server::Control;
use serde_json::{Value, json};

use crate::{ErrorStyle, Server, ToolError, Tools};

/// What a [`Parity::check`] compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    /// Actions the server's principal may run: each described on both.
    pub actions: usize,
    /// Requests asked of both faces.
    pub requests: usize,
}

/// Where the two faces disagreed, one line each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mismatch {
    pub problems: Vec<String>,
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MCP and the CLI disagree {} time(s):",
            self.problems.len()
        )?;
        for problem in &self.problems {
            write!(f, "\n- {problem}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Mismatch {}

/// A parity check of one server.
#[derive(Debug)]
pub struct Parity<'a> {
    server: &'a Server,
    runs: Vec<(String, Option<Value>)>,
}

/// One answer, as either face gave it: its JSON and whether it failed.
type Said = (Value, bool);

impl<'a> Parity<'a> {
    /// A check of `server`'s list.
    #[must_use]
    pub const fn new(server: &'a Server) -> Self {
        Self {
            server,
            runs: Vec::new(),
        }
    }

    /// Also run `id` with `args` on both faces and compare: for actions
    /// that change something, on a list built over test doubles.
    #[must_use]
    pub fn run(mut self, id: impl Into<String>, args: Option<Value>) -> Self {
        self.runs.push((id.into(), args));
        self
    }

    /// Serve the server's surface on a socket of its own and ask both
    /// faces everything.
    ///
    /// # Errors
    ///
    /// Every request the two answered differently, or the socket could
    /// not be served.
    pub fn check(self) -> Result<Report, Mismatch> {
        let fail = |problem: String| Mismatch {
            problems: vec![problem],
        };
        // The generic tools, in JSON style: what the socket speaks.
        let mcp = self
            .server
            .clone()
            .with_tools(Tools::Generic)
            .with_errors(ErrorStyle::Json);
        let path = scratch_socket();
        let serving = Control::at(&path, self.server.surface().clone())
            .serve()
            .map_err(|e| fail(format!("serving {}: {e}", path.display())))?;
        let client = Client::connect(serving.path()).map_err(|e| fail(e.error))?;
        let mut both = Both {
            mcp,
            client,
            problems: Vec::new(),
            requests: 0,
        };
        let who = self.server.surface().principal();

        let tools: Vec<String> = both
            .mcp
            .tools_for(who)
            .iter()
            .filter_map(|t| t["name"].as_str().map(str::to_owned))
            .collect();
        let methods: Vec<String> = Method::TOOLS.iter().map(|m| m.name().to_owned()).collect();
        if tools != methods {
            both.problems
                .push(format!("MCP's tools are {tools:?}, the CLI's {methods:?}"));
        }

        let (listed, _) = both.ask(Method::Actions, json!({}));
        for effect in ["read-only", "mutating", "destructive"] {
            both.ask(Method::Actions, json!({ "effect": effect }));
        }
        if both.resource("actions://list") != listed["actions"] {
            both.problems
                .push("actions://list is not what `actions` lists".to_owned());
        }
        let schema = said_by_socket(&mut both.client, Method::Schema, &json!({}));
        if (both.resource("actions://schema"), false) != schema {
            both.problems
                .push("actions://schema is not the CLI's `schema`".to_owned());
        }

        let infos = self.server.list().infos_for(who);
        let ids: Vec<&str> = listed["actions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| a["id"].as_str())
            .collect();
        let expected: Vec<&str> = infos.iter().map(|i| i.id.as_str()).collect();
        if ids != expected {
            both.problems.push(format!(
                "`actions` lists {ids:?}, the list has {expected:?} for {}",
                who.describe()
            ));
        }
        for info in &infos {
            both.ask(Method::Describe, json!({ "id": info.id }));
            if info.effect == Effect::ReadOnly {
                both.ask(Method::Run, json!({ "id": info.id }));
            }
        }
        for (id, args) in &self.runs {
            let mut params = json!({ "id": id });
            if let (Some(args), Some(object)) = (args, params.as_object_mut()) {
                object.insert("args".to_owned(), args.clone());
            }
            both.ask(Method::Run, params);
        }
        both.ask(Method::Describe, json!({"id": "PARITY_NO_SUCH_ACTION"}));
        both.ask(Method::Run, json!({"id": "PARITY_NO_SUCH_ACTION"}));
        both.ask(
            Method::Run,
            json!({"id": "PARITY_NO_SUCH_ACTION", "stray": 1}),
        );
        both.ask(Method::Watch, json!({"since": 0, "max": 500}));
        drop(serving);
        if both.problems.is_empty() {
            Ok(Report {
                actions: infos.len(),
                requests: both.requests,
            })
        } else {
            Err(Mismatch {
                problems: both.problems,
            })
        }
    }
}

/// The two faces, and what they have disagreed on.
struct Both {
    mcp: Server,
    client: Client,
    problems: Vec<String>,
    requests: usize,
}

impl Both {
    /// `method` with `params` asked of both; what MCP said.
    fn ask(&mut self, method: Method, params: Value) -> Said {
        self.requests = self.requests.saturating_add(1);
        let over_mcp = said_by_mcp(&self.mcp, method, &params);
        let over_socket = said_by_socket(&mut self.client, method, &params);
        // Each face's call asks a person on its own: the tickets differ.
        let (over_mcp, over_socket) = (unticketed(over_mcp), unticketed(over_socket));
        if over_mcp != over_socket {
            self.problems.push(format!(
                "{} {params}: MCP said {over_mcp:?}, the CLI {over_socket:?}",
                method.name()
            ));
        }
        over_mcp
    }

    /// An MCP resource's JSON.
    fn resource(&self, uri: &str) -> Value {
        let message = json!({"jsonrpc": "2.0", "id": 1, "method": "resources/read",
            "params": {"uri": uri}});
        self.mcp
            .handle(&message.to_string())
            .and_then(|r| serde_json::from_str::<Value>(&r).ok())
            .and_then(|r| {
                r["result"]["contents"][0]["text"]
                    .as_str()
                    .map(str::to_owned)
            })
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(Value::Null)
    }
}

/// `said` with every confirmation's ticket and time left blanked: each
/// call that needs approval is issued a ticket of its own.
fn unticketed((mut value, failed): Said) -> Said {
    fn blank(value: &mut Value) {
        match value {
            Value::Object(object) => {
                if object.get("code").and_then(Value::as_str) == Some("confirmation_required") {
                    object.insert("ticket".to_owned(), json!("<ticket>"));
                    object.insert("expires_in_ms".to_owned(), json!(0));
                }
                object.values_mut().for_each(blank);
            }
            Value::Array(items) => items.iter_mut().for_each(blank),
            _ => {}
        }
    }
    blank(&mut value);
    (value, failed)
}

/// What MCP said: a tool result's JSON and `isError`, or a refusal as the
/// socket words it (`bad_request` and the same message).
fn said_by_mcp(server: &Server, method: Method, params: &Value) -> Said {
    let called = futures::executor::block_on(server.call_tool(
        &ActionCaller::app(),
        method.name(),
        params.clone(),
    ));
    match called {
        Ok(result) => {
            let text = result["content"][0]["text"].as_str().unwrap_or_default();
            let value =
                serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned()));
            (value, result["isError"] == true)
        }
        Err(ToolError::BadParams(message)) => {
            (json!({"code": "bad_request", "error": message}), true)
        }
        Err(ToolError::NoTool(name)) => (json!({"code": "no_method", "error": name}), true),
    }
}

/// What the socket said.
fn said_by_socket(client: &mut Client, method: Method, params: &Value) -> Said {
    match client.call_raw(method.name(), params) {
        Ok(Reply::Ok(value)) => (value, false),
        Ok(Reply::Err(value)) => (value, true),
        Err(failure) => (failure.to_json(), true),
    }
}

/// A socket path no other check uses.
fn scratch_socket() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("architect-parity-{}-{n}.sock", std::process::id()))
}
