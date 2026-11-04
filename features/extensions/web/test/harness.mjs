// What the node tests share: components built for wasm32-wasip2 and
// transpiled by jco, and a Web Worker made of a node worker thread.
import { Worker } from 'node:worker_threads';
import { existsSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { transpile } from '../transpile.mjs';

const here = dirname(fileURLToPath(import.meta.url));
export const root = resolve(here, '../../../..');

/// Where the components were built (`ARCHITECT_WASI_GUESTS`, else this
/// checkout's `target/wasm32-wasip2/debug`); null when they were not.
export function guests() {
  const dir = process.env.ARCHITECT_WASI_GUESTS ?? join(root, 'target/wasm32-wasip2/debug');
  return existsSync(join(dir, 'architect_lab_wasi_counter.wasm')) ? dir : null;
}

const done = new Map();

/// `name`'s component, transpiled once: its module's URL.
export function transpiled(dir, name) {
  if (!done.has(name)) {
    const out = join(root, 'target/wasi-web', name);
    mkdirSync(out, { recursive: true });
    done.set(name, pathToFileURL(transpile(join(dir, `${name}.wasm`), out, name)).href);
  }
  return done.get(name);
}

/// A node worker thread in a Web Worker's shape.
export function spawn() {
  const w = new Worker(join(here, 'node-worker.mjs'));
  const shaped = {
    onmessage: null,
    onerror: null,
    postMessage: (m) => w.postMessage(m),
    terminate: () => w.terminate(),
  };
  w.on("message", (data) => { if (process.env.TRACE) console.error("<-", data.t, data.op ?? data.name ?? data.why ?? ""); shaped.onmessage?.({ data }); });
  w.on('error', (e) => shaped.onerror?.(e));
  return shaped;
}
