---
"architect-shell" = "minor"
"architect-extensions" = "minor"
"architect-workspace" = "minor"
"architect-commands" = "minor"
"architect-app" = "none"
"architect-sandbox-media" = "none"
"architect-sandbox-board" = "none"
---

The phone's keyboard bar is the Typing Toolbar (#316): the same pill that scrolls sideways and the round hide-keyboard button apart at the right, now filled through a slot. The shell exposes `TypingTools` (given the `Typing` in force: the focused editor's context and actions, and the pages and views the focus is in, which say so with `use_typing_scope`; every workspace pane does for its view kind) and draws every item itself (`TypingItem`: buttons, toggles that follow their action's `checked` state, dropdowns opening above the bar, separators, widgets), keeping the field's focus through any press. `architect-extensions` fills it with toolbars of the new place `where = "typing"`: the focused editor's own items as the built-in toolbar of its context (`typing/<context>`, `Toolbars::provide_editor`; `Toolbars::provide` for any toolbar the app adds at run time, `Source::Provided`), then every extension's whose `when` is in force (`view:<kind>`, `editor:<context>`), from its manifest or added at run time while its page is mounted. A person reorders, hides or adds items per context with the toolbar editor (`toolbars/typing-<context>.toml`); with the editor on, the bar's toolbars show above it. `TOOLBAR_*` positions may be numbers. The lab's Board has a chord sheet (`lab.board/chart`) with a typing toolbar: a sharps toggle, the chords, an Insert dropdown and the tempo widget. API breaks (pre-1.0): `CommandRunner` gained `checked`; `ToolbarPlace` gained `Typing`; `toolbars::Source` gained `Provided`; the editing bar's drawing moved from `bar.rs` to `typing.rs`.
