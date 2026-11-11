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
(`apps/sandbox/board/extension.toml`), and its page `apps/sandbox/board/pages/kf.tsx`
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

A ```` ```base ```` fence is drawn the same way, as its views over the
vault. Its YAML, or one line naming a `.base` file, is a base as
[Bases](@/features/bases.md) reads one. The vault's own evaluator runs each
view, and the site writes it out as a table, cards or a list, each row
linked to its note's page. Bases draws it unless an extension claims
`fence:base` first, and `Options::bases` turns it off. This one lists the
first chapters of the guide:

```base
filters:
  and:
    - file.inFolder("guide")
views:
  - type: list
    name: From the guide
    limit: 4
```

**Media plays, and lyrics follow it.** An embedded audio or video
attachment (`![[take.mp3]]`) is the browser's own player, marked
`data-site-media`. A ```` ```lrc ```` fence, or an embedded `.lrc`
attachment, is synced lyrics: parsed at build time
(`architect_sites::lyrics::parse_lrc`: `[mm:ss.xx]` stamps, several to a
line, `[offset:]`, word stamps dropped) and drawn as lines, each with the
second it starts. The page's script plays them: the line at the playhead
is marked and scrolled into view, a click on a line seeks. This one
follows nothing, as there is no audio on this page; click a line and it
waits:

```lrc
[00:00.00]Joy to the world
[00:03.20]The Lord is come
[00:06.40]Let earth receive her King
```

