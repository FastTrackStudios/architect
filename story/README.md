# architect-story

Component browser **and** automated visual regression testing for Dioxus
0.7+ UI libraries.

A Lookbook-style interactive shell where every component variant is
addressable; the same registry is consumed by a headless Blitz runner
that takes PNG snapshots, diffs them against committed baselines, and
fuzzes interactions to flush out panics and visual regressions — without
a human in the loop.

Built to back FastTrack Studios projects (`architect-ui`, `frame-ui`, …) but
useful to anyone on Dioxus 0.7+.

## Status

Pre-MVP. Scaffold only — see `docs/roadmap.md`.

## Crate layout

| Crate                 | Purpose                                                                 | Heavy deps          |
|-----------------------|-------------------------------------------------------------------------|---------------------|
| `architect-story-core`        | `Story`, `Knob`, `Interaction` types + linkme registry                  | linkme              |
| `architect-story-macros`      | `#[story]`, `#[story_test]`, `#[states]` proc-macros                    | syn, quote          |
| `architect-story-runtime`     | Dioxus-aware glue: render thunks, `KnobSource`, interaction executor    | dioxus              |
| `architect-story-shell`       | Interactive sidebar + preview Dioxus component                          | dioxus              |
| `architect-story-snapshots`         | Headless Blitz renderer, settling loop, DSSIM diff, baseline harness    | dioxus-native, dssim |
| `architect-story-fuzz`        | Interaction fuzzing via proptest                                        | proptest             |

Consumers normally depend on **only** `architect-story-runtime` (story declarations)
+ `architect-story-shell` (interactive browser) + `architect-story-snapshots` (snapshot harness).

## Why not lookbook?

[`matthunz/lookbook`](https://github.com/matthunz/lookbook) is the
closest prior art and inspired the `#[preview]`/`#[story]` ergonomic.
However:

- last commit Sept 2024 (~14 months stale, pinned to dioxus 0.6-alpha)
- web-only (Blitz/desktop never landed)
- hard-deps `dioxus-material` for theming
- registry is a `thread_local! RefCell` — usable by a running app, but
  invisible to a headless VRT runner
- only single-string and `Json<T>` knobs

We re-use the API ergonomics but redesign the registry (compile-time via
`linkme`), the knob system (typed: Bool/Enum/Number/String/Color), and
the shell (renderer-agnostic, no theming dep).

## Why not just use `blitz-vrt`?

[`tonybierman/blitz-vrt`](https://github.com/tonybierman/blitz-vrt) is
new (April 2026) and ships exactly the rendering + diffing pieces we
need: settling loop, in-process `NetProvider`, DSSIM threshold tuning,
Docker-based font pinning, debug-build guard. We plan to either vendor
those pieces (no LICENSE file at time of writing — needs to be resolved
before depending) or rebuild from scratch with the same design.

What `blitz-vrt` does **not** provide and we do:

- a story registry the consumer iterates declaratively
- typed knobs / state matrix
- interaction scripts (click/type/key/scroll) so we can snapshot
  *post-interaction* states, not just initial render
- fuzz testing
- cross-renderer parity story (Blitz vs wry vs web vs mobile)

## Consuming from another project

This repo is private. Pull it via SSH git dep with a pinned commit so
upstream churn doesn't break consumers.

In your workspace `Cargo.toml`:

```toml
[workspace.dependencies]
architect-story-runtime   = { git = "ssh://git@github.com/FastTrackStudios/architect", rev = "<sha>" }
architect-story-shell     = { git = "ssh://git@github.com/FastTrackStudios/architect", rev = "<sha>" }
architect-story-snapshots = { git = "ssh://git@github.com/FastTrackStudios/architect", rev = "<sha>" }
```

In a UI crate that declares stories — keep it light, only the runtime:

```toml
# crates/architect-ui/Cargo.toml  (or  crates/frame-ui/Cargo.toml)
[dependencies]
architect-story-runtime = { workspace = true, optional = true }

[features]
stories = ["dep:architect-story-runtime"]
```

In the **app** that hosts the interactive Lookbook:

```toml
# apps/showcase/Cargo.toml
[dependencies]
architect-story-shell = { workspace = true }
architect-ui       = { workspace = true, features = ["stories"] }   # or frame-ui
```

In a **`tests/` crate** that runs snapshot regressions:

```toml
[dev-dependencies]
architect-story-snapshots = { workspace = true }
architect-ui           = { workspace = true, features = ["stories"] }
```

Why split it this way: `architect-story-runtime` is small (depends only on
`dioxus`, `linkme`, `architect-story-core`); a UI library can opt into story
declarations via a feature flag without forcing every downstream
consumer to compile Blitz, dssim, or proptest. Apps that actually run
the shell or snapshot tests pay for the heavier deps separately.

### Cross-project compatibility

- **dioxus version**: workspace pins `dioxus = "0.7"` (loose minor) so
  `frame` (0.7.2) and `architect-ui` (0.7.6) both resolve cleanly. Do not pin
  patch versions in this repo.
- **`linkme` registry**: each consuming crate gets its own segment of
  the global `STORIES` slice; mixing stories from `architect-ui` and
  `frame-ui` in one host app Just Works because the slice is one global
  collection populated at link time.
- **No architect-ui or frame-ui types leak into this repo.** All trait
  bounds are on `dioxus_core::Element` and `dioxus_html::*`. Any
  Dioxus 0.7 component crate can declare stories.

## Hacking

```sh
cargo check --workspace
```

Snapshot tests only:

```sh
cargo test --release -p architect-story-snapshots
```

(release build is enforced — `compile_error!` triggers under
`debug_assertions` because Stylo/Parley produce *incorrect* renders in
debug, not just slow ones).

## License

Dual MIT / Apache-2.0.
