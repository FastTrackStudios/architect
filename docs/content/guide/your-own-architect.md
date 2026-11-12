+++
title = "Your own Architect"
description = "A distribution of Architect as code: its name, bundle id, icon and folder, and the extensions you choose compiled in, built for desktop, the web and the node with cargo xtask dist."
weight = 77
+++

Anyone can build an Architect of their own with the extensions they want
**compiled in** (linked) instead of installed. A linked extension runs in
the app's own process, with no process boundary and no WASI runtime between
it and the window. It is the same extension the store installs: same id,
same version, and a development copy still overrides it. Everything else
stays installable at run time, as in every build.

The Architect app is one such build. The dev sandbox is another. Studio's
app is a third: Architect with Studio's setlists, charts and sessions
compiled in, under Studio's name.

## A distribution is a crate

A distribution is a binary crate whose `main` is one line
(`apps/distribution/src/lib.rs` `Distribution`):

```rust
fn main() -> std::process::ExitCode {
    architect_desktop::run(
        architect_desktop::distribution::architect()   // the Architect app's own list
            .named("Studio")
            .bundle_id("app.fasttrackstudio.studio")
            .icon(architect_desktop::Icon::svg(include_str!("../icon.svg")))
            .linked(&setlists::ext::Setlists),
    )
}
```

- `architect()` is the Architect app: its first-party extensions
  (Projects, Tasks and the Time Tracker as the Project Management bundle;
  the Calendar, Day Plans, Threads, Notify, Files). Start from
  `Distribution::new("Name")` instead for none of them.
- `named` gives it a name. Its app name (the control socket's,
  `<app> actions list`), its folder under the platform's config dir and
  its bundle id follow the name; set them with `app`, `folder` and
  `bundle_id`.
- `linked(&X)` compiles in one extension, and `bundle(Bundle::new(..).member(&X))`
  compiles in several that work together (#367). An extension listed twice
  is compiled in once.
- `registry(Registry::git(..))` adds a registry its people install from,
  after the store's, which every distribution has
  (`architect_distribution::store`). `without_registries()` drops them
  (the person's own still work); `without_installs()` turns registries and
  development installs off entirely, as the sandbox does (#493).
- `block(FnBlock::new(..))` gives the node a Rust renderer for a fence
  (Keyflow's engraver for `kf`), drawn on share links and Sites before
  the linked extensions' embed pages.

Every face of the build reads this one list:

| face | what it does with the list |
| --- | --- |
| the window | links each extension (`apps/ui/src/extensions.rs` `use_lab_extensions`), beside the core every build has: Bases, the theme picker, the Command Palette, Navigator, the extensions manager, the settings page |
| the node (`<bin> serve`) | draws each extension's `[[embeds]] page` that claims a fence on share links and Sites, and the distribution's own `block`s (`Distribution::blocks`; the page ships in the extension's `Extension::files`) |
| the web bundle | `/app/distribution.json` (`Distribution::describe`): the web app takes its name and lists the extensions compiled in |

`run` keeps everything the Architect app does: first run, vaults,
accounts, the control socket, MCP, `--headless`, `serve` and its
`files`/`agent` commands. `<bin> distribution` prints what the build is,
as JSON, and `<bin> distribution icon` prints its icon.

### Its `Cargo.toml`

```toml
[features]
default = ["gui"]
gui = ["architect-desktop/gui"]   # the window; off, the binary is the node

[dependencies]
architect-desktop = { git = "https://github.com/FastTrackStudios/architect", tag = "vX.Y.Z", default-features = false, features = ["vox"] }
setlists = { git = "https://github.com/FastTrackStudios/studio", tag = "…", features = ["extension"] }
```

`vox` is required: it is the node's services. Pin one architect tag, the
one the extensions you link pin, so the lockfile has one architect source.
Copy the `[patch]` tables from architect's root `Cargo.toml` too: they pin
the forks it builds against (Blitz, dioxus-native, vox), and a patch only
applies from the workspace root.

## Building one

`cargo xtask dist` builds every face of a distribution the way the release
builds the Architect app, by the same scripts (`xtask/src/dist.rs`). Run it
from an architect checkout at the tag the crate pins:

```sh
cargo xtask dist desktop node web --manifest-path ../my-architect/Cargo.toml
cargo xtask dist node-image web-image --manifest-path ../my-architect/Cargo.toml
cargo xtask dist all           # no --manifest-path: the Architect app itself
```

| target | how | out (in `--out`, default `target/dist`) |
| --- | --- | --- |
| `desktop` | the window's build, then `scripts/desktop/linux.sh` (an AppImage) or `scripts/desktop/macos.sh` (a signed dmg, on airlock) | `<Name>-linux-x86_64.AppImage` |
| `node` | `--no-default-features` and `--cfg no_workspace_hack`, as the release's node | the binary |
| `node-image` | the node in `flake.nix`'s `architect-image-prebuilt` | `<folder>-node.tar` |
| `web` | `scripts/web-bundle.sh`, with the node's `distribution` as `/app/distribution.json` | `web/` |
| `web-image` | the bundle in `flake.nix`'s `architect-web-image-prebuilt` | `<folder>-web.tar` |

The scripts take who the app is from the built binary (its name, binary,
bundle id and icon, as `APP_*` variables), so nothing is written twice. The
images are docker archives: load them with `docker load` or push them with
`skopeo copy docker-archive:<tar> docker://…`. Each is a deployment of its
own: the [chart](@/guide/deploy.md) with `image.registry`,
`node.image.repository` and `web.image.repository` pointed at them.

The release workflow (`.github/workflows/release.yml`) builds the
Architect app through the same command: `cargo xtask dist node`, `web` and
`desktop`.

`--features a,b` turns on features of the distribution crate (with
`--manifest-path`), such as `node-wasi` for a node that draws installed WASI
components' blocks as the release's node does. The standard release's own
distribution crate, `dist/standard`, is built this way from the
[core extensions](@/guide/core-extensions.md) list.

## The web

The web app (`apps/web`) is one wasm for every distribution: it reads
`/app/distribution.json` for its name and lists what is compiled in. The
deployment's node is the same binary running `serve`, so its share links
and Sites draw the blocks those extensions claim. Their views and actions
don't run in a browser yet: a linked extension needs the extension host,
which is native only today, and Architect's shell on the web
(#495).

## Proving it

A crate outside this repo that links Studio's setlists builds the desktop
app and the web bundle this way. Launched in a test session, its setlists
and libraries are in the navigator. Its bundle passes `apps/web/test/web.test.mjs` in a headless
Firefox. `apps/distribution/tests/distribution.rs` covers the list, the
identity and a linked extension's page drawing its fence on a Site.
