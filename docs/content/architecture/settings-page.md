+++
title = "Settings: declared once, a page for free"
description = "An app declares its settings as typed structs; architect stores them (per device or synced to the person), and builds the Settings page — T3 Code's design — with search, reset and restore."
weight = 56
+++

> **Status: proposed (2026-10-31).** For review before any code. T3 Code's
> settings are the design to match; [app settings](@/architecture/settings.md)
> (`Store<T>`, per device) is the storage that exists.

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
  [`Store<T>`](@/architecture/settings.md)), `Synced` (the person's, on
  every device: an architect service like Task's `UserPrefs`, cached
  locally, saved coalesced), or `Project`.
- The derive also gives typed access: `use_settings::<Audio>()` returns a
  signal of the value and `update(|a| a.buffer = 256)`; a change is
  stored and seen by everything reading it.
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

## Open questions

1. **Synced settings' storage:** one generic per-user document of all
   synced sections (a CRDT doc), or a service per app?
2. **Per-project overrides** (T3 Code's "Applying settings for [project]"
   sentence): needed now, or later?
3. **Settings an app reads before the UI exists** (the audio engine's
   buffer): read from the store directly with the same type — confirm
   that is enough.
