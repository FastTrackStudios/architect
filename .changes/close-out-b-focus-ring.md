---
"architect-shell" = "patch"
"architect-settings" = "minor"
"architect-guide" = "minor"
"architect-app" = "none"
"architect-sandbox-board" = "none"
"architect-sandbox-media" = "none"
---

A focused text field rings in the theme's accent, not Blitz's browser blue (#215): Blitz never matches `:focus-visible` and its own sheet outlines a focused `input`/`textarea` in `#4D90FE`, so `shell_css` now also rings `input`, `textarea` and `select` on `:focus`, and a framed field with a bare input takes `class: "focus-ring"` (rung on `:focus-within`); the settings page's search box and text fields use them. `Persist::local` keeps an app's look on this machine alone (device layer in `appearance.json`, person's in `person.loro`); the lab's own appearance wiring (`LabAppearance`, `persist`) is gone for it (#185). A scenario's `@click` can hold modifiers (`@click{mods (ctrl)}`), and the lab's scenario `extension-placement-and-links` checks that a view opens in its manifest's placement and a Ctrl-clicked link opens with the extension whose opener claims its URL (#239). The Board's benchmark has a WASI row (the lab's WASI counter) (#272). The features page lists each contribution kind (#179). API breaks (pre-1.0): `Do::Click` gained `mods`; the lab's `theme::LabAppearance` and `theme::persist` are gone.
