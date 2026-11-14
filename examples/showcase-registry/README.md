# Showcase registry

The local test registry of the extension showcase (#710, rows L1-L4): a
folder registry of signed archives, and a vault that needs some of them.
It is checked by `features/extensions/tests/showcase_registry.rs` and by
`cargo xtask ext-index examples/showcase-registry --key examples/showcase-registry/keys/showcase.pub`.

| file | what it is |
|---|---|
| `index.toml` | the index: one `[[extensions]]` entry per version |
| `archives/` | the signed archives it lists |
| `keys/showcase.pub` | the public half of the throwaway key they are signed with (the secret half was thrown away) |
| `packages/` | the packages the archives are made from |
| `vault/` | a vault that needs extensions: `.architect/extensions.toml` names them, `.architect/extensions/` carries one |
| `make.sh` | makes the archives, the index and the key again |

What each one shows:

| extension | shows |
|---|---|
| `showcase.greeter` 0.1.0 | L1: installed through a consent prompt listing what it asks in words (the selection, notifications) |
| `showcase.greeter` 0.2.0 | L3: an update, with its changelog, that asks for one more thing (the clipboard) and waits for a yes |
| `showcase.word-count` | L2: required by the vault, installed by "Install all" |
| `showcase.tiny` | L2: carried in the vault's `.architect/extensions/`, in no registry |
| `showcase.mac-only` | L4: `platforms = ["macos"]`: refused here off macOS, with why, and listed as such by the vault |

## In the app

Add it in Settings → Extensions → Registries and development installs
(a folder, with the key in `keys/showcase.pub`), or with
`EXT_REGISTRY_ADD {name: "Showcase", folder: "<checkout>/examples/showcase-registry", keys: [{name: "Showcase", key: "<the .pub's second line>"}]}`.
Then open `vault/` as the vault (`ARCHITECT_VAULT=<checkout>/examples/showcase-registry/vault`,
in a scratch copy: installing writes nothing there, but the app keeps its
layouts in `.architect/`): it lists what is missing, with "Install all".

## The showcase's own extensions

`make.sh` signs more package folders into the same registry, for the
showcase harness's examples (`examples/showcase/`):

```sh
examples/showcase-registry/make.sh examples/showcase/extensions/<name> ...
```

Every archive is signed again with a new key each time, so `keys/showcase.pub`
changes with them; commit the whole folder.
