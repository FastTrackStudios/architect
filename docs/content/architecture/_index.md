+++
title = "Architecture"
description = "The patterns the architect template is showcasing."
weight = 20
+++

architect's architecture is opinionated: contracts are the source of
truth, implementations are pluggable, and the monorepo layout makes the
distinction visible at a glance.

## Pages in this section

- [The architect pattern](@/architecture/pattern.md) — what
  `#[derive(architect::Entity)]` emits and how to write an entity.
- [Multi-backend features](@/architecture/backends.md) — one contract,
  multiple implementations behind one facade.
- [Extensibility](@/architecture/extensibility.md) — in-tree vs
  third-party implementations; how the contract crate makes both
  paths interchangeable.
- [Construction: Resource & Scope](@/architecture/construction.md) —
  build the backend (`config → pool → repo`) with dependent
  composition, memoization, and LIFO teardown.
- [Monorepo layout](@/architecture/layout.md) — one crate per feature,
  with its layers as cargo features; apps under `apps/<app>/`.
- [The in-process transport](@/architecture/local.md) —
  `LocalServer`: typed clients over a vox memory link; desktop, CLI,
  and tests with no server.
- [The HTTP face](@/architecture/http.md) — `#[architect::service]`:
  one trait, vox and HTTP+JSON, clients for both; how a host mounts the
  generated faces next to its plugins.
- [Testing strata](@/architecture/testing.md) — native unit, native
  integration, browser e2e: which lives where.
- [Server push](@/architecture/streams.md) — `PubSub`, Entity events,
  `#[subscribe]` streams, the client hooks, and the real-time-safety
  rules for audio-thread producers.
- [Idioms & enforcement](@/architecture/idioms.md) — the vox + Dioxus
  conventions (data flows through vox, never Dioxus server functions)
  and the CI gates that keep them honest.
- [Spec coverage](@/architecture/specs.md) — per-feature
  `features/<feature>/spec/*.md` tracked by tracey.
- [The auth feature](@/architecture/auth.md) — the session RPC surface,
  `AuthServerMiddleware`, the client session kit, and the engine
  underneath.
- [Scheduling & resilience](@/architecture/scheduling.md) — `schedule`
  retry/repeat policies (exponential backoff, jitter, caps) and the
  `platform` clock they run on (deterministic `TestClock` included);
  how clients tolerate a flaky server.
- [Collaboration](@/architecture/collaboration.md) — Loro CRDT
  documents, `DocSync` over vox, the multi-doc `DocRegistry`, presence
  and persistence.
- [Peers: documents between nodes](@/architecture/peers.md) —
  `crdt::peer`: pairwise exchange with no hub, and per-document access
  checked by every node.
- [Async diagnostics](@/architecture/diagnostics.md) — a design note for
  moiré instrumentation of the axum_ws adapter (not in the tree today).
- [Commands: the Action List](@/architecture/commands.md) — one
  registry every key, menu, palette, CLI and MCP model runs an app's
  commands through; `architect-mcp`; per-action timings.
