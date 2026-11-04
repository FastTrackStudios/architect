# Runtime themes (`architect-theme`)

The full page: `docs/content/architecture/theming.md`. This note is the
working set.

## Rules

- One `ThemeStore` per app (`features/theme/src/store.rs` `ThemeStore`).
  Never keep a second copy of colours: read the store's `Resolved`, or a
  typed slice of it (`FromResolved`).
- Never hard-code a colour in a view. Use a token (`t.ink`, `var(--card)`,
  `ctx.theme.color("brand")`). A colour the app needs and architect does
  not name is a token of the app's own (below), not a literal.
- A person's choices are layers (`Layer`, `LayerKind`), never edits to
  theme files. Files are authored. The org's, the person's and the
  device's layers are settings: keep them with
  `architect_settings::appearance::Persist` (feature `appearance`), never
  a store of your own.
- The Appearance page is architect's (`use_appearance_section`, features
  `appearance` + `ui`). Don't write an app's own theme picker page.
- Writes return whether anything changed. Don't redraw or persist on
  `false`.
- An unknown token is magenta on purpose (`number` is `NaN`). Fix the
  name, don't add a fallback.
- App tokens stay off architect-ui's `:root` CSS. Opt a namespace in with
  `ThemeStoreProvider { css_namespaces: vec!["arrange".into()] }`
  (`--arrange-item-edge`).
- A view that paints gets roles of its own (`editor.background`,
  `editor.syntax.chord`, …) with defaults over architect's tokens, set as
  the app's `App` layer, and reads them as a `FromResolved` slice. The
  lab's Keyflow editor is the example (`apps/lab/ui/src/keyflow.rs`
  `EDITOR_TOKENS`, `EditorColours`).

## Recipes

**Give an app themes (Dioxus, desk).**

```rust,ignore
let theme = architect_theme::use_theme_store(ThemeStore::default);
use_hook(|| {
    let store = theme.store();
    let _ = architect_theme::actions::register(&list, &store);    // THEME_*
    let dir = app_dir.join("themes");
    let _ = architect_theme::fs::install(&dir, SHIPPED);          // the app's own themes
    let dirs = ThemeDirs::new().folder(&dir);
    dirs.install_into(&store);
    architect_theme::fs::watch(&store, &dirs).ok()                 // keep it alive
});
// The OS's light/dark: theme.set_system(..) at start and on WindowEvent::ThemeChanged.
rsx! { ThemeStoreProvider { AppShell { /* no theme: follows the store */ } } }
```

The lab does exactly this: `apps/lab/ui/src/theme.rs`.

**A view's own slice, redrawn only when it changes.**

```rust,ignore
#[derive(PartialEq)]
struct Meter { safe: Rgba, warn: Rgba, clip: Rgba }
impl FromResolved for Meter {
    fn from_resolved(r: &Resolved) -> Self {
        Self { safe: r.color("signal-safe"), warn: r.color("signal-warn"), clip: r.color("signal-danger") }
    }
}
let meter = use_themed::<Meter>();          // Dioxus: a memo
let mut kept = Projected::<Meter>::new(&r); // anywhere: kept.update(&next) -> changed?
```

**An app's own tokens (Studio).** Namespace them (`arrange.item.edge`,
`mixer.meter.safe`: dot-separated `[a-z0-9-]` segments). Ship the
defaults as an `App` layer of expressions over architect's tokens, so they
follow every palette; a theme file or a person still overrides them.

```rust,ignore
store.set(LayerKind::App, Layer::default()
    .token("arrange.item.edge", "mix($border, $foreground, 0.2)") // OKLab, t 0..1
    .token("arrange.grid.bar", "alpha($ink, 0.18)")
    .token("arrange.cursor.play", "$warn")
    .token("arrange.item.body-alpha", "0.42"));
// in from_resolved: r.color("arrange.item.edge"), r.number("arrange.item.body-alpha")
```

Expressions (`features/theme/src/expr.rs`) are `$name`, `mix(a, b, t)`,
`alpha(a, t)`, nested; anything else is a literal. They evaluate after
every layer is in, so a reference sees the final value. An unknown name or
a cycle is magenta and a `Problem` (`ThemeStore::problems`,
`THEME_PROBLEMS`). `Resolved::source(name)` (`TokenSource`) says who set a
value; `Resolved::misses()` lists names asked for and absent;
`THEME_INSPECT {prefix}` shows both to an agent.

**A theme built in memory (an import).**
`Theme::from_tokens(id, scheme, tokens)` then
`store.insert_theme(theme)`: replaces the one of that id at run time,
bumps the revision only if what shows changed, survives `reload`.

**A render thread or worker.** Poll `store.revision()` (atomic, no lock)
each frame. When it moved, `projected.update(&store.current())`, and
re-record only on `true`. `Projected::revision()` stamps a recording.

**Extensions add themes and tokens.** In `extension.toml`:
`[[themes]] path = "themes/x.json"` (`features/extensions/src/manifest.rs`
`ThemeDecl`) and `[tokens] "clicker.face" = "mix($card, $brand, 0.22)"`
(`Manifest::tokens`, namespace `Manifest::token_namespace`: the id's last
part). Give the host the store and it does the rest, diffed on reload,
withdrawn on unload:

