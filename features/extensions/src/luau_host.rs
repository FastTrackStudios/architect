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
//! -- Hooks made at run time (crate::host_events): one of its actions on an
//! -- event or a schedule; the key takes it off again.
//! local key = host.subscribe({ on = "doc.saved", action = "ACME_LINT" })
//! host.subscribe({ every = "5m", action = "ACME_SYNC" })
//! host.unsubscribe(key); local heard = host.events()
//! -- The app's documents (crate::documents): reads = ["documents"] lists,
//! -- reads and watches them, writes = ["documents"] changes them.
//! local docs = host.list_documents("notes")         -- {{path, name, space}, …}
//! host.create_document("notes/Inbox.md", "# Inbox\n")
//! host.edit_document("notes/Inbox.md", {           -- one transaction, one undo step
//!     { insert = { at = 0, text = "---\n---\n" } },
//!     { append = { text = "- [ ] call Sam\n" } },
//! }, "Add a task")
//! host.rename_document("notes/Inbox.md", "notes/Today.md")
//! host.move_document("notes/Today.md", "archive")  -- archive/Today.md
//! host.delete_document("archive/Today.md")
//! host.watch_documents({ action = "ACME_REINDEX" }) -- doc.created/saved/renamed/deleted
//! -- The editor being typed in (crate::editor): reads or writes =
//! -- ["editor"] reads it, writes = ["editor"] edits and decorates it.
//! local ed = host.editor()   -- {editor, context, text, selection, cursor, node, version}
//! if ed.node.kind == "list-item" then
//!     host.edit_editor({                            -- one transaction, one undo step
//!         changes = { { from = ed.node.line_from, to = ed.node.line_from, insert = "  " } },
//!         name = "Indent", base = ed.version,
//!     })
//! end
//! host.decorate_editor({ { kind = "mark", from = 0, to = 4, class = "todo" } })
//! -- The network (crate::net): to the hosts in capabilities.hosts, each
//! -- redirect checked; a secret named in a header, put in by the host.
//! local r = host.fetch("https://api.example.com/v1/items", {
//!     headers = { Authorization = "Bearer {{secret:api_key}}" },
//! })                                               -- {status, url, headers, body}
//! local s = host.fetch({ url = "https://api.example.com/feed", stream = true })
//! repeat local part = host.read_body(s.stream) until part.done   -- {chunk, done}
//! local known = host.secret("api_key").set; host.ask_secret("api_key")
//! -- MIDI ports and OSC endpoints (crate::ports): those capabilities.ports
//! -- names; what one hears is a port.message event to it (a hook's
//! -- args.detail), or waits for port_receive.
//! local midi = host.ports("midi")                  -- {{kind, name, direction}, …}
//! local p = host.open_port("midi", "X-Touch").port
//! host.port_send(p, { type = "cc", channel = 1, number = 7, value = 100 })
//! local o = host.open_port("osc", "127.0.0.1:9000", 9001).port   -- hears on 9001
//! host.port_send(o, { address = "/track/1/volume", args = { 0.5 } })
//! for _, m in host.port_receive(o) do host.log(m.address) end
//! host.close_port(p)
//! -- Control surfaces (crate::ports): the app's targets, its own surfaces.
//! local kinds = host.targets(); host.surface_bank("fake8", 1)
//! ```
//!
//! `pick`, `prompt` and `menu` wait for the person; a cancel is `nil`.
//!
//! Each function but `call` raises the host's refusal as the script's
//! error (`refused (actions): …`), which `pcall` catches.

use architect_scripting::luau::mlua::{self, Lua, LuaSerdeExt as _, MultiValue, Value as LuaValue};
use architect_scripting::luau::{Module, raise};
use serde_json::{Value, json};

use crate::hostapi::ExtensionApi;
use crate::hostcall::HostAnswer;
use crate::luau_calls::{HOST_FUNCTIONS, host_request, raw_request};

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

/// A call's arguments as JSON, in place (`nil` → `null`).
fn args_of(lua: &Lua, args: MultiValue) -> mlua::Result<Vec<Value>> {
    args.into_iter().map(|v| json_of(lua, v)).collect()
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
        // `name(args…)` → one request (`crate::luau_calls`, the one
        // mapping a browser's Luau uses too), its value or the refusal
        // raised.
        for name in HOST_FUNCTIONS {
            let api = self.api.clone();
            t.set(
                *name,
                lua.create_async_function(move |lua, args: MultiValue| {
                    let api = api.clone();
                    let request = args_of(&lua, args)
                        .and_then(|args| host_request(name, &args).map_err(|e| raise(&lua, e)));
                    async move {
                        match api.call(request?).await {
                            // `null` (nothing set, a cancel) is `nil`.
                            Ok(Value::Null) => Ok(LuaValue::Nil),
                            Ok(v) => lua.to_value(&v),
                            Err(e) => Err(raise(&lua, e.to_string())),
                        }
                    }
                })?,
            )?;
        }
        // The answer as a table, never raised.
        let api = self.api.clone();
        t.set(
            "call",
            lua.create_async_function(move |lua, request: LuaValue| {
                let api = api.clone();
                let request = json_of(&lua, request);
                async move {
                    let answer = match request.map(raw_request) {
                        Ok(Ok(request)) => HostAnswer::from(api.call(request).await),
                        Ok(Err(message)) => HostAnswer::Ok(
                            json!({ "error": { "kind": "failed", "message": message } }),
                        ),
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
