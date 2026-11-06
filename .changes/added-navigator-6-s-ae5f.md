---
"architect-navigator" = "minor"
"architect-navigator-extension" = "minor"
"architect-shell" = "minor"
---

Added Navigator 6's keyboard map, icons and colours (#339): every pane key is a KeyAction with a stable id (pane:move-up, list:select-all, list:open, search:close, …) bound to Hotkeys in a KeyMap (architect_navigator::keys), remappable and kept in ViewState::keys (NavigatorState::bind_keys, run_key), with Vim's h j k l as the vim_keys setting; icons, colours and backgrounds for folders, tags, properties and notes (NavigatorState::paint, Paint, Icon: Lucide names, Iconize's, packs, emoji, vault SVGs), read from and written to frontmatter (FileOp::RemoveProperty), by a note's name or type, inherited by the inherit_*_colors settings; architect_shell::lucide draws any Lucide icon by name (LucideIcon, glyphs, names); the actions NAVIGATOR_ICON, NAVIGATOR_KEYS and NAVIGATOR_KEY, and NAVIGATOR_COLOUR for any target and backgrounds
