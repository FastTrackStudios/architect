---
"architect-theme" = "major"
"architect-ui" = "minor"
---

Added an app's own theme tokens to architect-theme: dotted token names (`arrange.item.edge`) in theme files, layers, `Theme::from_tokens` and `THEME_TOKEN_SET`; values that name other tokens (`$name`, `mix(a, b, t)`, `alpha(a, t)`, module `expr`), evaluated after every layer, where an unknown name or a cycle draws magenta and is a `Problem`; `Resolved::number`, `Resolved::source` (`TokenSource`, `Origin`), `Resolved::misses`, `Resolved::problems`, `Resolved::css_tokens`, `Resolved::css_namespaced` and `css_name`; `Projected::revision`; a lock-free `ThemeStore::revision`; `ThemeStore::insert_theme` / `remove_theme` with `Source::Memory` themes that survive a reload; `THEME_INSPECT`, and an optional `layer` on `THEME_TOKEN_SET` / `THEME_TOKEN_RESET`. architect-ui's `ThemeStoreProvider` writes only architect's undotted tokens as CSS variables, and its `css_namespaces` prop (with `ThemeStyle::from_resolved_with`, `ThemeState::from_resolved_with`) opts an app's namespaces in

Migration: `Resolved::with_overrides` is removed; layers' overrides go through the store (`ThemeStore::set` / `set_token`). `Composed` has two new public fields (`chosen_by`, `set_by`), so code that builds one by literal adds them. `Source` has a new variant, `Source::Memory`, so an exhaustive `match` on it adds an arm. An override value that starts with `$`, `mix(` or `alpha(` is now evaluated as an expression rather than kept as text.
