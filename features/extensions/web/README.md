# @fasttrackstudio/architect

Run [Architect](https://github.com/FastTrackStudios/architect)'s extensions in any web app. An extension is a
WASI component: the same `.wasm` file the Architect app runs. This package has no dependencies. It:

- loads the extension, pinned by its lock, into an opaque-origin sandbox frame (its JS never gets your
  origin);
- runs its actions in Web Workers with memory and time limits, with types;
- mounts its views in a shadow root under the share pages' sanitiser rules.

The extension reaches only what you grant.

```sh
npm install @fasttrackstudio/architect
npm install -D @fasttrackstudio/architect-cli
npx architect-ext build-web my_extension.wasm public/my-extension --manifest extension.toml --types src/my-extension
```

```ts
import { isSupported, loadExtension, memoryHost, mountView } from '@fasttrackstudio/architect';
import { bind, integrity, type Actions } from './my-extension/actions.js';

const ext = await loadExtension<Actions>('/my-extension/', {
  integrity,                       // its files pinned by the lock build-web wrote
  host: memoryHost(),              // what it may ask the app for, answered in memory
  grant: { actions: ['THEME_'] },  // nothing is granted unless you grant it
});
await ext.run('LAB_WASI_COUNTER_ADD', { by: 5 });  // checked at compile time
const view = mountView(ext, 'counter', document.querySelector('#counter')!);
```

- **React**: `import { ArchitectView } from '@fasttrackstudio/architect/react'`.
- **Node (24+)**: `loadExtension(source, { spawn })`, with `spawn` from `@fasttrackstudio/architect/node`.
  Node has no sandbox frame, so a node host trusts the extension's JS.
- **Browsers**: it needs JS Promise Integration (`isSupported()`): Chromium 137+, Firefox and node 24+;
  not Safari yet.
- **CSP**: your page's CSP names the sandbox script's hash (`SANDBOX_HASH`).

The guide covers the security model, the CSP, the limits, the sizes and what is not there yet:
[Extensions in your own web app](https://architect.fasttrackstudio.app/docs/guide/extensions-in-your-web-app/).

Licensed MIT OR Apache-2.0.
