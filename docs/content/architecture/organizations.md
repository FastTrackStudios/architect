+++
title = "Organizations: the spine everything is scoped by"
description = "The organization (workspace) is architect's, not an app's: files, documents, settings, extensions, presence, permissions and the shell's switcher all know which org they are in, so every app gets multi-org behaviour without writing it."
weight = 52
+++

> **Status: the client's org layer landed (2026-11-01)** in
> `architect-orgs` (`features/orgs`): `features/orgs/src/model.rs` `Orgs`
> (the person's orgs, the window's `Selection`, the device's cache),
> `Role`, `Capabilities` (the app's, by role, with per-org grants);
> `features/orgs/src/auth.rs` `from_bundles` (fts-auth's
> `list_organizations` answer); `features/orgs/src/ui.rs` `use_org`,
> `OrgHandle::switcher` (the shell's switcher). **Scoping and confinement
> landed (2026-11-01):** the live fetch (`features/orgs/src/auth.rs`
> `fetch` with auth-client's session, `features/orgs/src/ui.rs`
> `use_orgs_live` on every reconnect, `features/orgs/src/refresh.rs`
> `Refresh`); the scope every layer keys by (`features/orgs/src/scope.rs`
> `Scope`, `OrgId::path`, `OrgId::doc_id`) and the All read (`read_all`,
> `features/orgs/src/opener.rs` `scoped_opener` for ⌘O); the server's
> confinement (`features/orgs/src/server.rs` `confine`, `current_org`,
> `ConfinedEngine`, `MembersResolver`). Still to come: files, documents,
> settings' org levels and presence adopting the scope (each keys its
> data by `OrgId::path` as it lands), and Task and Studio moving onto it.
> Proposed (2026-10-31): the
> model exists ([auth](@/architecture/auth.md): organizations, members,
> roles, teams, invitations, invite links, devices, linked agents, served
> over vox and HTTP from one declaration); this page makes it the scope
> of everything else.

Every FastTrackStudio app is used by a person who belongs to
organizations: their own (a personal org), a band, a church team, a
studio. Today each app learns "which org am I in" its own way — Task's
`OrgSelection`, Studio's spaces, its org drivers, the Files backend's
per-org folders. Here the org is architect's: one current org per
window (or "all"), and every architect feature reads it.

## One identity server, every app

The orgs live in one place: the identity server (`apps/auth-server`
here, deployed for FastTrackStudio as `fts-auth` at
`auth.fasttrackstudio.app`). Every app — Task, Studio, Keyflow,
Ignition, the next one — signs in there and reads its orgs, members,
roles and teams from there, so **an org is the same org in every app**:
the band made in Task is the band in Studio, its members are the same
people, an invitation accepted once is accepted everywhere, and signing
in once signs in to all of them.

- **Apps are relying parties.** Each app's own server keeps its own data
  (Task's vault, Studio's sessions, the files an org stores) keyed by the
  shared org id and user id; none keeps a copy of who is in the org. A
  change of membership on the identity server is a change in every app.
- **Roles are the org's; capabilities are the app's.** The identity
  server says Cody is an *admin* of the band and Jess a *member*; each
  app maps a role (and any per-app grant) to its own capabilities
  (`files.write`, `studio.mix`). An app may add app-specific grants for a
  member without the identity server knowing what they mean.
- **The client:** architect's org layer is a client of the identity
  server (`auth-client`, over vox, or OIDC for the web), caching the
  person's orgs on the device so the switcher and the scopes below work
  offline and refresh when back.
- **Devices and agents** are registered there too (auth has them): a
  device is the person's on every app, and a linked agent's grants are
  per org, for every app it is allowed in.

## The org, and what hangs off it

| Feature | Scoped by the org |
|---|---|
| [App shell](@/architecture/app-shell.md) | the workspace switcher *is* the org switcher (or a space within it); "All organizations" where an app has an aggregate view |
| [Files](@/architecture/documents.md#files) | an org's roots, its storage locations and quotas, members' access by role and per-folder grants; a person's own files are their personal org's |
| [Documents](@/architecture/documents.md) | the catalogue is per org; a document belongs to one, syncs to its server, is shared within it, and moves between orgs by an explicit move |
| [Settings](@/architecture/settings-page.md) | three levels: the person's (follow them everywhere), the person's within an org, and the org's own (policies an admin sets: defaults, what members may change) |
| [Extensions](@/architecture/extensions.md) | an org's registry and its approved list; extensions an org requires for its projects install with them; an admin can allow or block an extension, or a capability, for everyone |
| [Actions](@/architecture/actions-everywhere.md) | an action's permission is a role in the current org (a member may edit, a viewer may not); disabled with that reason |
| Presence | the org's roster (who is online, what they are in), and each document's |
| Agents | a linked agent acts as its owner, within the orgs it is granted, with the capabilities granted there |
| [Workspace](@/architecture/workspace.md) | screensets and keymaps can be shared by an org (the band's "Live" layout) beside each person's own |

## How an app sees it

<!-- doc-check: off (proposed names) -->
```rust
let org = use_org();                 // the window's current org (or All), its role for me, its members
org.can("files.write")               // a capability, from my role and grants here
org.switch(OrgSelection::One(id));   // the shell's switcher calls this
```
<!-- doc-check: on -->

An architect feature takes the org from the call's identity on the
server and from `use_org()` on the client; an app passes it nowhere. A
server mounting architect's features gets tenancy with them: every
service is confined to the caller's org, every row and blob is the
org's, and what crosses orgs (a shared link, a moved document, a
federated server) is explicit.

## The scope, as built

One convention for every layer, so files, documents, settings and
presence agree without knowing each other:

| Layer | Keyed by |
|---|---|
| Files | an org's roots under `OrgId::path("files/…")` |
| Documents | the catalogue `OrgId::doc_id("catalogue")`; a document's blob under `OrgId::path("docs/<id>")` |
| Settings | the org's own under `OrgId::path("settings")`, a person's within it under `OrgId::path("settings/<user>")`; a person's everywhere stays unscoped |
| Presence | the org's roster is the presence channel of `OrgId::doc_id("presence")`; a document's stays the document's |
| Permissions | resources `org/<id>/…` (what `OrgId::path` returns) on a lane in no org |

**Client.** The window's `Scope` (`OrgHandle::scope`) is its org, or in
All every org; a read asks `read_all(&scope, |org| …)` and gets every
org's items tagged with their org (`Scoped`), the orgs that did not
answer listed apart (`Aggregate::failed`), so an offline org hides none
of the others. A write goes to `Scope::writes_to`, which is `None` in
All: the person picks an org first. ⌘O registers `scoped_opener`, which
asks the window's org (or all of them, naming each item's org) and opens
an item in the org it came from.

**The list.** `use_orgs_live` fetches the person's orgs
(`architect_orgs::fetch`: `OrganizationService::list_organizations`
with auth-client's stored session, over vox or HTTP) when the
connection comes up and again on every reconnect
(`Connection::generation`); a failed fetch keeps the device's cached
list and says why (`OrgHandle::stale`). Every app asks the same identity
server with the same session, so every app shows the same orgs.

**Server.** A per-org lane (`/org/{slug}/vox`) is scoped to its org,
outside the gate: on an `EngineHost`, `vox_at`'s resolver returns
`Lane::new(router).scoped(org)` and the host gates the router inside the
org; on a transport the app serves itself, `confine(org, gated)` wraps the
gated handler (nothing else fits, so the org can't end up inside the
check). Every call on it, the gate's check included, runs in that org
(`current_org()`), and services key their data by it. The gate's
identity is wrapped in `MembersResolver`, which learns each session's
orgs from the identity server (`IdentityServer` over an
`OrganizationService` client, at most once a minute per session); its
engine is `ConfinedEngine`, which refuses a caller who is not a member
of the call's org, any resource in another org, and whatever the
caller's role there does not allow. A lane in no org serves the
All-organizations reads: its resources name their org, and a service
reads across `Members::scope_of` the caller.

## Personal, team, and all

- **Personal:** every person has one; what they make before joining
  anything lives there, and moves to an org when they share it.
- **Teams** within an org (auth has them): access and presence by team
  (the rhythm section, the worship team).
- **All:** an aggregate over the orgs a person is in, for reading (the
  library across them, ⌘O across them); writing always picks one org.

## From what exists

- architect-auth: organizations, members, roles, teams, invitations,
  invite links, devices, agent links — the model and its services;
  `apps/auth-server`, the server, deployed as fts-auth so one account
  spans the apps (Task used to keep an auth store per org).
- architect-permissions: capabilities and the gate on every service call.
- Task: `OrgSelection` and the org switcher (→ the shell), per-org sync
  drivers (→ documents), the Files backend's org confinement (→ files).
- Studio: spaces (→ the switcher), its account's org drivers.

## Open questions

1. **Spaces inside an org** (Studio's "spaces" today): teams, or a
   separate grouping?
2. **One org per window**, or per pane (a document from org A beside one
   from org B in one window)?
3. **Org settings that override a person's**: lock them (a church's
   default key and click), or only set defaults?
