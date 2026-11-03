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
- An unknown token is magenta on purpose. Fix the name, don't add a
  fallback.

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

**A domain app's roles (Studio).** Name them with the app's prefix
(`arrange-clip`, `mixer-strip`, `midi-note`). Derive defaults from
architect's tokens in the slice's `from_resolved`
(`r.get("arrange-clip")` first, else a mix of `card` and `brand`). Themes
and people set them in `overrides` / `THEME_TOKEN_SET`.

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
  layer. A `Preview` covers both until `THEME_PREVIEW_END`.
- A store change made off the UI thread (an action, the watcher) reaches
  Dioxus on the next turn, through `use_theme_store`'s channel. Don't
  read the store in render and expect it to subscribe: read
  `use_resolved` / `use_themed`.
