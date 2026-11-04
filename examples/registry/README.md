# Extension registry (skeleton)

The skeleton of the FastTrackStudios extension registry: copy this folder
to the root of a new repository (`FastTrackStudios/extensions`) and it is
one. An app adds it as a `GitRegistry` and installs from its index; the
reusable workflow `.github/workflows/extension-release.yml` in architect
opens a pull request here each time an extension is released.

What is in it:

| file | what it is |
|---|---|
| `index.toml` | the index: one `[[extensions]]` entry per version, read by `features/extensions/src/index.rs` |
| `index.schema.json` | the index's JSON Schema (editors with taplo read the `#:schema` line); `cargo xtask ext-index` checks the same rules |
| `archives/` | archives kept in the repository; a real entry's `archive` is usually its release asset's URL instead |
| `keys/throwaway.pub` | the public half of the throwaway key the sample is signed with (the secret half was discarded) |
| `.github/workflows/check.yml` | the registry's CI: `cargo xtask ext-index` on every push and pull request |

## An entry

```toml
[[extensions]]
id = "fts.studio"
name = "Studio"
version = "1.2.0"
kind = "process"
description = "REAPER, from the app."
archive = "https://github.com/FastTrackStudios/studio/releases/download/ext-v1.2.0/fts.studio-1.2.0.archext"
sha256 = "…"                          # the archive's, lower-case hex
platforms = ["aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"]
changelog = "The mixer view."         # what "update available" shows
```

`path` (a folder here) and `git` + `rev` (a repository at a tag) still
work for unsigned packages a registry vouches for by being the registry;
an `archive` is installed only once its signature checks.

## Checking it

From an architect checkout:

```sh
cargo xtask ext-index examples/registry --key examples/registry/keys/throwaway.pub
```

It checks the schema, fetches each archive (a path here, or a URL with
`curl`), compares its sha256 with the entry's, verifies its signature
against the org release key (and each `--key`), and compares the archive's
id, version, kind and platforms with the entry's.

## Trust

An app trusts the org release key by default (`Trust::org`, the public half
compiled in). A registry's own keys are trusted only for its own archives,
and only when the app says so (`GitRegistry::trusting`). An archive signed
by any other key, unsigned, or changed after signing is refused before
anything of it is written.

## Making the sample again

```sh
cargo xtask ext-keygen /tmp/regkey
cargo xtask ext-pack examples/extension/registry/hello --out examples/registry/archives/example.hello-0.1.0.archext
cargo xtask ext-sign examples/registry/archives/example.hello-0.1.0.archext --key /tmp/regkey/throwaway.key
cp /tmp/regkey/throwaway.pub examples/registry/keys/
sha256sum examples/registry/archives/example.hello-0.1.0.archext   # into index.toml
```
