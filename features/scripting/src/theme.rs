//! The app's theme for Luau scripts (feature `luau`).
//!
//! A [`Module`] that gives a script `theme`: a frozen snapshot of the
//! theme showing, keyed by its revision and made again only when the
//! revision moves (herdr's `__theme_revision`).
//!
//! | `theme.` | gives |
//! |---|---|
//! | `color(name)` | the token as Dear `ImGui`'s colour integer, `0xRRGGBBAA`; magenta for a name the theme does not have |
//! | `value(name)` | its CSS text, or `nil` |
//! | `number(name)` | a plain number (`0.42`, `12`), or `nil` |
//! | `scheme`, `id`, `revision` | `"light"` or `"dark"`, the theme's id, its revision |
//! | `snapshot()` | the whole theme as a read-only table `{id, scheme, revision, tokens}`: the same table until the theme changes |
//! | `subscribe(fn)` | a live script: `fn(snapshot)` at the start of the first frame after each change |
//!
//! What a run reads is the theme as it was when its state was made, and a
//! live script's at the start of each frame ([`Module::frame`]): one frame
//! never sees two themes. A script that draws (an `ImGui` window) reads
//! its colours every frame and so follows a switch by itself; one that
//! keeps colours it worked out compares `theme.revision`, or subscribes.

use std::sync::{Arc, Mutex, PoisonError};

use architect_theme::{Resolved, Rgba, ThemeStore, color::MAGENTA};
use mlua::{Lua, Value as LuaValue};

use crate::luau::Module;

/// Where [`ThemeModule`] reads the theme: the app's store, or a theme a
/// host hands over ([`ThemeCell`]).
pub trait ThemeSource: Send + Sync {
    /// A number that moves whenever [`Self::current`] changes; cheap,
    /// read every frame.
    fn revision(&self) -> u64;

    /// The theme now.
    fn current(&self) -> Resolved;
}

impl ThemeSource for ThemeStore {
    fn revision(&self) -> u64 {
        Self::revision(self)
    }

    fn current(&self) -> Resolved {
        Self::current(self)
    }
}

/// A theme a host sets as it changes (a canvas session's, a process's):
/// a [`ThemeSource`] without a store. Cheap to clone; clones share it.
#[derive(Clone, Debug, Default)]
pub struct ThemeCell(Arc<Mutex<(u64, Resolved)>>);

impl ThemeCell {
    #[must_use]
    pub fn new(theme: Resolved) -> Self {
        Self(Arc::new(Mutex::new((0, theme))))
    }

    /// Sets the theme; false (and the revision kept) when it is the one
    /// there already.
    pub fn set(&self, theme: &Resolved) -> bool {
        let mut cell = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if cell.1 == *theme && cell.1.revision() == theme.revision() {
            return false;
        }
        cell.0 = cell.0.wrapping_add(1);
        cell.1 = theme.clone();
        true
    }
}

impl ThemeSource for ThemeCell {
    fn revision(&self) -> u64 {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).0
    }

    fn current(&self) -> Resolved {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .1
            .clone()
    }
}

/// The snapshot, its key and the subscribers, in each state's registry.
const SNAPSHOT: &str = "architect.theme.snapshot";
const KEY: &str = "architect.theme.key";
const SUBSCRIBERS: &str = "architect.theme.subscribers";

/// `theme`, over a [`ThemeSource`].
#[derive(Clone)]
pub struct ThemeModule {
    source: Arc<dyn ThemeSource>,
}

impl std::fmt::Debug for ThemeModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThemeModule")
            .field("revision", &self.source.revision())
            .finish()
    }
}

impl ThemeModule {
    /// Scripts read `source`'s theme: the app's [`ThemeStore`], or a
    /// [`ThemeCell`].
    #[must_use]
    pub fn new(source: impl ThemeSource + 'static) -> Self {
        Self {
            source: Arc::new(source),
        }
    }

