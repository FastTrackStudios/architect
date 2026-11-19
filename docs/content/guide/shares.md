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
- **What a link permits is on the link**, and changes without its URL
  changing: who opens it (`Audience`), how far it plays (`Listen`), what
  it offers to keep (`Download`: a recording that only plays is still
  played by the app's own player, and simply not listed to save), and
  whether a project it carries opens read-only or may be saved back
  (`ProjectAccess`). The app reads these from the node's answers
  (`ListedFile::stream_only`, `GuestProject::access`) and hides or shows
  its own controls; it never draws another page for them.
- **A share with a domain of its own** (`ShareChange::Domain`) is a
  publication, a [Site](@/features/sites.md), not a link. Its
  `/s/<code>/` link is still the app.

How the node serves a link, its audiences, files and players:
[Deploy, Share links](@/guide/deploy.md#share-links). What an extension
gets as a guest: [Extensions, A share link's guests](@/architecture/extensions.md#a-share-link-s-guests).

## Known gaps

The code still has second implementations from before #646. Each is a
bug against this rule, with an issue saying what to remove and which
permission gate replaces it:

| Where | What differs | Issue |
| --- | --- | --- |
| `apps/web/src/app/guest.rs` `GuestPage`, `apps/web/src/guest_view.rs` | the web app swaps its vault views for a guest-only page (its own note list, sets, files view) over an empty vault source | #804 |
| `apps/web/src/app/mod.rs` `WebApp`, `own_palette`; `features/pages/src/host.rs` `AppHost::guest`; `features/extensions/src/embeds.rs` `GuestEmbedView` | a guest runs none of the app's extensions; the palette is redrawn by the app; on the desktop only a page embed draws guest content | #805 (and #495 G1) |
| `features/sites/src/plain.rs`, `PlainBlock`; node-side block drawing (`features/sites/src/installed.rs`) | the node builds each link as a scoped Site and hands notes as plain blocks, drawn by the node's own extension set | #806 |
| `apps/ui/src/guest.rs` `GuestPage` | the native app's **Shared** tab is its own renderer of plain blocks | #807 |

Until they close, the docs that describe these paths describe what runs
today, not the design.
