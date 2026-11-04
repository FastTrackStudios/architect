---
"architect-canvas" = "major"
"architect-commands" = "major"
"architect-extensions" = "major"
"architect-lab-ui" = "major"
"architect-settings" = "major"
"architect-shell" = "major"
"architect-theme" = "major"
"architect-ui" = "major"
"architect-workspace" = "major"
---

Added runtime themes: architect-theme (a palette resolved into every semantic token, the cascade of layers, ThemeStore, strict hot-reloading theme files, THEME_* actions); ShellTheme's fields are now Arc<str> (no longer Copy) with ShellTheme::from_resolved, AppShell follows the theme store when given no theme, architect-ui's presets are architect-theme's built-in files and ThemeStoreProvider follows a store, PaintPane hands views the resolved theme as ThemeTokens, Icon.color is an Arc<str>, shell_tokens names the accent trio brand/brand-ink/brand-soft, and extension.toml may declare [[themes]]

Migration: `ShellTheme` is `Clone` but no longer `Copy`, and its fields are `Arc<str>`. Clone it (`shell.theme.clone()`), borrow a field where a `&str` is wanted (`&t.ink`, or `&*t.ink` in an `if`/`else` beside a literal), and give an `Icon` `color: t.ink.clone()` (or `"…".into()`). A helper that returned `&'static str` from a theme returns `&str`. `AppShell`'s and `WorkspaceView`'s `theme` props are `Option<ShellTheme>` (passing a `ShellTheme` still works); leave them out to follow the app's theme store. A paint view that read `"accent"` from `ctx.theme` reads `"brand"` (`"accent"` is architect-ui's hover wash in a resolved theme). Code that built `ShellTheme { accent: "#…", .. }` writes `"#…".into()`.
