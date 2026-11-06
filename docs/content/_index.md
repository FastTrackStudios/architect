+++
title = "architect"
description = "A Rust framework for local-first, multiplayer, collaborative apps: declare each service and document once, and architect gives you every wire, offline and peer-to-peer sync, and one Action List that keys, menus, scripts, extensions and MCP all run."
weight = 0
+++

A Rust framework for local-first, multiplayer, collaborative apps:
declare each service and document once, and architect gives you every
wire, offline and peer-to-peer sync, and one Action List that keys,
menus, scripts, extensions and MCP all run.

`#[architect::entity]`, `#[architect::error]` and `#[architect::service]`
on plain Rust types write the wire types, the vox and HTTP clients and
routes, the SeaORM storage and the Action List actions. The repo doubles
as a working **reference monorepo**: the facade crate lives in
`crates/architect`, its proc-macros in `crates/`, the features in
`features/`, and `examples/greeter` is the reference feature (an entity,
a service, its error, a backend and a server, tested over every wire)
that a real project copies. `apps/auth-server` is the full-size server
built the same way. `apps/desktop` and `apps/mobile` are the Architect app
(its UI in `apps/ui`), and `apps/sandbox` the same app on made-up data,
where the UI is designed.

## Where to start

- [Getting started](@/getting-started/_index.md): get the dev shell up,
  build the workspace, run the reference feature's tests.
- [**Build a feature, end to end**](@/getting-started/walkthrough.md):
  the fastest way to learn how to *use* architect: define an entity, pick
  a backend, serve it, consume it remote or in-process.
- [Script an app, end to end](@/getting-started/scripting.md): actions on
  an Action List, driven from Luau as one undo step, saved as actions and
  run by an agent over MCP (`examples/scripting`).
- [Write an extension, end to end](@/getting-started/extension.md): a
  manifest, a Luau overlay and a process extension with a remote view,
  loaded from a registry and hot-reloaded (`examples/extension`).
- [Build an app with architect](@/guide/_index.md): the guide, chapter by
  chapter, from the contract to the shell, commands, settings, documents,
  sign-in and extensions.
- [Features](@/features/_index.md): one page per feature: what it is, the
  least code to use it, a tour of its code, the tests to read, its status.
- [Architecture](@/architecture/_index.md): the design pages: contracts
  vs. implementations, multi-backend features, the facade-with-cargo-features
  split, documents, peers, the workspace.
- [Reference](@/reference/_index.md): the crate map, derive
  attributes, and what the macro emits.

## Why these patterns

- **Wire formats live in one source struct.** No parallel DTOs, no
  serde-on-one-side-facet-on-the-other.
- **Backends are swappable behind a trait the macro emits.** Same
  code, same wasm tests, different storage.
- **Work stays on the device.** Documents are CRDTs kept in a file on
  the device and synced to the org's server and to peers when a link is
  up ([documents](@/architecture/documents.md)).
- **One command per capability.** Keys, menus, the palette, scripts,
  extensions and MCP all run the same Action List action by id.
- **Multi-target by construction.** wasm32, native server, desktop —
  all linked by the same feature crate, whose default feature set is
  the contract ([layout](@/architecture/layout.md)).
- **Monorepo template.** `apps/<app>/<role>` + `features/<feature>/`
  scales to many apps and features without crate name collisions.
