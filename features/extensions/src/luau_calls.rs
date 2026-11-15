//! What a Luau extension's `host` and `ext` functions ask, apart from
//! Luau: each call's arguments, as JSON, to one host request or one
//! change to its contributions.
//!
//! The one implementation of both. On a desk `crate::luau_host` (mlua)
//! turns a call's Luau values into JSON and calls [`host_request`]; in a
//! browser the script runs in a Web Worker (`architect_pages::web::luau`)
//! and each call arrives as a message of the same JSON, answered here and
//! by the extension's `ExtensionApi`. `docs/content/guide/showcase.md`
//! lists what runs where.

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::contrib::{Contribution, Contributions, StateWrite};
use crate::hostcall::HostRequest;
use crate::manifest::Manifest;

/// The functions of `host` that make one request each (`host.call`, the
/// raw one, and `host.id` are apart).
pub const HOST_FUNCTIONS: &[&str] = &[
    "run",
    "org",
    "selection",
    "open",
    "read_document",
    "query_vault",
    "list_documents",
    "create_document",
    "edit_document",
    "rename_document",
    "move_document",
    "delete_document",
    "watch_documents",
    "editor",
    "edit_editor",
    "decorate_editor",
    "read_file",
    "write_file",
    "setting",
    "set_setting",
    "data",
    "set_data",
    "log",
    "service",
    "point",
    "window",
    "notify",
    "clipboard_read",
    "clipboard_write",
    "pick",
    "prompt",
    "menu",
    "subscribe",
    "unsubscribe",
    "events",
    "ports",
    "open_port",
    "port_send",
    "port_receive",
    "close_port",
    "targets",
    "surfaces",
    "surface_bank",
    "fetch",
    "read_body",
    "close_body",
    "secret",
    "ask_secret",
    "store_secret",
    "decorate",
];

/// The functions of `ext`.
pub const EXT_FUNCTIONS: &[&str] = &["contribute", "state", "clear"];

/// A JSON value's type, in Luau's words.
fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "nil",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) | Value::Object(_) => "table",
    }
}

/// The arguments of one call to `name`.
struct Args<'a> {
    name: &'a str,
    args: &'a [Value],
}

impl Args<'_> {
    fn at(&self, i: usize) -> &Value {
        self.args.get(i).unwrap_or(&Value::Null)
    }

    fn bad(&self, i: usize, expected: &str) -> String {
        format!(
            "bad argument #{} to '{}' ({expected} expected, got {})",
            i + 1,
            self.name,
            type_name(self.at(i))
        )
    }

    /// A string (a number as Luau writes it, as Luau coerces one).
    fn opt_str(&self, i: usize) -> Result<Option<String>, String> {
        match self.at(i) {
            Value::Null => Ok(None),
            Value::String(s) => Ok(Some(s.clone())),
            Value::Number(n) => Ok(Some(n.as_i64().map_or_else(
                || n.as_f64().map_or_else(|| n.to_string(), |f| f.to_string()),
                |i| i.to_string(),
            ))),
            _ => Err(self.bad(i, "string")),
        }
    }

    fn str(&self, i: usize) -> Result<String, String> {
        self.opt_str(i)?.ok_or_else(|| self.bad(i, "string"))
    }

    fn opt_u64(&self, i: usize) -> Result<Option<u64>, String> {
        match self.at(i) {
            Value::Null => Ok(None),
            Value::Number(n) => n
                .as_u64()
                .or_else(|| {
                    n.as_f64()
                        .filter(|f| f.fract() == 0.0 && *f >= 0.0 && *f < 1.8e19)
                        // Whole and in range, checked above.
                        .map(|f| {
                            #[allow(
                                clippy::cast_possible_truncation,
                                clippy::cast_sign_loss,
                                clippy::as_conversions
                            )]
                            let n = f as u64;
                            n
                        })
                })
                .map(Some)
                .ok_or_else(|| self.bad(i, "integer")),
            _ => Err(self.bad(i, "number")),
        }
    }

    fn opt_u32(&self, i: usize) -> Result<Option<u32>, String> {
        self.opt_u64(i)?
            .map(|n| u32::try_from(n).map_err(|_| self.bad(i, "integer")))
            .transpose()
    }

    fn opt_u16(&self, i: usize) -> Result<Option<u16>, String> {
        self.opt_u64(i)?
            .map(|n| u16::try_from(n).map_err(|_| self.bad(i, "integer")))
            .transpose()
    }

    fn opt_bool(&self, i: usize) -> Result<Option<bool>, String> {
        match self.at(i) {
            Value::Null => Ok(None),
            Value::Bool(b) => Ok(Some(*b)),
            _ => Err(self.bad(i, "boolean")),
        }
    }

    /// A table of a request's fields (`nil`: its defaults), with `op`.
    fn request(&self, i: usize) -> Result<HostRequest, String> {
        let mut v = self.at(i).clone();
        if v.is_null() {
            v = json!({});
        }
        let Value::Object(map) = &mut v else {
            return Err(format!("{}: a table of its fields", self.name));
        };
        map.insert("op".to_owned(), Value::String(self.name.to_owned()));
        lenient(v).map_err(|e| format!("{}: {e}", self.name))
    }
}

