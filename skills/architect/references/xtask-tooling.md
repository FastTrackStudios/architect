# architect xtask, boundaries, CI and releases

Subcommands are declared in `xtask/src/main.rs` (`Command`); `cargo xtask --help` is the live list. Layout ADR: `docs/content/architecture/layout.md`.

## Commands

| Command | What it does | Code |
|---|---|---|
| `cargo xtask feature new <name>` | writes ONE crate `features/<name>/` (contract as the default feature set, wasm-clean; in-memory backend behind `memory`; `vox` and `ui` flags; a `spec/`; a passing test), registers it in workspace members and `[workspace.dependencies]`, adds boundaries rows, runs rustfmt | `xtask/src/scaffold.rs` `feature_new`; acceptance test `xtask/tests/feature_new.rs` |
| `cargo xtask tags [-v]` | every `vX.Y.Z` tag must descend from every lower-numbered one | `xtask/src/tags.rs` `run`, `GRANDFATHERED` |
| `cargo xtask boundaries [--config F] [-v]` | resolves each `boundaries.toml` row with `cargo tree`, runs the layer rules, ratchets counts | `xtask/src/boundaries.rs` `run_cli` |
| `cargo xtask surfaces` | every member whose `src/` serves MCP also serves the `actions` CLI / control socket (and the reverse), each with an `architect_mcp::parity` test; MCP's generic tools built from `Method::TOOLS` | `xtask/src/surfaces.rs` `run_cli`, `problems` |
| `cargo xtask docs` | every repo path, page link, `just` recipe, `cargo xtask` command and `-p` package named in `docs/content/**/*.md`, `README.md` and `skills/**/*.md` must exist | `xtask/src/docs.rs` `run_cli`, `check_text` |
| `cargo xtask fleet-bump <TAG> <MANIFEST or DIR>... [-n]` | rewrites architect-owned git dep lines to `tag = "<TAG>"` | `xtask/src/fleet.rs` `run`, `OWNED_PREFIXES` |
| `cargo xtask auth-mirror --from URL [--out F] [--redact-emails]` | pulls a server-side-scrubbed copy of an auth server's org graph (needs `AUTH_ADMIN_TOKEN`) | `xtask/src/mirror.rs` `run` |

In the architect repo `cargo xtask` is `cargo run -p xtask --`. Consumers build the binary from the tag they pin (`cargo install --locked --git https://github.com/FastTrackStudios/architect --tag <TAG> xtask`) and run it from their own root. Task aliases `cargo xtask` to its own codegen xtask, so call architect's binary by path there.

## `cargo xtask docs`: writing checkable docs and skills

- A token in a code span or fenced block whose first segment is a top-level directory of the repo is a path and must exist; `:12-40` line suffixes are stripped. Name the symbol beside the path instead of a line number: `` `crates/architect/src/action_list.rs` `ActionList::register_with` ``.
- Other repos' paths take a repo prefix and are skipped: `task:apps/server/src/mcp.rs`, `studio:apps/session/daw/src/actions.rs`.
- Placeholders (`<name>`, `*`, `{x}`, `…`, `$X`, `|`) are skipped; so is anything between `<!-- doc-check: off -->` and `<!-- doc-check: on -->`.
- `just <recipe>`, `cargo xtask <cmd>`, `cargo run -p xtask -- <cmd>` and `cargo … -p <package>` are checked against the justfile, the xtask binary and the workspace packages.
- CI runs it on every PR (job `lint`, step `docs`); `just docs-check` runs it locally.

## boundaries

