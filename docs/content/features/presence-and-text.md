+++
title = "Presence and collaborative text"
description = "Who is here and where: pointers as places in a view's coordinates, selections, carets and playheads; text that merges, with remote carets."
weight = 24
+++

## What it is and when to use it

Presence is who is in a document and what they are pointing at, selecting,
typing in or playing. It travels on the document's ephemeral presence
channel, never in the document. A pointer is a `Place`: a named view space
with named coordinates (`session.arrange`, time 12.5, track "Kick"), never
pixels, so each view draws it in its own pixels. `Here` publishes this
person's state, throttled; `Roster` collects everyone else's and draws
pointers a little behind and between samples so they move smoothly.

Collaborative text is `TextBinding`: it binds one `LoroText` to an editor.
Local edits become operations; others' edits come back transformed against
what the editor has queued, and carets travel as stable cursors, so a remote
caret follows its character. `#[architect(crdt(text))]` on a `String` field of
a `crdt` entity makes it mergeable text instead of last-writer-wins. Design:
[Documents](@/architecture/documents.md) (collaborative text, presence and
cursors) and [Collaboration](@/architecture/collaboration.md).

<!-- media: presence-pointers-carets — two people in one note: the other's coloured pointer moving over the view and their caret following the text they type -->

## A minimal example

`crdt::presence` and `crdt::text` are wasm-clean; `PresencePeer` needs
`crdt/vox`; the hooks `crdt/dioxus`. After `features/crdt/crdt/tests/collab.rs`:

```rust
use crdt::presence::{Here, PeerState, Place, Roster, clock_ms};
use crdt::sync::PresencePeer;
use crdt::text::{TextBinding, TextOp};

let (peer, mut driver) = PresencePeer::new(doc_id, 30_000);
tokio::spawn(async move { let _ = driver.run(&presence_client).await; });

let here = Here::new(peer.clone(), "ann", PeerState::new("ann", "Ann"));
here.pointer(Place::new("session.arrange").with("time", 12.5).with("track", "Kick"), clock_ms());

let binding = TextBinding::new(doc.loro(), doc.loro().get_text("body"));
binding.on_local(&[TextOp::Insert { pos: 0, text: "Hi".into() }])?;
binding.sync_into(&mut editor);             // editor: impl EditorBinding, after each revision
here.edit_state(clock_ms(), |s| s.caret = binding.caret(0, 2));

let mut roster = Roster::default();
roster.sync("ann", &peer.states(), clock_ms());
let carets = binding.remote_carets(&roster);
let dots = roster.pointers(clock_ms(), |p| {
    (p.space == "session.arrange").then(|| (p.num("time")? * 10.0, 0.0))
});
```

In Dioxus: `use_presence_channel_keyed(doc_id, 30_000)`, then `use_here`,
`use_roster` and `use_presence_frame`. To draw over a workspace pane, turn
the roster into `PresenceMarks`, `use_presence_overlay().set(marks)`, and put
`PresenceLayer { overlay }` as the view's last child.

## Cursors come with the pane

A view does not have to do any of the above to have cursors. Inside a
`PaneRoom` (`use_pane_room`, provided once by the app over the presence
channel of what it has open) every workspace pane publishes this person's
pointer and draws everyone else's, and each tab shows a dot for every person
whose pane it is. The feature is `architect-workspace/presence`.

- **No code:** a pane's space is its own box, `pane:<view kind>:<title>`, with
  the pointer as fractions of it. Two people's panes of one kind and title are
  one place, whatever their sizes.
- **One line, for a view that shows a thing:** `use_pane_subject(path)` names
  what the pane shows (a note's path, a setlist's id) in place of its title.
- **A view that scrolls or zooms** declares its space on every render, and a
  pointer then stays on the same bar of the same track at any zoom:

```rust
use architect_workspace::pane_space::{Axis, ViewSpace};
use architect_workspace::use_pane_space;

use_pane_space(ViewSpace::mapped(
    "session.arrange",
    Axis::linear("time", scroll_seconds, seconds_per_px),
    Axis::lanes("track", track_ids, ruler_px, lane_px),
));
```

