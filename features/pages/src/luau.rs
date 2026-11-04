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
//! `theme.value(name)`, `theme.scheme`, `theme.revision`, `notify(text)`
//! and `print`. Luau's sandbox holds; each call into the page has
//! [`crate::runner::PageLimits::frame`] before its interrupt stops it.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use architect_scripting::luau::mlua::{self, Lua, LuaSerdeExt as _, Value as LuaValue};
use architect_scripting::vm::{Guard, Stop};
use serde_json::Value;

use crate::api::{
    DataEvent, Element, Notice, PageEvent, PageHost as _, ThemeSnapshot, WorkerMessage,
};
use crate::files::PageFile;
use crate::runner::{Engine, Load, Services};

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
        let (api, table) = Self::install(&lua, services, &theme).map_err(|e| err(&e))?;
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
        Ok(Self {
            lua,
            guard,
            page: table,
            services: services.clone(),
            theme,
        })
    }

    /// The host's globals and the builder: what the prelude returns.
    fn install(
        lua: &Lua,
        services: &Services,
        theme: &Arc<Mutex<ThemeSnapshot>>,
    ) -> mlua::Result<(mlua::Table, mlua::Table)> {
        let host = lua.create_table()?;
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

    fn shared(&mut self, _event: &DataEvent) -> Result<(), String> {
        Ok(())
    }

    fn worker(&mut self, _message: &WorkerMessage) -> Result<(), String> {
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
        Ok(None)
    }

    fn dirty(&self) -> bool {
        self.page
            .get::<mlua::Function>("dirty")
            .and_then(|f| f.call::<bool>(()))
            .unwrap_or(false)
    }
}
