# Web view board (#222)

A full TypeScript/npm app as an Architect extension: Vite, TypeScript, [uPlot](https://github.com/leeoniya/uPlot)
for the chart and [SortableJS](https://github.com/SortableJS/Sortable) for the drag-and-drop list, in a
pane. It is the showcase's R6 (architect#710).

- `app/`: the app's source. `npm run build` writes `../package/dist/`.
- `package/`: the extension: `extension.toml` (a `data` extension whose view has `kind = "web"`,
  `src = "dist/index.html"`) and the built `dist/`.

`just webview-example KEY=<secret key>` builds it, packs `package/` into `target/webview-board.archext` and
signs it (`cargo xtask ext-keygen target/keys` makes a throwaway key; an app trusts it with
`Installs::with_trust`, or `cargo xtask ext-verify --key`).

Its code runs only in its web view: a wry child web view on a desk (`architect-desktop --features webview`;
on Linux under X11, or Wayland with `ARCHITECT_WEBVIEW_X11=1`), a sandboxed iframe in a browser. It reaches
the app through `@fasttrackstudio/architect/webview` (`window.architect`): its cards are its extension data
(`set_data`), its Notify button is `notify` (granted by `capabilities.ui`), and it follows the app's theme
through the CSS custom properties the host gives it.
