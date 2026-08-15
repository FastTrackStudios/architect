+++
title = "architect"
description = "Facet-native, vox-friendly entity framework for Rust — one derive, every surface."
weight = 0
+++

`architect` is the entity framework + monorepo template that powers
contract-first Rust projects: one `#[derive(Entity)]` on a plain struct
yields a wasm-clean wire type, an auto-generated vox repository trait,
and (under `--features server`) a SeaORM-backed implementation.

The repo doubles as a working **reference monorepo**: the facade crate
lives in `crates/architect`, its proc-macros in `features/macros/`, and
`examples/app/` (a full-stack demo with `features/` + Dioxus
web/desktop apps) shows the layout and testing patterns a real project
would copy.

## Where to start

- [Getting started](@/getting-started/_index.md) — get the dev shell up,
  run the e2e tests, see the contract round-trip from a browser.
- [**Build a feature, end to end**](@/getting-started/walkthrough.md) —
  the fastest way to learn how to *use* architect: define an entity, pick
  a backend, serve it, consume it remote or in-process.
- [Architecture](@/architecture/_index.md) — the patterns the template
  is showcasing: contracts vs. implementations, multi-backend features,
  the facade-with-cargo-features split, the per-feature UI layer.
- [Reference](@/reference/_index.md) — the crate map, derive
  attributes, and what the macro emits.

## Why these patterns

- **Wire formats live in one source struct.** No parallel DTOs, no
  serde-on-one-side-facet-on-the-other.
- **Backends are swappable behind a trait the macro emits.** Same
  code, same wasm tests, different storage.
- **Multi-target by construction.** wasm32, native server, desktop —
  all linked by the same `<feature>-proto` crate.
- **Monorepo template.** `apps/<app>/<role>` + `features/<feature>/`
  scales to many apps and features without crate name collisions.
