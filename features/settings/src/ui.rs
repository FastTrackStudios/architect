//! The settings page (feature `ui`), T3 Code's: on a desk a sheet over
//! the window — the sections, grouped, with search, on the left; on the
//! right the section's title, "Restore defaults", and its settings in
//! cards of rows — and on a phone the grouped list, a section opening as
//! its own page. Every row: the title, what it does, the control; a dot
//! and a reset arrow when it is not the default.
//!
//! "Settings…" (⌘,) is a command, so the palette, the menu bar and a
//! model open it too. A search hit opens its section, scrolls to its row
//! and pulses it. `use_settings::<T>()` is a derived section's typed
//! value.

// The `rsx!` loops iterate rows collected so a signal's read is let go
// before drawing.
#![allow(clippy::needless_collect)]

use architect_shell::{ShellTheme, try_use_shell};
use dioxus::prelude::*;
use lucide_dioxus::{
    AudioLines, Bell, ChevronDown, ChevronLeft, ChevronRight, Keyboard, Minus, Monitor, Palette,
    PanelBottom, Plus, Puzzle, RotateCcw, Search, Settings2, Shield, Timer, Type, User, X,
};

use crate::derive::Settings;
use crate::model::{Choice, Control, Owner, Scope, Section, Setting, Value};
use crate::registry::Registry;
use crate::values::{OrgSettings, ScopedValues, Values};

/// The settings, in context: the registry (the app's sections and every
/// loaded extension's), the values by scope, and the page.
#[derive(Clone, Copy)]
pub struct SettingsHandle {
    pub registry: Signal<Registry>,
    pub values: Signal<ScopedValues>,
    /// The section on show; `None` when the page is closed.
    pub open: Signal<Option<String>>,
    /// The setting a search hit went to, and how many times one has (so
    /// the same one pulses again).
    pub focus: Signal<Option<(String, u32)>>,
    on_change: CopyValue<Option<Callback<SettingsChange>>>,
}

/// What changed, for the store of its scope to keep.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsChange {
    pub scope: Scope,
    /// Everything that scope now holds.
    pub values: Values,
    /// The keys that changed (a synced store writes only these).
    pub keys: Vec<String>,
}

/// Puts the settings in context. `on_change` keeps the values: each
/// change goes to the store of its scope.
pub fn use_settings_provider(
    registry: impl FnOnce() -> Registry,
    values: impl FnOnce() -> ScopedValues,
    on_change: Option<Callback<SettingsChange>>,
) -> SettingsHandle {
    use_context_provider(|| SettingsHandle::new(registry(), values(), on_change))
}

/// A derived section's typed value: `value` redraws what reads it;
/// `update` changes it, storing each field that changed in its scope.
pub struct UseSettings<T: 'static> {
    pub handle: SettingsHandle,
    pub value: Memo<T>,
}

impl<T> Clone for UseSettings<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for UseSettings<T> {}

impl<T: Settings + PartialEq> UseSettings<T> {
    /// The value now (and redraws when it changes).
    #[must_use]
    pub fn read(&self) -> T {
        self.value.read().clone()
    }

    /// Changes the value: `update(|a| a.buffer = 256)`. A field the
    /// org has locked, or a value its setting refuses, is let go.
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        let before = self.value.peek().clone();
        let mut after = before.clone();
        f(&mut after);
        let old = before.entries();
        for (key, v) in after.entries() {
            if !old.iter().any(|(k, o)| *k == key && *o == v) {
                self.handle.set(&key, v);
            }
        }
    }
}

/// Section `T`'s value, typed: `use_settings::<Audio>()`. Registers the
/// section when it is not yet. Under no settings provider the value is
/// this component's own (defaults, kept nowhere).
#[must_use]
pub fn use_settings<T: Settings + PartialEq>() -> UseSettings<T> {
    let outer = try_use_settings();
    let handle = use_hook(move || {
        let h = outer
            .unwrap_or_else(|| SettingsHandle::new(Registry::new(), ScopedValues::new(), None));
        if h.registry.peek().section(T::id()).is_none() {
            let mut registry = h.registry;
            let _ = registry.write().register(T::section());
        }
        h
    });
    let value = use_memo(move || T::read(&handle.registry.read(), &handle.values.read()));
    UseSettings { handle, value }
}

/// The settings above, if any.
#[must_use]
pub fn try_use_settings() -> Option<SettingsHandle> {
    try_use_context()
}

/// Setting `key`'s value (its default when not set), redrawn when it
/// changes.
#[must_use]
pub fn use_setting(key: &str) -> Option<Value> {
    let s = try_use_settings()?;
    let registry = s.registry.read();
    s.values.read().get(&registry, key)
}

/// The setting a search hit is on: the hit's key and how many hits
/// there have been.
fn focused(h: SettingsHandle) -> Option<(String, u32)> {
    (h.focus)()
}

