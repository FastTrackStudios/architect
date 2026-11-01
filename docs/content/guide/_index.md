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

These chapters are being written. Their pages will be:

- `ui.md`: the UI shell.
- `ui-primitives.md`: the UI primitives architect ships (components,
  the shell's and the workspace's pieces, the command overlays, the
  settings page).
- `commands.md`: commands, menus and keys.
- `settings.md`: settings.
- `documents.md`: documents, sync and files.
- `identity.md`: sign-in, accounts and organizations.
- `extending.md`: scripting and extensions.
