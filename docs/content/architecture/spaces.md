+++
title = "Spaces: rooms with collaborators, below the org"
description = "A space names part of an org — a project, a wiki, a folder, a document, a session — and has its own members (from this org, from others, link guests), roles that compile to permits, per-resource grants and an org policy that caps them. One model drives the gate, documents and files."
weight = 53
+++

> **Status: the model and its enforcement landed (2026-11-01, #196)** in
> `architect-spaces` (`features/spaces`). Invites by email, invite links,
> share links and guest lanes are #197; the share sheet and members panel
> are #198.

An [organization](@/architecture/organizations.md) is the unit people
belong to. A **space** is the unit they are *invited into*: a project's
subtree, a wiki, a folder of files, one document, a Studio session. Task
used to hand-roll this three ways (share links, cross-org Sharing Grants,
wiki Editors); Studio needs the same for sessions. Spaces are the one
system both use.

## The model

`features/spaces/src/model.rs`, wasm-clean.

- A `Space` lives in one org and names what it covers by the permission
  resources the org's services already check, org-relative: its `scope`
  (`vault/Alpha/`, `doc/<id>`, `files/<root>/Mixes/`) and its `exclude`
  carve-outs (`vault/Alpha/Invoices/`).
- A `Member` is a `Subject`: a `User` (by identity server id, whether or
  not they are in the org) or a `Guest` (a share link's holder,
  `Principal::Guest`, minted by #197). A member has an optional role, any
  number of `SpaceGrant`s (actions on one resource inside the scope), and
  an optional expiry.
- A `SpaceRole` is a named set of actions held over the whole scope. Every
  space has `viewer` (read), `commenter` (read, comment), `editor` (read,
  comment, write, download) and `manager` (`*`, so `admin`: changes the
  space and its members); a space may define its own or replace a built-in
  for itself.
- `Space::rules_for` compiles a member to permission `Rule`s: the role's
  actions over each scope pattern, each grant, and `read` (plus `admin`
  for a manager) on the space's own resource `spaces/<id>`.
- The org's `SpacePolicy` caps what a space may give by how a member
  stands to the org (`Standing`): people from other orgs are capped at
  `external_cap` (default read, comment, write, download: they work in a
  space but do not manage it), guests at `guest_cap` (read, comment), and
  either can be turned off. The org's own members are not capped: a viewer
  of the org made an editor of a space writes in that space.

Effective access is **the org role ∪ the space role, capped by the
policy**. `OrgSpaces::allows` is the whole decision.

## Enforcement: one model, every plane

`features/spaces/src/server/mod.rs` `Spaces` keeps every org's spaces in
memory (read once from a `SpaceStore`: `MemoryStore`, or `FileStore` under
`<data>/org/<id>/spaces/`), writes each change through after appending it
to the audit trail, and is three things a server mounts:

1. **The gate.** `Spaces::engine` is a `PermissionEngine`
   (`features/spaces/src/server/engine.rs` `SpacesEngine`) that allows
   only what a space gives. It sits beside the org's engine:
   `CompositeEngine::new().push(confined).push(spaces.engine())`. The org
   is the resource's (`org/<id>/…`) or the lane's (`current_org`), exactly
   as for `ConfinedEngine`. Documents ride the same engine:
   `DocPermissions` (what `DocRegistry::with_permissions` and
   `DocsHost::with_permissions` ask) checks `doc/<id>` through it, so a
   space over a document is enough.
2. **Files.** `Spaces` is a `GrantSource`
   (`features/files/src/server/mod.rs` `GrantSource`,
   `FilesServer::with_grants`): a scope pattern `files/<root>/<path>`
   becomes a files `Grant` on that folder for each member who holds
   actions there, generated on every call (`read` → Read + History,
   `write` → Write, `admin` → Share). Nobody hand-makes those grants, and
   nothing has to clean them up.
3. **The service.** `SpacesService` (`features/spaces/src/service.rs`):
   create, list, read, update, delete a space; set and remove members;
   grant and revoke; define roles; the caller's `effective` rules (what a
   UI greys out by); the space's `audit` trail; the org's `policy`. The
   org comes from the lane, the caller from the gate. A space's managers
   and the org's owners and admins change it; its members see it; a
   space someone may not see is `NotFound`.

**Revoke is retroactive.** Every check reads the state as it is, so a
removed member, a deleted space, a narrowed scope or a tightened policy
binds on the next call — on the same connection.

## The gate's contract, again

The gate is method-level: it checks a permit's coarse resource
(`doc/{doc}` is asked as `doc/**`). `SpacesEngine` answers a coarse
question by overlap — allowed if the caller holds the action on
*something* under it — so a commenter of one document gets past the gate
on `comment`, and not on `write`. **A service a space reaches must then
check the exact resource** in its handler (`PermissionEngine::check` with
`caller()`), which `DocPermissions` and the files feature already do. A
handler that skips it hands a space member every resource behind the
method.

## Designed for invites and links (#197) and the UI (#198)

- A link guest is already a `Subject::Guest`; a guest lane's
  `StaticPrincipal(Principal::Guest { link_id })` resolves to it, and the
  policy caps it. #197 adds the link rows and lanes, not a new model.
- Accepting an email invite is `set_member` for the person who signed in;
  an invite link carries a role and adds the holder as a `User`.
- `effective`, `audit` and `SpaceSummary` are the share sheet's and the
  members panel's data.

## Limits

- A files grant is a whole folder, so a carve-out may not sit inside a
  files scope pattern (`validate_reach` refuses it).
- A document sync session already attached keeps running after a removal
  until it reattaches (`DocRegistry` checks on attach).
- Membership's *standing* comes from `Members`, which the gate's
  `MembersResolver` fills as people sign in; the policy is applied at
  check time, when the caller has signed in.
