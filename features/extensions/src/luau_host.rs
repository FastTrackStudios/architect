//! `host` in a Luau extension's scripts: the host service
//! (`crate::hostapi`), as functions.
//!
//! ```lua
//! local status = host.run("APP_STATUS")          -- an app action, as the extension
//! local sel = host.selection()                    -- needs reads = ["selection"]
//! local accent = host.setting("fts.metronome.accent")
//! host.set_data("last_run", os.time())
//! host.log("counted in")
//! local answer = host.call({ op = "org" })        -- {ok = …} or {error = {kind, …}}, never raises
//! local bpm = host.service("lab.tempo", "get")    -- a service it [requires], its provider's action
//! local items = host.point("lab.notes.cards")     -- one of its own points' items
//! host.window("prefs", true, { tab = "audio" })   -- opens one of its own windows
//! -- The app's interface (capabilities.ui):
//! host.notify("Imported 12 notes", "success")     -- ui = ["notify"]
//! host.clipboard_write("hello"); local t = host.clipboard_read()   -- "clipboard"
//! local path = host.pick({ kind = "open", filters = {{ name = "CSV", extensions = {"csv"} }} })
//! local name = host.prompt({ kind = "text", title = "Name?", default = "untitled" })
//! local ok = host.prompt({ kind = "confirm", title = "Replace it?" })
//! local id = host.menu({ x = 120, y = 80, items = {{ id = "copy", label = "Copy" }} })
//! host.decorate({ kind = "file", id = "notes/a.md" }, { icon = "star", badge = "3" })
//! ```
//!
//! `pick`, `prompt` and `menu` wait for the person; a cancel is `nil`.
//!
//! Each function but `call` raises the host's refusal as the script's
//! error (`refused (actions): …`), which `pcall` catches.

use architect_scripting::luau::mlua::{self, Lua, LuaSerdeExt as _, Value as LuaValue};
use architect_scripting::luau::{Module, raise};
use serde_json::{Value, json};

use crate::hostapi::ExtensionApi;
use crate::hostcall::{HostAnswer, HostRequest};

/// The `host` global, acting for whoever ran the script.
pub struct HostModule {
    api: ExtensionApi,
}

impl HostModule {
    pub const fn new(api: ExtensionApi) -> Self {
        Self { api }
    }
}

/// A Luau value as JSON (`nil` → `null`).
fn json_of(lua: &Lua, value: LuaValue) -> mlua::Result<Value> {
    match value {
        LuaValue::Nil => Ok(Value::Null),
        v => lua.from_value(v),
    }
}

/// Request `op` from a table of its fields (`nil`: the defaults).
fn request_of(lua: &Lua, op: &str, fields: Option<LuaValue>) -> mlua::Result<HostRequest> {
    let mut v = fields.map_or(Ok(Value::Null), |f| json_of(lua, f))?;
    if v.is_null() {
        v = json!({});
    }
    let Value::Object(map) = &mut v else {
        return Err(mlua::Error::runtime(format!("{op}: a table of its fields")));
    };
    map.insert("op".to_owned(), Value::String(op.to_owned()));
    serde_json::from_value(v).map_err(|e| mlua::Error::runtime(format!("{op}: {e}")))
}

