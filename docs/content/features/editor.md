+++
title = "Editor"
description = "architect-editor: the one door to FastTrackStudios/editor (its document model, TextEditor and CodeEditor re-exported), the language registry that drives the editor's, grammars linked into the app, [[languages]], and the syntax.* theme tokens."
weight = 58
+++

## What it is and when to use it

The editor (the document model, the markdown pass, the Dioxus component,
tree-sitter highlighting, the LSP client) lives in its own repository,
FastTrackStudios/editor, and stays there: it depends on nothing above it,
which is why Task, Keyflow and Session can all embed it. Architect is
where it is **offered from** (#512). `architect-editor` (`features/editor`)
is a thin crate over the editor's crates, so an app or an extension gets
the editor by depending on architect, at the revision architect pins, and
never pins the editor repository itself. The pin is one revision, written
once per editor crate in the root `Cargo.toml` (`[workspace.dependencies]`
and the `[patch]` table under it); it moves in one PR.

On top of the re-exports it holds two things that are architect's own:

- the **language registry** (`features/editor/src/languages.rs` `Languages`):
  what the editor asks by file type or fence name, filled by the app and by
  extensions' `[[languages]]`, and made the editor's own registry by
  `syntax::drive`;
- the **syntax tokens** (`features/editor/src/theme.rs` `TOKENS`):
  `syntax.keyword`, `syntax.string`, … as theme tokens, so every theme
  styles every language.

| Feature | Module | What it gives |
|---|---|---|
| none | `ext`, `languages`, `theme` | the editor's extension surface as data (`editor:crates/editor-ext`), the registry and the tokens. Wasm-clean: serde only, no tree-sitter, no Dioxus |
| `state` | `state` | the document model (`editor:crates/editor-state`): `EditorState`, `TransactionSpec`, `History`, `markdown` |
| `syntax` | `syntax` | the registry made the editor's (`drive`), grammars the app links (`link_grammar`), the editor's host-language API (`HostLanguage`, `Syntax`, `LanguageFn`, `register_language`, `language_for_path`, `languages_revision`, …), a registered language highlighted, each span named by its theme token (`editor:features/syntax`, `editor:crates/editor-state/src/languages.rs`) |
| `ui` + `web` or `native` | `ui` | the components (`editor:features/ui`): `TextEditor` (`Editor`), `CodeEditor`, `CODE_EDITOR_CLASS`, `EDITOR_CSS`, `DecorationSource`, `dispatch_with_history`, `WidgetMounts`, the palette and completion helpers; and `use_languages_revision`, `use_language`, `language_named`, which follow the registry. One renderer per binary |
| `stories` | `stories` | `CodeEditor` in the story gallery (`code_editor_rust`) |
| `lsp` | `lsp` | the editor's LSP client (`editor:features/lsp`), `LanguageServers` (a language's server per root, supervised) and, with `ui`, `use_code_document`. Native only: nothing on wasm |
| `lsp-stub` | `lsp::stub` | a stub language server for tests |

`Diagnostic` and `Severity` (feature `state`, also as `diagnostic`) are
what `CodeEditor`'s `diagnostics` prop takes.

Use it whenever code in architect, or in an app built on architect, wants
the editor. Architect's own users are on it: the app's note editor
(`apps/ui/src/note_editor.rs`, features `native`), the extension host's
editor calls (`features/extensions/src/editor.rs`, the contract) and the
vault's frontmatter parsing (`features/vault/vault-live/src/sync.rs`,
feature `state`).

## A minimal example

A read-only editor over a document, a fence in it highlighted:

```rust
use architect_editor::state::{EditorState, markdown};
use architect_editor::ui::{DecorationSource, Editor, TransactionEvent};
use dioxus::prelude::*;

#[component]
fn Source() -> Element {
    let state = use_signal(|| EditorState::new("```rs\nfn main() {}\n```\n"));
    let decorations = use_hook(|| DecorationSource::new(markdown::live_preview));
    let on_transaction = use_callback(|event: TransactionEvent| {
        if event.is_edit() { /* event.doc_after is the text now */ }
    });
    rsx! { Editor { state, decorations, on_transaction, editable: false } }
}
```

`features/extensions/tests/editor_view.rs` is this as a linked
extension's view.

## The code editor

