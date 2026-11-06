+++
title = "Sites"
description = "Sites: any vault published as a static site, with working links, tags, backlinks, search and a page's contents, in architect's themes, readable on a phone. Extensions draw the blocks they claim at build time, and the architect server hosts the result."
weight = 56
[taxonomies]
tags = ["sites", "vault", "extensions", "server"]
+++

## What it is and when to use it

Sites publishes a vault (a folder of markdown notes, Obsidian's or a docs
folder written for Zola) as a static site of finished HTML. A reader's
browser gets complete pages and needs no markdown parser, no vault and no
renderer. These docs are the first site built with it, from
`docs/content`.

Use it to publish documentation or a wiki from a repo, or to host a
project's docs from the server it already runs.

It comes in three parts:

- **The engine**, `architect-sites` (`features/sites`). It reads a vault,
  draws every page and writes the site. It moved here from Task's
  `task:features/ssg` (its spec: `task:features/ssg/spec/ssg.md`) and now runs on
  architect's vault.
- **The command**, `architect` (`apps/cli`). `architect site build`
  builds a site once, and `architect site serve` serves it on localhost,
  rebuilding and reloading the browser on every save.
- **Hosting**: `EngineHost::site` serves a built site over the architect
  server's HTTP face, with cache headers.

**The vault does the reading.** Its walker finds the notes and
attachments. Its link index (`LinkIndex`) resolves every `[[wikilink]]`,
markdown link and Zola `@/page.md` link, and gives the backlinks. Its
parser gives the tags. A wikilink the index leaves unresolved still
resolves if it names a note's file or title, in any case and with spaces
or dashes (`[[Key Meter Changes]]` finds `key-meter-changes.md`), as Task's
guides expect. A link that names nothing is reported against its note and
drawn as marked text. A `--strict` build fails on it.

**Extensions draw their blocks.** A fenced block an extension claims is
drawn by that extension at build time, as static HTML. The Board's
`[[embeds]]` entry claims ```` ```kf ```` fences
(`apps/lab/board/extension.toml`), and its page `apps/lab/board/pages/kf.tsx`
draws this one. The page runs once per block, and the element tree it draws
is written out as HTML, the same tree a page draws in a pane:

```kf
[Verse]
| C | G | Am | F |
| C | G | F | F |
[Chorus]
| F | G | C | Am |
| F | G | C | C |
```

**The layout is pure components.** Each Dioxus component in
`architect_sites::ui` depends only on the built site. None has hooks,
state or I/O. A pre-rendered route and a hydrated one therefore always
agree, which the browser islands of the web page runner will need. Search,
the theme toggle and the phone menu are one small script over the finished
page.

## A minimal example

Build these docs and serve them with live reload:

```sh
just docs          # architect site build --vault docs/content --out target/site
just docs-serve    # architect site serve, on http://127.0.0.1:4321/
```

Build any other vault:

```sh
cargo run -p architect-cli -- site build --vault ~/notes --out /tmp/notes-site
```

From Rust, register what draws your blocks and build:

```rust,ignore
use architect_sites::{Blocks, Drawn, FnBlock, Options, PageBlocks};

let blocks = Blocks::new()
    // An extension's embed pages, for the fences they claim.
    .with(PageBlocks::load("apps/lab/board".as_ref())?)
    // Or a Rust function, for a fence of your own.
    .with(FnBlock::new("my.app", &["shout"], |block| {
        Ok(Drawn::Html(format!("<p>{}</p>", block.source.to_uppercase())))
    }));
let report = architect_sites::build(&Options::new("docs/content", ""), &blocks, "target/site".as_ref())?;
```

Host a built site from your server:

```rust,ignore
EngineHost::new(router, gate, "0.0.0.0:4040")
    .site("/docs", "target/site")
    .serve()
    .await?;
```

## Code tour

1. **`features/sites/src/site.rs`**, `Site::read`. It opens the vault,
   reads each note's frontmatter (`Meta`: YAML or TOML; `weight` or
   `order`; `tags`; `draft: true` or `publish: false` hides a note), and
   lays the folders out in reading order. Each folder's page is its
   `_index.md`, `index.md` or `README.md`, or a generated one. Then it
   draws every note with links resolved through the index, and collects
   tags, backlinks and problems.
2. **`features/sites/src/markdown.rs`**, `render`. One pulldown-cmark pass.
   It resolves links, gives every heading an `id` (Zola's rule, so anchors
   written for Zola still land) and hands each claimed fence to `Blocks`.
3. **`features/sites/src/blocks.rs`**: `BlockRenderer`, `Blocks`, `Drawn`
   and `FnBlock`. **`features/sites/src/page_blocks.rs`**, `PageBlocks`, is
   an extension's embed page drawing a block. **`features/sites/src/element_html.rs`**,
   `elements_to_html`, writes a page's element tree out as HTML. It escapes
   text, and drops handlers, scripts and `javascript:` URLs.
4. **`features/sites/src/ui.rs`**: the pure components (the header, the
   navigator-style sidebar, the page, its contents, tags, backlinks, and
   previous and next).
5. **`features/sites/src/write.rs`**, `write`. It writes the pages, tag
   pages, `404.html`, a sitemap (with `--url`) and the copied attachments.
   The stylesheet (the theme's tokens, then `features/sites/assets/site.css`),
   the script and the search index go under `_site/`, named by content. The
   site is built beside `--out` and swapped in whole. A folder the command
   did not write is never replaced.
6. **`crates/architect/src/host/site.rs`**: `EngineHost::site`, `site_router`
   and `cache_policy`. Pages get `no-cache`, `_site/` assets
   `public, max-age=31536000, immutable`, and other files an hour. A
   missing path gets the site's `404.html` with a 404 status.
7. **`apps/cli/src/main.rs`**: the `architect` command. `site serve` serves
   `site_router` and pushes one event per rebuild on `/_architect/reload`.

## Tests to read

- `features/sites/tests/build.rs`: a small vault built end to end. It
  covers TOML and YAML frontmatter, a wikilink resolved by name, a Zola
  link, backlinks, tags, an attachment, a drawn block, a skipped draft, a
  reported broken link, a strict build failing, and a rebuild that never
  replaces another folder.
- `features/sites/src/markdown.rs` (its tests): code stays verbatim,
  headings get unique ids, and a failed block is reported.
- `crates/architect/tests/host_site.rs`: a site served at the root and
  nested, with its cache headers, its 404 page, and the host's own routes
  still winning.
- `features/vault/vault-obsidian/src/links.rs`, the test
  `root_relative_links_resolve_from_the_vault_root`: Zola's `@/` links are
  edges in the index.

## Status

Done in architect (#364, first part):

- `architect site build` and `architect site serve` (live reload), with
  `just docs` and `just docs-serve`;
- pages, the navigation tree, tags, backlinks, search and a page's
  contents, in architect's themes, light and dark, readable on a phone;
- ```` ```kf ```` blocks drawn by the Board's own page, through
  `BlockRenderer`. A ```` ```base ```` view plugs in the same way, as a
  `BlockRenderer` that claims `fence:base` and reads the vault;
- `EngineHost::site` and `site_router`, with cache headers.

Still to do for #364: building another repo's docs folder
(`--repo github.com/<owner>/<repo> --path docs`), blocks that become
interactive islands in the browser (#325), base views (#346), an
extension's own theme tokens on its blocks, the link graph, and deploying
architect.fasttrackstudio.app from FastTrackStudios/architect-deploy. The
Sites extension that manages all of this from the app is #374.