    /// Makes the snapshot again when the source moved; whether it did.
    fn refresh(&self, lua: &Lua) -> mlua::Result<bool> {
        let key = self.source.revision();
        let have: Option<u64> = lua.named_registry_value(KEY)?;
        if have == Some(key) {
            return Ok(false);
        }
        let snapshot = snapshot(lua, &self.source.current())?;
        lua.set_named_registry_value(SNAPSHOT, snapshot)?;
        lua.set_named_registry_value(KEY, key)?;
        Ok(have.is_some())
    }
}

/// The theme as a read-only table: `{id, scheme, revision, tokens}`.
fn snapshot(lua: &Lua, theme: &Resolved) -> mlua::Result<mlua::Table> {
    let tokens = lua.create_table()?;
    for (name, value) in theme.tokens() {
        tokens.set(name.as_str(), value.as_str())?;
    }
    tokens.set_readonly(true);
    let t = lua.create_table()?;
    t.set("id", theme.id())?;
    t.set("scheme", theme.scheme().as_str())?;
    t.set("revision", theme.revision())?;
    t.set("tokens", tokens)?;
    t.set_readonly(true);
    Ok(t)
}

/// Token `name`'s text in the snapshot.
fn token(lua: &Lua, name: &str) -> mlua::Result<Option<String>> {
    let snapshot: mlua::Table = lua.named_registry_value(SNAPSHOT)?;
    let tokens: mlua::Table = snapshot.get("tokens")?;
    tokens.get(name)
}

impl Module for ThemeModule {
    fn global(&self) -> &'static str {
        "theme"
    }

    fn path(&self) -> &'static str {
        "theme"
    }

    fn live_fields(&self) -> bool {
        true
    }

    fn install(&self, lua: &Lua) -> mlua::Result<LuaValue> {
        lua.set_named_registry_value(SUBSCRIBERS, lua.create_table()?)?;
        self.refresh(lua)?;
        let t = lua.create_table()?;
        t.set(
            "color",
            lua.create_function(|lua, name: String| {
                let colour = token(lua, &name)?
                    .and_then(|v| Rgba::parse(&v))
                    .unwrap_or(MAGENTA);
                Ok(colour.packed())
            })?,
        )?;
        t.set(
            "value",
            lua.create_function(|lua, name: String| token(lua, &name))?,
        )?;
        t.set(
            "number",
            lua.create_function(|lua, name: String| {
                Ok(token(lua, &name)?
                    .and_then(|v| v.trim().parse::<f64>().ok())
                    .filter(|n| n.is_finite()))
            })?,
        )?;
        t.set(
            "snapshot",
            lua.create_function(|lua, (): ()| lua.named_registry_value::<mlua::Table>(SNAPSHOT))?,
        )?;
        t.set(
            "subscribe",
            lua.create_function(|lua, f: mlua::Function| {
                let subscribers: mlua::Table = lua.named_registry_value(SUBSCRIBERS)?;
                subscribers.push(f)
            })?,
        )?;
        // `theme.scheme`, `theme.id`, `theme.revision`: the snapshot's.
        let meta = lua.create_table()?;
        meta.set(
            "__index",
            lua.create_function(|lua, (_, key): (LuaValue, String)| {
                let snapshot: mlua::Table = lua.named_registry_value(SNAPSHOT)?;
                match key.as_str() {
                    "scheme" | "id" | "revision" => snapshot.get::<LuaValue>(key),
                    _ => Ok(LuaValue::Nil),
                }
            })?,
        )?;
        t.set_metatable(Some(meta))?;
        Ok(LuaValue::Table(t))
    }

    fn frame(&self, lua: &Lua) -> mlua::Result<()> {
        if !self.refresh(lua)? {
            return Ok(());
        }
        let snapshot: mlua::Table = lua.named_registry_value(SNAPSHOT)?;
        let subscribers: mlua::Table = lua.named_registry_value(SUBSCRIBERS)?;
        for f in subscribers.sequence_values::<mlua::Function>() {
            f?.call::<()>(snapshot.clone())?;
        }
        Ok(())
    }
}