/// `value` as `T`; an empty table Luau handed as `{}` read as `[]` where
/// `T` wants a list (Luau's tables are both, and an empty one says
/// neither).
///
/// # Errors
///
/// It is not a `T` either way.
pub fn lenient<T: DeserializeOwned>(value: Value) -> Result<T, serde_json::Error> {
    fn listed(v: Value) -> Value {
        match v {
            Value::Object(m) if m.is_empty() => Value::Array(Vec::new()),
            Value::Object(m) => Value::Object(m.into_iter().map(|(k, v)| (k, listed(v))).collect()),
            Value::Array(a) => Value::Array(a.into_iter().map(listed).collect()),
            other => other,
        }
    }
    match serde_json::from_value::<T>(value.clone()) {
        Ok(t) => Ok(t),
        Err(first) => serde_json::from_value(listed(value)).map_err(|_| first),
    }
}

/// The request `host.<name>(args…)` makes, from its arguments as JSON
/// (`nil` is `null`, in place).
///
/// # Errors
///
/// `name` is no function of `host`, or its arguments are not what it
/// takes: the script's error, as a desk's says it.
#[allow(clippy::too_many_lines)]
pub fn host_request(name: &str, args: &[Value]) -> Result<HostRequest, String> {
    let a = Args { name, args };
    let value = |i: usize| a.at(i).clone();
    let from = |v: Value| lenient::<HostRequest>(v).map_err(|e| format!("{name}: {e}"));
    Ok(match name {
        "run" => HostRequest::Run {
            action: a.str(0)?,
            args: value(1),
            ticket: None,
        },
        "org" => HostRequest::Org,
        "selection" => HostRequest::Selection,
        "open" => HostRequest::Open { path: a.str(0)? },
        "read_document" => HostRequest::ReadDocument { path: a.str(0)? },
        "query_vault" | "watch_documents" | "edit_editor" | "prompt" | "menu" | "subscribe" => {
            a.request(0)?
        }
        "pick" => a.request(0)?,
        "list_documents" => HostRequest::ListDocuments {
            under: a.opt_str(0)?,
        },
        "create_document" => HostRequest::CreateDocument {
            path: a.str(0)?,
            text: a.opt_str(1)?.unwrap_or_default(),
        },
        "edit_document" => from(json!({
            "op": "edit_document", "path": a.str(0)?, "edits": value(1), "name": a.opt_str(2)?,
        }))?,
        "rename_document" => HostRequest::RenameDocument {
            path: a.str(0)?,
            to: a.str(1)?,
        },
        "move_document" => HostRequest::MoveDocument {
            path: a.str(0)?,
            folder: a.str(1)?,
        },
        "delete_document" => HostRequest::DeleteDocument { path: a.str(0)? },
        "editor" => HostRequest::EditorState,
        "decorate_editor" => from(json!({ "op": "decorate_editor", "decorations": value(0) }))?,
        "read_file" => HostRequest::ReadFile { path: a.str(0)? },
        "write_file" => HostRequest::WriteFile {
            path: a.str(0)?,
            text: a.str(1)?,
        },
        "setting" => HostRequest::Setting { key: a.str(0)? },
        "set_setting" => HostRequest::SetSetting {
            key: a.str(0)?,
            value: value(1),
        },
        "data" => HostRequest::Data { key: a.str(0)? },
        "set_data" => HostRequest::SetData {
            key: a.str(0)?,
            value: Some(value(1)).filter(|v| !v.is_null()),
        },
        "log" => HostRequest::Log {
            text: a.str(0)?,
            level: a.opt_str(1)?,
        },
        "service" => HostRequest::Service {
            service: a.str(0)?,
            method: a.str(1)?,
            args: value(2),
        },
        "point" => HostRequest::Point { point: a.str(0)? },
        "window" => HostRequest::Window {
            id: a.str(0)?,
            open: a.opt_bool(1)?.unwrap_or(true),
            state: value(2),
        },
        "notify" => HostRequest::Notify {
            text: a.str(0)?,
            kind: a.opt_str(1)?,
            title: a.opt_str(2)?,
        },
        "clipboard_read" => HostRequest::ClipboardRead,
        "clipboard_write" => HostRequest::ClipboardWrite { text: a.str(0)? },
        "unsubscribe" => HostRequest::Unsubscribe { key: a.str(0)? },
        "events" => HostRequest::Events,
        "ports" => HostRequest::Ports {
            kind: a.opt_str(0)?,
        },
        "open_port" => HostRequest::OpenPort {
            kind: a.str(0)?,
            name: a.str(1)?,
            listen: a.opt_u16(2)?,
        },
        "port_send" => HostRequest::PortSend {
            port: a.str(0)?,
            message: value(1),
        },
        "port_receive" => HostRequest::PortReceive {
            port: a.str(0)?,
            max: a.opt_u32(1)?,
        },
        "close_port" => HostRequest::ClosePort { port: a.str(0)? },
        "targets" => HostRequest::Targets,
        "surfaces" => HostRequest::Surfaces,
        "surface_bank" => HostRequest::SurfaceBank {
            surface: a.str(0)?,
            bank: a.opt_u32(1)?.ok_or_else(|| a.bad(1, "number"))?,
        },
        // `fetch(url, options?)` or `fetch({url = …, …})`.
        "fetch" => match a.at(0) {
            Value::String(url) => {
                let mut v = value(1);
                if !v.is_object() {
                    v = json!({});
                }
                if let Value::Object(map) = &mut v {
                    map.insert("url".to_owned(), Value::String(url.clone()));
                    map.insert("op".to_owned(), Value::String("fetch".to_owned()));
                }
                from(v)?
            }
            _ => a.request(0)?,
        },
        "read_body" => HostRequest::ReadBody {
            stream: a.str(0)?,
            max: a.opt_u64(1)?,
        },
        "close_body" => HostRequest::CloseBody { stream: a.str(0)? },
        "secret" => HostRequest::Secret { name: a.str(0)? },
        "ask_secret" => HostRequest::AskSecret { name: a.str(0)? },
        "store_secret" => HostRequest::StoreSecret {
            name: a.str(0)?,
            value: a.str(1)?,
        },
        "decorate" => {
            from(json!({ "op": "decorate", "target": value(0), "decoration": value(1) }))?
        }
        other => return Err(format!("host has no function {other}")),
    })
}