impl Module for HostModule {
    fn global(&self) -> &'static str {
        "host"
    }

    fn path(&self) -> &'static str {
        "host"
    }

    fn install(&self, lua: &Lua) -> mlua::Result<LuaValue> {
        let t = lua.create_table()?;
        t.set("id", self.api.id().to_owned())?;
        // `name(args…)` → one request, its value or the refusal raised.
        macro_rules! op {
            ($name:literal, $args:ty, |$lua:ident, $a:ident| $make:expr) => {{
                let api = self.api.clone();
                t.set(
                    $name,
                    lua.create_async_function(move |$lua, $a: $args| {
                        let api = api.clone();
                        let request: mlua::Result<HostRequest> = $make;
                        async move {
                            let request = request?;
                            match api.call(request).await {
                                // `null` (nothing set, a cancel) is `nil`.
                                Ok(Value::Null) => Ok(LuaValue::Nil),
                                Ok(v) => $lua.to_value(&v),
                                Err(e) => Err(raise(&$lua, e.to_string())),
                            }
                        }
                    })?,
                )?;
            }};
        }
        op!("run", (String, Option<LuaValue>), |lua, a| {
            let (action, args) = a;
            args.map_or(Ok(Value::Null), |v| json_of(&lua, v))
                .map(|args| HostRequest::Run {
                    action,
                    args,
                    ticket: None,
                })
        });
        op!("org", (), |lua, _a| Ok(HostRequest::Org));
        op!("selection", (), |lua, _a| Ok(HostRequest::Selection));
        op!("open", String, |lua, a| Ok(HostRequest::Open { path: a }));
        op!("read_document", String, |lua, a| Ok(
            HostRequest::ReadDocument { path: a }
        ));
        op!("read_file", String, |lua, a| Ok(HostRequest::ReadFile {
            path: a
        }));
        op!("write_file", (String, String), |lua, a| Ok(
            HostRequest::WriteFile {
                path: a.0,
                text: a.1
            }
        ));
        op!("setting", String, |lua, a| Ok(HostRequest::Setting {
            key: a
        }));
        op!("set_setting", (String, LuaValue), |lua, a| json_of(
            &lua, a.1
        )
        .map(|value| HostRequest::SetSetting { key: a.0, value }));
        op!("data", String, |lua, a| Ok(HostRequest::Data { key: a }));
        op!("set_data", (String, LuaValue), |lua, a| json_of(&lua, a.1)
            .map(|v| HostRequest::SetData {
                key: a.0,
                value: (!v.is_null()).then_some(v),
            }));
        op!("log", (String, Option<String>), |lua, a| Ok(
            HostRequest::Log {
                text: a.0,
                level: a.1
            }
        ));
        op!("service", (String, String, Option<LuaValue>), |lua, a| {
            let (service, method, args) = a;
            args.map_or(Ok(Value::Null), |v| json_of(&lua, v))
                .map(|args| HostRequest::Service {
                    service,
                    method,
                    args,
                })
        });
        op!("point", String, |lua, a| Ok(HostRequest::Point {
            point: a
        }));
        op!(
            "window",
            (String, Option<bool>, Option<LuaValue>),
            |lua, a| {
                let (id, open, state) = a;
                state
                    .map_or(Ok(Value::Null), |v| json_of(&lua, v))
                    .map(|state| HostRequest::Window {
                        id,
                        open: open.unwrap_or(true),
                        state,
                    })
            }
        );
        op!("notify", (String, Option<String>, Option<String>), |lua, a| Ok(
            HostRequest::Notify {
                text: a.0,
                kind: a.1,
                title: a.2
            }
        ));
        op!("clipboard_read", (), |lua, _a| Ok(HostRequest::ClipboardRead));
        op!("clipboard_write", String, |lua, a| Ok(
            HostRequest::ClipboardWrite { text: a }
        ));
        op!("pick", Option<LuaValue>, |lua, a| request_of(
            &lua, "pick", a
        ));
        op!("prompt", LuaValue, |lua, a| request_of(
            &lua,
            "prompt",
            Some(a)
        ));
        op!("menu", LuaValue, |lua, a| request_of(&lua, "menu", Some(a)));
        op!("decorate", (LuaValue, Option<LuaValue>), |lua, a| {
            let (target, decoration) = a;
            json_of(&lua, target).and_then(|target| {
                let decoration = decoration.map_or(Ok(Value::Null), |d| json_of(&lua, d))?;
                serde_json::from_value(
                    json!({ "op": "decorate", "target": target, "decoration": decoration }),
                )
                .map_err(|e| mlua::Error::runtime(format!("decorate: {e}")))
            })
        });
        // The answer as a table, never raised.
        let api = self.api.clone();
        t.set(
            "call",
            lua.create_async_function(move |lua, request: LuaValue| {
                let api = api.clone();
                let request = json_of(&lua, request);
                async move {
                    let answer = match request.map(serde_json::from_value::<HostRequest>) {
                        Ok(Ok(request)) => HostAnswer::from(api.call(request).await),
                        Ok(Err(e)) => HostAnswer::Ok(json!({ "error": { "kind": "failed", "message": format!("not a host request: {e}") } })),
                        Err(e) => return Err(e),
                    };
                    let value = serde_json::to_value(&answer).unwrap_or(Value::Null);
                    lua.to_value(&value)
                }
            })?,
        )?;
        Ok(LuaValue::Table(t))
    }
}
