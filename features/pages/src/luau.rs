//! Luau element pages (feature `luau`).
//!
//! A `.luau` page draws with the element builder (`div():child(...):on_click(...)`), on the same edit
//! path a TSX page takes — its tree diffed into `wire::Edit`s on its own
//! thread, the window only applying frames.
//!
//! ```lua
//! local n = 0
//! render(function()
//!   return div():style({ padding = 12, color = "$foreground" })
//!     :child(span():child("clicked " .. n))
//!     :child(button():child("+1"):on_click(function() n += 1 end))
//! end)
//! ```
//!
//! Its globals: the builders (`div`, `span`, `button`, `input`, `p`, `ul`,
//! `li`, `h1`…`h3`, `el(tag)`, `text(s)`), `render(fn)`, `invalidate()`,
//! `actions.run(id, args?)` (it waits for the answer on the page's thread,
//! the action on the app's runtime), `state.get/set` (shared state),
//! `embed()` (an inline embed's input: `{claim, source, props}`, or nil
//! in a pane; it renders again when that changes),
//! `theme.value(name)`, `theme.scheme`, `theme.revision`, `notify(text)`,
//! the app's interface as the page's extension (`ui.pick`, `ui.prompt`,
//! `ui.menu(items, at?)`, `ui.decorate(target, decoration?)`, `ui.call`,
//! `clipboard.read()`, `clipboard.write(text)`: each waits for its answer;
//! a cancel is nil), hooks made at run time (`events.subscribe(hook)`,
//! `events.unsubscribe(key)`, `events.list()`), the app's documents
//! (`documents.list`, `read`, `create`, `edit`, `rename`, `move`,
//! `delete`, `watch`), the network (`net.fetch(url, options?)`,
//! `net.read(stream, max?)`, `net.close(stream)`, `net.secret(name)`,
//! `net.ask_secret(name)`) and `print`. Luau's sandbox holds; each call into the page has
//! [`crate::runner::PageLimits::frame`] before its interrupt stops it.
//!
//! **Timers**: `set_timeout(fn, ms)`, `set_interval(fn, ms)`,
//! `clear_timeout(id)` / `clear_interval(id)`, as a TSX page's; each runs
//! on the page's thread and renders after.
//!
//! **Workers**: `worker.spawn("primes.luau", args)` runs a `.luau` file of
//! the page's folder in a Luau state of its own on the scripting pool
//! (`architect_scripting::worker`, the same workers a Luau view has),
//! with the page's [`crate::runner::PageLimits::worker_memory`] and
//! `worker_time`. The handle `w` has `w:post(msg)`, `w:cancel()`,
//! `w:status()`, `w:progress()`, `w:result()`, and the callbacks
//! `w:on_message(fn)`, `w:on_progress(fn)`, `w:on_done(fn)` (or
//! `worker.on_message(w, fn)`…), run on the page's thread as its news
//! comes, a frame after each. The worker's own side is a Luau view's
//! worker's: `args`, `worker.post`, `worker.progress`, `worker.receive`,
//! `worker.on_message`, `worker.cancelled`. It stops with its page.

use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::task::{Context, Waker};
use std::time::{Duration, Instant};

use architect_scripting::luau::Limits;
use architect_scripting::luau::mlua::{self, Lua, LuaSerdeExt as _, Value as LuaValue};
use architect_scripting::vm::{Guard, Stop, Wake};
use architect_scripting::worker::{self, Workers};
use serde_json::Value;

use crate::api::{
    DataEvent, Element, Notice, PageEvent, PageHost as _, ThemeSnapshot, WorkerMessage,
};
use crate::files::PageFile;
use crate::runner::{Engine, Inbox, Load, Services};

const PRELUDE: &str = include_str!("luau_prelude.luau");

/// The [`Load`] for Luau pages.
#[must_use]
pub fn loader() -> Load {
    Arc::new(|page: &PageFile, services: &Services| {
        let engine: Box<dyn Engine> = Box::new(LuauEngine::load(page, services)?);
        Ok(engine)
    })
}

