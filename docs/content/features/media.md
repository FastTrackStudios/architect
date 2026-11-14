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

Feature `ui` adds the list, `MediaList` (`features/media/src/ui.rs`): Dioxus,
the shell's theme and the playhead it plays on. In the apps, a collection note
draws as this list at the note's head, in the place an extension's claimed view
takes. Each row shows the item's cover, its title
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

Where it draws:

- **The web app** (`apps/web/src/app/collection.rs` `CollectionHead`): a
  member's rows come from their own vault, with the demo's recordings as the
  demo plays them. The list plays as the app's own source `app:list`. The
  note's words show as a reading (`apps/web/src/reading.rs` `blocks`), with
  the listing fence left out and its text area folded under "Edit the note's
  text".
- **The desktop and phone apps** (`apps/ui/src/collection.rs`
  `CollectionHead`): the list is the fallback of the note type's `EmbedView`,
  so an extension that claims the type draws instead. Rows come from the vault
  on disk, and covers are `data:` URLs read from inside it (`art_url`, which
  the Now Playing card uses too). The Properties bar leaves the list's rows
  out (`PropertiesBar`'s `hidden`), and the editor folds a listing fence to one
  line until the caret goes into it (`NoteEditor`'s `fold_listing`,
  `architect_media::listing_fences`). The demo vault on disk gets the demo's
  silent recording at each song's `recording` (`apps/ui/src/vaults.rs`
  `write_demo`), so its setlist plays as the web demo's does. Playback is the
  native silent clock until audible native playback lands.
- **A share link's guest** (`apps/web/src/app/guest.rs`, #679): the list comes
  from what the link's node gives the guest, never from a vault. Each
  `ListedNote` (`features/spaces/src/guest.rs`) carries its `subtitle`,
  `duration_ms` and `media`, and a collection carries its `sets`
  (`ListedSet`). The node resolves each set's links only among the notes the
  link lists (`apps/desktop/src/node/shares.rs` `listing`), so a link to a
  note outside the share is in no set, and a guest never learns of it. Art and
  media are only the files the link serves at its layer. A note's
  `media`/`recording` file is one of its files on a link
  (`features/sites/src/listing.rs`), and its `cover` is its art when it has
  no `art` (`features/sites/src/art.rs` `art_link`). The list plays as the
  link's source `app:share`.
- **Layers.** A recording or cover a note names only by `media:`,
  `recording:` or `cover:` (linked in no note's body) is the Reference
  layer's unless a `[[share_layers]]` rule places it
  (`features/sites/src/listing.rs` `Entry` `listening`,
  `apps/desktop/src/node/share_layers.rs` `LayerRules::place`): Reference is
  where a link serves what an engine plays a song's mix from, so a Page link
  serves no recording or cover it did not serve before, and a Reference link
  (the default) or a Full one serves them. An `art:` image and a file linked
  in a body stay the page's.
- **The words a guest reads.** What the node hands a guest of a note
  (`ShareGuest::read`: its blocks, and its text for a view that reads the
  note) has every listing fence cut to the link
  (`features/media/src/fence.rs` `within`; `apps/desktop/src/node/shares.rs`
  `within_listing`): a row naming no note the link lists is gone, title and
  all, and a file the link does not serve is emptied from a row kept. A
  block an extension drew from such a fence at the build is left out whole.

## Tests to read

- `apps/web/test/app-media-list.test.mjs` (`just app-media-list-test`): in
  `/app/?demo`, Merry & Bright draws as both sets with nine rows, four
  greyed, each cover; no `sets`/`songs` rows; the fence folded; a row pressed
  plays on the card and is marked; the end of a song moves on. The same as a
  guest of a share link, whose node never lists a song outside the share, nor
  names it in the setlist's words (`_guest/read`). The text area keeps what
  was typed when the extension listing arrives late. At 390 px and 1280 px,
  light and dark.
- `apps/desktop/tests/shares.rs`
  `a_guest_never_reads_a_listing_fences_row_for_a_note_outside_the_link`,
  `a_recording_named_only_by_a_property_is_the_references_never_the_pages`.
- `features/media/src/fence.rs`: a row of a note outside is gone, a file
  held back is emptied, a field is a file by its path.
- `apps/web/test/app-ext-host.test.mjs`: an island that claims the setlist
  draws instead of the core list.
- `apps/web/src/guest_view.rs` `collection_rows`: a guest's rows from the
  node's sets, only what the link serves plays.
- `apps/ui/src/collection.rs`: the demo vault's setlist lists two sets and
  nine rows, four greyed, every cover as a `data:` URL, nothing outside the
  vault.
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
