---
"architect-app" = "minor"
"architect-editor" = "minor"
"architect-extensions" = "minor"
---

architect-editor: the editor pin moves to editor main (editor#32). CodeEditor and TextEditor, Diagnostic and Severity, and the editor host-language API are re-exported. syntax::drive makes architect's language registry the editor's, so a language's fences and code editors are highlighted while its extension is on, and editors on screen draw again (ui::use_languages_revision, use_language). Grammars are linked into the binary: grammar = "linked:<name>" names one the app or a linked extension hands over with syntax::link_grammar. A grammar file is refused at load, because the editor's tree-sitter cannot load one. CodeEditor is in the story gallery with a golden. Breaking: syntax::Unsupported::OwnQuery is gone (a language's own query now works), and NotLinked carries the grammar.