/// A Luau page's state, on its thread.
pub struct LuauEngine {
    lua: Lua,
    guard: Arc<Guard>,
    page: mlua::Table,
    services: Services,
    theme: Arc<Mutex<ThemeSnapshot>>,
    /// Its workers' news: set when any came, and how it is noticed.
    news: Wake,
    waker: Waker,
}

/// Wakes a page's thread: its workers' news came.
struct InboxWaker(Mutex<mpsc::Sender<Inbox>>);

impl std::task::Wake for InboxWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let _ = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send(Inbox::Wake);
    }
}

impl std::fmt::Debug for LuauEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LuauEngine")
            .field("services", &self.services)
            .finish_non_exhaustive()
    }
}

fn err(e: &mlua::Error) -> String {
    e.to_string().lines().next().unwrap_or_default().to_owned()
}

impl LuauEngine {
    /// Loads `page`: the runtime, then the page's script, which calls
    /// `render(fn)`.
    ///
    /// # Errors
    ///
    /// It does not parse (with its line), it throws, or it runs past its
    /// time.
    pub fn load(page: &PageFile, services: &Services) -> Result<Self, String> {
        let lua = Lua::new();
        lua.set_memory_limit(services.limits.memory)
            .map_err(|e| err(&e))?;
        let guard = Arc::new(Guard::new(u64::MAX, vec![services.stop.clone()]));
        let g = Arc::clone(&guard);
        lua.set_interrupt(move |_| match g.step() {
            None => Ok(mlua::VmState::Continue),
            Some(Stop::Time(allowed)) => Err(mlua::Error::runtime(format!(
                "ran past its time of {} ms: hand heavy work to a worker",
                allowed.as_millis()
            ))),
            Some(_) => Err(mlua::Error::runtime("it was stopped")),
        });
        let source = std::fs::read_to_string(&page.path).map_err(|e| e.to_string())?;
        let name = page
            .path
            .file_name()
            .map_or_else(|| "page".to_owned(), |n| n.to_string_lossy().into_owned());
        let theme = Arc::new(Mutex::new(services.host.theme_snapshot()));
        let news = Wake::default();
        let (api, table) =
            Self::install(&lua, page, services, &theme, &news).map_err(|e| err(&e))?;
        let globals = lua.globals();
        for pair in api.pairs::<String, LuaValue>() {
            let (k, v) = pair.map_err(|e| err(&e))?;
            globals.set(k, v).map_err(|e| err(&e))?;
        }
        // Luau resolves `Global.field` once as a chunk loads, in a
        // sandbox: the theme's fields move, so they are read each time.
        lua.set_compiler(mlua::chunk::Compiler::new().set_mutable_globals(["theme".to_owned()]));
        lua.sandbox(true).map_err(|e| err(&e))?;
        guard.deadline(Some(services.limits.load));
        let loaded = lua
            .load(&source)
            .set_name(format!("={name}"))
            .exec()
            .map_err(|e| err(&e));
        guard.deadline(None);
        loaded?;
        let waker = Waker::from(Arc::new(InboxWaker(Mutex::new(services.inbox.clone()))));
        Ok(Self {
            lua,
            guard,
            page: table,
            services: services.clone(),
            theme,
            news,
            waker,
        })
    }

