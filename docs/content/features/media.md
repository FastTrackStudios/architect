+++
title = "Media"
description = "architect-media: media items and collections read from a note's frontmatter, drawn as a native list that plays on the Now Playing card."
weight = 59
+++

## What it is and when to use it

`architect-media` holds Architect's core media primitives (#668): what any
extension's audio or video work builds on, with nothing music-specific in it.

- A **media item** is a note that names something to play: its `title`, its
  `subtitle` (or `artist`), its `duration` (seconds, or `m:ss`), its media
  (`media`, or `recording`: an audio or video file in the vault) and its
  `art` (or `cover`). Of each pair, the first that names a vault file wins: a
  URL in `media` leaves `recording` to play.
- A **collection** is a note that lists items in order, in named sections.
  Any note carrying the fields below is one, whatever its `type`;
  `type: playlist` is the generic name.

```yaml
---
title: Merry & Bright
type: setlist            # any type; `playlist` is the generic one
sets:                    # or `sections`
  - name: Opening Set
    songs:               # or `items`
      - "[[Songs/Christmas/Sleigh Ride/Sleigh Ride|Sleigh Ride]]"
songs:                   # or `items`: a flat list
  - "[[Songs/Christmas/Sleigh Ride/Sleigh Ride|Sleigh Ride]]"
---
```

`sets`/`songs` is the shape Studio's setlists write (studio#406), and
`sections`/`items` are the generic names. When a note has both sections and
a flat list, links from the flat list that no section names come after the
sections, unnamed: a flat link to a note a section already lists is left out
by the note it resolves to, however it is written. A list is read up to
`collection::MAX_ROWS` (1,000) links, and `Collection::resolve` resolves them
against an index of the paths (`link::Index`), not a walk per link.

Key, tempo, meter, charts, lyrics and arrangement belong to Studio. Core never
reads them.

## A minimal example

```rust
use architect_media::{Collection, Item, collection::{self, Row}, front};

let fields = front::fields(&setlist_text);
let list = Collection::read(&fields).expect("a collection");
// Resolve only against the paths this viewer may read.
let resolved = list.resolve("Setlists/Merry & Bright.md", vault_notes.iter().map(String::as_str));
let rows = resolved.rows(|path| {
    let item = Item::read(path, &front::fields(&read(path)?));
    Some(Row::of(item, vault_files.iter().map(String::as_str)))
});
// Every row that plays, in list order: the playhead's queue.
let queue = collection::queue(&rows);
```

Resolution is against the paths the caller passes and nothing else
(`link::resolve`): a member's whole vault, or only the notes a share link lists
to its guest. A link to anything else stays unresolved; its row shows the
link's alias, greyed, with nothing to play.

## The native list

In the apps, a collection note draws as a list at the note's head, in the place
an extension's claimed view takes. Each row shows the item's cover, its title
over its subtitle, and its length. Each section gets a heading. A row with
nothing to play is greyed. Pressing a row plays it: the whole collection
becomes one queue on the playhead (`architect.playhead`, #589), the playing
row is marked, next and previous follow the list, and the end of an item moves
on to the next.

- An extension whose island claims the note's type (`SiteIsland::embed_note`,
  #659) draws instead of the core list.
- While the core list is drawn, the property table leaves out the
  `sets`/`sections`/`songs`/`items` rows. A reading view folds away a fence
  that repeats the list (`is_listing_fence`: `playlist`, or a name ending
  `-playlist` or `-setlist`, such as Studio's ```` ```studio-setlist ````).

## Tests to read

- `features/media/src/collection.rs`: Studio's named sets with the flat list's
  remainder; the generic names; rows numbered across sections; only what plays
  is queued; a guest resolves only the listed notes.
- `features/media/src/item.rs`: fields and their aliases, lengths as clocks, a
  media link outside the vault is none.
- `features/media/src/link.rs`: wikilinks, markdown links and plain paths; no
  URL, absolute path or `..`; no guess between two notes of one name; an
  index resolves as a walk does.

## Status

The model is done. Not yet: video in a pane, comments at a timestamp, and
collections a Base's query produces.
