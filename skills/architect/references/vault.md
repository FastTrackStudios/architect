# Vault: notes, properties, links, tags and the index

A vault is a folder of markdown: notes, their properties (YAML frontmatter), the links between them (`[[wikilinks]]`, embeds, block refs) and their tags, plus Obsidian's `.base` views over them. The files on disk are the truth; everything else (the page index, the link graph, a note's CRDT document) is read from them. It moved here from Task with #333 (the crate names are Task's, so Task re-points by path); its rules are `features/vault/spec/vault.md` (tracey ids `vault.*`, `.config/tracey/config.styx`). Licence: MIT OR Apache-2.0, like the rest of architect.

## Pieces

| Crate | What | Entry points |
|---|---|---|
| `vault-proto` (`features/vault/vault-proto`, wasm-clean; `vox` the services) | the wire: byte-level replication and the link-graph read service, their payloads, the names every walker agrees on | `features/vault/vault-proto/src/service.rs` `VaultSync`; `features/vault/vault-proto/src/graph.rs` `VaultGraph`; a node's notes, `features/vault/vault-proto/src/notes.rs` `Notes` (`list`, `read`, `write`, `create`: a new note only where none is, else `NotesError::Exists`; `NoteRef`, `NotesError`; with `vox`; its `NOTES_*` actions with the `actions` feature, which only the node (`apps/desktop`) enables: it pulls in the native dispatcher, and `xtask boundaries` counts it), served by the node on its org lane and called by the web app; `features/vault/vault-proto/src/manifest.rs` `Manifest`; `features/vault/vault-proto/src/consts.rs` `PRIVATE_MARKER`, `is_within_private`; `features/vault/vault-proto/src/collab.rs` `VAULT_DOC_NAMESPACE` |
| `vault-live` (`features/vault/vault-live`, native) | the filesystem vault: snapshot, block index, watcher, the canonical `VaultSync` backend, `.base` parsing and evaluation, typed property schemas | `features/vault/vault-live/src/vault.rs` `Vault` (`open`, `snapshot`, `reload`); `features/vault/vault-live/src/blocks.rs` `BlockIndex`; `features/vault/vault-live/src/sync.rs` `Backend`; `features/vault/vault-live/src/mutate.rs` `write_atomic`, `bind_sink`; `features/vault/vault-live/src/bases.rs` `parse`, `BaseEval`, `view_table`, `register_formula_fn`, `filter_clauses`; `features/vault/vault-proto/src/grouping.rs` `group_by` (a base's `groupBy`, the navigator's list); `features/vault/vault-live/src/property_schema.rs` `PropertySchemaRegistry` (`with_builtins`, `insert`, `validate`); `features/vault/vault-live/src/shelf_boundary.rs` `DEFAULT_SHELF_MARKERS`, `set_shelf_markers`; `features/vault/vault-live/src/write_guard.rs` `WriteGuard` |
| `vault-obsidian` (`features/vault/vault-obsidian`, native) | Obsidian's reading of a folder: parsed pages, the link index (backlinks, unresolved, orphans), properties, aliases, outline, tasks, word count, base queries, frontmatter edits, the link-graph backend | `features/vault/vault-obsidian/src/vault.rs` `Vault`; `features/vault/vault-obsidian/src/links.rs` `LinkIndex`; `features/vault/vault-obsidian/src/mutate.rs` `set_property`, `create_page`, `save_base`; `features/vault/vault-obsidian/src/base_query.rs` `run_base`; `features/vault/vault-obsidian/src/graph.rs` `GraphBackend`; reading a file never through a symlink, `open_own` (re-exported from `vault-live`) |
| `vault-entity` (`features/vault/vault-entity`) | typed records kept as notes: one trait per type, a store with list/get/create/update/delete over an indexed vault | `features/vault/vault-entity/src/store.rs` `VaultEntity`, `VaultEntityStore`; `features/vault/vault-entity/src/index.rs` `PageIndex`; `features/vault/vault-entity/src/slug.rs` `slugify` |
| `vault-doc` (`features/vault/vault-doc`) | a note as a Loro document (body text + props map), its markdown in and out, a device's replica of it, a book of records on documents | `features/vault/vault-doc/src/doc.rs` (the doc shape); `features/vault/vault-doc/src/markdown.rs` `parse`, `materialise`; `features/vault/vault-doc/src/replica.rs` `Replica`; `features/vault/vault-doc/src/book.rs` `Book` |
| `vault-collab` (`features/vault/vault-collab`, native) | every vault file as a co-edited Loro document on architect's `DocRegistry`: files become docs, docs write back to files after a debounce, echoes of its own writes are told apart, a lineage check on attach | `features/vault/vault-collab/src/lib.rs` `VaultCollab` (`new`, `with_debounce`, `registry`, `watch_vault`, `is_echo`), `diff_text_ops` |
| `vault-sync-client` (`features/vault/vault-sync-client`) | a device's pull/push of a vault folder over `VaultSync`: index the local tree, plan against the server's manifest, apply one op at a time | `features/vault/vault-sync-client/src/lib.rs` `index_local`, `plan_sync`, `apply_one`, `SyncSummary` |
| `links-proto`, `links` (`features/links`) | typed links (a relation, confidence and visibility between any two nodes) and their JSONL store, served as `LinksService`; `actions` adds `LINKS_*` over a vault's wikilinks | `features/links/links-proto/src/service.rs` `LinksService`; `features/links/links-proto/src/link.rs` `TypedLink`, `Relation`; `features/links/links-proto/src/node.rs` `NodeRef`, `NodeKind`; `features/links/links/src/store.rs` `Store`; `features/links/links/src/actions.rs` `register` |
| `tag-proto`, `tag` (`features/tags`) | the tag registry (a tag's name, icon and colour, `Records/tags.json` in the vault), served as `TagService`; `actions` adds `TAGS_*` | `features/tags/tag-proto/src/service/tags.rs` `TagService`; `features/tags/tag-proto/src/tag.rs` `Tag`; `features/tags/tag/src/vault_tags.rs` `VaultTags`; `features/tags/tag/src/actions.rs` `register` |
| `vault` (`features/vault/vault`, the facade) | re-exports the rest by feature (`live`, `obsidian`, `vox`); `actions` adds the `VAULT_*` actions | `features/vault/vault/src/actions.rs` `OpenVault`, `register`, `read_note`, `page_tags` |

## Rules it keeps (`features/vault/spec/vault.md`)

- **Lookup is constant-time** (`vault.index.lookup`): by id or by type from `PageIndex`, never by walking the vault.
- **A page is parsed once per change** (`vault.index.parse-once`), keyed by content, not by clock.
- **Indexing is incremental** (`vault.index.incremental`): one changed file costs one parse; no edit rescans.
- **A parse failure costs one page** (`vault.index.tolerant`): it is reported with path and reason and stays listed; the rest load.
- **Writes lock what they touch** (`vault.write.granular`) and **are atomic per file** (`vault.write.atomic`: temp file beside it, then rename; the frontmatter the write did not touch keeps its order and spelling).
- **A private directory stays on its node** (`vault.private.node-only`): a folder with `.fts-private` is in no manifest, walk or replica and is not served by path.
- **Bases follow links** (`vault.bases.links`): `file.links`, `file.hasLink`, `x.exists()`, `note.song.key` through a link, formulas once per row.
- **The index reads links as a renderer does:** a link written from the vault's root (Zola's `@/guide/ui.md`, or `/guide/ui.md`) resolves there; a markdown link's text may wrap a line; a fence right under a line of text is still code, so a `#tag` or `[[link]]` inside it is neither (`features/vault/vault-obsidian/src/links.rs` `root_relative`, `features/vault/vault-obsidian/src/obsidian_parse.rs` `parse_page`). A page's tags, frontmatter and inline: `vault::obsidian::page_tags`. A site built from the vault (`architect-sites`) takes its links, backlinks and tags from here.
- **No symlink is a vault file,** in any view of the vault: Obsidian's walk (`features/vault/vault-obsidian/src/walker.rs` `walk_vault`), the live walk (`features/vault/vault-live/src/walker.rs` `walk_vault`) and the sync manifest behind `VaultSync`, `Notes` (`NOTES_LIST`, `NOTES_READ`) and the doc peer (`features/vault/vault-live/src/sync.rs` `Backend`) list no symlink, inside the vault or out, and walk into no symlinked folder, since a link inside the vault can still name a private folder or `.git`. The rules live in `features/vault/vault-live/src/own.rs`:
  - Read or copy a listed file with `open_own` or `read_own` (no link followed at any step, re-checked on the open handle), never `std::fs::read` or `std::fs::copy` on its path: a site build, a share's zip, the vault's page reads and the backend's `get_file` do. A symlink reads as not found.
  - Write, move or delete only a path `through_symlink` says is free of links: the backend's `put_file`, `delete_file`, `move_file`, `set_folder` and `open_collab` refuse one (`Refused`), so nothing is written through a link or removes one.
  - Hiding a symlink deletes nothing: the watcher reports neither a `Put` nor a `Delete` for a link (a note swapped for one, or a dangling one), so no document is tombstoned and no peer removes its copy; a sync client (`features/vault/vault-sync-client/src/lib.rs` `write_file`) neither indexes a link nor writes through one. Hard links are not told apart.