```rust,ignore
let host = Host::new(list, sections).with_themes(store.clone());
let runner = ProcessRunner::new()?.with_theme(store.clone()); // theme as data to processes
```

Without a host: `store.contribute(id, host::theme_contribution(&package))`
/ `store.withdraw(id)` (`features/theme/src/store.rs` `Contribution`).
`THEME_LIST` says `extension:<id>`. A broken file is a `THEME_PROBLEMS`
entry, never a failed load.

**An extension's GUI follows the theme.**

- Process view (Dioxus over the wire): style with `var(--…)`. Its own
  namespace reaches `:root` only if the app opts in
  (`ThemeStoreProvider { css_namespaces }`).
- Process that computes colours or paints: `guest::use_theme()` in a view,
  `Guest::theme()`, `Guest::on_theme(|r| …)`; each an
  `architect_theme::Resolved` (wire 3, `wire::ThemeData`).
- Luau: `Runtime::with_module(Arc::new(ThemeModule::new(store)))`
  (`features/scripting/src/theme.rs`); scripts read `theme.color(name)`
  (`0xRRGGBBAA`), `theme.number`, `theme.value`, `theme.scheme`,
  `theme.revision`, `theme.snapshot()`, `theme.subscribe(fn)`.
- `ImGui`: `use_script_canvas` follows the store above it (restyle and a
  frame on every change); headless, `Session::start_themed` then
  `Session::set_theme` on each change. `Style::from_theme` maps tokens
  (`imgui.<col>` first, else derived; `features/canvas/src/imgui.rs`
  `Col`). Scripts override with `ImGui.PushStyleColor(ctx,
  ImGui.Col_Button, theme.color('x'))`.

**Keep the look (person synced, device local, org default).**

```rust,ignore
use architect_settings::appearance::{Persist, Saved};
use architect_settings::{DeviceValues, DocValues, Scope, ScopedValues};
let device = DeviceValues::open_named("my-app", "appearance")?; // its own file
let person = DocValues::new(synced_doc.loro());                  // the person's synced doc
let values = ScopedValues { device: device.load(), synced: person.load(), org, ..Default::default() };
let persist = Persist::start(&store, &values, Duration::from_millis(250), move |s: &Saved| match s.scope {
    Scope::Device => { let _ = device.save(&s.values); }
    Scope::Synced => { let _ = person.save_keys(&s.values, &s.keys); }
    Scope::Project => {}
});
// another device's edit: persist.replace(Scope::Synced, &person.load());
// the org switched: persist.set_org(org_settings);
architect_settings::use_appearance_section(); // under use_theme_store + use_settings_provider
```

Keys: `appearance.theme` (`id` or `light,dark`), `appearance.mode`,
`appearance.token.<name>` (`features/settings/src/appearance.rs`). The
lab does it in `apps/lab/ui/src/theme.rs` `persist`.

## Pitfalls

- `ShellTheme` is no longer `Copy`: its fields are `Arc<str>`. Clone it
  (cheap), borrow fields (`&t.ink`), and an `Icon`'s `color` is an
  `Arc<str>` (`t.ink.clone()`).
- In a resolved theme the shell's accent is `brand` / `brand-ink` /
  `brand-soft`. `accent` is architect-ui's hover wash. Paint views read
  `brand`.
- `THEME_SET` and `THEME_TOKEN_SET` write the `Person` layer (kept) unless
  given `layer` (`device`, `pick` for this run only; never `builtin` or
  `extension`). A `Device` choice hides the person's: setting the
  person's then changes nothing that shows.
- Keep the device's appearance values in a file of their own
  (`DeviceValues::open_named`): the settings page saves its device
  values whole, over anything else in its file.
- `subscribe` hears only what shows; something that keeps layers listens
  with `subscribe_layers` (a choice under another layer's moves nothing
  that shows).
- The `Extension` layer sits under the theme's own overrides (a theme
  file restyles an extension's tokens); every other layer sits over them.
- In Luau a module whose fields move must say `live_fields()`: Luau
  folds `Global.field` once when a sandboxed chunk loads.
- A process built for wire 3 is refused by a host before it (exact
  check); a host takes wire 2 to 6 (`WIRE_MIN`..`WIRE`).
- A pop-out window is a document of its own: what reaches it is what its
  `Bridge` carries. It follows the theme because `PopoutSync` carries the
  `ThemeHandle` and `RootCss` and shares the host's `ShellTheme` signal; a
  copy taken at open (`peek().clone()`) stays the theme it opened with.
- An override of `border` that reads `$border` is a cycle, not "the value
  below": a reference always sees the final value. A `Preview` covers both until `THEME_PREVIEW_END`.
- `document::Style` is inserted once: a sheet built from the theme's
  values keeps the first theme. Write the sheet with `var(--…)` only and
  set the values as variables on the view's root from a `use_themed`
  slice (`apps/lab/ui/src/keyflow.rs` `EditorStyles`).
- A store change made off the UI thread (an action, the watcher) reaches
  Dioxus on the next turn, through `use_theme_store`'s channel. Don't
  read the store in render and expect it to subscribe: read
  `use_resolved` / `use_themed`.
