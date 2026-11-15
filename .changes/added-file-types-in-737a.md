---
"architect-desktop" = "minor"
"architect-distribution" = "minor"
"architect-extensions" = "minor"
"architect-navigator" = "minor"
"architect-web" = "minor"
"vault" = "minor"
"vault-live" = "minor"
"vault-obsidian" = "minor"
"vault-proto" = "minor"
"vault-query" = "minor"
---

Added `[[file_types]]` in an extension manifest: an extension declares a file type (`extension`, `name`, `icon`, `note_type`) and its files become vault notes like `.md` ones (frontmatter as properties, the body as text) everywhere: the walkers, `OpenVault`, the vault index (`type` defaulted to `note_type`), Bases, the documents API, the navigator, the node and the web. Core asks the registry `vault_proto::note_types` (`register`, `is_note_path`, `is_note_extension`, `note_type_of`, `note_stem`, `declared`; also `vault::note_types`), never `.md`. The host registers declared types on load, the node from `Distribution::file_types` and `NodeHost::load`; `EXT_FILE_TYPES` lists them. A type declared with `body = "text"` (`register_with`, `body_is_markdown`) is read only for its frontmatter and `[[links]]`, not inline `#tags` or task lines (`obsidian_parse::parse_note`, which the vault, the index, Bases rows and the navigator parse through). The web app registers its distribution's types before it builds a vault. Changed `NoteFile::is_markdown` to `NoteFile::is_note`.
