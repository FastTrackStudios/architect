# @fasttrackstudio/architect-cli

Makes an Architect extension's WASI component ready for
[`@fasttrackstudio/architect`](https://www.npmjs.com/package/@fasttrackstudio/architect).
This is a build tool: install it as a dev dependency. It depends on jco; the runtime package has no
dependencies.

```sh
npm install -D @fasttrackstudio/architect-cli
npx architect-ext build-web my_extension.wasm public/my-extension --manifest extension.toml --types src/my-extension
```

`build-web` does five things:

- It checks the manifest and refuses a bad id, version or action.
- It transpiles the component with jco.
- It writes `architect.json` (the manifest and the world version) and `architect.lock.json` (a SHA-256 of
  every file).
- It writes the typed actions (`actions.d.ts`, `actions.js`).
- It prints the size of everything a page downloads, and the `integrity` to pin the lock with:

```ts
await loadExtension('/my-extension/', { integrity: 'sha256-…', host, grant });
```

## TypeScript extensions

An extension written only in TypeScript (`kind = "typescript"`): its actions are `.ts` files beside its TSX
pages, run by the app on its pages' runtime (QuickJS natively, a Web Worker in a browser). No Rust, no WASM
toolchain.

```sh
npx architect-ext new --typescript my.recipes recipes   # a manifest, an action, a page, its types
npx architect-ext check recipes                         # the manifest and its files, then tsc --noEmit
npx architect-ext pack recipes --key release.key        # recipes's .archext, signed
```

- `types` writes `types/architect.d.ts` (the app's runtime: `actions`, `documents`, `net`, `ui`, `defineAction`…)
  and `types/actions.d.ts`, each action's arguments from its manifest's `params`, so
  `defineAction("MY_RECIPES_SCALE", (args, ctx) => …)` is typed by the manifest. `check` writes them again.
- `check` needs TypeScript (`npm install -D typescript`).
- `pack` writes the archive format the app installs (`archext.json`, its minisign signature, the files),
  signed with a minisign secret key (`--key`, or the key's text in `ARCHITECT_EXT_SIGNING_KEY`); unencrypted, as
  `minisign -G -W` or `architect-ext keygen <dir>` (throwaway keys, for trials) write it.
- An action imports `"architect"` and files of its own folder; a library is bundled into one file first
  (`esbuild src/x.ts --bundle --format=esm --outfile=actions/x.js`).

Guide: [Extensions in your own web app](https://architect.fasttrackstudio.app/docs/guide/extensions-in-your-web-app/).
Licensed MIT OR Apache-2.0.
