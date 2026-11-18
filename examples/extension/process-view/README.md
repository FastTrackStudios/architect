# Process view (#838)

A pane drawn by a program of its own, as an extension: the lab's process-view demo
(`features/process-view/src/demo.rs`, `demo::Orbits`, served by `process-view-demo`) in a package whose only
view has `kind = "process"`. It is the showcase's U3 row for runtime extensions (architect#840).

- `package/extension.toml`: a `data` extension. Its view names the program in the package that draws it
  (`program = "bin/process-view-demo"`), and `capabilities.programs` asks to run it.
- `package/bin/`: the program, built by `just process-view-example` (not checked in).

The app's process-view pane (`architect-process-view`'s `extension` feature, in every build of the
app's window on Linux and macOS, the release's included) starts the program when the pane opens, shows its GPU frames
(a shared dma-buf texture on Linux, an IOSurface on macOS, or shared memory), forwards the pane's input,
size and focus, starts it again when it dies (saying why) and when a development copy is rebuilt, and stops
it when the pane closes or the extension unloads. On Windows, iOS and the web the extension still loads and
its pane says why it cannot run there.

Load it as a development copy:

```sh
just process-view-example
architect dev --extension examples/extension/process-view/package
```

Alone in an isolated instance, the window just the view: `architect dev --extension
examples/extension/process-view/package --only demo.process-view --hide all --run DEMO_PROCESS_VIEW_OPEN`.
A development copy is not asked: it runs with what it asks. As an installed copy the program runs only
once a person grants it: at install, or in the pane, which asks with **Allow** when the grant is missing
(taken back, or an update that asks for it). The test that loads it installed, then in development, and receives its frames is
`features/process-view/tests/extension_kind.rs`.
