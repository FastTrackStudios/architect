# architect-ui, showcase, ui-snapshot, architect-story

Crate `architect-ui` (`features/ui/architect-ui`); house rules in `features/ui/AGENTS.md`; guide `docs/content/architecture/composing-the-ui.md`. Use the Dioxus git rev pinned in architect's root `Cargo.toml`; consumers must match it.

## Pieces

| Piece | What it does | Entry points |
|---|---|---|
| `components/` | ~70 modules of Dioxus components (list below) | `features/ui/architect-ui/src/components/mod.rs` |
| `layout/`, `typography/` | `Divider`, `HStack`, `Spacer`, `VStack`; `Heading`, `Text` | `features/ui/architect-ui/src/layout`, `features/ui/architect-ui/src/typography` |
| `theme/` | `ThemeProvider`, `ThemeScope`, `ThemeRuntimeStyle`, `ThemeSwitcher`; presets; CSS-var tokens | `features/ui/architect-ui/src/theme/runtime.rs` `ThemeProvider`, `theme_preset`, `default_theme_state`; `features/ui/architect-ui/src/theme/tokens.rs` |
| `cn` | Tailwind class merge | `features/ui/architect-ui/src/cn.rs` `merge`, `merge_slice`, `cn` |
| Embedded CSS | `THEME_CSS`, `UTILITIES_CSS` consts | `features/ui/architect-ui/src/lib.rs` |
| `prelude` | flat re-export of components, theme, layout, typography | `features/ui/architect-ui/src/prelude.rs` |
| Showcase | single-screen component board; desktop app around it | `features/ui/architect-ui/src/showcase.rs` `Showcase`; `features/ui/showcase` |
| Stories | `#[story]` registrations | `features/ui/architect-ui/src/stories.rs` |
| `ui-snapshot` | PNG render, diff CLI, pixel probes | `features/ui/ui-snapshot/src/main.rs`, `features/ui/ui-snapshot/tests/pixel_probes.rs` |
| `architect-story-*` | story registry, macros, shell, snapshots, parity, fuzz | `features/story/README.md` |

## Wire-up checklist (Dioxus app)

