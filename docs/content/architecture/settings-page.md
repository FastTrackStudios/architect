+++
title = "Settings: declared once, a page for free"
description = "An app declares its settings as typed structs; architect stores them (per device or synced to the person), and builds the Settings page — T3 Code's design — with search, reset and restore."
weight = 56
+++

> **Status: the registry and the page landed (2026-11-01)** in
> `architect-settings` (`features/settings`). Settings are data
> (`features/settings/src/model.rs` `Section`, `Setting`, `Control`,
> `Value`), so an extension registers its own at run time
> (`features/settings/src/registry.rs` `Registry::register`,
> `Registry::remove_owner`; `features/settings/src/ui.rs` `use_section`) as the app does at
> launch; `Values` checks, resets and restores them; `DeviceValues`
> (feature `store`) keeps the device's; `SettingsPage` (feature `ui`) is
> T3 Code's page, opened by the `APP_SETTINGS` command (⌘,).
>
> **The derive, the scopes and search to the row landed (2026-11-01).**
> `#[derive(Settings)]` (`features/settings/src/derive.rs` `Settings`)
> makes a facet struct a section, and `use_settings::<T>()`
> (`features/settings/src/ui.rs` `use_settings`) its typed value;
> `ScopedValues` (`features/settings/src/values.rs`) holds values by
> scope over an org's `OrgSettings`; `ProjectValues`
> (`features/settings/src/store.rs`) and `DocValues`
> (`features/settings/src/synced.rs`, feature `synced`) store the
> `Project` and `Synced` scopes; a search hit opens its section, scrolls
> to its row and pulses it (`SettingsHandle::show_setting`). Still to
> come: Task's and Studio's panels as structs; `Option<T>` and `KeyChord`
> fields; which org's settings apply (the current org, with
> organizations' scoping); an admin's page to edit them.

Every app has a Settings page and every app writes it by hand: Task's six
panels, Studio's and Session's `SettingsPanel`s, T3 Code's twenty
sections — and in each, every row repeats what the type already says, and
the "restore defaults" list repeats every key again. Here an app declares
its settings once; the store, the page, its search, the reset arrows and
"Restore defaults" all come from the declaration.

## Declaring

<!-- doc-check: off (proposed names) -->
```rust
/// Appearance
#[derive(Facet, Settings, Default)]
#[settings(section = "Appearance", icon = "palette", scope = Device)]
pub struct Appearance {
    /// Theme
    /// Follow the system, or always light or dark.
    #[setting(control = cards)]
    pub theme: ThemeMode,               // an enum: its variants are the choices

    /// Text size
    #[setting(control = slider(80..=140, step = 5), unit = "%")]
    pub text_size: u32,

    /// Reduce motion
    /// Drawers and menus appear without moving.
    pub reduce_motion: bool,            // a bool is a switch
}

/// Audio
#[derive(Facet, Settings, Default)]
#[settings(section = "Audio", group = "Studio", icon = "audio-lines", scope = Device)]
pub struct Audio {
    /// Buffer size
    /// Smaller is quicker to play, and harder for the computer.
    #[setting(control = select, choices = [64, 128, 256, 512, 1024])]
    pub buffer: u32,
}

/// Editing (follows the person to every device)
#[derive(Facet, Settings, Default)]
#[settings(section = "Editing", scope = Synced)]
pub struct Editing { /* … */ }
```
<!-- doc-check: on -->

- **The words come from the doc comment:** its first line is the title,
  the rest the description; a struct's is its section's title.
- **The control comes from the type** unless `control` says otherwise:
  `bool` → switch; an enum → select (or `segmented`, `cards`, `list`);
  numbers → a stepper field (or `slider(range)`); `String` → text;
  `Option<T>` → T with an off; a `KeyChord` → a shortcut recorder;
  a struct → a nested group.
