#!/usr/bin/env node
// Transpiles a WASI component extension for the browser with jco: core
// wasm and an `instantiate` function (`<out>/<name>.js`), the shape
// features/extensions/web/instance.js loads.
//
//   node features/extensions/web/transpile.mjs <component.wasm> <out-dir> [name]
//
// (`npm ci` in features/extensions/web first: jco is pinned in its
// package.json; installed as @fasttrackstudio/architect, jco comes with
// it, and `architect build-web` runs this.) The world's imports that wait for the host are async,
// through JS Promise Integration (JSPI): the component is suspended while
// the main thread answers. So is every export, since any may reach one.
import { execFileSync } from 'node:child_process';
import { basename, join } from 'node:path';
import { fileURLToPath } from 'node:url';

/// jco's command line, wherever npm put jco (beside this file in the
/// repo, hoisted in an embedder's node_modules).
function jco() {
  return fileURLToPath(new URL('./jco.js', import.meta.resolve('@bytecodealliance/jco')));
}

/// The host's imports that wait for the page to answer (every one but
/// `theme`, `log` and `now`): instance.js answers each through the host
/// service, so each must suspend the component (a synchronous import
/// handed a Promise lowers it as its answer).
export const HOST_ASYNC = [
  'run-action', 'setting', 'set-setting', 'call', 'call-service', 'notify', 'clipboard-read', 'clipboard-write',
  'pick', 'prompt', 'menu', 'decorate', 'subscribe', 'unsubscribe', 'list-documents', 'read-document',
  'create-document', 'edit-document', 'rename-document', 'move-document', 'delete-document', 'watch-documents',
  'editor-state', 'edit-editor', 'decorate-editor', 'fetch', 'read-body', 'close-body', 'secret-set', 'ask-secret',
  'ports', 'open-port', 'port-send', 'port-receive', 'close-port',
];

/// The imports that wait: the host service, a worker's messages, a sleep.
export const ASYNC_IMPORTS = [
  ...HOST_ASYNC.map((f) => `architect:extension/host#${f}`),
  'architect:extension/worker#spawn',
  'architect:extension/worker#post',
  'architect:extension/worker#receive',
  'wasi:io/poll#poll',
  'wasi:io/poll#[method]pollable.block',
];

/// Every export of the world.
export const ASYNC_EXPORTS = [
  ...['init', 'actions', 'views', 'invoke', 'render', 'on-event', 'pending', 'close', 'save-state', 'tick'].map(
    (f) => `architect:extension/guest#${f}`,
  ),
  ...['run', 'on-message', 'on-progress', 'on-done'].map((f) => `architect:extension/work#${f}`),
];

export function transpile(component, out, name = basename(component, '.wasm')) {
  execFileSync(
    process.execPath,
    [
      jco(),
      'transpile', component, '-o', out, '--name', name,
      '--instantiation', 'async', '--no-typescript', '--quiet',
      '--async-mode', 'jspi',
      '--async-imports', ...ASYNC_IMPORTS,
      '--async-exports', ...ASYNC_EXPORTS,
    ],
    { stdio: 'inherit' },
  );
  return join(out, `${name}.js`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [component, out, name] = process.argv.slice(2);
  if (!component || !out) {
    console.error('usage: transpile.mjs <component.wasm> <out-dir> [name]');
    process.exit(2);
  }
  console.log(transpile(component, out, name));
}
