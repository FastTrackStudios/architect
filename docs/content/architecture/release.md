+++
title = "Signed self-update"
description = "Replace a running binary with a newer GitHub Release, only after its signature, checksum and version check out."
weight = 65
+++

`architect::release` (feature `release`, native only) keeps a binary
installed outside a package manager on the newest release. An app
describes its releases once; the check, the verification and the swap are
shared.

## Use it

```rust,ignore
use architect::release::{self, Args, Options, Target, Updater};

/// The org release key's public half (the base64 line of its `.pub`).
/// `None` in a build that pins no key: see below.
const RELEASE_KEY: Option<&str> = Some("RWQ...");

let opts = match Options::parse(&args)? {
    Args::Run(opts) => opts,
    Args::Help => return Ok(println!("{}", release::usage("task-server"))),
};
let exe = release::current_exe()?;
let outcome = Updater::new("task-server", "FastTrackStudios/task", release::platform_asset("task-server"))
    .token(std::env::var("GITHUB_TOKEN").ok())
    .run(Target { current: env!("CARGO_PKG_VERSION"), exe: &exe, key: RELEASE_KEY }, &opts)
    .await?;
println!("{outcome}");
```

`Options` is `--check`, `--to <X.Y.Z>` (also a downgrade) and
`--then <command…>`. The `Outcome` says what happened (`NoRelease`,
`Current`, `AlreadyInstalled`, `Available`, `Installed`) and prints as a
one-line report.

## What a run checks

| Step | What happens | Refused as |
|---|---|---|
| Find | The newest `<prefix>X.Y.Z` release (default prefix `v`; `.tag_prefix("desktop-v")`) that is not a draft or prerelease and carries the asset and `SHA256SUMS`. With `--to`, exactly that version. | `NotFound` (only with `--to`) |
| Key | A build with `key: None` stops here, before downloading anything. | `NoSigningKey` |
| Signature | `SHA256SUMS.minisig` must be by the key, and its trusted comment must be `<app> <tag> SHA256SUMS` (`trusted_comment`), so a real signature copied from an older release is refused. | `Unsigned`, `BadSignature`, `BadKey` |
| Checksum | The asset streams into a staging file beside the binary (random name, `O_EXCL`, `0700`) and must hash to what the signed sums list. | `NotListed`, `ChecksumMismatch` |
| Smoke test | The staged binary runs with `--version` (30 s limit) and must print exactly `<app> <version>`. | `SmokeTest` |
| Swap | The staged file is renamed over the binary: one atomic step on one filesystem. | `Io` |
| Hook | `--then` runs only after a new binary went in. | `Hook` (the new binary **is** installed) |

Any refusal before the swap leaves the installed binary byte-identical and
removes the staging file. A binary under `/nix/store` is never replaced
(`NixStore`): a Nix host moves its flake pin instead.

## The release key

The fleet has one org-wide minisign key. Its secret half is a
release-workflow secret that signs every release's `SHA256SUMS`:

```sh
minisign -S -s release.key -m SHA256SUMS -t "task-server $TAG SHA256SUMS"
```

Each app compiles in the public half and passes it as `key`. The signature
is what makes the checksums mean anything: whoever can write a release's
assets can replace the binary and its sums together.

`key: None` is the explicit "this build pins no key" (a dev build). It is
never a silent skip: `--check` still reports updates, and installing fails
with `NoSigningKey`.

## The GitHub token

`.token(Some(..))` sends a bearer token for the rate limit, only to URLs
under the API base (`.api_base`, default `https://api.github.com`), never to
asset downloads.

## Not yet

- macOS: renaming over a running bare binary works as on Linux, but a `.app`
  bundle (code signature, Gatekeeper, the Task desktop's staged-bundle
  swap) is not handled here. Check the bare-binary swap on a Mac
  before relying on it there.
- Windows: renaming over a running `.exe` fails, so the old binary stays
  in place.

Extracted from Task's `task-server self-update` (STA-387).
