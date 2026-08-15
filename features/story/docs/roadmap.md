# Roadmap

Phased plan from scaffold (today) to production-grade VRT + fuzzing.

## Phase 0 — scaffold (DONE)

- Workspace skeleton with six crates (`core`, `macros`, `runtime`, `shell`, `snapshots`, `fuzz`).
- `architect-story-core` defines `Story`, `KnobSpec`, `KnobValue`, `KnobKind`,
  `Interaction`, `Selector`, `WaitCondition`, `InteractionScript`, and
  the `STORIES` / `INTERACTION_SCRIPTS` `linkme` slices.
- `architect-story-runtime` defines `RenderFn`, `KnobSource`, `render_fn` cast.
- `architect-story-snapshots` enforces release builds via `compile_error!`.
- Stubs everywhere else.

## Phase 1 — hand-rolled stories prove the registry (1 day)

- Hand-write 1-2 `Story` values for an `architect-ui` component (no macro yet).
- Wire `architect-story-shell::Lookbook` to read from `STORIES`, render the
  selected story by casting `render` back to `RenderFn`.
- Implement `KnobSource` against a `Signal<HashMap<&'static str, KnobValue>>`.
- Render shell in architect-ui's existing native app (replaces the showcase grid
  in dev only) — proves the registry survives dynamic linking on Blitz.

## Phase 2 — `#[story]` macro (2-3 days)

- Parse fn signature + per-arg `#[knob(default=…)]` + rustdoc.
- Infer `KnobKind` (start with `bool`, `i64`, `f64`, `&str`/`String`).
- Emit:
  - the user fn untouched
  - `static <UPPER>_STORY: Story = Story { … }`
  - `static <UPPER>_RENDER: RenderFn = |src| { user_fn(read_knob…(src), …) };`
  - `#[distributed_slice(STORIES)] static <UPPER>_REG: &Story = &<UPPER>_STORY;`
- Add a `derive(Knob)` companion to support enum knobs cleanly.
- Manual `#[states(...)]` attribute for explicit matrices on enums and
  String-valued cases.

## Phase 3 — `architect-story-snapshots` MVP (DONE)

Implemented from scratch (not vendored from blitz-vrt — the design is
similar but the code is ours, so no LICENSE issue to resolve).

Shipped:

- `render::render_story(story, knobs, &RenderConfig) -> Vec<u8>` —
  builds a `DioxusDocument` via the patched `FastTrackStudios/blitz`
  fork, runs a settling loop (`poll → resolve`, capped at
  `RenderConfig::max_settle_iters`, default 64), paints through
  `anyrender_vello_cpu::VelloCpuImageRenderer`, encodes the RGBA buffer
  via the `png` crate.
- Thread-local snapshot dispatch lets us hand a zero-arg `Component` to
  `VirtualDom` while still recovering the per-call render thunk +
  knob-value map.
- Runtime debug-build guard: panics with a useful message if invoked
  under `debug_assertions` (Stylo/Parley produce *incorrect* renders
  in debug, not just slow ones). Replaces the earlier
  `compile_error!` that blocked even `cargo check` on the dev profile.
- `diff::compare(baseline, candidate, threshold)` — DSSIM perceptual
  diff via `dssim-core`. PNG decode normalises to RGBA8. Default
  threshold `0.001` per the blitz-vrt tuning.
- `harness::assert_snapshot(story, &SnapshotConfig)` — used inside
  `#[test]` fns. Auto-creates baselines on first run; writes candidate
  PNGs to `diff_output/` on mismatch and panics with both paths.
  `FTS_STORY_UPDATE_SNAPSHOTS=1` forces a refresh.
- `docker/Dockerfile` (ubuntu:24.04 + pinned Liberation/Noto fonts +
  Rust toolchain) for byte-stable cross-machine rendering.
- `.github/workflows/snapshots.yml` builds the image, runs the workspace
  tests, uploads diffs/baselines as artefacts on failure or refresh.

