#!/usr/bin/env node
// Transpiles a WASI component extension for the browser with jco: core
// wasm and an `instantiate` function (`<out>/<name>.js`), the shape
// features/extensions/web/instance.js loads.
//
//   node features/extensions/web/transpile.mjs <component.wasm> <out-dir> [name]
//
// (`npm ci` in features/extensions/web first: jco is pinned in its
// package.json.) The world's imports that wait for the host are async,
// through JS Promise Integration (JSPI): the component is suspended while
// the main thread answers. So is every export, since any may reach one.
import { execFileSync } from 'node:child_process';
import { basename, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));

/// The imports that wait: the host service, a worker's messages, a sleep.
export const ASYNC_IMPORTS = [
  'architect:extension/host#run-action',
  'architect:extension/host#setting',
  'architect:extension/host#set-setting',
  'architect:extension/host#call',
  'architect:extension/worker#spawn',
  'architect:extension/worker#post',
  'architect:extension/worker#receive',
  'wasi:io/poll#poll',
  'wasi:io/poll#[method]pollable.block',
];

/// Every export of the world.
export const ASYNC_EXPORTS = [
  ...['init', 'actions', 'views', 'invoke', 'render', 'on-event', 'pending', 'close', 'save-state'].map(
    (f) => `architect:extension/guest#${f}`,
  ),
  ...['run', 'on-message', 'on-progress', 'on-done'].map((f) => `architect:extension/work#${f}`),
];

export function transpile(component, out, name = basename(component, '.wasm')) {
  const jco = join(here, 'node_modules/.bin/jco');
  execFileSync(
    jco,
    [
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