/// `host.call(request)`'s request: the raw one, read from its table.
///
/// # Errors
///
/// It is no host request.
pub fn raw_request(request: Value) -> Result<HostRequest, String> {
    lenient(request).map_err(|e| format!("not a host request: {e}"))
}

/// `ext.<name>(args…)` for `manifest`'s extension, on `contributions`:
/// `contribute(table)` puts what it adds at run time (checked as its
/// manifest's tables are) and answers the diff; `state{…}` writes its
/// runtime state; `clear(key?)` takes it off and answers how many went.
///
/// # Errors
///
/// What it gave is not what the function takes, or the check refused it:
/// the script's error.
pub fn ext_call(
    manifest: &Manifest,
    contributions: &Contributions,
    name: &str,
    args: &[Value],
    now_ms: u64,
) -> Result<Value, String> {
    let a = Args { name, args };
    match name {
        "contribute" => {
            let added: Contribution = lenient(a.at(0).clone()).map_err(|e| e.to_string())?;
            let views = manifest.view_ids();
            added
                .check(&manifest.rules(&views))
                .map_err(|e| e.to_string())?;
            let diff = contributions
                .put_runtime(&manifest.id, added)
                .ok_or_else(|| format!("{} is not loaded", manifest.id))?;
            serde_json::to_value(&diff).map_err(|e| e.to_string())
        }
        "state" => {
            let mut write: StateWrite = lenient(a.at(0).clone()).map_err(|e| e.to_string())?;
            write.source = format!("ext:{}", manifest.id);
            let applied = contributions
                .set_state(write, now_ms)
                .map_err(|e| e.to_string())?;
            serde_json::to_value(&applied).map_err(|e| e.to_string())
        }
        "clear" => {
            let key = a.opt_str(0)?;
            Ok(json!(contributions.clear_state(
                &format!("ext:{}", manifest.id),
                key.as_deref()
            )))
        }
        other => Err(format!("ext has no function {other}")),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{HOST_FUNCTIONS, host_request};
    use crate::hostcall::HostRequest;

    /// Every function `host` lists makes a request from plausible
    /// arguments, and the request is the op it is named for.
    #[test]
    fn every_host_function_makes_its_request() {
        for name in HOST_FUNCTIONS {
            let args = match *name {
                "edit_document" => vec![json!("a.md"), json!([]), json!(null)],
                "decorate_editor" => vec![json!({})],
                "decorate" => vec![json!({"kind": "file", "id": "a"}), json!({"icon": "star"})],
                "fetch" => vec![json!("https://example.com")],
                "surface_bank" => vec![json!("s"), json!(1)],
                "port_send" | "set_setting" | "set_data" | "run" | "service" => {
                    vec![json!("a"), json!("b"), json!(null)]
                }
                "prompt" => vec![json!({"kind": "text", "title": "t"})],
                "menu" => vec![json!({"x": 1, "y": 2, "items": []})],
                "subscribe" => vec![json!({"on": "doc.saved", "action": "A"})],
                "watch_documents" => vec![json!({"action": "A"})],
                "edit_editor" => vec![json!({"changes": []})],
                "query_vault" => vec![json!({})],
                "pick" | "org" | "selection" | "editor" | "clipboard_read" | "events"
                | "targets" | "surfaces" | "ports" | "list_documents" => vec![],
                "window" => vec![json!("prefs"), json!(true), json!(null)],
                "notify" => vec![json!("hi"), json!(null), json!("t")],
                "open_port" => vec![json!("midi"), json!("x"), json!(9001)],
                "port_receive" | "read_body" => vec![json!("p"), json!(4)],
                _ => vec![json!("a"), json!("b")],
            };
            let request = host_request(name, &args).unwrap_or_else(|e| panic!("{name}: {e}"));
            let op = request.op();
            assert!(
                op == *name || matches!((*name, op), ("editor", "editor_state") | ("org", "org")),
                "{name} made {op}"
            );
        }
    }

    #[test]
    fn arguments_are_checked_as_luau_says() {
        let e = host_request("open", &[json!(true)]).expect_err("a boolean is no path");
        assert_eq!(
            e,
            "bad argument #1 to 'open' (string expected, got boolean)"
        );
        // A number is a string, as Luau coerces one.
        assert!(matches!(
            host_request("setting", &[json!(3)]),
            Ok(HostRequest::Setting { key }) if key == "3"
        ));
        // An empty table is an empty list where one is wanted.
        assert!(host_request("decorate_editor", &[json!({})]).is_ok());
    }
}