/// Registers `section` while the calling component is mounted (an
/// extension's settings, there while it is loaded).
pub fn use_section(make: impl FnOnce() -> Section) {
    let handle = try_use_settings();
    let owner = use_hook(move || {
        let h = handle?;
        let section = make();
        let id = section.id.clone();
        let mut registry = h.registry;
        let _ = registry.write().register(section);
        Some(id)
    });
    use_drop(move || {
        if let (Some(h), Some(id)) = (handle, owner) {
            let mut registry = h.registry;
            registry.write().remove(&id);
        }
    });
}

impl SettingsHandle {
    /// A handle of its own (not in context): `use_settings_provider`
    /// is the usual way.
    #[must_use]
    pub fn new(
        registry: Registry,
        values: ScopedValues,
        on_change: Option<Callback<SettingsChange>>,
    ) -> Self {
        Self {
            registry: Signal::new(registry),
            values: Signal::new(values),
            open: Signal::new(None),
            focus: Signal::new(None),
            on_change: CopyValue::new(on_change),
        }
    }

    /// Sets `key`; a value it does not take, or a locked setting, is let
    /// go.
    pub fn set(&self, key: &str, value: Value) {
        let mut values = self.values;
        let scope = {
            let registry = self.registry.peek();
            values.write().set(&registry, key, value).ok()
        };
        if let Some(scope) = scope {
            self.changed(scope, vec![key.to_owned()]);
        }
    }

    pub fn reset(&self, key: &str) {
        let mut values = self.values;
        let scope = {
            let registry = self.registry.peek();
            values.write().reset(&registry, key)
        };
        if let Some(scope) = scope {
            self.changed(scope, vec![key.to_owned()]);
        }
    }

    /// Puts every setting of section `id` back; how many.
    #[must_use = "how many settings were put back"]
    pub fn restore(&self, id: &str) -> usize {
        let mut values = self.values;
        let changed: Vec<(Scope, String)> = {
            let registry = self.registry.peek();
            let scoped = values.peek();
            scoped
                .changed_in(&registry, id)
                .into_iter()
                .map(|s| (s.scope, s.key.clone()))
                .collect()
        };
        {
            let registry = self.registry.peek();
            let _ = values.write().restore_section(&registry, id);
        }
        for scope in [Scope::Device, Scope::Synced, Scope::Project] {
            let keys: Vec<String> = changed
                .iter()
                .filter(|(s, _)| *s == scope)
                .map(|(_, k)| k.clone())
                .collect();
            if !keys.is_empty() {
                self.changed(scope, keys);
            }
        }
        changed.len()
    }

    fn changed(&self, scope: Scope, keys: Vec<String>) {
        if let Some(cb) = *self.on_change.peek() {
            cb.call(SettingsChange {
                scope,
                values: self.values.peek().of(scope).clone(),
                keys,
            });
        }
    }

    /// Puts in `scope`'s values as its store has them now (another device
    /// changed them). Not a change to keep: `on_change` is not called.
    pub fn replace(&self, scope: Scope, values: Values) {
        let mut v = self.values;
        v.write().replace(scope, values);
    }

    /// The current org's defaults and locks (the org switched, or an
    /// admin changed them).
    pub fn set_org(&self, org: OrgSettings) {
        let mut v = self.values;
        v.write().org = org;
    }

    /// Opens the page at setting `key`'s section, scrolled to its row,
    /// which pulses.
    pub fn show_setting(&self, key: &str) {
        let section = self.registry.peek().setting(key).map(|(s, _)| s.id.clone());
        let Some(section) = section else { return };
        let n = self
            .focus
            .peek()
            .as_ref()
            .map_or(0, |(_, n)| n.wrapping_add(1));
        let mut focus = self.focus;
        focus.set(Some((key.to_owned(), n)));
        let mut open = self.open;
        open.set(Some(section));
    }

    /// Opens the page at section `id` (the first when `None`).
    pub fn show(&self, id: Option<&str>) {
        let first = self
            .registry
            .peek()
            .sections()
            .first()
            .map(|s| s.id.clone());
        let mut open = self.open;
        open.set(id.map(str::to_owned).or(first));
    }

    pub fn close(&self) {
        let mut open = self.open;
        open.set(None);
        let mut focus = self.focus;
        focus.set(None);
    }

    /// Removes every section an extension registered (it unloaded).
    pub fn remove_extension(&self, id: &str) {
        let mut registry = self.registry;
        registry
            .write()
            .remove_owner(&Owner::Extension(id.to_owned()));
    }
}

fn theme() -> ShellTheme {
    try_use_shell().map(|s| s.theme).unwrap_or_default()
}

fn phone() -> bool {
    try_use_shell().is_some_and(|s| s.form() == architect_shell::FormFactor::Phone)
}

/// A section's icon, by its lucide name.
fn icon(name: &str, size: usize, color: &'static str) -> Element {
    match name {
        "palette" => rsx! { Palette { size, color } },
        "keyboard" => rsx! { Keyboard { size, color } },
        "audio-lines" => rsx! { AudioLines { size, color } },
        "bell" => rsx! { Bell { size, color } },
        "user" => rsx! { User { size, color } },
        "shield" => rsx! { Shield { size, color } },
        "puzzle" => rsx! { Puzzle { size, color } },
        "monitor" => rsx! { Monitor { size, color } },
        "type" => rsx! { Type { size, color } },
        "timer" => rsx! { Timer { size, color } },
        "panel-bottom" => rsx! { PanelBottom { size, color } },
        _ => rsx! { Settings2 { size, color } },
    }
}

