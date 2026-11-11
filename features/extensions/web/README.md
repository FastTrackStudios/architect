# @fasttrackstudio/architect

Run [Architect](https://github.com/FastTrackStudios/architect)'s extensions in any web app. An extension is a
WASI component: the same `.wasm` file the Architect app runs. This package loads it in a Web Worker with
memory and time limits, calls its actions with types, and mounts its views into your page. The extension
reaches only what you grant it.

```sh
npm install @fasttrackstudio/architect
npx architect build-web my_extension.wasm public/my-extension --manifest extension.toml
```

`build-web` transpiles the component with [jco](https://github.com/bytecodealliance/jco) and writes the
extension's metadata (`architect.json`) and its typed actions (`actions.d.ts`, `actions.js`). It also prints
what a page downloads (raw, gzip, brotli).

```ts
import { loadExtension, memoryHost, mountView } from '@fasttrackstudio/architect';
import { bind, type Actions } from './my-extension/actions.js';

const ext = await loadExtension<Actions>('/my-extension/', {
  host: memoryHost(),              // what it may ask the app for, answered in memory
  grant: { actions: ['THEME_'] },  // nothing is granted unless you grant it
});
await ext.run('LAB_WASI_COUNTER_ADD', { by: 5 });  // checked at compile time
await bind(ext).add({ by: 1 });
const view = mountView(ext, 'counter', document.querySelector('#counter')!);
```

React: `import { ArchitectView } from '@fasttrackstudio/architect/react'`. Node (24+):
`loadExtension(source, { spawn })`, with `spawn` from `@fasttrackstudio/architect/node`.

It needs JS Promise Integration (Chromium 137+, Firefox, node 24+; not Safari yet). For the CSP your
page needs, the limits, the sizes and what is not there yet, see the guide:
[Extensions in your own web app](https://architect.fasttrackstudio.app/docs/guide/extensions-in-your-web-app/).

Licensed MIT OR Apache-2.0.