`CodeEditor` edits a source file in one language: the whole buffer
highlighted, a line number per line, the bracket pair at the caret, Enter
keeping the indent, Tab and Shift-Tab, and diagnostics underlined with
their line's number marked. It is the same component as `TextEditor` on
the same `Signal<EditorState>`; `language` and `diagnostics` are reactive.

```rust
use architect_editor::state::EditorState;
use architect_editor::ui::{CodeEditor, use_language};
use architect_editor::{Diagnostic, Severity};
use dioxus::prelude::*;

#[component]
fn Source(path: String, text: String) -> Element {
    let state = use_signal(|| EditorState::new(text.clone()));
    // The language for the path, kept up to date as languages come and go.
    let language = use_language(path);
    let diagnostics = use_signal(Vec::<Diagnostic>::new);
    rsx! {
        document::Style { {format!("{}\n{}", architect_editor::ui::EDITOR_CSS, architect_editor::theme::css())} }
        CodeEditor { state, language, diagnostics }
    }
}
```

`features/editor/src/stories.rs` `code_editor_rust` is it in the story
gallery: a Rust buffer in the theme's `syntax.*` tokens with one warning;
`features/editor/tests/story.rs` plays it headlessly on Blitz against its
golden (`features/editor/tests/golden/code-rust.html`).

## Languages

A language is data (`features/editor/src/languages.rs` `Language`): an id,
a name, the file extensions and fence names it answers to, which
tree-sitter grammar it is highlighted with, its highlights query, its
comment token, and its LSP server's command and arguments when it has
one. A grammar is always linked into the binary:

| `grammar =` | `Grammar` | What |
|---|---|---|
| `"linked"` (the default) | `Linked` | one compiled into the editor (Rust, TypeScript, JavaScript, Python, JSON, Bash, Markdown), found by the language's id or a fence name |
| `"linked:<name>"` | `Host(name)` | one the app or a linked extension links in and hands over in Rust: `syntax::link_grammar(name, grammar, query)` |
| a file (`"x.wasm"`) | `File` | refused by `Language::check`, so a manifest naming one does not load (below) |

`highlights` (a query file in the package, its text read at load) is used
in place of the grammar's own query, so a language can run a built-in
grammar with a query of its own.

```rust
use architect_editor::languages::{Language, Languages, builtin};

// The app's own: the grammars the editor has compiled in.
let languages = Languages::with_app(builtin());
// An owner's languages go in together and out together.
languages.set("lab.keyflow", vec![
    Language::linked("keyflow", "Keyflow chart").extensions(&["kf"]).fences(&["kf"]),
]);
assert_eq!(languages.for_path("songs/a.kf").unwrap().language.id, "keyflow");
languages.withdraw("lab.keyflow");
```

`Languages` is asked with `get` (an id), `for_fence`, `for_extension`,
`for_path` and `resolve` (any of them). A language two owners register is
the first's. `revision` moves on every change.