- **An extension in any runtime** (a process, a WASI component, a page, Luau)
  does the same with actions, under the capability `actions = ["CURSORS_"]`:
  `CURSORS_SPACE_SET {pane, space, x, y}` and `CURSORS_SPACE_CLEAR {pane}`
  declare a pane's space, `CURSORS_HERE {place}` publishes a pointer from a
  view that draws its own surface, and `CURSORS_ROSTER {space?}` reads who is
  here, with a `revision` to poll on. The app puts them on its list with
  `architect_workspace::pane_space::register`.

Selections with a range in a declared space are washed in the person's colour
the same way. Not yet: a roster-changed event (poll `CURSORS_ROSTER`'s
revision), remote carets in an extension's own editor, and the navigator's
"who has this open". The Architect app mounts the room
(`apps/ui/src/cursors.rs` `use_app_cursors`) over a channel of its own until
it joins a vault's presence channel, so nobody else is on it yet.

## Code tour

1. `features/crdt/crdt/src/presence.rs` `Place` (`new`, `with`, `num`): a
   pointer as a place in a view space.
2. `features/crdt/crdt/src/presence.rs` `PeerState`, `Selection`, `TextCaret`,
   `PlayState`: what a person publishes; colour from `color_for`.
3. `features/crdt/crdt/src/presence.rs` `Here` (`pointer`, `edit_state`,
   `play`, `leave`) over a `PresenceSink`, with a throttle that never drops the
   last value.
4. `features/crdt/crdt/src/presence.rs` `Roster` (`sync`, `pointers`,
   `selections_in`, `carets_in`) and `PointerTrail`.
5. `features/crdt/crdt/src/text.rs` `TextBinding` (`on_local`, `pull`,
   `sync_into`, `caret`, `remote_carets`, `set_composing`), `EditorBinding`.
6. `features/crdt/crdt/src/codec.rs` `write_text`, `read_text_or_legacy`,
   `apply_text_ops`: what `crdt(text)` uses.
7. `features/crdt/crdt/src/hooks.rs` `use_presence_channel_keyed`, `use_here`,
   `use_roster`, `use_presence_frame`.
8. `features/workspace/src/host/presence.rs` `PresenceLayer`,
   `use_presence_overlay`, and `features/workspace/src/presence.rs` `PresenceMarks`.
9. `features/workspace/src/pane_space.rs` `ViewSpace`, `Axis`, `marks`,
   `PresenceBoard`, `register`; `features/workspace/src/host/pane_presence.rs`
   `PaneRoom`, `use_pane_room`, `use_pane_space`, `use_pane_subject`.

An editor reports its edits as `TextOp`s to `on_local`. The binding
transforms them against remote operations it has queued and commits with its
own origin, so its subscription skips them; `SyncedDoc` carries the update.
On the other side the subscription queues the operations, and `pull` or
`sync_into` hands them to the editor (or one replacement if the editor
drifted). A pointer goes the other way: `Here::pointer` throttles and writes
it to the presence peer; the receiver's `Roster::sync` adds the sample to a
trail, and `pointers(now, place)` interpolates it and maps it into that view's
pixels.

## Tests to read

- `features/crdt/crdt/tests/collab.rs`: two people edit one note and see each
  other's carets; a pointer reaches the other side as a place in a view.
- `features/crdt/tests/derive/tests/crdt_text.rs`: concurrent edits keep both;
  plain strings stay last-writer-wins; a legacy string upgrades on write.
- Unit tests in `features/crdt/crdt/src/text.rs` (typing over queued edits,
  composition, a caret following its character, a convergence proptest) and in
  `features/crdt/crdt/src/presence.rs` (trails, the throttle).

## Status

Done in architect, including the workspace overlay. Not yet: a shared clock,
follow mode, an org-wide roster, `editor-crdt` implementing `EditorBinding`.
Positions are Unicode scalar offsets; an editor that counts UTF-16 converts at
its edge. The pane's own cursors are tested headlessly
(`features/workspace/tests/pane_cursors.rs`).

- **Task** uses the presence channel hooks (`task:crates/ui/src/collab.rs`)
  with its own roster UI; `Here`, `Roster`, `TextBinding` and `crdt(text)` are
  still to adopt.
- **Studio** has its own presence (`studio:features/session/src/sync/presence.rs`);
  its arrangement and chart editor are still to move onto these.