- **Scope** is where a value lives: `Device` (this device:
  [`Store<T>`](@/architecture/settings.md), `DeviceValues`), `Synced`
  (the person's, on every device: `DocValues`, below), or `Project`
  (the open project's: `ProjectValues` in the project's directory, or a
  `DocValues` over the project's own document).
- The derive also gives typed access: `use_settings::<Audio>()` returns a
  memo of the value and `update(|a| a.buffer = 256)`; each field that
  changed is stored in its scope and seen by everything reading it.
  Outside the UI, `Audio::read(&registry, &values)` is the same value
  (the audio engine reads its buffer that way, before any page exists).
- Today the derive takes `bool`, integers, floats, `String`, enums of
  unit variants and one level of nested struct; a field of any other
  type is left off the page and keeps its default.

## Scopes, synced, and the org

`ScopedValues` holds the three scopes' values and the org's
`OrgSettings` under them. A value reads, in order: the org's lock, its
own scope's value, the org's default, the declared default. A locked
row stays on the page, dimmed, saying "Set by your organization";
setting it is refused (`ValueError::Locked`). Which org's settings
apply is the app's to say (`SettingsHandle::set_org` when the window's
org changes).

Every change reaches the provider's `on_change` as a `SettingsChange`
(the scope, its values, the keys that changed), so the app sends it to
that scope's store. The `Synced` store is a CRDT document: `DocValues`
keeps the values in its root map, one entry per key, so two devices
that changed different settings offline both keep theirs, and a key
changed on both takes the later write. Every device opens the person's
document under the same id (`synced_doc_id(app, person)`) with
`crdt::docs`' `Lineage::Merge`, syncs it like any app document, writes
only the changed keys (`save_keys`), and on `on_remote_change` puts
`load()` back with `SettingsHandle::replace`.
- An app adds a hand-written section where a generated one cannot do
  (Account, About, a theme library): `Section::custom("Account", icon,
  || rsx! { … })`, in the same page and search.

## The page (T3 Code's)

**On a desk:** a modal over the window (rounded, shadowed, up to 80rem by
60rem; ⌘, opens it, Escape closes). Its left column lists the sections in
groups (a group's heading small and muted; each section an icon and a
name), a search field at its top (`/` or ⌘F). Its right column: a title
row with the breadcrumb (Settings › Appearance) and the page's actions
("Restore defaults"), then the section's groups, at most 56rem wide.

**On a phone:** the iOS grouped list. The root lists the sections in the
same groups (rounded cards, 24pt corners; a row 56pt: an icon, the name,
its value muted on the right, a chevron); a section pushes its own page
of grouped rows. The search filters sections. On an iPad the root is the
sidebar and the section beside it.

**A row:** the title (medium weight), its description under it (small,
muted), the control on the right; under a narrow width the control drops
below. Beside the title, two small slots: a dot when the value is not the
default, and a reset arrow (on hover; always on touch) that puts the
default back. A row that cannot be changed now (signed out, no device)
stays, dimmed, saying why.

**The page's actions:** "Restore defaults" lists every changed setting
in the section and asks before resetting them. Search finds a setting by
its title, its description and any `aliases`, opens its section and
scrolls to the row, which pulses.

**The controls:** a switch; a select (a small trigger, the choices in a
menu); a segmented control; a slider with its value in a chip ("65%");
cards for a few visual choices (Light / Dark / System); a stepper field;
a text field (saved on blur when it parses); a shortcut recorder (press
to record, chips for keys).

## Where it opens

- The account row at the bottom left of the [app shell](@/architecture/app-shell.md)
  and the phone's drawer have Settings.
- ⌘, on a Mac ("Settings…" in the app menu: [app menus](@/architecture/app-menus.md)).
- An action, `app.settings { section? }`, so the palette and a model can
  open it at a section.

## From what exists

- `architect::settings::Store<T>` stores the `Device` scope as it is.
- `architect::schema` already walks a facet struct's fields (names, docs,
  defaults) for the MCP schema; the derive reads the same, plus its
  attributes.
- Task's `UserPrefs` entity and `PrefsService` are the pattern for the
  `Synced` scope, generalised so any app's settings structs can sync.
- architect-form's `Field` gives the text and number controls their
  parse-and-validate behaviour.

## Decided

1. **Synced settings' storage:** one per-person CRDT document per app
   (`DocValues`), synced as an app document; no service per app.
2. **Per-project values:** a setting declared `Project` lives in the
   project's store, not as an override of a device value.
3. **Settings an app reads before the UI exists:** `T::read` over the
   values loaded from the stores, the same type the page uses.
