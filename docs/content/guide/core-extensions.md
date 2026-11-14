+++
title = "Core extensions"
description = "What the standard release of Architect includes: extensions that live in their own repos, chosen by reference in one list, linked into the build or shipped as packages beside it."
weight = 78
+++

Architect's standard release (the desktop app, the node a hosted
Architect runs, the iPhone and iPad app, later the web app) includes a chosen set
of extensions: the **core extensions**. Each one lives in its **own** repo,
Studio's setlists in Studio's, a community extension in its author's.
Architect holds a reference to it, never its code (#631).

There are two tiers:

| | Listed (the store, `extensions/index.toml`) | Core (`extensions/core-extensions.toml`) |
| --- | --- | --- |
| How it gets there | its publisher lists it by pull request | the owner chooses it; anyone (a person or an agent) edits the list when asked |
| Present | after a person installs it | in the standard build |
| Pinned | each version the publisher lists | one exact version: tag, commit, signed release |

## The list

`extensions/core-extensions.toml` has one `[[core]]` entry per extension
(its JSON Schema: `extensions/core-extensions.schema.json`):

```toml
[[core]]
id        = "studio-setlists"           # the extension's id
publisher = "FastTrackStudios"          # whose key signs its releases
source    = "https://github.com/FastTrackStudios/studio"
version   = "26.11.4"                   # its release, exactly
tag       = "v26.11.4"                  # the tag of that release
rev       = "93a2d28a19852cff177ce268c38f5e5d4808bfa9"   # the commit the tag names

[core.signature]                        # the publisher's minisign signature over the release's sums
key     = "org"                         # `org` (the org release key) or the publisher's public key
sums    = "https://github.com/FastTrackStudios/studio/releases/download/v26.11.4/SHA256SUMS"
minisig = "https://github.com/FastTrackStudios/studio/releases/download/v26.11.4/SHA256SUMS.minisig"
comment = "studio v26.11.4 SHA256SUMS"  # its trusted comment, which names the tag

[core.linked]                           # its form on these platforms: compiled in
crate     = "setlists"
features  = ["extension", "share"]
platforms = ["desktop", "node"]
extension = "setlists::ext::Setlists"   # Distribution::linked
blocks    = "setlists::share::blocks"   # Distribution::block, on share links and Sites
patch."https://github.com/FastTrackStudios/daw" = ["daw", "dawfile-reaper", "fts-sample"]
```

An entry has a **form per platform**, one form for each platform it names
(`desktop`, `node`, `ios`, `web`):

- **linked**: compiled into the standard build from its crate at `rev`.
  Needed where a package cannot run or cannot draw: Blitz widgets and
  paint views, iOS (no downloaded native code), the browser until #495,
  and the node's share blocks. `patch` copies the extension repo's own
  `[patch]` tables that its crates need (a patch only applies from the
  workspace root).
- **packaged**: a file the publisher's release carries (`archive`, its
  `sha256`), fetched, checked against the pin and the signed sums, and
  shipped beside the binary in `core/<id>/`. Today the one kind is a
  `folder` taken from a `.tar.gz`, used as a share-page island
  (`[core.packaged.island]`, `Distribution::island`): Studio's island comes
  from Studio's own signed node release this way, built once, by Studio.
  Bundled `.archext` packages (present on first run, updated through the
  registry within their major) are the next step; the list can already
  name one (the commented community example in the file).

