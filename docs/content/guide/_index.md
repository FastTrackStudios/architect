+++
title = "Build an app with architect"
description = "A chapter-by-chapter walk through building a real app on architect: the contract, the backend, the server, the clients, the tests and the release."
weight = 12
sort_by = "weight"
+++

This guide is the long form of [the walkthrough](@/getting-started/walkthrough.md).
The walkthrough shows the whole flow on one page; these chapters go
through it again in the order you build an app, with the decisions each
step asks you to make and the code that answers them. Every code block
is taken from, or matches, code in this repo, and each chapter ends
with the architecture pages that cover its subject in full.

The running example is `examples/greeter`: an entity (`Note`), a
service (`Greeter`) with its error and a stream, an in-memory backend,
a server, and tests that run one scenario over every wire. It is small
enough to read in one sitting, and every chapter adds to it or points
at the larger code that does the same thing for real
(`apps/auth-server`, and Task and studio in their own repos).

## Part 1: the backend half

1. [The shape of an architect app](@/guide/shape.md): the workspace
   layout, the layers and `boundaries.toml`, cargo features, the dev
   shell, the toolchain, the build cache and `cargo xtask feature new`.
2. [The contract](@/guide/contract.md): entities, errors and services,
   what each attribute generates, and choosing wire types.
3. [The backend](@/guide/backend.md): implementing a service, storage
   and migrations, layers and the per-call context, and telemetry as
   wide events.
4. [The server](@/guide/server.md): `EngineHost`, the transports (vox
   over WebSocket, iroh and in-process; the HTTP face), protocol mounts
   and health.
5. [Clients](@/guide/client.md): the generated vox and HTTP clients,
   connecting and the connection lifecycle, offline and optimistic
   state, forms.
6. [Testing](@/guide/testing.md): one scenario on every wire, in-process
   vox, fakes, property tests, and what CI runs.
7. [Shipping](@/guide/shipping.md): versions and tags, consumers pinning
   a tag, and tagging only a green main commit.

## Part 2: the app half

8. [The app shell and the workspace](@/guide/ui.md): mount
   `architect-shell`, give it tabs, sides and a status line, and hand
   `architect-workspace` your views for tab groups, splits, docks,
   pop-out windows and screensets.
9. [The UI primitives](@/guide/ui-primitives.md): `architect-ui`'s
   components and theme tokens, the shell's page parts, every kind of
   tab and pane, the command overlays, the settings page, and
   `architect-story` to browse them.
10. [Commands, keys and menus](@/guide/commands.md): the Action List,
    the ⌘K palette, ⌘O, the action list window and its binding editor,
    keys through `architect-input`, menus, and MCP.
11. [Settings](@/guide/settings.md): settings declared once as a typed
    struct, stored by scope under an org's defaults and locks, and the
    settings page.
12. [Documents, sync and files](@/guide/documents.md): CRDT documents,
    kinds and named undo, the document file, org sync, peer to peer,
    presence and collaborative text, blobs, and Files.
13. [Sign-in, permissions and organizations](@/guide/identity.md): the
    identity server, the session and the device, `PermissionsGate`, and
    the organization as the scope of everything.
14. [Scripts, extensions and canvases](@/guide/extending.md): Luau over
    the Action List, extensions with manifests and hot reload, process
    extensions with remote views, and canvases over a view.