    /// The `.luau` files of the page's folder, the page's own left out:
    /// the workers it may spawn, each by its file name.
    fn workers(page: &PageFile, services: &Services) -> Workers {
        let limits = Limits {
            memory: services.limits.worker_memory,
            time: Some(services.limits.worker_time),
            ..Limits::worker()
        };
        let mut workers = Workers::new().with_limits(limits);
        let Ok(entries) = std::fs::read_dir(page.root()) else {
            return workers;
        };
        let mut found: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| *p != page.path && p.extension().is_some_and(|e| e == "luau"))
            .collect();
        found.sort();
        for path in found {
            if let (Some(name), Ok(source)) = (
                path.file_name().and_then(|n| n.to_str()),
                std::fs::read_to_string(&path),
            ) {
                workers = workers.with_source(name, source);
            }
        }
        workers
    }

    /// The host's globals and the builder: what the prelude returns.
    fn install(
        lua: &Lua,
        file: &PageFile,
        services: &Services,
        theme: &Arc<Mutex<ThemeSnapshot>>,
        news: &Wake,
    ) -> mlua::Result<(mlua::Table, mlua::Table)> {
        let host = lua.create_table()?;
        let started = Instant::now();
        host.set(
            "now",
            lua.create_function(move |_, ()| Ok(started.elapsed().as_secs_f64() * 1000.0))?,
        )?;
        lua.globals().set("__host", host)?;
        let (api, page): (mlua::Table, mlua::Table) =
            lua.load(PRELUDE).set_name("=architect").eval()?;
        lua.globals().set("__host", LuaValue::Nil)?;
        let actions = lua.create_table()?;
        let h = Arc::clone(&services.host);
        actions.set(
            "run",
            lua.create_function(move |lua, (id, args): (String, Option<LuaValue>)| {
                let args: Value = args
                    .map(|a| lua.from_value(a))
                    .transpose()?
                    .unwrap_or(Value::Null);
                let out = futures::executor::block_on(h.run(id, args.to_string()))
                    .map_err(|e| mlua::Error::runtime(e.to_string()))?;
                let v: Value = serde_json::from_str(&out).unwrap_or(Value::Null);
                lua.to_value(&v)
            })?,
        )?;
        api.set("actions", actions)?;
        let state = lua.create_table()?;
        let h = Arc::clone(&services.host);
        state.set(
            "get",
            lua.create_function(move |lua, key: String| {
                lua.to_value(&h.shared_state().get(&key).unwrap_or(Value::Null))
            })?,
        )?;
        let h = Arc::clone(&services.host);
        state.set(
            "set",
            lua.create_function(move |lua, (key, v): (String, LuaValue)| {
                h.shared_state().set(&key, lua.from_value(v)?);
                Ok(())
            })?,
        )?;
        api.set("state", state)?;
        let workers = Self::workers(file, services).install_owner(
            lua,
            Some(news.clone()),
            vec![services.stop.clone()],
        )?;
        api.set("worker", workers)?;
        let embed = services.embed.clone();
        api.set(
            "embed",
            lua.create_function(move |lua, ()| {
                embed.as_ref().map_or(Ok(LuaValue::Nil), |cell| {
                    let now = cell.lock().unwrap_or_else(PoisonError::into_inner).clone();
                    lua.to_value(&now)
                })
            })?,
        )?;
        let h = Arc::clone(&services.host);
        api.set(
            "notify",
            lua.create_function(move |_, (text, kind): (String, Option<String>)| {
                h.show(&Notice {
                    text,
                    kind: kind.unwrap_or_else(|| "info".to_owned()),
                });
                Ok(())
            })?,
        )?;
        // The app's interface, as the page's extension (capabilities.ui):
        // each call waits for its answer on the page's thread; a cancel is
        // nil.
        let ui = lua.create_table()?;
        let clipboard = lua.create_table()?;
        let ask = |op: &'static str, fields: fn(&Lua, mlua::MultiValue) -> mlua::Result<Value>| {
            let h = Arc::clone(&services.host);
            lua.create_function(move |lua, args: mlua::MultiValue| {
                let mut request = fields(lua, args)?;
                if let Value::Object(map) = &mut request {
                    map.insert("op".to_owned(), Value::String(op.to_owned()));
                }
                let out = futures::executor::block_on(h.host_call(request.to_string()))
                    .map_err(|e| mlua::Error::runtime(e.to_string()))?;
                match serde_json::from_str::<Value>(&out).unwrap_or(Value::Null) {
                    Value::Null => Ok(LuaValue::Nil),
                    v => lua.to_value(&v),
                }
            })
        };
        let table = |lua: &Lua, args: mlua::MultiValue| -> mlua::Result<Value> {
            match args.into_iter().next() {
                None | Some(LuaValue::Nil) => Ok(Value::Object(serde_json::Map::new())),
                Some(v) => lua.from_value(v),
            }
        };
        ui.set("pick", ask("pick", table)?)?;
        ui.set("prompt", ask("prompt", table)?)?;
        ui.set(
            "menu",
            ask("menu", |lua, args| {
                let mut it = args.into_iter();
                let items: Value = it.next().map_or(Ok(Value::Null), |v| lua.from_value(v))?;
                let mut out = it
                    .next()
                    .map_or(Ok(Value::Object(serde_json::Map::new())), |v| {
                        lua.from_value(v)
                    })?;
                if let Value::Object(map) = &mut out {
                    map.insert("items".to_owned(), items);
                }
                Ok(out)
            })?,
        )?;
        ui.set(
            "decorate",
            ask("decorate", |lua, args| {
                let mut it = args.into_iter();
                let target: Value = it.next().map_or(Ok(Value::Null), |v| lua.from_value(v))?;
                let decoration: Value = it.next().map_or(Ok(Value::Null), |v| lua.from_value(v))?;
                Ok(serde_json::json!({ "target": target, "decoration": decoration }))
            })?,
        )?;
        ui.set(
            "call",
            ask("", |lua, args| {
                args.into_iter()
                    .next()
                    .map_or(Ok(Value::Null), |v| lua.from_value(v))
            })?,
        )?;
        clipboard.set(
            "read",
            ask("clipboard_read", |_, _| {
                Ok(Value::Object(serde_json::Map::new()))
            })?,
        )?;
        clipboard.set(
            "write",
            ask("clipboard_write", |lua, args| {
                let text: String = args
                    .into_iter()
                    .next()
                    .map_or(Ok(String::new()), |v| lua.from_value(v))?;
                Ok(serde_json::json!({ "text": text }))
            })?,
        )?;
        // Hooks made at run time, as the page's extension.
        let events = lua.create_table()?;
        events.set("subscribe", ask("subscribe", table)?)?;
        events.set(
            "unsubscribe",
            ask("unsubscribe", |lua, args| {
                let key: String = args
                    .into_iter()
                    .next()
                    .map_or(Ok(String::new()), |v| lua.from_value(v))?;
                Ok(serde_json::json!({ "key": key }))
            })?,
        )?;
        events.set(
            "list",
            ask("events", |_, _| Ok(Value::Object(serde_json::Map::new())))?,
        )?;
        // The app's documents, as the page's extension.
        let documents = lua.create_table()?;
        documents.set(
            "list",
            ask("list_documents", |lua, args| {
                let under: Option<String> = args
                    .into_iter()
                    .next()
                    .map_or(Ok(None), |v| lua.from_value(v))?;
                Ok(serde_json::json!({ "under": under }))
            })?,
        )?;
        documents.set(
            "read",
            ask("read_document", |lua, args| {
                let path: String = args
                    .into_iter()
                    .next()
                    .map_or(Ok(String::new()), |v| lua.from_value(v))?;
                Ok(serde_json::json!({ "path": path }))
            })?,
        )?;
        documents.set(
            "create",
            ask("create_document", |lua, args| {
                let mut it = args.into_iter();
                let path: String = it.next().map_or(Ok(String::new()), |v| lua.from_value(v))?;
                let text: Option<String> = it.next().map_or(Ok(None), |v| lua.from_value(v))?;
                Ok(serde_json::json!({ "path": path, "text": text.unwrap_or_default() }))
            })?,
        )?;
        documents.set(
            "edit",
            ask("edit_document", |lua, args| {
                let mut it = args.into_iter();
                let path: String = it.next().map_or(Ok(String::new()), |v| lua.from_value(v))?;
                let edits: Value = it.next().map_or(Ok(Value::Null), |v| lua.from_value(v))?;
                let name: Option<String> = it.next().map_or(Ok(None), |v| lua.from_value(v))?;
                Ok(serde_json::json!({ "path": path, "edits": edits, "name": name }))
            })?,
        )?;
        documents.set(
            "rename",
            ask("rename_document", |lua, args| {
                let mut it = args.into_iter();
                let path: String = it.next().map_or(Ok(String::new()), |v| lua.from_value(v))?;
                let to: String = it.next().map_or(Ok(String::new()), |v| lua.from_value(v))?;
                Ok(serde_json::json!({ "path": path, "to": to }))
            })?,
        )?;
        documents.set(
            "move",
            ask("move_document", |lua, args| {
                let mut it = args.into_iter();
                let path: String = it.next().map_or(Ok(String::new()), |v| lua.from_value(v))?;
                let folder: String = it.next().map_or(Ok(String::new()), |v| lua.from_value(v))?;
                Ok(serde_json::json!({ "path": path, "folder": folder }))
            })?,
        )?;
        documents.set(
            "delete",
            ask("delete_document", |lua, args| {
                let path: String = args
                    .into_iter()
                    .next()
                    .map_or(Ok(String::new()), |v| lua.from_value(v))?;
                Ok(serde_json::json!({ "path": path }))
            })?,
        )?;
        documents.set("watch", ask("watch_documents", table)?)?;
        api.set("documents", documents)?;
        // The network, as the page's extension (capabilities.hosts).
        let net = lua.create_table()?;
        net.set(
            "fetch",
            ask("fetch", |lua, args| {
                let mut it = args.into_iter();
                let url: String = it.next().map_or(Ok(String::new()), |v| lua.from_value(v))?;
                let mut options: Value =
                    it.next().map_or(Ok(Value::Null), |v| lua.from_value(v))?;
                if !options.is_object() {
                    options = Value::Object(serde_json::Map::new());
                }
                if let Value::Object(map) = &mut options {
                    map.insert("url".to_owned(), Value::String(url));
                }
                Ok(options)
            })?,
        )?;
        net.set(
            "read",
            ask("read_body", |lua, args| {
                let mut it = args.into_iter();
                let stream: String = it.next().map_or(Ok(String::new()), |v| lua.from_value(v))?;
                let max: Option<u64> = it.next().map_or(Ok(None), |v| lua.from_value(v))?;
                Ok(serde_json::json!({ "stream": stream, "max": max }))
            })?,
        )?;
        net.set(
            "close",
            ask("close_body", |lua, args| {
                let stream: String = args
                    .into_iter()
                    .next()
                    .map_or(Ok(String::new()), |v| lua.from_value(v))?;
                Ok(serde_json::json!({ "stream": stream }))
            })?,
        )?;
        for (name, op) in [("secret", "secret"), ("ask_secret", "ask_secret")] {
            net.set(
                name,
                ask(op, |lua, args| {
                    let name: String = args
                        .into_iter()
                        .next()
                        .map_or(Ok(String::new()), |v| lua.from_value(v))?;
                    Ok(serde_json::json!({ "name": name }))
                })?,
            )?;
        }
        api.set("net", net)?;
        api.set("ui", ui)?;
        api.set("clipboard", clipboard)?;
        api.set("events", events)?;
        let (log, ext) = (services.log.clone(), services.extension.clone());
        api.set(
            "print",
            lua.create_function(move |_, values: mlua::MultiValue| {
                let line: Vec<String> = values
                    .iter()
                    .map(|v| v.to_string().unwrap_or_default())
                    .collect();
                log.line(&ext, "info", &line.join("\t"));
                Ok(())
            })?,
        )?;
        // The theme: read through, so a frozen global follows a change.
        let meta = lua.create_table()?;
        let now = Arc::clone(theme);
        let value = {
            let now = Arc::clone(theme);
            lua.create_function(move |_, name: String| {
                let t = now.lock().unwrap_or_else(PoisonError::into_inner);
                Ok(t.tokens
                    .iter()
                    .find(|tok| tok.name == name)
                    .map(|tok| tok.value.clone()))
            })?
        };
        meta.set(
            "__index",
            lua.create_function(move |lua, (_, key): (LuaValue, String)| {
                let t = now.lock().unwrap_or_else(PoisonError::into_inner);
                match key.as_str() {
                    "scheme" => lua.to_value(&t.scheme),
                    "id" => lua.to_value(&t.id),
                    "revision" => lua.to_value(&t.revision),
                    "value" => Ok(LuaValue::Function(value.clone())),
                    _ => Ok(LuaValue::Nil),
                }
            })?,
        )?;
        let theme_table = lua.create_table()?;
        theme_table.set_metatable(Some(meta))?;
        api.set("theme", theme_table)?;
        Ok((api, page))
    }

    /// Calls `page[name](args)` within a frame's budget.
    fn call<A: mlua::IntoLuaMulti, R: mlua::FromLuaMulti>(
        &self,
        name: &str,
        args: A,
    ) -> Result<R, String> {
        self.guard.reset();
        self.guard.deadline(Some(self.services.limits.frame));
        let out = self
            .page
            .get::<mlua::Function>(name)
            .and_then(|f| f.call::<R>(args))
            .map_err(|e| err(&e));
        self.guard.deadline(None);
        out
    }
}