`dist/standard` builds `desktop`, `node` and (linked forms only)
`ios` today: `src/main.rs` runs `architect_desktop::run` or, on iOS,
`architect_mobile::run`. Each linked crate is compiled in only where its
platforms say: a crate not listed for `ios` is a
`cfg(not(target_os = "ios"))` dependency and is never built for the
phone, and one the node doesn't need is optional, turned on by `gui`.
Nothing fetches packaged forms into an iOS bundle yet, and `web` is not
built. `cargo xtask core check` names what is not built.
`cargo xtask dist ios --manifest-path dist/standard/Cargo.toml` builds
the iOS app (#640; [your own Architect](@/guide/your-own-architect.md#ios)).

## The standard build: `dist/standard`

`dist/standard` is the distribution crate of the standard release, **outside
architect's workspace** (its own `[workspace]`, its own lock), the shape of
[your own Architect](@/guide/your-own-architect.md):

- its `Cargo.toml` and `dist/standard/src/list.rs` are generated from the
  list by `cargo xtask core sync`: each linked entry is a git dependency at
  its tag, and a call in `list::linked`; each packaged one an entry in
  `PACKAGED`. Adding a core extension is an edit to the list, not
  hand-written Rust;
- every architect crate in its graph is patched back to this checkout, under
  both spellings of architect's URL (`https://github.com/FastTrackStudios/architect`
  and `https://www.github.com/FastTrackStudios/architect`), so Studio's and
  Keyflow's dependencies on an architect tag become this checkout: **one
  Architect** is compiled;
- it copies the root `Cargo.toml`'s `[patch]` tables (its forks) and
  `[profile]` tables, which apply only from a workspace root;
- `dist/standard/src/lib.rs` is the distribution:
  `architect_desktop::distribution::architect()` (the Architect app, its
  name, bundle id and folder) with the list's linked extensions, and the
  packaged islands read from `core/<id>/` beside the binary, each checked
  against its own `architect.lock.json` before it is served
  (`dist/standard/src/packaged.rs`).

Nothing in the workspace depends on it, so the framework builds and
releases without it.

**Task's crates.** They pin an old Architect (v0.10.0), which the patch
would capture, so no linked core extension may pull one:
`cargo xtask core check` and `graph` fail on any. Studio's `setlists`
with `extension` and `share` pulls none (Session's `library`, `publish` and
`project-sync` features, which do, stay off).

## Commands

| command | network | what it does |
| --- | --- | --- |
| `cargo xtask core check` | no | the list's rules; `dist/standard` is what `sync` writes; its lock holds one Architect, no Task crate, each linked crate at its pin. CI's fast check |
| `cargo xtask core sync` | resolves | writes `dist/standard`'s `Cargo.toml`, `src/list.rs` and lock |
| `cargo xtask core verify` | yes | each tag still names its `rev`; each release's sums are signed by the publisher's key with the pinned comment; each packaged archive is listed in them with its `sha256` |
| `cargo xtask core fetch --platform node --into <dir>` | yes | each packaged folder for the platform, checked (sha256, signed sums, its `BUILT_FROM` commit), into `<dir>/core/<id>/` |
| `cargo xtask core graph` | resolves | `dist/standard` resolved as cargo builds it: fails on a second Architect, a Task crate or a moved pin |

Building it is `cargo xtask dist` with `--manifest-path`:

```sh
cargo xtask dist desktop --manifest-path dist/standard/Cargo.toml
cargo xtask dist node --manifest-path dist/standard/Cargo.toml --features node-wasi
cargo xtask core fetch --platform node --into <the folder the node is in>
```

## Adding, bumping or removing one

1. Edit `extensions/core-extensions.toml`: for a bump, `version`, `tag`,
   `rev` (`git ls-remote <source> refs/tags/<tag>^{}`), the signature's
   URLs and comment, and each packaged `archive` and `sha256`.
2. `cargo xtask core verify`: every pin against its source.
3. `cargo xtask core sync`, then build `dist/standard` (above).
4. Commit the list and `dist/standard` together. A bump whose extension
   changes the capabilities it declares, or its publisher key, says so at
   the top of the PR.

Agents make these edits when the owner asks; nothing in the repo restricts
who may.

## CI

- `pr.yml` (fast checks) and `checks.yml` (lint) run `cargo xtask core
  check`. It reads files only: architect's required checks never wait on
  another repo compiling.
- `.github/workflows/standard.yml` builds `dist/standard` against main on
  every push to main and nightly, on the self-hosted pool main's heavy jobs
  use: the list, `core graph`, the desktop binary, the node with its
  packaged folders fetched, and `dist/standard`'s tests. It is **not**
  required: a framework change that breaks a core extension shows up there
  before a release, without blocking the framework.

Not wired yet: the release (`release.yml`) does not build the standard app;
the standard release needs `dist/standard` built green from the tag, and the
server's node would then come from architect's release.
