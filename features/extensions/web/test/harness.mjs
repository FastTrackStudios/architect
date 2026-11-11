// What the node tests share: components built for wasm32-wasip2 and
// transpiled by jco, and a Web Worker made of a node worker thread.
import { Worker } from 'node:worker_threads';
import { existsSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { transpile } from '../transpile.mjs';

const here = dirname(fileURLToPath(import.meta.url));
export const root = resolve(here, '../../../..');

/// Where the components were built: `ARCHITECT_WASI_GUESTS`, else this
/// checkout's `target/wasi-guest/wasm32-wasip2/debug` (where
/// features/extensions/tests/wasi_web.rs builds them) or
/// `target/wasm32-wasip2/debug`. Not built, the tests FAIL, saying how to
/// build them, unless `ALLOW_SKIP=1` says skipping them is meant (then
/// null: they skip).
export function guests() {
  // Named, it is the only place looked in: a wrong path fails, it does
  // not fall back to another build.
  const named = process.env.ARCHITECT_WASI_GUESTS;
  for (const dir of named ? [named] : [join(root, 'target/wasi-guest/wasm32-wasip2/debug'), join(root, 'target/wasm32-wasip2/debug')])
    if (dir && existsSync(join(dir, 'architect_sandbox_wasi_counter.wasm'))) return dir;
  if (process.env.ALLOW_SKIP === '1') return null;
  throw new Error(
    'the WASI components these tests run are not built: run `cargo nextest run -p architect-extensions --test wasi_web` ' +
      '(it builds them into target/wasi-guest), or `devenv --profile wasi shell -- cargo build -p architect-sandbox-wasi-counter ' +
      '-p architect-extension-wasi-hazards --target wasm32-wasip2`, or set ARCHITECT_WASI_GUESTS; ALLOW_SKIP=1 skips them on purpose',
  );
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
