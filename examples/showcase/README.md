# The extension showcase's examples

Small example extensions, one per capability or group of capabilities
from architect#710, each written the way a third party would: a folder
with an `extension.toml`, loaded from a registry or as a development
copy. Each says, in its manifest's description and its note in the
showcase vault (`apps/ui/showcase-vault`), which Obsidian plugins or apps
it stands in for.

`just showcase` (`scripts/showcase.sh`) runs the app on a scratch copy
of the vault with every showcase extension loaded as a development copy
(edit one and save, and it reloads):

- every folder here;
- the sandbox's Luau extensions: `apps/sandbox/extensions/{outline,surface,metronome,tidy}`;
- its process extensions, `apps/sandbox/{kanban,sketch,board,clicker}`
  and `examples/extension/counter`, their programs built first and staged
  beside their packages in the scratch folder;
- the WASI counter, `apps/sandbox/wasi-counter`, built for `wasm32-wasip2`;
- the web view board, `examples/extension/webview-board`, its Vite app
  built into `package/dist` when that is missing or older than its sources.

The showcase registry's packages (`examples/showcase-registry`) are left
out on purpose: they show installing from a registry.

The first run builds the standard distribution, which takes several
minutes; the script says so and shows cargo's progress before the app
opens.

Without the script, the app carries the showcase too: its first run offers
**Explore the extension showcase** (on the desk and on iOS), and the vault's
**Install all** installs the Luau, TypeScript and data packages here and
the sandbox's Luau ones from a registry built into the app
(`apps/ui/src/showcase.rs`), through one consent prompt, with no network.

The vault names all of them in `apps/ui/showcase-vault/.architect/extensions.toml`
(the format of "What a project needs"): opened anywhere, it lists what is
missing, with **Install all**. On the desk every one is loaded, so nothing
is missing. The process extensions, the WASI component and the web view
are optional. Their signed archives will travel in the vault's
`.architect/extensions/` once a release signs them with the org key (on a
fresh device a vault's archives are trusted by that key only); until then
the web and iOS list them as not found.

The matrix of what runs where is `docs/content/guide/showcase.md`.