Smoke test: `crates/architect-ui/tests/snapshots.rs` in the consuming
`architect-ui` repo asserts 4 stories. Run with `cargo test --release -p
architect-ui --features stories`. First invocation populates
`crates/architect-ui/snapshots/`.

Out of scope here (lands in Phase 4): post-interaction snapshots
(open the dropdown, then snapshot). Initial-render only for now.

## Phase 4 — interaction scripts (2 days)

- `Interaction` executor against `DioxusDocument`:
  - `Click(Selector)` → resolve selector → synthesize `UiEvent::PointerDown`
    + `PointerUp` → poll until idle
  - `Type(s)` → focus check → `UiEvent::KeyDown`/`KeyUp` for each char
  - `Key(KeyAction)` → direct keyboard event
  - `Scroll` → `UiEvent::Wheel` on target
  - `Wait(Idle | Mounted | Millis)` → poll-loop / time-bounded
  - `Snapshot(name)` → render + write PNG (or diff if baseline exists)
  - `Assert` → invoke trait-object predicate
- `Selector::Role` resolution via Blitz's accessibility tree (the patched
  fork already enables `accessibility` by default on `dioxus-native-dom`).
- `#[story_test]` macro emits an `InteractionScript` and registers it
  via `INTERACTION_SCRIPTS`.

## Phase 5 — fuzzing (2 days)

- For each story:
  - enumerate focusable nodes from the rendered tree
  - generate `proptest` strategies for `Interaction` sequences (length
    1-N, weighted toward Click/Hover/Type)
  - run sequence; assert no panic, no `tracing::error!`, no NaN/inf in
    layout, all dialogs closeable
  - on failure, persist the minimised counterexample as JSON and a
    final-state PNG under `fuzz-failures/`

## Phase 6 — cross-renderer parity (later)

- `architect-story-snapshots` already covers Blitz.
- Add `architect-story-snapshots-wry` (uses `webview.screenshot()`).
- Add `architect-story-snapshots-web` (Playwright shell that loads the Dioxus web
  build and screenshots).
- Add `architect-story-snapshots-mobile` (xcrun simctl io / adb screencap).
- All four write into the same `snapshots/` layout. Each renderer can
  pin its own baseline (`snapshots/<story>__<state>__<renderer>.png`)
  and a meta-test asserts cross-renderer DSSIM is below a generous
  threshold to flag drift.

## Multi-project support

This crate is designed to back **all** FastTrack Studios UI projects, not
just `architect-ui`. Concretely:

- `architect-ui` (dioxus 0.7.6) — primary driver, will host the first stories.
- `frame-ui` (dioxus 0.7.2) — secondary consumer; once `frame-ui` adds
  meaningful components the same `#[story]` attribute applies. Pin both
  to the same `architect-story` git rev.
- Future UI crates — same pattern.

To make sure new features don't accidentally lock to `architect-ui`:

- **Never import `architect-ui`** from any crate in this repo. Trait bounds
  stay on `dioxus_core::Element`.
- The example app under `examples/` (when added) should be a tiny,
  hand-rolled component, not an `architect-ui` re-export.
- CI matrix should run `cargo check` against a fixture that imports
  *only* `dioxus` plus this crate, to catch accidental coupling.

## Open questions

- **`linkme` on wasm**: works but with caveats; may need `inventory` as
  fallback. Decide before phase 2 lands.
- **Auto state-matrix explosion**: cartesian over Bool + small Enum is
  fine; cap at e.g. 32 combinations per story. Beyond that the macro
  fails compilation and the user must write `#[states(...)]`.
- **Snapshot storage**: in-tree under `snapshots/` keeps the diff
  visible in PR review but bloats the repo. Investigate Git LFS or a
  sidecar repo if tests proliferate.
- **License of vendored `blitz-vrt` pieces**: file the issue **before**
  writing any vendoring code so attribution can be set up correctly.