/// The page, over the window while a section is open. Mount it once,
/// inside the app.
#[component]
pub fn SettingsPage() -> Element {
    let Some(h) = try_use_settings() else {
        return rsx! {};
    };
    // "Settings…" (⌘,): in the palette and the menu bar.
    architect_commands::use_command(
        architect_commands::Command::new("APP_SETTINGS", "Settings…")
            .group("App")
            .shortcut("⌘,"),
        move || h.show(None),
    );
    // The palette's `#`: a setting by name, opened at its row.
    architect_commands::use_palette_opener(architect_commands::Scope::Settings, move || {
        architect_commands::Opener::new(
            "setting",
            "Settings",
            move |query| {
                let registry = h.registry.peek();
                registry
                    .search(query)
                    .into_iter()
                    .take(20)
                    .filter_map(|hit| {
                        let (section, setting) = registry.setting(&hit.key)?;
                        Some(
                            architect_commands::OpenItem::new(
                                "setting",
                                hit.key.clone(),
                                setting.title.clone(),
                            )
                            .detail(section.title.clone()),
                        )
                    })
                    .collect()
            },
            move |item, _| h.show_setting(&item.id),
        )
    });
    if (h.open)().is_none() {
        return rsx! {};
    }
    if phone() {
        rsx! { PhoneSettings {} }
    } else {
        rsx! { DeskSettings {} }
    }
}