- Each row: `cargo tree --locked -p <pkg> -e normal` with the row's features; L0 rows resolve for `wasm32-unknown-unknown`.
- Rules: `contract-clean` (L0 must not reach engine or ui packages, a tokio runtime, or a transport-host/server `architect` feature), `ui-reach` (L4 must not reach engine packages or transport/server features; Dioxus is fine), `transport-host` (no member outside `apps/` enables a host in its default set), `lints` (`[lints] workspace = true`), `single-source` (one lock source per architect-family package), `unmapped` (every non-proc-macro crate under `features/` has a row), plus ratcheted `[[idiom]]` source counts (none configured in architect itself; Task uses them).
- An `[[idiom]]` `pattern` matches code only (never comments or string/char literal contents). `path-arg = true` counts a call only when its first argument is a `/`-rooted string literal (axum's `.route("/…", …)`, even with the path on the next line), so `fn route()`, `x.route(path)` and Dioxus helpers don't count. `skip-tests = true` ignores `tests/`, `benches/` and `#[cfg(test)]` items. `allow` exempts paths. Matcher: `xtask/src/boundaries.rs` `idiom_lines`.
- Output: `[rule-id] detail` per violation. `FAIL key: N violations, baseline M` means the count went up: fix it, never raise the baseline. `lower [baseline] key to N` means it went down: lower it in the same PR.
- It reads `Cargo.lock` with `--locked`: a stale lock fails the run.

## fleet-bump

- `cargo run -p xtask -- fleet-bump v0.9.24 ../task ../studio` or `just fleet-bump TAG DIR...`; a directory means its `Cargo.toml`; `-n` reports without writing.
- Edits a line only if its key is `architect`, `auth`, `crdt`, `ui-snapshot` or `<prefix>-*` and the line contains `FastTrackStudios/architect`; replaces the first `tag`/`rev`/`branch` value with `tag = "<TAG>"`. An owned line with none of those is a hard error for that file.
- It does not touch `Cargo.lock`, does not check the tag exists or is ordered, skips owned deps split over several lines, and aborts on the first failing manifest with earlier ones already rewritten.

## CI (`.github/workflows/pr.yml`, `.github/workflows/checks.yml`)

- A PR runs `pr.yml` only: one job, `check` (the status branch protection requires), with `cargo fmt --all -- --check` and a stale-`Cargo.lock` check, about a minute. The full suite below runs after the merge, on every push to main; when main goes red, revert or fix forward.
- `checks.yml`, on main: `CARGO_BUILD_WARNINGS: deny` (every warning fails the job; no `RUSTFLAGS`, see Fleet toolchain). Jobs run on the self-hosted THEBATTLESHIP pool (setup in `.github/actions/shared-rust`: the flake's default shell, rust 1.99.1 (the `rust-toolchain.toml` pin) via `RUSTUP_TOOLCHAIN`, a per-slot target dir over the host's kache), in parallel: `lint` (fmt, `clippy --workspace --all-targets` with no `--all-features`, `boundaries`, `docs`, `surfaces`), `test` (`test --workspace`, includes xtask tests) and `wasm`, the wasm checks of the framework crates (architect, atom, form, permissions, auth-proto, auth-client, crdt, architect-input, plus architect-input with `dioxus`).
- Job `tags` (ubuntu, full history) runs `cargo run -p xtask -- tags --verbose`.
- Not in CI: clippy `--all-features` (`just lint` runs it), `just ui-css-check`, `just auth-ui-css-check`, `just auth-e2e`, ui-snapshot and parity tests.

## Fleet toolchain

Every fleet Rust repo (architect, Task, studio, keyflow, frame) compiles with one rustc, one set of rustflags and one dependency dev profile. kache keys each crate on the rustc version, every rustflag (in order), lint levels, target, features, opt-level and debuginfo, and the bytes of each crate it links against; any difference rebuilds the whole graph per repo instead of sharing one build of Dioxus, Blitz and the registry crates. Architect's copy is the reference: `rust-toolchain.toml`, `.cargo/config.toml`, the `# ── Dependencies: the fleet's settings ──` block in the root `Cargo.toml`, `flake.nix`.

- Toolchain: `rust-toolchain.toml` `channel = "1.99.1"` (components `rustfmt`, `clippy`, `rust-src`; target `wasm32-unknown-unknown` where the repo builds wasm). A nix pin is `pkgs.rust-bin.stable."1.99.1"` (rust-overlay) and moves with the file. No nightly for builds; a separate nightly only for a tool (rustfmt options, udeps) is fine.
- `.cargo/config.toml`, these tables and flag order exactly (a `target.<triple>` table replaces `[build]`, so each repeats the frame-pointer flag):
  ```toml
  [build]
  rustflags = ["-C", "force-frame-pointers=yes"]

  [target.x86_64-unknown-linux-gnu]
  rustflags = [
    "-C", "force-frame-pointers=yes",
    "-C", "link-arg=-fuse-ld=mold",
    "-C", "target-cpu=x86-64-v3",
  ]

  [target.x86_64-apple-darwin]
  rustflags = [
    "-C", "force-frame-pointers=yes",
    "-C", "target-cpu=x86-64-v3",
  ]

  [target.wasm32-unknown-unknown]
  rustflags = [
    "-C", "force-frame-pointers=yes",
    "--cfg", 'getrandom_backend="wasm_js"',
  ]
  ```
  Frame pointers are for samply/perf on the audio path, `x86-64-v3` (AVX2/FMA, Haswell and later) for studio's DSP. mold must be in every dev shell, CI shell and nix build (crane) that links for x86-64 Linux.
- Dependency dev profile: `[profile.dev.package."*"]` `opt-level = 1`, `debug = false`; `[profile.dev.build-override]` `opt-level = 3`, `debug = false`; `[profile.dev-dbg.package."*"]` `debug = 2`; and the same opt-level 3 list for shared hot crates (blake3, num-complex, phon, phon-engine, phon-jit, dioxus-core, dioxus-native-dom, blitz-dom, blitz-paint, blitz-shell, blitz-traits, anyrender, anyrender_vello, vello, vello_encoding, wgpu, wgpu-core, wgpu-hal, naga, stylo, selectors, taffy, parley, skrifa, read-fonts, swash, fontique, peniko, kurbo, plus studio's audio crates: rubato, fixed-resample, fast-interleave, dasp_frame, dasp_sample, rustfft, realfft, transpose, strength_reduce, primal-check, symphonia-core and its codecs/formats, zstd-safe, zstd-sys). Keep the entries for crates in your lockfile; cargo warns about the rest. Overrides on your own workspace members are free: members are never shared.
- CI never sets `RUSTFLAGS` or `CARGO_TARGET_<triple>_RUSTFLAGS` (either replaces the tables). Deny warnings with `CARGO_BUILD_WARNINGS: deny` (cargo's `build.warnings` config, which changes no compiler flag) or `cargo clippy … -- -D warnings` (clippy args only reach workspace members). `RUSTDOCFLAGS` for `cargo doc` is fine. `CARGO_PROFILE_*_DEBUG` and `CARGO_INCREMENTAL` env only reach workspace members once `package."*"` sets `debug`.
- Still not shared across repos: a crate whose resolved features differ between workspaces (cargo unifies features per workspace), a crate that is a workspace member in one repo and a git dependency in another (architect's own crates: path members here, git deps in Task), and different `[patch]` sources or revs.

## Cut a tag

1. Merge to `main` (CI runs on PRs and `main` pushes).
2. Add `## [X.Y.Z] - date` to `CHANGELOG.md` (the fleet's convention is a `docs(changelog): X.Y.Z` PR). The GitHub release body comes from `git log` since the previous tag (`.github/workflows/release.yml`), not from the changelog.
3. `git tag vX.Y.Z <commit>` and push; `release.yml` fires on `v*`.
4. `cargo xtask tags -v` must pass. Never move or renumber a published tag; `GRANDFATHERED` holds only `v0.0.2` and must never grow.
5. Re-pin consumers: `just fleet-bump vX.Y.Z ../task ../studio`, `cargo update -p architect`, then their boundaries run.

`[workspace.package] version` in the root `Cargo.toml` is not the release number; tags are.

## Pitfalls

1. Partial bump: fleet-bump leaves `Cargo.lock` old and `boundaries` fails until it is refreshed.
2. `tag="v"` without spaces fails the needle and errors; owned deps split across lines are skipped silently.
3. Ratchets go one way; lower a baseline in the PR that lowers the count.
4. `examples/greeter` is itself a transport-host row (`architect` with `host` on unconditionally); that is in the baseline.
5. CI clippy is laxer than `just lint`.
6. `cargo xtask tags` on a shallow clone gives false violations.
7. `just auth-replay` deletes its database by design.
8. Toolchain `rust-toolchain.toml`, `.cargo/config.toml`'s rustflags and the dependency dev profile are kept in lockstep with the consumers (Fleet toolchain above); bump them together.
