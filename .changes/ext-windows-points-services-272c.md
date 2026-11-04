---
"architect-extensions" = "minor"
"architect-settings" = "minor"
"architect-settings-sheet" = "minor"
"architect-workspace" = "minor"
"architect-pages" = "minor"
"architect-extension-wasi" = "minor"
"architect-lab-ui" = "none"
"architect-lab-media" = "none"
---

Extensions own windows, declare contribution points and offer services to each other (#272 part 3), and the settings page is a linked built-in extension on them. `[[windows]]` (`id`, `view`, `title`, `kind = "sheet" | "window"`, `opens`, size): a sheet over the main window or an OS window through the workspace's pop-out machinery (`Popout::page`), opened by an action the host runs, drawn the same for every runner and following the theme (`EXT_WINDOWS`, `EXT_WINDOW_CLOSE`). `reserved = ["APP_…"]` with `Host::allow_reserved` lets a built-in (and its development copy) own an app id. `[[points]]` with a JSON Schema and a `deliver` action; others add `[[contribute."<point>"]]`, the app `Host::contribute_app`; a bad item is a problem in `APP_STATUS`, not a failed load; the set is delivered live (`EXT_POINTS`). `[[sections]]` is sugar for the point `settings.sections`. `[provides]` / `[requires]` services: load order follows them, a missing or too-old provider leaves the dependent unloaded with a problem; one call path, the host service's `service` (`Handle::service`, Luau `host.service`, WIT `call-service`, pages `services.call` / `EXT_SERVICE_CALL`), checked as every host call. The settings page is `architect-settings-sheet` (`settings`: ⌘, / `APP_SETTINGS`, cloneable with `EXT_CLONE`); the core keeps its frame (`SettingsSheet`) and the palette's `#` (`SettingsPage`). Breaking (pre-1.0): `SettingsPage` no longer draws the page or registers `APP_SETTINGS` (link the settings extension and allow it the id); `SettingDecl` gains `scope`; `ExtensionView` gains `bare`; `HostRequest` gains `service`, `point`, `window`; `LoadError` gains `Reserved` and `Requires`.
