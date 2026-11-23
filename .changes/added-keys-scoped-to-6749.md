---
"architect-commands" = "minor"
"architect-extensions" = "minor"
"architect-workspace" = "minor"
---

Added keys scoped to a view: `[[keys]] when = "view:<view>"` binds a chord (plain letters too) in the keymap's `view:<extension>/<view>` context, where it runs while that view has the focus and never reaches the view; rebindable there like any key. New `KeyDecl::when`/`KeyDecl::context`, `Commands::set_view_keys`, `set_view_context`, `handle_view_key`, `handle_view_chord`, `Keymap::set_scoped`, `Keymap::binds_in`, and `architect_workspace::field_focused`/`FieldFocus` (a letter typed into a text field stays the field's)
