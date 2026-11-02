//! MCP for an app that is running elsewhere (feature `bridge`): a model's
//! generic tool calls forwarded to the app's control socket
//! (`architect-control`), one request each, and the answers rendered as
//! tool results.
//!
//! An app whose window owns its Action List (a Blitz desk app has no
//! runtime to serve MCP's HTTP transport on) still gets a model: its CLI's
//! `mcp` subcommand runs [`serve_stdio_via`] against its socket, and a
//! client configured with that command line drives the live app. What the
//! model is told is what [`crate::Server`] would tell it over the same
//! list — the same tools, the same JSON — since both render the same
//! answers. Generic tools only, in [`crate::ErrorStyle::Json`].

use std::path::Path;
use std::sync::{Mutex, PoisonError};

use architect_control::client::Client;
use architect_control::protocol::{Failure, Filter, Method, Reply, Request};
use serde_json::{Value, json};

use crate::{
    INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND, PARSE_ERROR, Tools, error_reply,
    generic_tools, initialized, instructions, resources, text_result,
};

/// A model's MCP session over a running app's control socket.
#[derive(Debug)]
pub struct Bridge {
    client: Mutex<Client>,
    name: String,
    version: String,
    instructions: String,
}

impl Bridge {
    /// Connect to the app serving `path`, and ask who it is.
    ///
    /// # Errors
    ///
    /// No app answers there, or it does not speak the protocol.
    pub fn connect(path: &Path) -> Result<Self, Failure> {
        let mut client = Client::connect(path)?;
        let hello = client.call(&Request::Hello)?;
        if let Some(failure) = hello.failure() {
            return Err(failure);
        }
        let text = |key: &str| {
            hello
                .value()
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        Ok(Self {
            name: text("name"),
            version: text("version"),
            client: Mutex::new(client),
            instructions: String::new(),
        })
    }

    /// What a model is told when it connects, before the generic part.
    #[must_use]
    pub fn with_instructions(mut self, text: impl Into<String>) -> Self {
        self.instructions = text.into();
        self
    }

    fn call(&self, request: &Request) -> Result<Reply, Failure> {
        self.client
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .call(request)
    }

    /// One JSON-RPC message in, its reply out (`None` for a
    /// notification), as [`crate::Server::handle`] answers it.
    #[must_use]
    pub fn handle(&self, message: &str) -> Option<String> {
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
        let id = id?;
        let reply = match self.dispatch(method, &params) {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => error_reply(&id, code, &message),
        };
        Some(reply.to_string())
    }

    fn dispatch(&self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => {
                let count = self
                    .call(&Request::Hello)
                    .ok()
                    .and_then(|r| r.value().get("actions").and_then(Value::as_u64))
                    .and_then(|n| usize::try_from(n).ok());
                Ok(initialized(
                    params,
                    &self.name,
                    &self.version,
                    instructions(&self.instructions, Tools::Generic, &self.name, count),
                ))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": generic_tools() })),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or((INVALID_PARAMS, "no tool name".to_owned()))?;
                if !Method::TOOLS.iter().any(|m| m.name() == name) {
                    return Err((INVALID_PARAMS, format!("no tool {name}")));
                }
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let request = Request::parse(name, &args)
                    .map_err(|refused| (INVALID_PARAMS, refused.error))?;
                Ok(match self.call(&request) {
                    Ok(Reply::Ok(value)) => text_result(&value, false),
                    Ok(Reply::Err(value)) => text_result(&value, true),
                    Err(failure) => text_result(&failure.to_json(), true),
                })
            }
            "resources/list" => Ok(json!({ "resources": resources() })),
            "resources/read" => {
                let uri = params
                    .get("uri")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let text = match uri {
                    "actions://list" => self
                        .call(&Request::Actions(Filter::default()))
                        .map(|r| r.value().get("actions").cloned().unwrap_or(Value::Null)),
                    "actions://schema" => self.call(&Request::Schema).map(|r| r.value().clone()),
                    _ => return Err((INVALID_PARAMS, format!("no resource {uri}"))),
                }
                .map_err(|failure| (INVALID_PARAMS, failure.error))?;
                Ok(json!({ "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": text.to_string(),
                }]}))
            }
            _ => Err((METHOD_NOT_FOUND, format!("no method {method}"))),
        }
    }
}

/// MCP over stdin/stdout for the app serving `path`, until stdin closes.
///
/// # Errors
///
/// No app answers there, or reading stdin or writing stdout failed.
pub fn serve_stdio_via(path: &Path) -> std::io::Result<()> {
    let bridge = Bridge::connect(path).map_err(std::io::Error::other)?;
    crate::serve_lines(|line| bridge.handle(line))
}
