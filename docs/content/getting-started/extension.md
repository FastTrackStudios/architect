+++
title = "Write an extension, end to end"
description = "A manifest, actions and settings, a Luau overlay with a tool window, a process extension with a remote view — loaded by a small host from a registry, hot-reloaded on save."
weight = 25
+++

This walks the extension flow against the reference example,
`examples/extension`: a registry with a Luau extension in it
(`examples/extension/registry`), a process extension
(`examples/extension/counter`) and a small host that runs both
(`examples/extension/host`). Each step points at the real file, so you
can read the working code alongside. The design is
[extensions](@/architecture/extensions.md).

## The mental model

```
extension.toml           ──▶  what it adds, read before anything runs:
                              [[actions]] [[sections]] [[overlays]] [[views]]

a registry               ──▶  index.toml + one folder per package
                              (a folder, or a git repository of one)

Host + Extensions        ──▶  actions on the Action List, sections on the
                              settings page, overlays over views, process
                              views in panes — all off again on unload

reload_changed() on a timer ─▶ a save reloads it, as a diff; a broken save
                              keeps the last good copy
```

You write the manifest and the code (Luau scripts, or a program). The
host decides where each part goes, and takes every part away again.

## 1. The manifest

`examples/extension/registry/hello/extension.toml` declares everything
`example.hello` adds:

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

`examples/extension/registry/hello/greet.luau` is the action: `args` is
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
writes to it (`features/extensions/src/ui.rs`).

## 4. An overlay with a tool window

```toml
[[overlays]]
id = "ruler"
view = "example.timeline"
plane = "over"
script = "ruler.luau"
```

`examples/extension/registry/hello/ruler.luau` is a live script, a
frame at a time: `Canvas` draws over every view of kind
`example.timeline` in that view's own coordinates (seconds across, lanes
down), and `ImGui` — ReaImGui's API — draws a tool window:

```lua
local ImGui = require 'imgui' '0.10'
local ctx = ImGui.CreateContext('Hello')
local ruler = Canvas.Layer('example.timeline', 'over', 'ruler')

local function loop()
  draw()                                   -- Canvas.Line / Canvas.Text at Canvas.TimeToX(t)
  local visible, open = ImGui.Begin(ctx, 'Ruler', true)
  if visible then
    _, every = ImGui.SliderInt(ctx, 'Every', every, 1, 8, '%d s')
    if ImGui.Button(ctx, 'Greet') then
      greeting = actions.run('EXAMPLE_HELLO_GREET', { name = 'ruler' })
    end
    ImGui.End(ctx)
  end
  if open then defer(loop) end
end
defer(loop)
```

The host runs it (`features/canvas/src/luau.rs` `Session`) on a runtime
gated like the extension's actions: `examples/extension/host/src/lib.rs`
`Overlay::runtime`. The functions scripts get are listed in
`features/canvas/src/luau.rs`.

## 5. A registry

`examples/extension/registry/index.toml` lists what the registry offers,
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

## 6. A process extension with a remote view

`examples/extension/counter` is a program of its own. Its manifest
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

and `examples/extension/counter/src/main.rs` serves it, the view a plain
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
(`features/extensions/src/remote.rs` `RemoteView`). When the process
dies the host runs on: the pane says why, the runner starts it again
with backoff, and the view comes back.

## 7. The host

`examples/extension/host/src/lib.rs` is what makes an app a host:

```rust
let mut host = Host::new(list, sections);                 // the Action List, the settings page
host = host.with_runner(ProcessRunner::new()?.with_processes(processes));
let extensions = Extensions::new(host, Installs::new(installed))
    .with_source(FolderRegistry::new(registry));          // or GitRegistry
extensions.install_actions()?;                            // EXT_SEARCH / EXT_INSTALL / EXT_UNINSTALL …

// on a timer
extensions.reload_changed();

// each loaded extension's overlays over the timeline, each started
// on `overlay.runtime(list)`
overlays(extensions.host(), TIMELINE)
```

`examples/extension/host/src/main.rs` puts it in a window: a timeline
that publishes its coordinate spaces (`Canvases::publish`) and mounts
what scripts draw over it (`CanvasLayers`), one `use_script_canvas` per
overlay keyed by its script (so a save starts it again), the tool
windows, the counter's `RemoteView`, and the settings page.

```sh
cargo build -p example-extension-counter
cargo run -p example-extension-host
```

## 8. Test it

```sh
cargo test -p example-extension-host
cargo test -p example-extension-counter
```

`examples/extension/host/tests/registry.rs` publishes the registry as a
git repository, installs Hello through `EXT_INSTALL`, runs its action,
sees its settings on the page, runs its overlay against a timeline and
clicks its tool window's button, saves a change and sees it reload,
saves a broken one and keeps the last good copy, and uninstalls it —
action, settings and overlay gone.
`examples/extension/counter/tests/survives_kill.rs` mounts the counter's
view in a headless Blitz document, clicks it, sends its process
`kill -9`, and sees the host start it again and the view come back.

## Your own

Copy `examples/extension/registry/hello` for a Luau or data extension,
`examples/extension/counter` for a process extension, and
`examples/extension/host/src/lib.rs` when your app becomes a host.
`examples/extension/README.md` has the checklist.
