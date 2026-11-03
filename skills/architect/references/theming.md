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
  theme files. Files are authored. The app persists the layers it owns.
- Writes return whether anything changed. Don't redraw or persist on
  `false`.
- An unknown token is magenta on purpose (`number` is `NaN`). Fix the
  name, don't add a fallback.
- App tokens stay off architect-ui's `:root` CSS. Opt a namespace in with
  `ThemeStoreProvider { css_namespaces: vec!["arrange".into()] }`
  (`--arrange-item-edge`).

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

**Extension themes.** `[[themes]] path = "themes/x.json"` in
`extension.toml` (`features/extensions/src/manifest.rs` `ThemeDecl`). The
app adds `ThemeDirs::extension(id, folder, manifest.theme_files())` and
reloads.

## Pitfalls

- `ShellTheme` is no longer `Copy`: its fields are `Arc<str>`. Clone it
  (cheap), borrow fields (`&t.ink`), and an `Icon`'s `color` is an
  `Arc<str>` (`t.ink.clone()`).
- In a resolved theme the shell's accent is `brand` / `brand-ink` /
  `brand-soft`. `accent` is architect-ui's hover wash. Paint views read
  `brand`.
- `THEME_SET` writes the `Pick` layer and `THEME_TOKEN_SET` the `Person`
  layer unless given `layer`.
- An override of `border` that reads `$border` is a cycle, not "the value
  below": a reference always sees the final value. A `Preview` covers both until `THEME_PREVIEW_END`.
- A store change made off the UI thread (an action, the watcher) reaches
  Dioxus on the next turn, through `use_theme_store`'s channel. Don't
  read the store in render and expect it to subscribe: read
  `use_resolved` / `use_themed`.