1. Dioxus at architect's git rev. `dioxus-primitives` and `lucide-dioxus` float on `branch = "main"` in manifests: the lock is the pin.
2. `architect-ui = { workspace = true }`; add `features = ["router"]` only for `Navbar*`.
3. Stylesheets, one of:
   - Injected: `document::Style { {architect_ui::THEME_CSS} }` and `document::Style { {architect_ui::UTILITIES_CSS} }`. Use this when the app's Tailwind does not scan architect-ui.
   - Scanned (studio's pattern): `@import` `fts-theme.css` and `@source` architect-ui's `src/**/*.rs` from a stable checkout path.
   Prefer `@import` of `features/ui/architect-ui/assets/fts-theme.css` over a vendored copy (Task vendors one; keeping it in step is manual).
4. Theme: `let state = use_signal(default_theme_state);` then `ThemeProvider { state, … }`. Presets via `theme_preset("doom-64")`; default preset name `"default"`.
5. Nested regions: `ThemeScope { styles, mode, allow_layout_scale }`.
6. Stories assume a host `ThemeProvider` and must not add one.
7. Classes used only inside architect-ui are missing from a downstream sheet unless you inject `UTILITIES_CSS` or scan the crate.

**Theme knobs.** `allow_layout_scale` is off by default; when off, `--spacing` is pinned to `0.25rem`. Turn it on only when the theme should resize `gap-*`, `px-*`, `size-*`. Tokens are CSS vars (`--background`, `--primary`, `--signal-*`, `--sidebar-*`); the Rust consts in `tokens` wrap them as `var(...)` strings for inline styles.

**Showcase.** `architect_ui::showcase::Showcase` is the visual parity board (dark by default; `renderer` labels the top bar). Run it from `features/ui/showcase` with `dx serve --platform desktop` (package `architect-ui-showcase`, desktop is its default feature).

**Command, Menubar, ContextMenu.** `Menubar*` and `ContextMenu*` wrap `dioxus_primitives`; `Command*` (`CommandDialog`, `CommandInput`, `CommandList`, `CommandGroup`, `CommandItem`, `CommandEmpty`, `CommandSeparator`, `CommandShortcut`) hand-rolls Escape, overlay click and focus, and fuzzy-scores via `combobox::fuzzy_best_score` (nucleo). None of them consumes `ActionInfo` yet: feed them from `ActionList::infos()` yourself.

## Components (by group)

- Inputs/forms: Button, ButtonGroup; Input, InputGroup, InputOtp; Textarea; NativeSelect; Checkbox; RadioGroup; Switch; Slider, RangeSlider; Toggle, ToggleGroup; Label; Field, FieldLabel, FieldDescription, FieldMessage; Form, FormField, FormMessage; ColorPicker; InlineEdit; SegmentedControl; Calendar; DatePicker*; Combobox*; SearchableDropdown; SearchableList.
- Select/menus: Select* (8), Dropdown* (6), Menubar* (8), ContextMenu* (6), Command* (8), NavigationMenu* (6), Navbar* (5, `router` feature).
- Overlays: Dialog*, AlertDialog*, Drawer*, Sheet*, Popover*, HoverCard*, Tooltip*, Toast + ToastProvider (`use_toast`).
- Data and display: Table* (9), DataTable (`DataTableView`, toolbar, footer, pagination, column visibility, filter; `use_data_table`), Card*, Avatar*, Badge, StatusBadge, StatusDot, Alert*, Progress, ProgressBar, Spinner, Skeleton*, Kbd, KeyValueRow, ListRow, EmptyState, SectionHeader, Item*, Breadcrumb*, Pagination*, NavTab, Accordion*, Collapsible*, Tabs/TabList/TabTrigger/TabContent, Carousel*, AspectRatio, ScrollArea, Resizable*, Toolbar*.
- Sidebar* (16, `SidebarProvider` … `SidebarMenuBadge`; `use_sidebar_compact`).
- Layout/type/theme: Divider, HStack, Spacer, VStack; Heading, Text; ThemeProvider, ThemeScope, ThemeRuntimeStyle, ThemeSwitcher; DirectionProvider (`use_direction`); `use_form`.

Before writing a widget in an app, check this list: consumers have re-implemented `Button`, `EmptyState`, `SectionHeader`, `Switch`, `Slider`, `Toolbar` and others locally.

## Pitfalls

- **Dead CSS is silent:** an architect-ui-only class renders unstyled downstream unless `UTILITIES_CSS` is injected or the crate scanned.
- **`assets/utilities.css` is generated and committed.** After any class change run `just ui-css` (needs `tailwindcss` on PATH) and commit; `just ui-css-check` compares the working tree to the index, and CI does not run it.
- **Snapshot and parity gates are local-only.** CI runs fmt, check, clippy, test, boundaries, docs and a wasm check that excludes the UI and story crates; nothing in CI runs `ui-snapshot` or parity.
- **`ui-snapshot check` fails on a fresh tree** for scenes without a reference PNG ("NO REFERENCE"); `features/ui/ui-snapshot/tests/reference` holds only the icons scene.
- **Debug builds:** story rendering panics in debug, and `features/ui/architect-ui/tests/snapshots.rs` is ignored under `debug_assertions`. Run story VRT with `cargo test --release -p architect-ui`; refresh baselines with `FTS_STORY_UPDATE_SNAPSHOTS=1`.
- **Parity needs a release showcase build plus Xvfb, xdotool and ImageMagick;** its tests in `features/ui/architect-ui/tests/parity.rs` are `#[ignore]`d.
- **Wasm:** the UI and story crates are native-only in CI.
- **Dioxus rules** (`features/ui/AGENTS.md`): Dioxus 0.7 APIs only (no `cx`, `Scope`, `use_state`); owned props, `Clone + PartialEq`; merge `class` last; prefer `dioxus_primitives` for behaviour; use architect `Select`, raw `<select>` only inside `NativeSelect`.
- **architect-story:** `#[story]`, the registry, runtime, snapshots and parity are real; `story_test`/`states` macros and the fuzz crate are stubs; the shell (`Lookbook`) is built but unused. No consumer uses a story crate yet.
- **Stale docs:** `features/ui/AGENTS.md` and `features/ui/README.md` still describe the old standalone-repo layout; trust the tree.
