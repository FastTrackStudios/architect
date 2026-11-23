+++
title = "architect"
description = "An application framework for Rust apps: services declared once with every wire, one Action List that keys, menus, scripts, extensions and MCP all run, and extensions of every kind."
weight = 0
+++

An application framework for Rust apps: declare each service once and
architect gives you every wire; put every command on one Action List that
keys, menus, scripts, extensions and MCP all run; and grow the app with
extensions of every kind (compiled in, a process, Luau, TypeScript, WASM
or plain data), loaded at run time while you develop.

`#[architect::entity]`, `#[architect::error]` and `#[architect::service]`
on plain Rust types write the wire types, the vox and HTTP clients and
routes and the Action List actions. The facade crate lives in
`crates/architect`, its proc-macros in `crates/`, the features
(the extension runtime, control and MCP, scenes and process views,
input, settings, auth) in `features/`. `apps/auth-server` is a
full-size server built the same way. The app's shell, workspace,
toolbars and UI are not here: they are FastTrackStudio/app-shell
(`app-shell:crates/app-shell-extensions` draws extensions in it).

## Where to start

- [Getting started](@/getting-started/_index.md): get the dev shell up,
  build the workspace, run the tests.
- [**Build a feature, end to end**](@/getting-started/walkthrough.md):
  the fastest way to learn how to *use* architect: define an entity, pick
  a backend, serve it, consume it remote or in-process.
- [Script an app, end to end](@/getting-started/scripting.md): actions on
  an Action List, driven from Luau as one undo step, saved as actions and
  run by an agent over MCP.
- [Write an extension, end to end](@/getting-started/extension.md): a
  manifest, a Luau overlay and a process extension with a remote view,
  loaded from a registry and hot-reloaded.
- [Build an app with architect](@/guide/_index.md): the guide, chapter by
  chapter, from the contract to commands, settings, documents, sign-in
  and extensions.
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
- **Monorepo template.** `features/<feature>/` scales to many features
  without crate name collisions.
