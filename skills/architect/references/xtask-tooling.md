# architect xtask, boundaries, CI and releases

Subcommands are declared in `xtask/src/main.rs` (`Command`); `cargo xtask --help` is the live list. Layout ADR: `docs/content/architecture/layout.md`.

## Commands

| Command | What it does | Code |
|---|---|---|
| `cargo xtask feature new <name>` | writes ONE crate `features/<name>/` (contract as the default feature set, wasm-clean; in-memory backend behind `memory`; `vox` and `ui` flags; a `spec/`; a passing test), registers it in workspace members and `[workspace.dependencies]`, adds boundaries rows, runs rustfmt | `xtask/src/scaffold.rs` `feature_new`; acceptance test `xtask/tests/feature_new.rs` |
| `cargo xtask tags [-v]` | every `vX.Y.Z` tag must descend from every lower-numbered one | `xtask/src/tags.rs` `run`, `GRANDFATHERED` |
| `cargo xtask boundaries [--config F] [-v]` | resolves each `boundaries.toml` row with `cargo tree`, runs the layer rules, ratchets counts | `xtask/src/boundaries.rs` `run_cli` |
| `cargo xtask surfaces` | every member whose `src/` serves MCP also serves the `actions` CLI / control socket (and the reverse), each with an `architect_mcp::parity` test; MCP's generic tools built from `Method::TOOLS` | `xtask/src/surfaces.rs` `run_cli`, `problems` |
| `cargo xtask feature-tests` | every test `cargo nextest run --workspace` leaves out (a `[[test]]` whose `required-features`, or a test whose `cfg(feature …)`, is off in the workspace's unified build, resolved with guppy) is run by a `[[pass]]` in `feature-tests.toml` or is a `[[skip]]` with its reason; no pass runs nothing new, no skip is stale. `--run` runs the passes (main's Tests job, `just test`) | `xtask/src/feature_tests.rs` `run_cli` |
| `cargo xtask docs` | every repo path, page link, `just` recipe, `cargo xtask` command and `-p` package named in `docs/content/**/*.md`, `README.md` and `skills/**/*.md` must exist | `xtask/src/docs.rs` `run_cli`, `check_text` |
| `cargo xtask fleet-bump <TAG> <MANIFEST or DIR>... [-n]` | rewrites architect-owned git dep lines to `tag = "<TAG>"` | `xtask/src/fleet.rs` `run`, `OWNED_PREFIXES` |
| `cargo xtask fleet-deps [--repo DIR]... [--check] [--report]` | writes the fleet's feature union into `workspace-hack/Cargo.toml` (FLEET section) from each consumer checkout; without `--repo`, only gates members' hack lines; `--check` (CI) verifies gates and that every entry is still built here; `--report` prints each consumer's feature differences | `xtask/src/fleet_deps.rs` `run`, `union`, `GATE` |
| `cargo xtask ext-pack <dir> [--out F]` | the unsigned `.archext` of the package in `<dir>`: its files, `bin/<target-triple>/` builds, a content index of SHA-256s; refuses a process build missing its `[entry]` program | `xtask/src/ext.rs` `pack_cli`; `features/extensions/archive/src/lib.rs` `pack` |
| `cargo xtask ext-sign <archive> [--key F] [--expect PUB] [--out F]` | signs the archive's index with a minisign secret key (unencrypted, `minisign -G -W`): `--key` file or `EXT_SIGNING_KEY` text; `--expect` refuses any other key; never prints the key | `xtask/src/ext.rs` `sign_cli`; `features/extensions/archive/src/sign.rs` `sign_archive` |
| `cargo xtask ext-verify <archive> [--key PUB]... [--no-org]` | opens it as an app would: signature (org key + `--key`s), hashes, this platform's build | `xtask/src/ext.rs` `verify_cli` |
| `cargo xtask ext-keygen <dir> [--name N]` | a throwaway key pair for trials and tests, never a release key | `xtask/src/ext.rs` `keygen_cli` |
| `cargo xtask ext-index [dir] [--key PUB]... [--no-org]` | checks a registry's `index.toml`: schema, each archive's pinned sha256, signature, and that it is what its entry says; a registry's CI | `xtask/src/ext.rs` `index_cli`; `features/extensions/archive/src/registry.rs` `check` |
| `cargo xtask dist <desktop|node|node-image|web|web-image|all>... [--manifest-path F] [--bin B] [--out D] [--profile P]` | builds a distribution of Architect (#491): from a distribution crate (its `main` `architect_desktop::run(..)`), else the Architect app; asks the built binary who it is (`<bin> distribution`), then runs the release's own `scripts/desktop/linux.sh` / `scripts/desktop/macos.sh` (`APP_*`), `scripts/web-bundle.sh` (`ARCHITECT_DISTRIBUTION`) and `flake.nix`'s prebuilt images (`ARCHITECT_IMAGE_NAME`, `ARCHITECT_IMAGE_BIN`); prints `key=path` per artifact; the release workflow builds through it | `xtask/src/dist.rs` `run_cli`; `docs/content/guide/your-own-architect.md` |
| `cargo xtask auth-mirror --from URL [--out F] [--redact-emails]` | pulls a server-side-scrubbed copy of an auth server's org graph (needs `AUTH_ADMIN_TOKEN`) | `xtask/src/mirror.rs` `run` |

In the architect repo `cargo xtask` is `cargo run -p xtask --`. Consumers build the binary from the tag they pin (`cargo install --locked --git https://github.com/FastTrackStudios/architect --tag <TAG> xtask`) and run it from their own root. Task aliases `cargo xtask` to its own codegen xtask, so call architect's binary by path there.

## `cargo xtask docs`: writing checkable docs and skills

- A token in a code span or fenced block whose first segment is a top-level directory of the repo is a path and must exist; `:12-40` line suffixes are stripped. Name the symbol beside the path instead of a line number: `` `crates/architect/src/action_list.rs` `ActionList::register_with` ``.
- Other repos' paths take a repo prefix and are skipped: `task:apps/server/src/mcp.rs`, `studio:apps/session/daw/src/actions.rs`.
- Placeholders (`<name>`, `*`, `{x}`, `…`, `$X`, `|`) are skipped; so is anything between `<!-- doc-check: off -->` and `<!-- doc-check: on -->`.
- `just <recipe>`, `cargo xtask <cmd>`, `cargo run -p xtask -- <cmd>` and `cargo … -p <package>` are checked against the justfile, the xtask binary and the workspace packages.
- CI runs it on every PR (job `lint`, step `docs`); `just docs-check` runs it locally.

## boundaries

- Each row: `cargo tree --locked -p <pkg> -e normal` with the row's features; L0 rows resolve for `wasm32-unknown-unknown`. It runs with `RUSTFLAGS="--cfg no_workspace_hack"`, which switches the workspace-hack off (see Fleet feature union), so rows check what a crate declares, not the fleet's union.
- Rules: `contract-clean` (L0 must not reach engine or ui packages, a tokio runtime, or a transport-host/server `architect` feature), `ui-reach` (L4 must not reach engine packages or transport/server features; Dioxus is fine), `transport-host` (no member outside `apps/` enables a host in its default set), `lints` (`[lints] workspace = true`), `single-source` (one lock source per architect-family package), `unmapped` (every non-proc-macro crate under `features/` has a row), plus ratcheted `[[idiom]]` source counts (none configured in architect itself; Task uses them).
- An `[[idiom]]` `pattern` matches code only (never comments or string/char literal contents). `path-arg = true` counts a call only when its first argument is a `/`-rooted string literal (axum's `.route("/…", …)`, even with the path on the next line), so `fn route()`, `x.route(path)` and Dioxus helpers don't count. `skip-tests = true` ignores `tests/`, `benches/` and `#[cfg(test)]` items. `allow` exempts paths. Matcher: `xtask/src/boundaries.rs` `idiom_lines`.
- Output: `[rule-id] detail` per violation. `FAIL key: N violations, baseline M` means the count went up: fix it, never raise the baseline. `lower [baseline] key to N` means it went down: lower it in the same PR.
- It reads `Cargo.lock` with `--locked`: a stale lock fails the run.

## fleet-bump

- `cargo run -p xtask -- fleet-bump v0.9.24 ../task ../studio` or `just fleet-bump TAG DIR...`; a directory means its `Cargo.toml`; `-n` reports without writing.
- Edits a line only if its key is `architect`, `auth`, `crdt`, `ui-snapshot` or `<prefix>-*` and the line contains `FastTrackStudios/architect`; replaces the first `tag`/`rev`/`branch` value with `tag = "<TAG>"`. An owned line with none of those is a hard error for that file.
- It does not touch `Cargo.lock`, does not check the tag exists or is ordered, skips owned deps split over several lines, and aborts on the first failing manifest with earlier ones already rewritten.

## CI (`.github/workflows/pr.yml`, `.github/workflows/checks.yml`)

- A PR runs `pr.yml` only: one hosted job, `check` (the status branch protection requires), in the same devenv shell as main's checks, with `fleet-doctor`, `cargo fmt --all -- --check`, a stale-`Cargo.lock` check, the workspace-hack check (`cargo hakari generate --diff`, `cargo hakari manage-deps --dry-run`) and `change files` (`cargo rail change check --since` the PR's base: every crate the PR changes needs a `.changes/*.md`; `just ci-changes` locally). The full suite below runs after the merge, on every push to main; PRs merge in batches without waiting on it, and main's run is the gate: when main goes red, fix forward or revert.
- `checks.yml`, on main: `CARGO_BUILD_WARNINGS: deny` (every warning fails the job; no `RUSTFLAGS`, see Fleet toolchain). Jobs run on the self-hosted THEBATTLESHIP pool (setup in `.github/actions/shared-rust`: the pinned devenv, then every `run` step in the repo's devenv shell (`shell: devenv shell -- bash -e {0}`), whose rustc `fleet-doctor` asserts is the pinned store path; a per-slot target dir over the host's kache), in parallel: `lint` (fmt, `clippy --workspace --all-targets` with no `--all-features`, `boundaries`, `docs`, `surfaces`, `feature-tests`, `fleet-deps --check`), `test` (`nextest run --workspace`, includes xtask tests, then `feature-tests --run`: each pass's `-p … --features …` run; the WASI tests with `ARCHITECT_WASI_REQUIRED`/`ARCHITECT_WASI_WEB_REQUIRED`, so they fail rather than skip) and `wasm`, the wasm checks of the framework crates (architect, atom, form, permissions, auth-proto, auth-client, crdt, architect-input, plus architect-input with `dioxus`).
- Job `tags` (ubuntu, full history) runs `cargo run -p xtask -- tags --verbose`.
- Not in CI: clippy `--all-features` (`just lint` runs it), `just ui-css-check`, `just auth-ui-css-check`, `just auth-e2e`, ui-snapshot and parity tests.

## Fleet toolchain

Every fleet Rust repo (architect, Task, studio, keyflow, frame) compiles with one rustc, one set of rustflags and one dependency dev profile. kache keys each crate on the rustc version, every rustflag (in order), lint levels, target, features, opt-level and debuginfo, and the bytes of each crate it links against; any difference rebuilds the whole graph per repo instead of sharing one build of Dioxus, Blitz and the registry crates. Architect's copy is the reference: `rust-toolchain.toml`, `.cargo/config.toml`, the `# ── Dependencies: the fleet's settings ──` block in the root `Cargo.toml`, `devenv.yaml` and `devenv.nix` (the dev shell, which imports the toolchain and mold from devenv-fleet).

- Toolchain: `rust-toolchain.toml` `channel = "1.99.1"` (components `rustfmt`, `clippy`, `rust-src`; target `wasm32-unknown-unknown` where the repo builds wasm). No nightly for builds; a separate nightly only for a tool (rustfmt options, udeps) is fine.
- The shell: every repo's `devenv.yaml` imports devenv-fleet's `rust` module (github.com/FastTrackStudios/devenv-fleet, by tag) and pins its `nixpkgs` and `rust-overlay` inputs to devenv-fleet's `pins.json`: nixpkgs `8ce4ef6c` (2026-09) and rust-overlay `5b9f1198`. The module sets the toolchain (`pkgs.rust-bin.stable."1.99.1"` with wasm32) as `languages.rust.toolchainPackage`, so every shell and CI job gets the same store path (`/nix/store/50x4m6r9zc0j2iz5cr6ppsac04z057zf-rust-default-1.99.1` on x86_64-linux): the nixpkgs pin decides it, so it moves for the whole fleet at once, through devenv-fleet. It keeps the GCC link driver (`clangLinker.enable = false`; devenv's default swaps mold for LLD) and adds mold, cargo-hakari, prek and cargo-rail; kache stays the host's (never set or unset `RUSTC_WRAPPER`). `devenv shell -- fleet-doctor` prints the rustc path, the wrapper and the linker, and fails off the pin. Bump with devenv-fleet's `scripts/fleet-pins --write .`; never hand-edit the revisions. `.envrc` is `eval "$(devenv direnvrc)"` then `use devenv` (a bare `use devenv` fails, silently in an agent shell), and never devenv's `dotenv` (it copies `.env` into the store; use direnv's `dotenv_if_exists`).
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
  Frame pointers are for samply/perf on the audio path, `x86-64-v3` (AVX2/FMA, Haswell and later) for studio's DSP. mold must be in every dev shell, CI shell and nix build (crane) that links for x86-64 Linux; check a binary with `readelf -p .comment` (`mold`, not `LLD`).
- Dependency dev profile: `[profile.dev.package."*"]` `opt-level = 1`, `debug = false`; `[profile.dev.build-override]` `opt-level = 3`, `debug = false`; `[profile.dev-dbg.package."*"]` `debug = 2`; and the same opt-level 3 list for shared hot crates (blake3, num-complex, phon, phon-engine, phon-jit, dioxus-core, dioxus-native-dom, blitz-dom, blitz-paint, blitz-shell, blitz-traits, anyrender, anyrender_vello, vello, vello_encoding, wgpu, wgpu-core, wgpu-hal, naga, stylo, selectors, taffy, parley, skrifa, read-fonts, swash, fontique, peniko, kurbo, plus studio's audio crates: rubato, fixed-resample, fast-interleave, dasp_frame, dasp_sample, rustfft, realfft, transpose, strength_reduce, primal-check, symphonia-core and its codecs/formats, zstd-safe, zstd-sys). Keep the entries for crates in your lockfile; cargo warns about the rest. Overrides on your own workspace members are free: members are never shared.
- CI never sets `RUSTFLAGS` or `CARGO_TARGET_<triple>_RUSTFLAGS` (either replaces the tables). Deny warnings with `CARGO_BUILD_WARNINGS: deny` (cargo's `build.warnings` config, which changes no compiler flag) or `cargo clippy … -- -D warnings` (clippy args only reach workspace members). `RUSTDOCFLAGS` for `cargo doc` is fine. `CARGO_PROFILE_*_DEBUG` and `CARGO_INCREMENTAL` env only reach workspace members once `package."*"` sets `debug`.
- Still not shared across repos: a crate locked at another version, or from another `[patch]` source or rev, in one repo (`cargo update -p <crate> --precise <version>` aligns it; the fleet keeps each crate at the highest version any repo locks in its semver line); a crate that is a workspace member in one repo and a git dependency in another (architect's own crates: path members here, git deps in Task); and anything above either. Feature differences are what the Fleet feature union removes.

## Fleet feature union

Cargo unifies features per workspace, so a crate every repo builds (tokio, Dioxus, reqwest, serde_json) resolved with different features in each, and kache rebuilt it, and everything above it, per repo. Two generated sections of `workspace-hack/Cargo.toml` (package `architect-workspace-hack`) fix that:

- HAKARI: this workspace's union, by cargo-hakari (`.config/hakari.toml`: `resolver = "2"`, the fleet's native `platforms`, `unify-target-host = "none"`, and `xtask` plus the proc-macro members in `traversal-excludes`, since a proc macro with the hack would build the whole union a second time for the host).
- FLEET: the consumers' features on top, by `cargo xtask fleet-deps`. It resolves each repo the way hakari does (`--workspace --all-targets`, with default and with all features, per platform, with guppy, hakari's resolver, and the hack crate left out), and writes the union for every third-party crate architect builds that a consumer builds too, where the repos don't already agree. The tables are spelled as cfgs (`[target.'cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))'.dependencies]`), because hakari's own tables already use the triples. Host-side crates (build scripts, proc macros) go in `build-dependencies`, which `workspace-hack/build.rs` makes cargo resolve.

Every member depends on the hack under `GATE` (`[target.'cfg(not(any(target_family = "wasm", no_workspace_hack)))'.dependencies]`). Consumers depend on architect, so the hack is in their graph and its features unify into theirs: a consumer's extra features become a no-op, and a shared crate builds the same everywhere. It's off on wasm because a whole-workspace union there would put the native server stack (tokio `net`, the desktop renderer) into every web build, so wasm builds aren't unified. `--cfg no_workspace_hack` switches it off for `xtask boundaries`.

- After a dependency change: `cargo hakari generate`, then `cargo xtask fleet-deps --repo ../task --repo ../studio --repo ../keyflow --repo ../frame` (consumer checkouts on their main). It starts the FLEET section from empty and repeats until it settles, since a feature can bring in crates (Dioxus `fullstack` brings `dioxus-server`). If the old section stops cargo resolving at all, delete the section by hand first.
- A new member: `cargo hakari manage-deps` adds its line under `[dependencies]`, then `cargo xtask fleet-deps` moves it under `GATE`. `manage-deps` can't remove a gated line: delete it by hand when you exclude a member.
- A consumer has its own hakari crate (package `workspace-hack`, the same `.config/hakari.toml` settings and gate). It aligns its `[patch]` tags and its locked versions with architect's, or its graph gets a second copy of the crate.
- Consumers inherit the union's crates: about 55 that architect's own graph didn't have before (Task's fullstack and server crates, `aws-lc-rs`/`aws-lc-sys` from Task's `tokio-rustls`). Dioxus `mobile` on Linux and macOS is the desktop renderer (wry, tao), which architect already built.

## Cut a tag

1. Merge to `main` with each PR's change file (`.changes/README`). A lone PR is squash-merged; a stack merges with merge commits titled with each PR's title.
2. `just release-next` names the next semver tag from the highest bump since the previous tag (before 1.0: `major` bumps the minor number, anything else the patch number); `just release-notes` prints the draft notes. Fix a note by editing its change file on `main` before tagging. Nothing edits `CHANGELOG.md`: it ends at v0.10.0.
3. `git tag vX.Y.Z <commit>` and push; `release.yml` fires on `v*` and publishes the GitHub Release with `scripts/release-notes.sh` `notes`: the change files added since the previous tag, never the commits.
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
9. A consumer that bumps the architect tag without aligning its `[patch]` tags (Blitz, vendor) gets two copies of each patched crate: the hack names architect's source.
10. `cargo hakari manage-deps` writes a new member's line ungated; until `cargo xtask fleet-deps` gates it, that member's wasm build pulls the native union and fails.
