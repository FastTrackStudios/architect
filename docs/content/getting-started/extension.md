+++
title = "Write an extension, end to end"
description = "A manifest, actions and settings, a process extension with a remote view — loaded by a host from a registry, hot-reloaded on save."
weight = 25
+++

This walks the extension flow on two small extensions — `example.hello`,
a Luau extension in a registry, and `example.counter`, a process
extension with a view — and the host that runs both. The runtime is
`architect-extensions` (`features/extensions`); the Dioxus side that draws
an extension's views and settings is in app-shell
(`app-shell:crates/app-shell-extensions`). The design is
[extensions](@/architecture/extensions.md).

## The mental model

```
extension.toml           ──▶  what it adds, read before anything runs:
                              [[actions]] [[sections]] [[views]]

a registry               ──▶  index.toml + one folder per package
                              (a folder, or a git repository of one)

Host + Extensions        ──▶  actions on the Action List, sections on the
                              settings page, process views in panes —
                              all off again on unload

reload_changed() on a timer ─▶ a save reloads it, as a diff; a broken save
                              keeps the last good copy
```

You write the manifest and the code (Luau scripts, or a program). The
host decides where each part goes, and takes every part away again.

## 1. The manifest

`hello/extension.toml` declares everything `example.hello` adds:

```toml
id = "example.hello"
name = "Hello"
version = "0.1.0"
kind = "luau"
api = "0.1"

[[actions]]
id = "EXAMPLE_HELLO_GREET"
name = "Hello: greet"
params = '{"type":"object","properties":{"name":{"type":"string"}}}'
script = "greet.luau"
```

The id names the extension's namespace. Every action id starts with
the prefix it gives (`example.hello` → `EXAMPLE_HELLO_`), every setting
key with `example.hello.`; anything outside is refused when the manifest
is read (`features/extensions/src/manifest.rs` `Manifest::check`).
`params` is JSON Schema: ⌘K builds its form from it, a model its call.

## 2. An action

`hello/greet.luau` is the action: `args` is
what the caller passed, the value it returns is the result.

```lua
local name = if args and args.name then args.name else "you"
return "Hello, " .. name .. "!"
```

Loaded, it is on the app's Action List like the app's own actions:
⌘K, menus, keys, MCP and other scripts all reach it. A Luau extension
runs inside its `[capabilities]`: its own actions, plus the namespaces
and the effect it names (`features/extensions/src/host.rs` `gate`).

## 3. Settings that come and go with it

```toml
[[sections]]
id = "hello"
title = "Hello"

[[sections.settings]]
key = "example.hello.greeting"
title = "Greeting"
control = "text"
default = "Hello"
```

A section is owned by its extension: on the settings page while it is
loaded, its values kept across a reload, gone when it is unloaded. In a
Dioxus app, `use_extension_sections()` is the settings page as the host
writes to it (`app-shell:crates/app-shell-extensions/src/ui.rs`).

## 4. A registry

The registry's `index.toml` lists what it offers,
each with the folder its package is in:

```toml
name = "Example registry"

[[extensions]]
id = "example.hello"
name = "Hello"
version = "0.1.0"
kind = "luau"
path = "hello"
```

The same folder is a registry three ways: `FolderRegistry` reads it where
it is (an install is a link, so an edit there is what loads),
`GitRegistry` clones a repository of it, and installs copy
(`features/extensions/src/install.rs`).

## 5. A process extension with a remote view

`example.counter` is a program of its own. Its manifest
names the program and declares what it serves:

```toml
id = "example.counter"
kind = "process"

[entry]
path = "example-extension-counter"

[[actions]]
id = "EXAMPLE_COUNTER_CLICKS"
name = "Counter: clicks"
effect = "read_only"

[[views]]
id = "counter"
title = "Counter"
```

and its `main` serves it, the view a plain
Dioxus component:

```rust
fn main() -> Result<(), GuestError> {
    Guest::new()
        .view("counter", Counter)
        .action("EXAMPLE_COUNTER_CLICKS", |_args| async {
            Ok(serde_json::json!(CLICKS.load(Ordering::Relaxed)))
        })
        .serve()
}
```

The host starts it with a socket in its environment, asks its hello
(which must cover every view and action the manifest declares), and
supervises it (`features/extensions/src/process.rs` `ProcessRunner`).
The view's `VirtualDom` runs in the extension's process; its edits come
over vox and `RemoteView` applies them in a sub-document of the host's
window, sending the pane's events back
(`app-shell:crates/app-shell-extensions/src/remote.rs` `RemoteView`). When the process
dies the host runs on: the pane says why, the runner starts it again
with backoff, and the view comes back.

## 6. The host

This is what makes an app a host:

```rust
let mut host = Host::new(list, sections);                 // the Action List, the settings page
host = host.with_runner(ProcessRunner::new()?.with_processes(processes));
let extensions = Extensions::new(host, Installs::new(installed))
    .with_source(FolderRegistry::new(registry));          // or GitRegistry
extensions.install_actions()?;                            // EXT_SEARCH / EXT_INSTALL / EXT_UNINSTALL …

// on a timer
extensions.reload_changed();
```

The app's window then mounts the counter's `RemoteView` in a pane and
the settings page with the extension's sections.

## 7. Test it

```sh
cargo test -p architect-extensions
```

`features/extensions/tests/registries.rs` installs from registries signed
by their publishers' keys, and development copies,
`features/extensions/tests/process_views.rs` serves a
process extension's views, and the WASI fixtures
(`features/extensions/tests/wasi-counter`) are extensions built as part
of the crate's tests.

## Your own

To run yours in an app as you write it, see "The development loop" in
[Writing an extension](@/extensions/authoring.md): a development copy
built from your folder overrides the installed one and reloads on save.