impl Engine for LuauEngine {
    fn render(&mut self) -> Result<Vec<Element>, String> {
        let tree: LuaValue = self.call("render", ())?;
        self.lua
            .from_value(tree)
            .map_err(|e| format!("what it rendered is not a tree: {e}"))
    }

    fn event(&mut self, event: PageEvent) -> Result<(), String> {
        let e = self.lua.to_value(&event).map_err(|e| err(&e))?;
        self.call("event", (event.handler, e))
    }

    fn answer(&mut self, _call: u32, _result: Result<String, String>) -> Result<(), String> {
        Ok(())
    }

    fn shared(&mut self, event: &DataEvent) -> Result<(), String> {
        if event.topic != crate::runner::EMBED_TOPIC {
            return Ok(());
        }
        // A new input is a new frame.
        self.lua
            .globals()
            .get::<mlua::Function>("invalidate")
            .and_then(|f| f.call::<()>(()))
            .map_err(|e| err(&e))
    }

    fn worker(&mut self, _message: &WorkerMessage) -> Result<(), String> {
        // A Luau page's workers tell it through its wake (`tick`).
        Ok(())
    }

    fn theme(&mut self, theme: &ThemeSnapshot) -> Result<(), String> {
        *self.theme.lock().unwrap_or_else(PoisonError::into_inner) = theme.clone();
        // A new theme is a new frame.
        self.lua
            .globals()
            .get::<mlua::Function>("invalidate")
            .and_then(|f| f.call::<()>(()))
            .map_err(|e| err(&e))
    }

    fn tick(&mut self) -> Result<Option<Duration>, String> {
        // Its workers' news first: their callbacks, then a frame.
        let mut cx = Context::from_waker(&self.waker);
        let came = std::pin::pin!(self.news.woken()).poll(&mut cx).is_ready();
        if came {
            self.guard.reset();
            self.guard.deadline(Some(self.services.limits.frame));
            let delivered = worker::deliver(&self.lua).map_err(|e| err(&e));
            self.guard.deadline(None);
            delivered?;
            self.lua
                .globals()
                .get::<mlua::Function>("invalidate")
                .and_then(|f| f.call::<()>(()))
                .map_err(|e| err(&e))?;
        }
        let ms: f64 = self.call("tick", ())?;
        Ok((ms >= 0.0).then(|| Duration::from_secs_f64(ms / 1000.0)))
    }

    fn dirty(&self) -> bool {
        self.page
            .get::<mlua::Function>("dirty")
            .and_then(|f| f.call::<bool>(()))
            .unwrap_or(false)
    }
}
