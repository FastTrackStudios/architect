# Vault: notes, properties, links, tags and the index

A vault is a folder of markdown: notes, their properties (YAML frontmatter), the links between them (`[[wikilinks]]`, embeds, block refs) and their tags, plus Obsidian's `.base` views over them. The files on disk are the truth; everything else (the page index, the link graph, a note's CRDT document) is read from them. It moved here from Task with #333 (the crate names are Task's, so Task re-points by path); its rules are `features/vault/spec/vault.md` (tracey ids `vault.*`, `.config/tracey/config.styx`). Licence: MIT OR Apache-2.0, like the rest of architect.

## Pieces

| Crate | What | Entry points |
|---|---|---|
| `vault-proto` (`features/vault/vault-proto`, wasm-clean; `vox` the services) | the wire: byte-level replication and the link-graph read service, their payloads, the names every walker agrees on | `features/vault/vault-proto/src/service.rs` `VaultSync`; `features/vault/vault-proto/src/graph.rs` `VaultGraph`; `features/vault/vault-proto/src/manifest.rs` `Manifest`; `features/vault/vault-proto/src/consts.rs` `PRIVATE_MARKER`, `is_within_private`; `features/vault/vault-proto/src/collab.rs` `VAULT_DOC_NAMESPACE` |
| `vault-live` (`features/vault/vault-live`, native) | the filesystem vault: snapshot, block index, watcher, the canonical `VaultSync` backend, `.base` parsing and evaluation, typed property schemas | `features/vault/vault-live/src/vault.rs` `Vault` (`open`, `snapshot`, `reload`); `features/vault/vault-live/src/blocks.rs` `BlockIndex`; `features/vault/vault-live/src/sync.rs` `Backend`; `features/vault/vault-live/src/mutate.rs` `write_atomic`, `bind_sink`; `features/vault/vault-live/src/bases.rs` `parse`, `BaseEval`, `view_table`, `register_formula_fn`; `features/vault/vault-live/src/property_schema.rs` `PropertySchemaRegistry` (`with_builtins`, `insert`, `validate`); `features/vault/vault-live/src/shelf_boundary.rs` `set_shelf_markers`; `features/vault/vault-live/src/write_guard.rs` `WriteGuard` |
| `vault-obsidian` (`features/vault/vault-obsidian`, native) | Obsidian's reading of a folder: parsed pages, the link index (backlinks, unresolved, orphans), properties, aliases, outline, tasks, word count, base queries, frontmatter edits, the link-graph backend | `features/vault/vault-obsidian/src/vault.rs` `Vault`; `features/vault/vault-obsidian/src/links.rs` `LinkIndex`; `features/vault/vault-obsidian/src/mutate.rs` `set_property`, `create_page`; `features/vault/vault-obsidian/src/base_query.rs` `run_base`; `features/vault/vault-obsidian/src/graph.rs` `GraphBackend` |
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

## What an app supplies

- **Its own kinds.** `PropertySchemaRegistry::with_builtins` has `node`, `person` and `daily`; an app `insert`s its own (Task: `task`, `project`, `area`).
- **Its formula functions.** `register_formula_fn("name", f)` adds a function to every base (Task: `transpose_key`).
- **Its nested-root markers.** `set_shelf_markers(&["project.md"])` once at start makes a folder holding one a root of its own, pruned from its parent's walk and manifest; with none set, no folder is.
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
| `VAULT_DELETE {path}` | destructive | deletes the note (an agent's call waits for a yes) |
| `VAULT_RELOAD` | mutating | reads the folder again |

Links and tags, over the same `OpenVault` (`links::actions::register`, `tag::actions::register`):

| action | effect | does |
|---|---|---|
| `LINKS_GRAPH` | read | every resolved wikilink as `{from, to}` |
| `LINKS_BACK {path}` | read | the notes linking to `path` |
| `LINKS_UNRESOLVED` | read | links that name no note, with the note they are in |
| `LINKS_ORPHANS` | read | notes nothing links to |
| `TAGS_LIST` | read | every tag in use, its note count, its icon and colour from the registry |
| `TAGS_NOTES {tag}` | read | notes tagged `tag` or a tag nested under it |

Tests: `features/vault/vault/tests/actions.rs`, `features/links/links/tests/actions.rs`, `features/tags/tag/tests/actions.rs` (over the lab's sample), `features/vault/vault-entity/tests/index.rs` (the index rules).

## In the lab

The lab opens `<its folder>/vault` (`ARCHITECT_LAB_CONFIG_DIR` moves its folder; `ARCHITECT_LAB_VAULT` names another vault), seeded on first run from `apps/lab/vault` (five notes with properties, links and tags, one unresolved link, a `.base`); `apps/lab/ui/src/notes.rs`. With a lab running: `lab-session.sh act <name> <worktree> run VAULT_READ --arg path=Welcome.md`.

## Still in Task

`NodeKind` still names Task's kinds of node (`Verse`, `Song`, `Sermon`, `Chart`, `Patch`) beside the generic ones; opening it to app-named kinds is a follow-up (a wire and storage change for Task).


Task's own layers on the vault stay there: its task, project, goal and area types (entity types over `vault-entity`, their property schemas), `transpose_key`, `project.md` as the shelf marker, its server wiring (which org's vault a lane reaches, the MCP tools such as `query_base` with their bounds), its vault UI (`task:features/links/links-ui`), the `sync_vault_links` example (scripture references) and the spec rules that are Task's (`task:docs/spec/storage.md`).
