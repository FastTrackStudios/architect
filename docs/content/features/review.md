+++
title = "Review"
description = "architect-review: comments pinned to a moment, a range or a part of the frame of a media item's video or recording, per version, kept as Threads."
weight = 60
+++

## What it is and when to use it

Review (`features/review`, extension id `review`, service `architect.review`)
is Architect's video and audio review, in the spirit of Lawn: open a cut, watch
or listen, leave a comment at a moment ("0:33 strobe a beat early") or over a
range ("1:00–1:15 the bridge drags"), maybe on a part of the frame, reply,
@mention, resolve, and when the next render lands, carry what is still open
over to it.

It is a linked extension (`features/review/src/lib.rs` `Review`, compiled into
the Architect distribution, `apps/distribution/src/lib.rs` `architect`), with
a process copy for development (`EXT_CLONE {"id":"review"}`).

## The model

- **A media item** is `architect-media`'s: a note naming what it plays
  (`docs/content/features/media.md`). Review reads its `title`,
  `subtitle`/`artist` and `duration`.
- **Tracks and versions.** A note lists what is reviewed in its `review`
  property; each track is one thing rendered again and again, its versions
  oldest first:

  ```yaml
  review:
    - name: Lights
      versions:
        - "[[Joy - Lights v1.mp4]]"
        - { file: "[[Joy - Lights v2.mp4]]", label: final }
    - name: Mix
      versions: ["[[Joy.ogg]]"]
  ```

  Without `review`, each of the note's `video`, `media` and `recording` files
  is a track of one version (`features/review/src/model.rs` `tracks`). A bare
  file name is the note's folder's; a file that is not audio or video is left
  out. `REVIEW_ADD_VERSION` writes the list (wikilinks, so a rename keeps it).
- **A comment is a Threads thread** on the item's note
  (`entityType: note`, `entityId: <the note>`), at `anchorPath`
  `media/<the version's vault path>`, with a **`moment`**: `{start, end?,
  rect?: {x, y, w, h}}`, seconds, the rect 0 to 1 of the frame
  (`features/threads/src/model.rs` `Moment`). `moment` is an optional,
  additive field of Threads (0.1.1): a thread without one is about its anchor
  as a whole, as every thread before it was, and Threads keeps only a valid one
  (finite, 0 to 24 h, an end past its start, a rect inside the frame). So:
  comments sync as notes do, Threads' pane, counts, badges and @mention
  notifications see them, and `THREADS_LIST {path, anchor_path:
  "media/<file>"}` lists one version's. There is no second store.