## What an app supplies

- **Its own kinds.** `PropertySchemaRegistry::with_builtins` has `node`, `person`, `daily` and `project` (#344: projects are architect's; its state, `start`, `due`, `up`, `members`, `icon`); an app `insert`s its own (Task: `task`, `area`).
- **Its formula functions.** `register_formula_fn("name", f)` adds a function to every base (Task: `transpose_key`).
- **Nested roots.** A folder holding `project.md` (`vault_proto::consts::PROJECT_PAGE`) is a project and a root of its own, pruned from its parent's walk and manifest; that is the default (`DEFAULT_SHELF_MARKERS`), so an app sets nothing. An app with another layout calls `set_shelf_markers` once at start (`&[]`: no folder is).
- **Who may write.** A `WriteGuard` on the `Backend`; the services' gate tables are the app's.

## Actions (`vault::actions`, feature `actions`)

`OpenVault::open(folder)` then `register(&list, &open)`:

| action | effect | does |
|---|---|---|
| `VAULT_STATUS` | read | folder, counts, bases that failed to parse, unresolved links |
| `VAULT_LIST {folder?}` | read | notes in path order: path, title, folder |
| `VAULT_READ {path}` | read | properties, body, tags, links out (and where each resolves), backlinks |
| `VAULT_SEARCH {query, limit?}` | read | notes whose title or text holds the query, with the first matching line |
| `VAULT_QUERY_BASE {path, view?}` | read | a `.base`'s views: columns and grouped rows |
| `VAULT_CREATE {path, body?, properties?}` | mutating | a new note (`properties` a JSON object); a taken path is refused |
| `VAULT_SET_PROPERTY {path, key, value}` | mutating | one property; `value` JSON, else text |
| `VAULT_REMOVE_PROPERTY {path, key}` | mutating | removes one property |
| `VAULT_APPEND {path, text}` | mutating | text at the end, on its own line |
| `VAULT_RENAME {from, to}` | mutating | renames or moves a note; every wikilink that named it is rewritten (by path when it was named by path); answers the notes relinked |
| `VAULT_DELETE {path}` | destructive | deletes the note (an agent's call waits for a yes) |
| `VAULT_RELOAD` | mutating | reads the folder again |

An app edits through the same `OpenVault` (`features/vault/vault/src/actions/edit.rs`): `text`, `save` (a note's text, marked as the app's own write), `create`, `untitled` (Obsidian's next `Untitled N.md`), `rename` (relinks, `Renamed`), `copy_file`, `move_folder` (each note through `rename`, so links by path follow), `copy_folder`, `trash` (to the vault's `.trash`, numbered), `folders` (empty ones too), `refresh` (what changed on disk since it was read: `Changes`) and `watch` (an edit outside the app, another editor or a `git pull`, refreshes it and hands over the `Changes`; the app's own saves are no change; dropping the `Watch` stops it). An open note's typing and an outside edit become one text with `features/vault/vault-doc/src/doc.rs` `merge3` (two forks of one Loro document, each side's changes kept). Tests: `features/vault/vault/tests/edit.rs`.

Links and tags, over the same `OpenVault` (`links::actions::register`, `tag::actions::register`):

| action | effect | does |
|---|---|---|
| `LINKS_GRAPH` | read | every resolved wikilink as `{from, to}` |
| `LINKS_BACK {path}` | read | the notes linking to `path` |
| `LINKS_UNRESOLVED` | read | links that name no note, with the note they are in |
| `LINKS_ORPHANS` | read | notes nothing links to |
| `TAGS_LIST` | read | every tag in use, its note count, its icon and colour from the registry |
| `TAGS_NOTES {tag}` | read | notes tagged `tag` or a tag nested under it |
| `TAGS_ADD {paths, tag}` | mutating | tags notes (a note with the tag or one above it is left; the tags below it go) |
| `TAGS_REMOVE {paths, tag}` | mutating | takes the tag itself (any case) off notes |
| `TAGS_CLEAR {paths}` | mutating | takes every tag off notes |
| `TAGS_RENAME {from, to}` | mutating | renames a tag and the tags below it in every note |
| `TAGS_DELETE {tag}` | destructive | takes a tag and the tags below it off every note |

The tag writes edit the markdown (`features/vault/vault-obsidian/src/tag_edit.rs` `edit_tags`, `TagEdit`): the frontmatter's `tags`/`tag` (and `#` aliases) and inline `#tags` as the parser reads them, not in code or links. `OpenVault::edit_tags` and `edit_tags_everywhere` do them from an app (the navigator's `VaultSource`); `OpenVault::remove_property` removes a property. Every frontmatter write (`edit_frontmatter`, `edit_tags`, `VAULT_SET_PROPERTY`) edits the block in place (`features/vault/vault-obsidian/src/frontmatter_write.rs` `write_frontmatter`): only the changed keys' lines are written again, a flow list stays a flow list, quotes stay, and the untouched lines are kept byte for byte.

Tests: `features/vault/vault/tests/actions.rs`, `features/links/links/tests/actions.rs`, `features/tags/tag/tests/actions.rs` (over the sandbox's sample), `features/vault/vault-entity/tests/index.rs` (the index rules).

## In the sandbox

`just sandbox` and `devenv up` open a scratch copy of the demo vault (`apps/ui/demo-vault`, made by `apps/ui/demo-vault.py`: nested folders, an empty one, 300 notes for scroll tests, every property type, nested tags, links, tasks, daily to yearly notes, projects, images and a PDF, bases, inbox captures) at `ARCHITECT_VAULT` under devenv's state, seeded from `ARCHITECT_VAULT_SEED` when missing; `just vault-reset` seeds it again; `APP_STATUS`'s `vault` says which folder is open. Without them the sandbox opens `<its folder>/vault` (`ARCHITECT_SANDBOX_CONFIG_DIR` moves its folder), seeded on first run from `apps/sandbox/vault` (five notes); `apps/ui/src/notes.rs`. Its notes open in the editor and save to their files (`apps/ui/src/vault.rs` `VaultNote`). With a sandbox running: `lab-session.sh act <name> <worktree> run VAULT_READ --arg path=Welcome.md`.

## Still in Task

`NodeKind` still names Task's kinds of node (`Verse`, `Song`, `Sermon`, `Chart`, `Patch`) beside the generic ones; opening it to app-named kinds is a follow-up (a wire and storage change for Task).


Task's own layers on the vault stay there: its task, goal and area types (entity types over `vault-entity`, their property schemas), `transpose_key`, its server wiring (which org's vault a lane reaches, the MCP tools such as `query_base` with their bounds), its vault UI (`task:features/links/links-ui`), the `sync_vault_links` example (scripture references) and the spec rules that are Task's (`task:docs/spec/storage.md`).

Bases (the views over `.base` files, their toolbar, the `BASES_*` actions, view types extensions add) are `architect-bases`: `references/bases.md`.
