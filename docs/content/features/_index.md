+++
title = "Features"
description = "One page per major architect feature: what it is, the least code to use it, a tour of its code, the tests to read, and its status."
weight = 18
sort_by = "weight"
+++

Each page has the same five sections:

1. **What it is and when to use it**, with a link to the design page under
   [Architecture](@/architecture/_index.md).
2. **A minimal example**: the least code an app writes.
3. **Code tour**: the files and symbols in reading order, and how one call
   flows through them.
4. **Tests to read**: the tests that show it working end to end.
5. **Status**: what is done in architect, and what Task and Studio still have
   to adopt.

When a page and the code disagree, the code is right.

## Core

- [Services and entities](@/features/services-and-entities.md): `#[architect::entity]`, `#[architect::service]`, `#[architect::rpc]` and the layers they emit.
- [Transports](@/features/transports.md): vox in process, over WebSocket and over iroh; the HTTP face with query arguments, ranged bytes and SSE.
- [EngineHost and protocol mounts](@/features/engine-host.md): one server binary, `/vox`, `/health`, `/version.json`, `vox_at`, `document`, `dav`, `otlp_forward`.
- [Errors](@/features/errors.md): `#[architect::error]`, `ClientError`, the HTTP error envelope.
- [Telemetry](@/features/telemetry.md): call spans as wide events, stages, `rpc.server.duration`, trace context across calls.
- [PubSub and streams](@/features/pubsub-and-streams.md): fan-out hubs, `#[subscribe]`, client hooks, the full-sink pump.

## Data

- [Documents](@/features/documents.md): `CrdtDoc`, kinds of document, named undo, `DocRegistry`.
- [The document file](@/features/document-file.md): `crdt::file`, one durable file per document.
- [Org sync driver](@/features/org-sync.md): `OrgSyncs`, `OrgDocs`, `Placement`.
- [Peer to peer](@/features/peer-to-peer.md): pairwise exchange, iroh, device admission, share links.
- [Devices](@/features/devices.md): the account's devices, per-org enrolment, pairing by code or QR, revoking, one admission table.
- [Presence and collaborative text](@/features/presence-and-text.md): `Here`, `Roster`, places, `TextBinding`, `crdt(text)`.
- [Blobs](@/features/blobs.md): content-addressed bytes fetched on demand, pinned and collected.
- [Files](@/features/files.md): `architect-files`, an org's files with versions, quota and access.
- [Attachments](@/features/attachments.md): `architect-attachments`, files attached to documents and records, offline first, access following the parent.
- [Bases](@/features/bases.md): `architect-bases`, a vault's `.base` files as table, cards and list views, view types extensions add, `BASES_*`.

## Identity

- [Auth](@/features/auth.md): the auth engine, its services, the client kit, `apps/auth-server`.
- [Permissions](@/features/permissions.md): `Principal`, permit tables, `PermissionsGate`, the Action List authorizer.
- [Organizations](@/features/organizations.md): `architect-orgs`, the current org, scopes, server confinement.

## App

- [Settings](@/features/settings.md): `#[derive(Settings)]`, scoped values, the settings page.
- [Commands and the Action List](@/features/commands.md): `ActionList`, the palette, the action list window, MCP.
- [Key bindings](@/features/key-bindings.md): `architect-input`, the person's keymap, the binding editor.
- [Menus](@/features/menus.md): `MenuBar` from the commands, native and in the window.
- [Workspace and shell](@/features/workspace-and-shell.md): tab groups, docks, pop-out windows, screensets, the app frame.
- [Atom and forms](@/features/atom-and-forms.md): the optimistic store, the offline call queue, validated fields.

## Extending

- [Scripting](@/features/scripting.md): Luau over the Action List, saved scripts as actions.
- [Extensions](@/features/extensions.md): run-time packages from registries, hot reload, project requirements.
- [Canvas overlays and ImGui-shaped UI](@/features/canvas.md): drawing over a view in its coordinates; tool windows for scripts.
- [Remote views](@/features/remote-views.md): a process extension's Dioxus views in workspace panes.
- [Navigator](@/features/navigator.md): folders beside notes in the sidebar, a port of Notebook Navigator (GPL-3.0), as a built-in extension.
- [A2UI: agents' generated UI](@/features/a2ui.md): an agent's form, list or approval drawn with architect-ui from a catalog, kept in Loro, its buttons asking before any write.
