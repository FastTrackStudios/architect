---
"architect-app" = "minor"
"architect-editor" = "minor"
"architect-extensions" = "minor"
---

Language servers through the host (#512 (e)): architect_editor::lsp::LanguageServers runs a language's [languages.server] for each language and workspace root. Servers restart with backoff and stop when their last document closes or their extension is disabled. ui::use_code_document wires a CodeEditor to a server (diagnostics, didChange, a format's or rename's edits applied as one step). The extension host gains Host::with_language_servers, open_code (an extension's server runs only with capabilities.languages = ["servers"], granted) and install_editor_actions (EDITOR_SERVERS, EDITOR_DIAGNOSTICS, EDITOR_HOVER, EDITOR_COMPLETE, EDITOR_DEFINITION, EDITOR_FORMAT, EDITOR_RENAME). The Architect app runs them. lsp::stub is a stub server for tests (feature lsp-stub). Native only.
