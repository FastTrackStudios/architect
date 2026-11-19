+++
title = "A share link is the app"
description = "A share link opens the Architect web app itself, as a guest of that link: the same app, views and extensions, with some permissions off. The rule, what the permissions are, and where the code still has a second implementation."
weight = 83
+++

**A share link is the Architect web app, opened as a guest of that
link.** It is not a separate site, viewer or app. `https://<node>/s/<code>/`
answers the web app's own page (`apps/desktop/src/node/share_shell.rs`),
and the app opens the link as its guest (`apps/web/src/lib.rs`
`share_route`, `Route::Share`). In the native app a link opens the same
way. The caller is `Principal::Guest` for that link
(`features/permissions/src/lib.rs` `Principal`) (#629, #646).

It is the **same app, the same views and the same extensions**. What a
guest can't do is a **permission**, never a different app:

| What a guest can't do | The permission that says so |
| --- | --- |
| See notes outside the link, or another audience's slice | the share's scope and audience, checked on the node for every request (`apps/desktop/src/node/shares.rs` `Shares::open`, `admit`) |
| See files above the link's layer | the layer (`[[share_layers]]`, #528) |
| Edit, rename, move or delete | the share is read-only (`ShareAccess`); a guest's capabilities carry no `writes` |
| Comment | allowed only when the share's access is "comments" |
| Run the person's actions, reach their vaults, folders, ports, secrets | an extension's capabilities for a guest are `features/extensions/src/share_needs.rs` `guest_capabilities`: the manifest's asks within read-only documents of the share, `GUEST_UI`, and the hosts the owner approved |
| Open a link for signed-in people without signing in | the audience (`org`, `invited:<emails>`): the node answers `401` / `403`, the app offers the sign-in |

## The rule for code and agents

- **Never build a guest-only renderer, page, view or extension loader.**
  A guest sees the link through the app's own Navigator, workspace, note
  views and the extensions that claim a note's type.
- **Gate, don't fork.** When a guest must not do something, add the gate
  (a capability left out of `guest_capabilities`, an action refused for
  `Principal::Guest`, a read the node does not serve), and keep the view
  the app's.
- **Extensions run for guests.** They are powerful by design and gated by
  consented capabilities (`features/extensions/src/hostcall.rs` `check`).
  A guest gets `guest_capabilities` and gives its own consent; nothing of
  the owner's grants carries over.
- **The data stays the node's call.** Everything a guest reads comes
  through the link's guest service (`features/spaces/src/guest.rs`
  `ShareGuest`), checked by the node. The app never decides what a guest
  may see.
- **What a guest may read is the link's permission; what a view is
  handed is the app's.** The node gives a guest a note's text and the
  link's files whenever the link's scope, audience and layer admit them
  (`apps/desktop/src/node/shares.rs` `GuestLane`), and knows nothing of
  the client: not its views, not its extensions. The app hands a view
  only what that view's extension declares it reads
  (`apps/web/src/ext_host.rs` `note_reads`, `note_props` for an
  island's view, `linked_reads`, `linked_note_props` for a linked one:
  text only with `reads = ["documents"]`, files only with `files` too,
  nothing with no read). That is a rule about extensions inside the app, the same for a
  member, never a second gate on the guest: a guest's browser already
  holds whatever the link admits.
- **The client's code is the web app's.** The islands a view is drawn
  with, their runtime and their frame documents (`/_ext/`) are fixed by
  the distribution's build, not by a node's data. A node still answers
  them; a static server can instead
  ([Deploy, Routing](@/guide/deploy.md#routing): `<app> web-ext`).
- **A share with a domain of its own** (`ShareChange::Domain`) is a
  publication, a [Site](@/features/sites.md), not a link. Its
  `/s/<code>/` link is still the app.

## The distribution's own linked views draw for a guest

A distribution's linked extensions are compiled into its web app: they
are the app itself, as much as the page around them is. So the web app
runs them for a share link's guest too, and lets their views draw the
guest's notes, the type's tabs and a note they ask to open
(`apps/web/src/app/mod.rs` `WebApp`). Two things keep that safe:

- **The host holds nothing of a person's.** A guest's host is built for
  a guest (`features/extensions/src/hostapi.rs` `Host::for_guest`): no
  session, no documents, no vault query service, no data store. Every
  host call that would reach a vault, kept data, files, the device or an
  app action is refused, whatever the extension's capabilities say
  (`features/extensions/src/hostcall.rs` `guest_check`). A note a view
  asks to open goes to the app's own opener, which opens only a note the
  link lists.
- **What a view is handed is still the link's permission.** The app
  hands a linked view the guest's note as the node gave it, by the same
  read rule as an island's (above), never a vault path.

Whether linked views may draw a guest's content is a policy the
embedding app sets, not a rule inside the embed
(`features/extensions/src/embeds.rs` `GuestEmbedPolicy`, read by
`GuestEmbedView`). The web app sets `GuestEmbedPolicy::own_linked`
because its process holds no one's session. The desktop sets none and
gets `pages_only`: a linked view there runs in the person's own app,
with their vaults, so only a page embed (on a host that reaches nothing)
draws a guest's content. `GuestEmbedPolicy::draws_linked` also asks the
host itself, so a policy set in error on a person's host still draws
nothing.

How the node serves a link, its audiences, files and players:
[Deploy, Share links](@/guide/deploy.md#share-links). What an extension
gets as a guest: [Extensions, A share link's guests](@/architecture/extensions.md#a-share-link-s-guests).

## Known gaps

The server still knows about the client in two places: a node answers
`/_ext/` (islands, runtime, frame documents, the two listings) from what
its distribution links in, and a share's page build draws blocks with
the node's own extension set (#806). The static tree (`<app> web-ext`,
`features/sites/src/island.rs` `export`) lets a deployment serve `/_ext`
without the node; no deployment in this repo (the chart, `just dev`)
does so yet, and the node keeps its routes until one does.

The code still has second implementations from before #646. Each is a
bug against this rule, with an issue saying what to remove and which
permission gate replaces it:

| Where | What differs | Issue |
| --- | --- | --- |
| `apps/web/src/app/guest.rs` `GuestPage`, `apps/web/src/guest_view.rs` | the web app swaps its vault views for a guest-only page (its own note list, sets, files view) over an empty vault source | #804 |
| `features/pages/src/host.rs` `AppHost::guest`; `features/extensions/src/embeds.rs` `GuestEmbedView` | the web app runs the distribution's linked extensions for a guest, but no fetched extension; on the desktop only a page embed draws guest content | #805 (and #495 G1) |
| `features/sites/src/plain.rs`, `PlainBlock`; node-side block drawing (`features/sites/src/installed.rs`) | the node builds each link as a scoped Site and hands notes as plain blocks, drawn by the node's own extension set | #806 |
| `apps/ui/src/guest.rs` `GuestPage` | the native app's **Shared** tab is its own renderer of plain blocks | #807 |

Until they close, the docs that describe these paths describe what runs
today, not the design.