- **Carry over.** `REVIEW_CARRY {path, from, to?}` copies each unresolved
  comment of `from` to `to` (the next version by default) with its replies
  (reply structure and authors kept, nobody notified: Threads' `notify:
  false`), then posts "Carried to v2." on the old one and resolves it.

## The view

`[[views]] id = "review"` (`features/review/src/views.rs` `ReviewView`):

- the title, the track chips, the version chips (each with its open count),
  and "Carry N open from v1" when the version before has open comments;
- the stage: the frame for a video (16:9) or a card for a recording, the big
  timecode, and the regions of the comments at the playhead; while a comment
  is being written on a video, a 16 × 9 grid takes a drag to mark a part of
  the frame;
- the transport: previous/next comment, −5, Play/Pause, +5, the time, and
  "Comment at m:ss";
- the timeline: what has played, each comment's marker in rows that never
  overlap (`model::lanes`; a range is a span, resolved ones grey), a click on
  a marker seeks to it, a click on the line seeks, a drag along it marks a
  range and opens the composer. It takes clicks in 200 equal cells
  (`model::CELLS`), so it never measures itself: the same in Blitz and in a
  browser;
- the comments on the version, by moment: the moment (press it to seek),
  "▢ frame" when it marks a region, the conversation with replies indented,
  a reply field, Resolve/Reopen, and "Resolved (n)" to show resolved ones.

Keys, with the view focused (fields keep their keys): **J** / ← back 5 s,
**K** / space play or pause, **L** / → on 5 s, **M** a comment at the
playhead, **[** / **]** the comment before / after, **Esc** drops the draft.

**The player is the playhead.** Opening a version puts it on the playhead as
Review's own source (`architect.playhead`), its comments as the item's
`marks`, paused; the view reads the position five times a second and seeks and
toggles through it, so Now Playing and the view move together. Without a
playhead it keeps its own paused clock and comments still work.

## Actions (and the service)

Method `m` of `architect.review` is `REVIEW_<M>`:

| Action | Does |
|---|---|
| `REVIEW_OPEN {path?, file?, open?}` | the view on an item (else the note being read) and a version (else its first track's newest); cues the playhead |
| `REVIEW_ITEM {path}` | title, length, tracks, versions with open/all comment counts |
| `REVIEW_LIST {path, file?, resolved?}` | the comments, by moment |
| `REVIEW_COMMENT {path, file, start, end?, rect?, body, kind?, author?}` | a comment at a moment (`start`/`end` in seconds or `m:ss`); refused for a file that is not a version |
| `REVIEW_ADD_VERSION {path, file, track?, label?}` | a new newest version |
| `REVIEW_CARRY {path, from, to?}` | open comments over to the next version |
| `REVIEW_PLAY {path?, file?, t?, play?}` | the version on the playhead at `t` |

Replies, resolve, reopen, edit and delete are Threads' own actions
(`THREADS_REPLY`, `THREADS_RESOLVE`, …) on the comment's thread id.

## Embedding it (Studio)

Another extension opens Review on one of its items through the service, never
by linking the crate: `services.call("architect.review", "open", {path,
file})` (or the action `REVIEW_OPEN`), which points the one `review` view at
that item and version and opens it as a pane; `open: false` only points it, for
a host that shows the view itself. For a song, Studio names the song's note and
its light-show video (`file`), or lets Review pick its first track's newest.
Today the view shows one item at a time (one per app); a held view (#764) of
`review` inside a remote view is the way to put it in another extension's
page once held views take a per-instance argument.

## Share links and guests

A share page is the normal web app as the link's guest, and Review is the same
view there. Whether a guest may comment is the link's, not the view's:

- The share's `ShareAccess` (`Read` or `Comment`,
  `features/spaces/src/share.rs`, set with `ShareChange::Access`) is what the
  guest app is told as `GuestLink.access` (`read` or `comment`,
  `features/spaces/src/guest.rs`).
- A guest's extensions run within `guest_capabilities`
  (`features/extensions/src/share_needs.rs`): no writes, no app actions.
  Review asks for none it would need as a guest: its writes are Threads'.

Not built yet: the guest's comments themselves. The design: `ShareGuest`
(the node's guest service) gains `threads(code, token, path)` (read, permit
`read` on `shares/{code}`) and `comment(code, token, …)` (permit `comment`),
which the node runs only when the link's access is `comment` and the note is
one the link lists, writing the Threads note under the guest's signed-in name
(or the name they typed, marked as a guest). In the guest app, the
`architect.threads` service is answered by that lane, so Review's view, its
composer and its reply fields work unchanged, read-only unless the link says
`comment`. A per-audience access ("this audience may comment"), capped by the
share's as `listen` is, is the next step after that.

## Platforms

| | Desktop | iOS | Web (signed in) | Share guest |
|---|---|---|---|---|
| Comments, versions, carry over | ✅ linked; tested over a real vault | linked (no process extensions there); not run | linked into the web app's distribution; not run | read-only today; comments not built (above) |
| Audible playback | the playhead's silent clock (native audio is the playhead's to add) | same | the app's `<audio>` element plays a version only where `apps/web/src/app/audio.rs` `media_url` gives a URL (#628) | the link's files |
| The video picture | not drawn (Blitz has no video element) | not drawn | not drawn: an extension view has no URL for a vault file (needs a host call for one) | not drawn |

## Tests to read

- `features/review/tests/review.rs`
  `a_bands_review_of_a_light_show_lives_in_threads`: Threads, the playhead and
  Review linked over a real vault: two tracks, comments at a time and over a
  range with a region on v1, one on v2, a reply; refusals; per-version lists;
  the thread notes' YAML; Threads' counts; the playhead's marks and position;
  a third version written to `review`; v1's open comment carried to v2 with
  its reply, the old one resolved.
- `features/review/tests/review.rs`
  `a_note_without_a_review_list_reviews_its_video_and_recording`: the default
  tracks, no playhead loaded, a range on a recording, a version added to a
  note with no list.
- `features/review/src/model.rs`: tracks and versions, anchors, marker rows,
  carry, keys and clocks.
- `features/threads/src/model.rs`
  `a_moment_is_kept_only_when_valid_and_old_threads_have_none`;
  `features/threads/tests/vault.rs` (a moment through the vault).
- `features/review/tests/shots.rs` (ignored; `SHOTS_DIR=…`): the view at
  390 px and 1280 px, light and dark, rendered headless with Blitz.
