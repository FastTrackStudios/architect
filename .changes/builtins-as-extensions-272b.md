---
"architect-extensions" = "minor"
"architect-commands" = "minor"
"architect-settings" = "minor"
"architect-theme-picker" = "minor"
"architect-lab-ui" = "none"
"architect-lab-media" = "none"
---

The theme picker and the extensions page are linked built-in extensions, added through the same manifest a third party writes (#272 part 2). New for every extension: ⌘O sources from `[[openers]] kind` (`action` answers `{query}` with `{items}`, the new `open` opens one), settings sections drawn by one of its views (`[[sections]] view`, `group = ""` for the untitled group; `architect_settings::SectionViews`), and `COMMANDS_SHOW` to open the palette or ⌘O on one source. The theme picker is `architect-theme-picker` (`theme.picker`: `THEME_PICKER_CHOOSE`, `_SCHEME`, `_FIND`, `_PICK`, all through the host service, cloneable with `EXT_CLONE`); the extensions manager's settings section and ⌘O's `ext:` come from its manifest (`MANAGER_FIND`, `MANAGER_SHOW`). Breaking (pre-1.0): `OpenerDecl` and `SectionDecl` gain `open` and `view`, `architect_settings::Section` gains `view`; `use_manager_page` no longer registers the section or the opener; the lab's `LAB_THEME_*` commands are gone (the picker's ids replace them).