#[component]
fn DeskSettings() -> Element {
    let Some(h) = try_use_settings() else {
        return rsx! {};
    };
    let t = theme();
    let mut query = use_signal(String::new);
    let current = (h.open)().unwrap_or_default();
    let registry = h.registry.read();
    let groups: Vec<(Option<String>, Vec<(String, String, String)>)> = registry
        .grouped()
        .into_iter()
        .map(|(g, list)| {
            (
                g,
                list.into_iter()
                    .map(|s| (s.id.clone(), s.title.clone(), s.icon.clone()))
                    .collect(),
            )
        })
        .collect();
    let hits = registry.search(&query.read());
    let section = registry.section(&current).cloned();
    drop(registry);
    rsx! {
        div {
            style: "position: absolute; top: 0; left: 0; right: 0; bottom: 0; z-index: {architect_shell::layer::POPOVER}; \
                    background: {t.scrim}; display: flex; align-items: center; justify-content: center;",
            onclick: move |_| h.close(),
            div {
                style: "width: 92%; max-width: 1100px; height: 86%; max-height: 760px; display: flex; overflow: hidden; \
                        border-radius: 14px; background: {t.desk}; border: 1px solid {t.rule}; box-shadow: {t.shadow_float}; \
                        font-family: {t.font}; color: {t.ink};",
                onclick: move |e| e.stop_propagation(),
                // The sections.
                div { style: "flex: 0 0 248px; display: flex; flex-direction: column; background: {t.sheet}; \
                              border-right: 1px solid {t.rule}; padding: 14px 10px; gap: 2px; overflow-y: scroll;",
                    div { style: "display: flex; align-items: center; gap: 8px; height: 32px; padding: 0 10px; margin-bottom: 10px; \
                                  border-radius: 8px; background: {t.desk}; border: 1px solid {t.rule};",
                        Search { size: 14, color: t.ink_3 }
                        // Blitz has no placeholder: the hint sits under an empty field.
                        if query.read().is_empty() {
                            span { style: "position: absolute; margin-left: 24px; color: {t.ink_3}; font-size: 13px; pointer-events: none;",
                                "Search settings"
                            }
                        }
                        input {
                            r#type: "text",
                            value: "{query}",
                            "aria-label": "Search settings",
                            style: "flex: 1; min-width: 0; border: none; outline: none; background: transparent; \
                                    color: {t.ink}; font-size: 13px; font-family: {t.font};",
                            oninput: move |e| query.set(e.value()),
                        }
                    }
                    if query.read().trim().is_empty() {
                        for (gi, (group, list)) in groups.into_iter().enumerate() {
                            if let Some(g) = group {
                                div { key: "g{gi}", style: "padding: 14px 10px 6px; color: {t.ink_3}; font-size: 11px; font-weight: 600;", "{g}" }
                            }
                            for (id, title, ic) in list {
                                SectionLink { key: "{id}", on: id == current, id, title, ic }
                            }
                        }
                    } else {
                        SearchResults { hits }
                    }
                }
                // The section.
                div { style: "flex: 1; min-width: 0; display: flex; flex-direction: column;",
                    div { style: "flex: 0 0 52px; display: flex; align-items: center; gap: 8px; padding: 0 16px 0 28px; \
                                  border-bottom: 1px solid {t.rule}; font-size: 13px;",
                        span { style: "color: {t.ink_3};", "Settings" }
                        ChevronRight { size: 14, color: t.ink_3 }
                        span { style: "font-weight: 600;", "{section.as_ref().map(|s| s.title.clone()).unwrap_or_default()}" }
                        span { style: "flex: 1;" }
                        if let Some(s) = section.clone() {
                            RestoreDefaults { id: s.id }
                        }
                        button {
                            style: "width: 28px; height: 28px; border: none; border-radius: 6px; background: transparent; \
                                    display: flex; align-items: center; justify-content: center; cursor: pointer;",
                            "aria-label": "Close settings",
                            onclick: move |_| h.close(),
                            X { size: 16, color: t.ink_2 }
                        }
                    }
                    div { style: "flex: 1; min-height: 0; overflow-y: scroll; padding: 24px 28px 40px;",
                        if let Some(s) = section {
                            SectionBody { section: s }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SectionLink(on: bool, id: String, title: String, ic: String) -> Element {
    let Some(h) = try_use_settings() else {
        return rsx! {};
    };
    let t = theme();
    let (fill, ink) = if on {
        (t.accent_soft, t.ink)
    } else {
        ("transparent", t.ink_2)
    };
    let icon_ink = if on { t.accent } else { t.ink_3 };
    rsx! {
        button {
            style: "height: 32px; display: flex; align-items: center; justify-content: flex-start; gap: 10px; padding: 0 10px; \
                    border: none; border-radius: 7px; background: {fill}; color: {ink}; font-size: 13px; font-weight: 500; \
                    text-align: left; cursor: pointer;",
            onclick: move |_| h.show(Some(&id)),
            {icon(&ic, 15, icon_ink)}
            span { "{title}" }
        }
    }
}

#[component]
fn SearchResults(hits: Vec<crate::registry::Hit>) -> Element {
    let Some(h) = try_use_settings() else {
        return rsx! {};
    };
    let t = theme();
    let registry = h.registry.read();
    let rows: Vec<(String, String, String)> = hits
        .iter()
        .take(20)
        .filter_map(|hit| {
            registry
                .setting(&hit.key)
                .map(|(s, set)| (set.key.clone(), set.title.clone(), s.title.clone()))
        })
        .collect();
    drop(registry);
    rsx! {
        if rows.is_empty() {
            div { style: "padding: 10px; color: {t.ink_3}; font-size: 12px;", "No setting matches" }
        }
        for (i, (key, title, section)) in rows.into_iter().enumerate() {
            button {
                key: "{i}",
                style: "display: flex; flex-direction: column; align-items: flex-start; justify-content: flex-start; gap: 1px; \
                        padding: 6px 10px; border: none; border-radius: 7px; background: transparent; text-align: left; cursor: pointer;",
                onclick: move |_| h.show_setting(&key),
                span { style: "color: {t.ink}; font-size: 13px;", "{title}" }
                span { style: "color: {t.ink_3}; font-size: 11px;", "{section}" }
            }
        }
    }
}

/// "Restore defaults": lists what it will reset, asks, resets.
#[component]
fn RestoreDefaults(id: String) -> Element {
    let Some(h) = try_use_settings() else {
        return rsx! {};
    };
    let t = theme();
    let mut asking = use_signal(|| false);
    let changed: Vec<String> = {
        let registry = h.registry.read();
        h.values
            .read()
            .changed_in(&registry, &id)
            .into_iter()
            .map(|s| s.title.clone())
            .collect()
    };
    if changed.is_empty() {
        return rsx! {};
    }
    let n = changed.len();
    rsx! {
        div { style: "position: relative;",
            button {
                style: "height: 28px; padding: 0 12px; border-radius: 7px; border: 1px solid {t.rule}; background: transparent; \
                        color: {t.ink_2}; font-size: 12px; cursor: pointer;",
                onclick: move |_| asking.toggle(),
                "Restore defaults"
            }
            if asking() {
                div {
                    style: "position: absolute; right: 0; top: 34px; z-index: 5; width: 280px; padding: 12px; border-radius: 10px; \
                            background: {t.pill}; border: 1px solid {t.rule}; box-shadow: {t.shadow_float}; font-size: 13px;",
                    div { style: "font-weight: 600; margin-bottom: 6px;", "Restore {n} settings?" }
                    for (i, title) in changed.into_iter().enumerate() {
                        div { key: "{i}", style: "color: {t.ink_2}; font-size: 12px; line-height: 1.6;", "{title}" }
                    }
                    div { style: "display: flex; justify-content: flex-end; gap: 8px; margin-top: 10px;",
                        button {
                            style: "height: 28px; padding: 0 10px; border-radius: 6px; border: 1px solid {t.rule}; background: transparent; \
                                    color: {t.ink_2}; font-size: 12px; cursor: pointer;",
                            onclick: move |_| asking.set(false),
                            "Cancel"
                        }
                        button {
                            style: "height: 28px; padding: 0 10px; border-radius: 6px; border: none; background: {t.accent}; \
                                    color: {t.accent_ink}; font-size: 12px; font-weight: 600; cursor: pointer;",
                            onclick: move |_| {
                                let _ = h.restore(&id);
                                asking.set(false);
                            },
                            "Restore"
                        }
                    }
                }
            }
        }
    }
}

/// A section's blocks: each a heading and a card of rows.
#[component]
fn SectionBody(section: Section) -> Element {
    let t = theme();
    rsx! {
        div { style: "max-width: 760px; display: flex; flex-direction: column; gap: 26px;",
            if let Owner::Extension(ext) = &section.owner {
                div { style: "color: {t.ink_3}; font-size: 12px;", "From the {ext} extension" }
            }
            for (bi, block) in section.blocks.into_iter().enumerate() {
                div { key: "{bi}", style: "display: flex; flex-direction: column; gap: 8px;",
                    if let Some(title) = block.title {
                        div { style: "color: {t.ink_2}; font-size: 12px; font-weight: 600; padding-left: 2px;", "{title}" }
                    }
                    div { style: "display: flex; flex-direction: column; border-radius: 10px; background: {t.sheet}; \
                                  border: 1px solid {t.rule}; overflow: hidden;",
                        for (i, setting) in block.settings.into_iter().enumerate() {
                            Row { key: "{setting.key}", setting, first: i == 0 }
                        }
                    }
                }
            }
        }
    }
}

/// One setting: its words, then its control; a dot and a reset arrow
/// when it is not the default; dimmed, saying why, when the org has
/// locked it. A search hit on it scrolls to it and pulses it.
#[component]
fn Row(setting: Setting, first: bool) -> Element {
    let Some(h) = try_use_settings() else {
        return rsx! {};
    };
    let t = theme();
    let mut mounted = use_signal(|| None::<std::rc::Rc<MountedData>>);
    let this = setting.key.clone();
    let pulse = focused(h).filter(|(k, _)| *k == this).map(|(_, n)| n);
    use_effect(move || {
        let on = focused(h).is_some_and(|(k, _)| k == this);
        if let (true, Some(m)) = (on, mounted()) {
            spawn(async move {
                let _ = m.scroll_to(ScrollBehavior::Smooth).await;
            });
        }
    });
    let (value, changed, locked) = {
        let registry = h.registry.read();
        let values = h.values.read();
        (
            values.get(&registry, &setting.key),
            values.is_changed(&registry, &setting.key),
            values.is_locked(&setting.key),
        )
    };
    let Some(value) = value else {
        return rsx! {};
    };
    let rule = if first { "transparent" } else { t.rule };
    let key = setting.key.clone();
    let narrow = phone();
    let dir = if narrow { "column" } else { "row" };
    let align = if narrow { "stretch" } else { "center" };
    let (fade, reach) = if locked {
        ("0.5", "none")
    } else {
        ("1", "auto")
    };
    let row_id = format!("setting-{}", setting.key);
    rsx! {
        div {
            id: "{row_id}",
            style: "position: relative; display: flex; flex-direction: {dir}; align-items: {align}; gap: 14px; padding: 14px 16px; \
                    border-top: 1px solid {rule};",
            onmounted: move |e| mounted.set(Some(e.data())),
            if let Some(n) = pulse {
                // Keyed by the hit's count, so another hit on it pulses again.
                div {
                    key: "{n}",
                    style: "position: absolute; top: 0; left: 0; right: 0; bottom: 0; pointer-events: none; \
                            background: {t.accent_soft}; animation: architect-settings-pulse 900ms ease-out 2 forwards;",
                }
                style { "@keyframes architect-settings-pulse {{ 0% {{ opacity: 1; }} 100% {{ opacity: 0; }} }}" }
            }
            div { style: "position: relative; flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px;",
                div { style: "display: flex; align-items: center; gap: 8px;",
                    span { style: "font-size: 13px; font-weight: 550; color: {t.ink};", "{setting.title}" }
                    if changed {
                        span { style: "width: 6px; height: 6px; border-radius: 3px; background: {t.accent};", title: "Changed" }
                        button {
                            style: "width: 20px; height: 20px; border: none; border-radius: 5px; background: transparent; \
                                    display: flex; align-items: center; justify-content: center; cursor: pointer;",
                            title: "Put back the default",
                            "aria-label": "Reset {setting.title}",
                            onclick: move |_| h.reset(&key),
                            RotateCcw { size: 12, color: t.ink_3 }
                        }
                    }
                }
                if !setting.description.is_empty() {
                    span { style: "font-size: 12px; line-height: 1.5; color: {t.ink_3};", "{setting.description}" }
                }
                if locked {
                    span { style: "font-size: 12px; color: {t.ink_3};", "Set by your organization" }
                }
            }
            div { style: "position: relative; opacity: {fade}; pointer-events: {reach};",
                ControlView { setting_key: setting.key, control: setting.control, value }
            }
        }
    }
}

#[component]
fn ControlView(setting_key: String, control: Control, value: Value) -> Element {
    let Some(h) = try_use_settings() else {
        return rsx! {};
    };
    let t = theme();
    let set = EventHandler::new(move |v: Value| h.set(&setting_key, v));
    match control {
        Control::Switch => {
            let on = value.as_bool().unwrap_or(false);
            let (track, knob) = if on {
                (t.accent, "18px")
            } else {
                (t.rule, "2px")
            };
            rsx! {
                button {
                    role: "switch",
                    "aria-checked": "{on}",
                    style: "position: relative; flex-shrink: 0; width: 36px; height: 20px; border: none; border-radius: 10px; \
                            background: {track}; cursor: pointer;",
                    onclick: move |_| set.call(Value::Bool(!on)),
                    span { style: "position: absolute; top: 2px; left: {knob}; width: 16px; height: 16px; border-radius: 8px; \
                                   background: #ffffff;" }
                }
            }
        }
        Control::Segmented(choices) => rsx! {
            div { style: "flex-shrink: 0; display: flex; padding: 2px; gap: 2px; border-radius: 8px; background: {t.desk}; \
                          border: 1px solid {t.rule};",
                for (i, c) in choices.into_iter().enumerate() {
                    ChoiceButton { key: "{i}", on: c.value == value, choice: c, onpick: set, kind: 0 }
                }
            }
        },
        Control::Cards(choices) => rsx! {
            div { style: "flex-shrink: 0; display: flex; gap: 8px;",
                for (i, c) in choices.into_iter().enumerate() {
                    ChoiceButton { key: "{i}", on: c.value == value, choice: c, onpick: set, kind: 1 }
                }
            }
        },
        Control::Select(choices) => rsx! { SelectView { choices, value, onpick: set } },
        Control::Stepper {
            min,
            max,
            step,
            unit,
        } => {
            let n = value.as_int().unwrap_or(min);
            let shown = if unit.is_empty() {
                format!("{n}")
            } else {
                format!("{n} {unit}")
            };
            rsx! {
                div { style: "flex-shrink: 0; display: flex; align-items: center; height: 30px; border-radius: 8px; \
                              border: 1px solid {t.rule}; background: {t.desk};",
                    StepButton { minus: true, onpress: move |()| set.call(Value::Int(n.saturating_sub(step))), enabled: n > min }
                    span { style: "min-width: 64px; text-align: center; font-size: 13px; font-variant-numeric: tabular-nums;", "{shown}" }
                    StepButton { minus: false, onpress: move |()| set.call(Value::Int(n.saturating_add(step))), enabled: n < max }
                }
            }
        }
        Control::Slider {
            min,
            max,
            step,
            unit,
        } => {
            rsx! { SliderView { min, max, step, unit, value: value.as_float().unwrap_or(min), onpick: set } }
        }
        Control::Text { placeholder } => rsx! {
            TextView { value: value.as_text().unwrap_or_default().to_owned(), placeholder, onpick: set }
        },
        Control::Shortcut => rsx! {
            ShortcutView { value: value.as_text().unwrap_or_default().to_owned(), onpick: set }
        },
    }
}

#[component]
fn ChoiceButton(on: bool, choice: Choice, onpick: EventHandler<Value>, kind: u8) -> Element {
    let t = theme();
    let v = choice.value.clone();
    if kind == 1 {
        let edge = if on { t.accent } else { t.rule };
        return rsx! {
            button {
                style: "width: 96px; min-height: 64px; display: flex; flex-direction: column; align-items: flex-start; \
                        justify-content: flex-end; gap: 2px; padding: 10px; border-radius: 10px; border: 1.5px solid {edge}; \
                        background: {t.desk}; text-align: left; cursor: pointer;",
                "aria-pressed": "{on}",
                onclick: move |_| onpick.call(v.clone()),
                span { style: "font-size: 13px; font-weight: 600; color: {t.ink};", "{choice.label}" }
                if !choice.detail.is_empty() {
                    span { style: "font-size: 11px; color: {t.ink_3};", "{choice.detail}" }
                }
            }
        };
    }
    let (fill, ink) = if on {
        (t.pill, t.ink)
    } else {
        ("transparent", t.ink_3)
    };
    rsx! {
        button {
            style: "height: 24px; padding: 0 10px; border: none; border-radius: 6px; background: {fill}; color: {ink}; \
                    font-size: 12px; font-weight: 550; cursor: pointer;",
            "aria-pressed": "{on}",
            onclick: move |_| onpick.call(v.clone()),
            "{choice.label}"
        }
    }
}

#[component]
fn SelectView(choices: Vec<Choice>, value: Value, onpick: EventHandler<Value>) -> Element {
    let t = theme();
    let mut open = use_signal(|| false);
    let label = choices
        .iter()
        .find(|c| c.value == value)
        .map(|c| c.label.clone())
        .unwrap_or_default();
    rsx! {
        div { style: "position: relative; flex-shrink: 0;",
            button {
                style: "height: 30px; min-width: 140px; display: flex; align-items: center; justify-content: space-between; gap: 8px; \
                        padding: 0 10px; border-radius: 8px; border: 1px solid {t.rule}; background: {t.desk}; color: {t.ink}; \
                        font-size: 13px; cursor: pointer;",
                onclick: move |_| open.toggle(),
                span { "{label}" }
                ChevronDown { size: 14, color: t.ink_3 }
            }
            if open() {
                div { style: "position: absolute; right: 0; top: 34px; z-index: 5; min-width: 160px; padding: 4px; border-radius: 9px; \
                              background: {t.pill}; border: 1px solid {t.rule}; box-shadow: {t.shadow_float};",
                    for (i, c) in choices.into_iter().enumerate() {
                        button {
                            key: "{i}",
                            style: "width: 100%; height: 28px; display: flex; align-items: center; padding: 0 10px; border: none; \
                                    border-radius: 6px; background: {pick_fill(c.value == value, &t)}; color: {t.ink}; font-size: 13px; \
                                    text-align: left; cursor: pointer;",
                            onclick: {
                                let v = c.value;
                                move |_| {
                                    onpick.call(v.clone());
                                    open.set(false);
                                }
                            },
                            "{c.label}"
                        }
                    }
                }
            }
        }
    }
}

const fn pick_fill(on: bool, t: &ShellTheme) -> &'static str {
    if on { t.accent_soft } else { "transparent" }
}

#[component]
fn StepButton(minus: bool, onpress: EventHandler<()>, enabled: bool) -> Element {
    let t = theme();
    let ink = if enabled { t.ink_2 } else { t.rule };
    rsx! {
        button {
            style: "width: 30px; height: 28px; border: none; background: transparent; display: flex; align-items: center; \
                    justify-content: center; cursor: pointer;",
            disabled: !enabled,
            "aria-label": if minus { "Less" } else { "More" },
            onclick: move |_| onpress.call(()),
            if minus {
                Minus { size: 14, color: ink }
            } else {
                Plus { size: 14, color: ink }
            }
        }
    }
}

/// How wide a slider's track is, points.
const TRACK: f64 = 160.0;

#[component]
fn SliderView(
    min: f64,
    max: f64,
    step: f64,
    unit: String,
    value: f64,
    onpick: EventHandler<Value>,
) -> Element {
    let t = theme();
    let mut held = use_signal(|| false);
    let span = (max - min).max(f64::EPSILON);
    let at = ((value - min) / span).clamp(0.0, 1.0) * TRACK;
    let pick = move |x: f64| {
        let v = (x / TRACK).clamp(0.0, 1.0).mul_add(span, min);
        onpick.call(Value::Float(v));
    };
    let shown = if step >= 1.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    };
    rsx! {
        div { style: "flex-shrink: 0; display: flex; align-items: center; gap: 10px;",
            div {
                style: "position: relative; width: {TRACK}px; height: 20px; touch-action: none; cursor: pointer;",
                onpointerdown: move |e| {
                    held.set(true);
                    pick(e.element_coordinates().x);
                },
                onpointermove: move |e| {
                    if held() {
                        pick(e.element_coordinates().x);
                    }
                },
                onpointerup: move |_| held.set(false),
                onpointerleave: move |_| held.set(false),
                div { style: "position: absolute; left: 0; right: 0; top: 8px; height: 4px; border-radius: 2px; background: {t.rule};" }
                div { style: "position: absolute; left: 0; width: {at}px; top: 8px; height: 4px; border-radius: 2px; background: {t.accent};" }
                div { style: "position: absolute; left: {at - 8.0}px; top: 2px; width: 16px; height: 16px; border-radius: 8px; \
                              background: #ffffff; pointer-events: none;" }
            }
            span { style: "min-width: 52px; padding: 2px 8px; border-radius: 6px; background: {t.desk}; border: 1px solid {t.rule}; \
                           text-align: center; font-size: 12px; font-variant-numeric: tabular-nums;",
                "{shown}{unit}"
            }
        }
    }
}

#[component]
fn TextView(value: String, placeholder: String, onpick: EventHandler<Value>) -> Element {
    let t = theme();
    let mut draft = use_signal(|| value.clone());
    rsx! {
        input {
            r#type: "text",
            value: "{draft}",
            "aria-label": "{placeholder}",
            style: "flex-shrink: 0; width: 220px; height: 30px; padding: 0 10px; border-radius: 8px; border: 1px solid {t.rule}; \
                    background: {t.desk}; color: {t.ink}; font-size: 13px; outline: none;",
            oninput: move |e| draft.set(e.value()),
            onblur: move |_| onpick.call(Value::Text(draft())),
            onkeydown: move |e| {
                if e.key() == Key::Named(NamedKey::Enter) {
                    onpick.call(Value::Text(draft()));
                }
            },
        }
    }
}

#[component]
fn ShortcutView(value: String, onpick: EventHandler<Value>) -> Element {
    let t = theme();
    let mut recording = use_signal(|| false);
    let edge = if recording() { t.accent } else { t.rule };
    let label = if recording() {
        "Press a shortcut…".to_owned()
    } else if value.is_empty() {
        "Record".to_owned()
    } else {
        value
    };
    rsx! {
        button {
            style: "flex-shrink: 0; min-width: 120px; height: 30px; padding: 0 10px; border-radius: 8px; border: 1px solid {edge}; \
                    background: {t.desk}; color: {t.ink}; font-size: 13px; cursor: pointer;",
            onclick: move |_| recording.toggle(),
            onkeydown: move |e| {
                if !recording() {
                    return;
                }
                e.prevent_default();
                if e.key() == Key::Named(NamedKey::Escape) {
                    recording.set(false);
                    return;
                }
                if let Some(chord) = chord(&e) {
                    onpick.call(Value::Text(chord));
                    recording.set(false);
                }
            },
            "{label}"
        }
    }
}

/// A key with modifiers, as commands show a shortcut (`⇧⌘P`); `None` for a
/// modifier alone.
fn chord(e: &KeyboardEvent) -> Option<String> {
    let key = match e.key() {
        Key::Character(c) => c.to_uppercase(),
        Key::Named(NamedKey::Tab) => "Tab".to_owned(),
        Key::Named(NamedKey::Enter) => "Return".to_owned(),
        Key::Named(NamedKey::ArrowLeft) => "←".to_owned(),
        Key::Named(NamedKey::ArrowRight) => "→".to_owned(),
        Key::Named(NamedKey::ArrowUp) => "↑".to_owned(),
        Key::Named(NamedKey::ArrowDown) => "↓".to_owned(),
        Key::Named(_) => return None,
    };
    let m = e.modifiers();
    let mut out = String::new();
    for (on, mark) in [
        (m.ctrl(), '⌃'),
        (m.alt(), '⌥'),
        (m.shift(), '⇧'),
        (m.meta(), '⌘'),
    ] {
        if on {
            out.push(mark);
        }
    }
    out.push_str(&key);
    Some(out)
}

/// On a phone: the grouped list, a section its own page.
#[component]
fn PhoneSettings() -> Element {
    let Some(h) = try_use_settings() else {
        return rsx! {};
    };
    let t = theme();
    // A search hit opens straight into its section.
    let mut in_section = use_signal(|| h.focus.peek().is_some());
    let current = (h.open)().unwrap_or_default();
    let registry = h.registry.read();
    let groups: Vec<(Option<String>, Vec<(String, String, String)>)> = registry
        .grouped()
        .into_iter()
        .map(|(g, list)| {
            (
                g,
                list.into_iter()
                    .map(|s| (s.id.clone(), s.title.clone(), s.icon.clone()))
                    .collect(),
            )
        })
        .collect();
    let section = registry.section(&current).cloned();
    drop(registry);
    let title = if in_section() {
        section
            .as_ref()
            .map(|s| s.title.clone())
            .unwrap_or_default()
    } else {
        "Settings".to_owned()
    };
    rsx! {
        div {
            style: "position: absolute; top: 0; left: 0; right: 0; bottom: 0; z-index: {architect_shell::layer::POPOVER}; \
                    display: flex; flex-direction: column; background: {t.desk}; font-family: {t.font}; color: {t.ink};",
            div { style: "flex: 0 0 56px; display: flex; align-items: center; gap: 6px; padding: 0 8px; margin-top: 44px;",
                if in_section() {
                    button {
                        style: "height: 44px; display: flex; align-items: center; gap: 2px; padding: 0 6px; border: none; \
                                background: transparent; color: {t.accent}; font-size: 16px; cursor: pointer;",
                        onclick: move |_| in_section.set(false),
                        ChevronLeft { size: 22, color: t.accent }
                        "Settings"
                    }
                }
                span { style: "flex: 1;" }
                button {
                    style: "width: 44px; height: 44px; border: none; background: transparent; display: flex; align-items: center; \
                            justify-content: center; cursor: pointer;",
                    "aria-label": "Close settings",
                    onclick: move |_| h.close(),
                    X { size: 20, color: t.ink_2 }
                }
            }
            div { style: "padding: 0 20px 10px; font-size: 30px; font-weight: 750; letter-spacing: -0.02em;", "{title}" }
            div { style: "flex: 1; min-height: 0; overflow-y: scroll; padding: 0 16px 40px;",
                if in_section() {
                    if let Some(s) = section {
                        SectionBody { section: s }
                    }
                } else {
                    for (gi, (group, list)) in groups.into_iter().enumerate() {
                        div { key: "{gi}", style: "margin-bottom: 22px;",
                            if let Some(g) = group {
                                div { style: "padding: 0 6px 8px; color: {t.ink_3}; font-size: 13px; font-weight: 600;", "{g}" }
                            }
                            div { style: "display: flex; flex-direction: column; border-radius: 24px; background: {t.sheet}; \
                                          border: 1px solid {t.rule}; overflow: hidden;",
                                for (i, (id, name, ic)) in list.into_iter().enumerate() {
                                    button {
                                        key: "{id}",
                                        style: "height: 56px; display: flex; align-items: center; gap: 12px; padding: 0 16px; border: none; \
                                                border-top: 1px solid {row_rule(i, &t)}; background: transparent; color: {t.ink}; \
                                                font-size: 16px; text-align: left; cursor: pointer;",
                                        onclick: move |_| {
                                            h.show(Some(&id));
                                            in_section.set(true);
                                        },
                                        {icon(&ic, 20, t.ink_2)}
                                        span { style: "flex: 1;", "{name}" }
                                        ChevronRight { size: 18, color: t.ink_3 }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

const fn row_rule(i: usize, t: &ShellTheme) -> &'static str {
    if i == 0 { "transparent" } else { t.rule }
}