Every block on a page can follow the same playhead, an extension's too
(`architect.playhead`; [views that follow
playback](@/architecture/extensions.md#views-that-follow-playback)). A share
link adds a queue across its notes ([a link that
plays](@/guide/deploy.md#a-link-that-plays)).

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

Publish one part of a vault (`Options::scope`, `architect_sites::Scope`):
a folder (rooted there, so its own note is the home page), a tag (nested
tags included), a list of notes, or the notes a `.base` file's views select.
The whole vault is still read, so links resolve, but only the scope's notes
become pages. A link to a note outside the scope is drawn as plain text
(`site-outside`, with no URL and no title), an embed of one is left out
entirely (its alt text too), and it is never reported as a problem. No
backlink, base row, search entry, tag page or feed lists a note outside it.
An attachment is copied only when a note in the scope links to it or embeds
it, and under a folder scope only when it is in the folder. A one-note scope
makes that note the home page. A share link is a scoped site
([deploy](@/guide/deploy.md)).

A filter narrows a scope further (`Options::filter`, #510): a Bases filter
expression, as in a `.base` file's `filters:` (`instrument == "violin"`).
Only the notes it holds for are published, and the rest are outside the
site exactly as notes outside the scope are. `features/sites/src/listing.rs`
gives the same selection as a list: `select` (the notes in list order, each
with the files it attaches), `groups` (by order, by note or by a property,
with each file's path in an archive) and `property_keys`.
`features/sites/src/archive.rs` `ZipWriter` writes such an archive as a
stream, with no I/O of its own. A share's named audiences, its files page
and its "Download all" are built from these.

```sh
cargo run -p architect-cli -- site build --vault ~/notes --folder Album --base /album/ --out /tmp/album
# or --tag mix, --note Album/track.md (repeatable), --base-scope Mixes.base
```

From Rust, register what draws your blocks and build:

```rust,ignore
use architect_sites::{Blocks, Drawn, FnBlock, InstalledBlocks, Options, PageBlocks};

let blocks = Blocks::new()
    // Every extension installed in a folder: embed pages, and (feature
    // `wasi`) a WASI component's embed views (`ComponentBlocks`).
    .with(InstalledBlocks::new("/srv/architect/extensions"))
    // An extension's embed pages, for the fences they claim.
    .with(PageBlocks::load("apps/sandbox/board".as_ref())?)
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
   an extension's embed page drawing a block. **`features/sites/src/component_blocks.rs`**,
   `ComponentBlocks` (feature `wasi`), mounts a WASI component's embed view
   once per block, in the extensions host's sandbox, and applies its edits
   without a window. **`features/sites/src/installed.rs`**, `InstalledBlocks`,
   is a folder of installed extensions, both kinds, read again when it
   changes (the node's, #488). **`features/sites/src/element_html.rs`**,
   `elements_to_html`, writes a page's element tree out as markup and
   hands it to **`features/sites/src/sanitize.rs`**, `sanitize` (for a
   block, `sanitize_block`, with the block's scope class), which
   re-writes whatever a renderer drew by allow-lists, deciding on decoded
   values:
   - **Elements:** inside an `<svg>`, its drawing elements (`svg`, `g`,
     `path`, `rect`, `circle`, `ellipse`, `line`, `polyline`, `polygon`,
     `text`, `tspan`, `textPath`, `defs`, `use`, `symbol`, `clipPath`,
     `mask`, `pattern`, `linearGradient`, `radialGradient`, `stop`,
     `marker`, `title`, `desc`, `image`, `a`, `style`); outside, basic
     HTML (text and headings, lists, tables, `a`, `img`, `audio`, `video`,
     `source`, `figure`, `details`, `time`, and `button` for the lyrics
     block). Scripts, form fields, SMIL animation, `<foreignObject>`,
     frames, `<template>`, `<noscript>` and raw-text elements go with
     their contents; any other element is left out, its contents kept.
   - **Attributes:** `class`, `title`, `lang`, `dir`, `role`, `aria-*`,
     `data-*` and `id` (but not the page's own: `site-*`, `share-*`,
     `comments`, `data-site-*`, `data-act`, nor a name the page's scripts
     read off `window`: `architect`, `addEventListener`, `EventSource`,
     `matchMedia`, …), an SVG's geometry and presentation, a few per
     element (`alt`, `controls`, `colspan`, …) and a sanitised `style`.
     Never `on…`, `srcset`, `ping`, `action`.
   - **URLs:** relative, `#…`, `https:` and `mailto:` for a link,
     relative or `https:` for media, and for an image (`<img src>`, an
     SVG `<image href>`) only relative or a raster `data:image/…`: a block
     loads no image from another site, which would be a beacon (a note's
     own images are the site's content and may). `<use>` only `#id`.
   - **CSS,** deliberately blunt. Nothing a block writes as CSS (a sheet,
     a `style` attribute, a presentation attribute) may hold a quote, a
     comment, a CSS escape, `var(`, a custom property (`--name`) or an
     at-rule (`@…`), so the text checked is the text a browser reads;
     nor `url(`, `image-set(`, `image(`, `src(`, `cross-fade(`,
     `element(`, `paint(`, `expression(`, `-moz-binding` or `behavior`;
     nor a `position` other than `static` or `relative`. A presentation
     attribute may use exactly `url(#id)`, pointing into its own drawing.
     A `style` attribute holding a quote, comment, escape or brace is
     dropped whole; otherwise it loses each declaration refused. A
     `<style>` sheet is kept only inside an `<svg>` of a block, only as
     flat `selector { declarations }` rules, and every selector is put
     under the block's own scope class (`.site-block-… sel`); a sheet
     with a nested or unbalanced brace, a selector naming `:root`,
     `html`, `body` or `:host` or starting with a combinator, or any
     declaration refused is dropped whole.
   - **The box.** The site, not the block, wraps each block in
     `<div class="site-block site-block-…" style="…">`
     (`features/sites/src/markdown.rs`; the class is `Block::scope`, the
     style `BLOCK_BOX_STYLE`: `contain: layout paint style`,
     `overflow: auto`, `position: relative`, `isolation: isolate`, the
     first three `!important`). Whatever a block draws is laid out,
     painted and stacked inside it: a block cannot cover or restyle the
     page, and what overflows scrolls inside the box.

   For extension authors: style a block with plain declarations, colours
   as values (`#f5a623`, `currentColor`), unquoted generic font families
   and attribute selectors (`[aria-current=time]`), or with presentation
   attributes on the SVG; theme variables (`var(--…)`) and `@font-face`
   do not come through, nor images from other sites (embed them as
   `data:` or ship them with the note).

   A block's markup comes from extensions and notes, so neither can be
   made to run away: `sanitize` takes time and space linear in its input
   on any input (one loop pass per byte at most, at most 256 elements open
   at once), an element tree is written no deeper than 256 levels and to
   at most 100,000 elements and 16 MiB, and a component's drawing stays a
   tree (an edit that puts one node in two places, or that would make a
   tree deeper than 256 levels, is refused, so no edit walks further up
   than that), holds at most 50,000 nodes and 16 MiB as built and as
   written, and stops at the 10 s draw deadline.
   **`features/sites/src/bases.rs`**, `BaseBlocks`, draws a ```` ```base ````
   fence's views with `vault::bases::view_table`.
4. **`features/sites/src/ui.rs`**: the pure components (the header, the
   navigator-style sidebar, the page, its contents, tags, backlinks, and
   previous and next).
5. **`features/sites/src/write.rs`**, `write`. It writes the pages, tag
   pages, `404.html`, a sitemap (with `--url`) and the copied attachments.
   The stylesheet (the theme's tokens, then `features/sites/assets/site.css`),
   the script and the search index go under `_site/`, named by content. The
   site is built beside `--out` and swapped in whole. A folder the command
   did not write is never replaced.
6. **`features/sites/src/media.rs`** and **`features/sites/src/lyrics.rs`**:
   which attachments play, and synced lyrics (`LyricsBlocks`, after the
   caller's renderers, so an extension claiming `fence:lrc` wins). The
   player is `features/sites/assets/player.js`, shipped in the site's one
   script.
7. **`crates/architect/src/host/site.rs`**: `EngineHost::site`, `site_router`
   and `cache_policy`. Pages get `no-cache`, `_site/` assets
   `public, max-age=31536000, immutable`, and other files an hour. A
   missing path gets the site's `404.html` with a 404 status.
8. **`apps/cli/src/main.rs`**: the `architect` command. `site serve` serves
   `site_router` and pushes one event per rebuild on `/_architect/reload`.

## Tests to read

- `features/sites/tests/build.rs`: a small vault built end to end. It
  covers TOML and YAML frontmatter, a wikilink resolved by name, a Zola
  link, backlinks, tags, an attachment, a drawn block, a skipped draft, a
  reported broken link, a strict build failing, and a rebuild that never
  replaces another folder.
- `features/sites/src/markdown.rs` (its tests): code stays verbatim,
  headings get unique ids, and a failed block is reported.
- `features/sites/src/lyrics.rs` (its tests): LRC stamps, offsets and the
  lines' markup; `features/sites/src/markdown.rs`
  `media_embeds_are_native_players_and_lyrics_are_lines`.
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
- ```` ```kf ```` blocks drawn by the Board's own page, and
  ```` ```base ```` blocks drawn as their views (`BaseBlocks`), both
  through `BlockRenderer`;
- `EngineHost::site` and `site_router`, with cache headers.

Still to do for #364: building another repo's docs folder
(`--repo github.com/<owner>/<repo> --path docs`), blocks that become
interactive islands in the browser (#325), a base's board and calendar
views (drawn as tables for now), an
extension's own theme tokens on its blocks, the link graph, and deploying
architect.fasttrackstudio.app from FastTrackStudios/architect-deploy. The
Sites extension that manages all of this from the app is #374.
