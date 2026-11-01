# An extension, end to end

A small host program and two extensions, to copy when you write your own
(`architect-extensions`, `features/extensions`). The walkthrough is
`docs/content/getting-started/extension.md`.

```text
examples/extension/
├── registry/                 a registry: a folder with an index
│   ├── index.toml            what it offers
│   └── hello/                a Luau extension
│       ├── extension.toml    its manifest: actions, settings, an overlay
│       ├── greet.luau        the action EXAMPLE_HELLO_GREET
│       └── ruler.luau        the overlay: a ruler over the timeline, a tool window
├── counter/                  a process extension (crate example-extension-counter)
│   ├── extension.toml        its manifest: an action, a view
│   ├── src/main.rs           Guest::new().view(..).action(..).serve()
│   └── tests/survives_kill.rs
└── host/                     the host (crate example-extension-host)
    ├── src/lib.rs            what makes an app a host
    ├── src/main.rs           the host with a window
    └── tests/registry.rs
```

## Run it

```sh
cargo build -p example-extension-counter
cargo run -p example-extension-host
```

The host finds `examples/extension/registry` above the working directory
(or at `EXAMPLE_EXTENSION_REGISTRY`), and runs the counter as a
development copy when its program was built beside the host's. Then:

- **Install Hello** installs `example.hello` from the registry. Its
  action is on the Action List (the next button runs it), its section is
  on the settings page (**Settings**), and its ruler draws over the
  timeline with a tool window under the counter. **Uninstall Hello**
  takes every part of it off again.
- **Edit `registry/hello/ruler.luau`** (the `every` it starts with, a
  colour) and save: the host reloads it within half a second and the
  ruler redraws. A save that does not parse keeps the last good copy.
- **Click the counter**, then `kill -9` its process (its pid is in the
  pane): the host runs on, starts it again, and the view comes back
  drawn by the new process.

## Test it

```sh
cargo test -p example-extension-host     # install, run, settings, overlay, reload, remove
cargo test -p example-extension-counter  # a click in the view, kill -9, back again
```

## Write your own

- **A Luau or data extension**: copy `registry/hello` into a registry
  (a folder with an `index.toml`, or a git repository of one), change its
  `id`, and keep every action id under the prefix the id gives
  (`acme.tools` → `ACME_TOOLS_`) and every setting key under `acme.tools.`.
  List it in the registry's `index.toml`.
- **A process extension**: copy `counter/`. Declare each view and
  action in `extension.toml` and serve each one from `main`; the host
  refuses a program whose hello lacks one. To publish it, put the built
  program in the package at the path `[entry]` names.
- **A host**: `host/src/lib.rs` is the part to copy — `extensions` (a
  `Host` with a process runner, a registry, the `EXT_*` actions) and
  `overlays` (each loaded extension's overlay, with a runtime gated by
  its capabilities) — plus a timer calling `reload_changed`.
