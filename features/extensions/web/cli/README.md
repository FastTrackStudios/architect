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

Guide: [Extensions in your own web app](https://architect.fasttrackstudio.app/docs/guide/extensions-in-your-web-app/).
Licensed MIT OR Apache-2.0.
