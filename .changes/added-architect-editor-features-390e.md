---
"architect-app" = "minor"
"architect-editor" = "minor"
"architect-extensions" = "minor"
"vault-live" = "none"
---

Added architect-editor (features/editor), the one door to FastTrackStudios/editor: its document model and Dioxus component re-exported (features state, ui with web or native, syntax), the language registry (Languages, Language, Grammar, Server, languages::builtin) and the syntax.* theme tokens (theme::TOKENS, theme::css). Extensions add languages with [[languages]] (LanguageDecl, Contributions::languages, Host::with_languages, Host::languages, EXT_LANGUAGES), in the registry while the extension is on. architect-extensions, vault-live and the app now reach the editor through architect-editor.
