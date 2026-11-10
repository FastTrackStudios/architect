+++
title = "Editor"
description = "architect-editor: the one door to FastTrackStudios/editor (its document model and component re-exported), the language registry the editor is given, [[languages]], and the syntax.* theme tokens."
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
  extensions' `[[languages]]`;
- the **syntax tokens** (`features/editor/src/theme.rs` `TOKENS`):
  `syntax.keyword`, `syntax.string`, … as theme tokens, so every theme
  styles every language.

| Feature | Module | What it gives |
|---|---|---|
| none | `ext`, `languages`, `theme` | the editor's extension surface as data (`editor:crates/editor-ext`), the registry and the tokens. Wasm-clean: serde only, no tree-sitter, no Dioxus |
| `state` | `state` | the document model (`editor:crates/editor-state`): `EditorState`, `TransactionSpec`, `History`, `markdown` |
| `syntax` | `syntax` | a registered language highlighted, each span named by its theme token (`editor:features/syntax`) |
| `ui` + `web` or `native` | `ui` | the component (`editor:features/ui`): `Editor`, `EDITOR_CSS`, `DecorationSource`, `dispatch_with_history`, `WidgetMounts`, the palette and completion helpers. One renderer per binary |

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

## Languages

A language is data (`features/editor/src/languages.rs` `Language`): an id,
a name, the file extensions and fence names it answers to, where its
tree-sitter grammar comes from (`Grammar::Linked`, one compiled into the
editor, or `Grammar::File`, a `.wasm` in its owner's package), its
highlights query, its comment token, and its LSP server's command and
arguments when it has one.

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
grammar = "grammars/keyflow.wasm"        # or "linked" for one compiled in
highlights = "queries/highlights.scm"
comment = "//"
[languages.server]                        # optional LSP; carried, not started
command = "kf-lsp"
args = ["--stdio"]
```

The app gives the host its registry with `Host::with_languages`
(`features/extensions/src/host.rs`); `Host::languages` hands it back, and
`EXT_LANGUAGES {id? | fence? | path?}` reads it from any runtime
(`features/extensions/src/contrib_actions.rs`).

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

## What the editor cannot do yet

At the revision architect pins, the registry reaches the highlighter only
for languages whose grammar is compiled into the editor:

- `editor:features/syntax/src/lib.rs` `highlight` takes the closed enum
  `Lang`; `grammar_for` (private) is the only source of a grammar and a
  query. There is no entry that takes a tree-sitter `Language` and a query
  text, and nothing loads a grammar from a `.wasm` file.
- `editor:crates/editor-state/src/markdown.rs` asks
  `editor_syntax::Lang::from_fence_tag` for a fence's language directly
  (`emit_fence_tokens`), with no seam for a host's registry.

So `syntax::highlight` answers `Unsupported::GrammarFile`, `OwnQuery` or
`NotLinked` for anything else, and a fence in the component is
highlighted by the editor's own table, which today agrees with
`languages::builtin`. Both are built in the editor repository and arrive
with the pin; nothing of the editor's is copied here.

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
| `features/editor/tests/syntax.rs` | Rust from the registry highlighted in `syntax.*` tokens; what is unsupported says why |
| `features/editor/tests/theme.rs` | every token resolves in a theme store, reaches CSS, and a person restyles one |
| `features/extensions/tests/interop.rs` | `languages_are_registered_while_the_extension_is_on`: enabling adds, disabling withdraws |
| `features/extensions/tests/editor_view.rs` | a linked extension's view holds the editor and hears its changes |

## Status

Steps (a) to (c) of #512, (c) for linked extensions only. Not yet:
`CodeEditor` (d), LSP servers started by the host (e), Keyflow on
`architect-editor` (f), grammars from files, and an editor in a remote
view, a WASI view or a page.
