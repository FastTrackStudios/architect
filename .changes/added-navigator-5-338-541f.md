---
"architect-app" = "minor"
"architect-navigator" = "major"
"architect-navigator-extension" = "minor"
"vault" = "minor"
"vault-live" = "minor"
---

Added Navigator 5 (#338): the filter search box (architect_navigator::search::FilterSearch: names and aliases, #tags, .properties, @dates on created or modified, folder:, ext:, has:task, exclusions, quoted text, AND/OR between tags and properties; kept across runs; NAVIGATOR_SEARCH_VAULT; a full-text provider through NavigatorSource::search_text, OpenVault::search_text and vault_live::sync::search_pages), shortcuts to notes, folders, tags, properties and saved searches (Shortcut, NAVIGATOR_SHORTCUTS, NAVIGATOR_OPEN_SHORTCUT_1..9), recent notes per profile, vault profiles (Profiles, VaultProfile, NAVIGATOR_PROFILE, NAVIGATOR_SELECT_PROFILE_1..3) with a banner and hiding of folders, tags, notes and files by pattern, frontmatter or tag (hidden::Hidden, NAVIGATOR_HIDE, NAVIGATOR_TOGGLE_HIDDEN), folders excluded from descendants, and file visibility

Migration: `architect_navigator::list::files_of` takes the profile's `&Hidden` as its last argument (`&Hidden::default()` for none); `ListContext`'s `query` field is gone, replaced by `search: &FilterSearch`, `full_text: Option<&BTreeSet<String>>` and `hidden: &Hidden`; `NavigatorState::query` returns an owned `Query` (the search read back as tags and properties, empty otherwise) and `set_query` takes `&Query`; set the whole search with `NavigatorState::set_search` (or `ui::Navigator::set_search`, which also asks the full text). `NavRowKind` has `Banner`, `Section`, `Shortcut` and `Recent` rows. A profile shows every file by default (`FileVisibility::All`).