An extension adds languages in its manifest, and they are in the app's
registry while it is on
([Extensions: languages](@/architecture/extensions.md#languages-for-the-editor)):

```toml
[[languages]]
id = "keyflow"
name = "Keyflow chart"
extensions = ["kf"]
fences = ["kf"]
grammar = "linked:keyflow"               # one the app links; "linked" for the editor's own
highlights = "queries/highlights.scm"
comment = "//"
[languages.server]                        # optional LSP
command = "kf-lsp"
args = ["--stdio"]
```

The app gives the host its registry with `Host::with_languages`
(`features/extensions/src/host.rs`); `Host::languages` hands it back, and
`EXT_LANGUAGES {id? | fence? | path?}` reads it from any runtime
(`features/extensions/src/contrib_actions.rs`).

### The registry drives the editor's

The editor has a registry of its own, process-wide
(`editor:crates/editor-state/src/languages.rs`): the markdown pass asks it
for a fence's language, and `language_for_path` answers it for a
`CodeEditor`. `syntax::drive(&languages)` makes architect's registry that
one: now and after every change (`Languages::on_change`) each language is
built into the editor's `Syntax` (`syntax::syntax_of`) and registered under
its id, fences and extensions, and one its owner withdrew is withdrawn. A
grammar linked later (`link_grammar`) or taken back (`unlink_grammar`)
brings it up to date too. A language that cannot be highlighted is left
out, and `syntax::problems` says why (`Unsupported`: a grammar file, a
`linked:` name nothing linked, no query, a query tree-sitter refused).

Every change moves the editor's `languages_revision`; a view on screen
hears it through `ui::use_languages_revision` (or `syntax::watch`) and
draws again: `use_language` is a memo over it, and the app's note editor
reads it in its decoration source so a fence's highlighting follows an
extension being turned on or off (`apps/ui/src/note_editor.rs`). The app
drives the registry it gives its host (`apps/ui/src/extensions.rs`
`app_languages`); one registry per process is driven.

```rust
// The app, at startup: its own grammar, linked in.
architect_editor::syntax::link_grammar("toml", tree_sitter_toml_ng::LANGUAGE, Some(TOML_HIGHLIGHTS));
let languages = Languages::with_app(builtin());
architect_editor::syntax::drive(&languages);
// An extension's `grammar = "linked:toml"` is now highlighted while it is on.
```

### Why grammars are linked

A grammar cannot be loaded from a `.wasm` file at run time with the
tree-sitter the editor uses. Loading needs tree-sitter's `wasm` feature
(`WasmStore`, built on `wasmtime-c-api`). `arborium-tree-sitter`, the
editor's tree-sitter, carries that module's source but its manifest
declares no `wasm` feature and no wasmtime dependency, so it can never be
compiled in; upstream `tree-sitter`'s `wasm` feature has a `Language`
type that does not interchange with arborium's, and its grammars run in
wasmtime, which does not build for `wasm32-unknown-unknown`, where the
editor also runs (`editor:docs/languages.md`). So an extension that
brings a language either is a linked extension and hands its grammar over
in Rust (`link_grammar` from its crate, then `grammar = "linked:<name>"`
in its manifest), or names one the app links, or one the editor has.

## Language servers

A language's `server` (`[languages.server] command, args`) is run by the
host while a code editor of the language is open
(`features/editor/src/lsp.rs` `LanguageServers`):

- **one per language and workspace root**, started when the first
  document of it opens (`LanguageServers::open`, a `Document`), stopped
  when the last one closes (the `Document` dropped);
- **supervised**: one that dies is started again after a wait that
  doubles (`Restart`: from 250 ms to 10 s, five times in a row; one that
  ran 30 s counts again), and its open documents are opened in it again
  with their text now; past the attempts it is `failed`. While it is
  down a document keeps its text and a request is answered with why;
- **stopped with its owner**: `LanguageServers::stop_owner`, which the
  extension host calls when an extension is disabled or unloaded.

Each server has one task that sends everything for it in order, so a
request never overtakes an edit made before it. A document
(`Document`, or a `DocRef` that does not hold it open) sends its edits
as `didChange` (`change`), hears its diagnostics (`watch_diagnostics`),
and asks `hover`, `completion`, `definition`, `format` and `rename`
(`format` and `rename` hand their edits to its editor,
`watch_edits`; a rename's edits to other files are counted, not
applied). `EditBatch::changes` is the edits as one change set.

### A `CodeEditor` with its server

`ui::use_code_document(open, state, history)` opens the file once (the
extension host's `Host::open_code`, below) and keeps it open while the
component lives; its `diagnostics` and `on_transaction` go to
`CodeEditor`, and a format or rename is applied to `state` as one step
while the text is still what the server answered for:

```rust
let code = use_code_document(|| host.open_code(&path, &root, &text).ok(), state, Some(history));
rsx! {
    CodeEditor {
        state,
        language: use_language(path.display().to_string()),
        diagnostics: code.diagnostics,
        on_transaction: code.on_transaction,
    }
}
```

### Through the extension host

The app gives its host the servers (`Host::with_language_servers`,
`architect-extensions` feature `lsp`; `features/extensions/src/lsp.rs`)
and puts `Host::install_editor_actions` on its list (the Architect app:
`apps/ui/src/extensions.rs` `language_servers`). `Host::open_code(path,
root, text)` finds the language by the file's extension and runs its
server only when the language is the app's own or its extension asks
`capabilities.languages = ["servers"]` and was granted it. The actions
act on the focused code document (or `path`) at its caret (or `at`, a
byte offset):

| Id | What |
|---|---|
| `EDITOR_SERVERS` | every server: language, root, owner, command, state, restarts, documents |
| `EDITOR_DIAGNOSTICS` | a document's diagnostics |
| `EDITOR_HOVER` | the hover text at a point |
| `EDITOR_COMPLETE` | the completions at a point |
| `EDITOR_DEFINITION` | where the symbol at a point is defined |
| `EDITOR_FORMAT` | formats the document, applied as one step |
| `EDITOR_RENAME` | renames the symbol at a point to `name` |

**What the editor lacks for the rest** (FastTrackStudios/editor): its
`hover` prop is a bare `fn(&EditorState, usize)` pointer and its
`completion` prop a synchronous source for its own `[[` and `#` triggers,
so a server's hover and completions cannot be drawn in the editor's
popups as they come (they are asked and answered through the actions
and `DocRef`). It needs an async hover and completion source. Code
actions and signature help are not asked yet. On the web there is no
server: `lsp` compiles to nothing on wasm, and highlighting still works.

## Highlight colours are theme tokens

`features/editor/src/theme.rs` `TOKENS` is fourteen tokens
(`syntax.keyword`, `.string`, `.comment`, `.number`, `.constant`,
`.function`, `.type`, `.variable`, `.property`, `.operator`,
`.punctuation`, `.attribute`, `.macro`, `.tag`), each with a default that
is an expression over architect's tokens, so it follows the palette
showing. An app does three things (`apps/ui/src/theme.rs` `app_layer`,
`apps/ui/src/note_editor.rs`):

1. adds `TOKENS` to its theme's `App` layer;
2. lists `theme::NAMESPACE` among the namespaces its theme writes as CSS
   variables (`--syntax-keyword`);
3. puts `theme::css()` after `ui::EDITOR_CSS`: the sheet that points the
   editor's token classes (`md-tok-k`, …) at those variables.

A theme file, an extension's `[tokens]` or a person then restyles any of
them (`features/editor/tests/theme.rs`). `theme::token_for_tag` maps one of
the editor's scope tags to its token, and `syntax::highlight(&languages,
name, source)` (feature `syntax`) returns a registered language's spans
already named by token (`features/editor/tests/syntax.rs`).

## An editor in an extension's view

A **linked** extension uses the component above
(`features/extensions/tests/editor_view.rs`). A process extension's remote
view, a WASI view and a page cannot mount one yet: all three draw through
one mechanism, the edit stream applied to a plain Blitz sub-document
(`features/extensions/src/remote.rs` `RemoteDocument`), which has no
host-drawn elements. The design for one is in #512.

## Tests to read

| Test | Shows |
|---|---|
| `features/editor/tests/languages.rs` | the registry by id, fence and file type; an owner adds, replaces and withdraws; a `[[languages]]` table as written |
| `features/editor/tests/syntax.rs` | Rust from the registry highlighted in `syntax.*` tokens; a built-in grammar with a language's own query; what is unsupported says why |
| `features/editor/tests/drive.rs` | the registry driving the editor's: a language on a linked grammar highlighted in a fence and found by path, withdrawn, back, the grammar unlinked |
| `features/editor/tests/story.rs` | `CodeEditor`'s story on Blitz against its golden |
| `features/editor/tests/lsp.rs` | servers against the stub: a document's diagnostic, hover, completion, format, rename and definition; one server per language and root; restarted when it dies; stopped with its owner; failed past its attempts |
| `features/editor/tests/lsp_view.rs` | a `CodeEditor` on Blitz wired by `use_code_document`: the server's warning drawn, its format in the text, the server gone with the editor |
| `features/extensions/tests/language_servers.rs` | through the host: refused without `capabilities.languages`, then the `EDITOR_*` actions, and the server stopped when the extension is turned off |
| `features/editor/tests/theme.rs` | every token resolves in a theme store, reaches CSS, and a person restyles one |
| `features/extensions/tests/interop.rs` | `languages_are_registered_while_the_extension_is_on`: enabling adds, disabling withdraws |
| `features/extensions/tests/editor_view.rs` | a linked extension's view holds the editor and hears its changes |

## Status

Steps (a) to (c) of #512, (c) for linked extensions only, (d):
`CodeEditor` with highlighting from the registry, and (e): language
servers run by the host, diagnostics in `CodeEditor` and the `EDITOR_*`
actions. Not yet: a server's hover and completions in the editor's own
popups (above), Keyflow on `architect-editor` (f), an editor in a remote
view, a WASI view or a page, and folding, find and replace and multiple
cursors in `CodeEditor` (the editor repository's
`editor:docs/languages.md` lists them). Grammars from files will not
come while the editor's tree-sitter cannot load one.
