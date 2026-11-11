// @fasttrackstudio/architect/node: what running an extension under node
// (24 or later: JSPI) needs that a browser has built in, a worker for
// each instance. `loadExtension(source, { host, spawn })`.
//
// Under node there is no sandbox frame: the extension's JS (jco's glue)
// runs in a worker thread of this process, with what node gives every
// worker (the file system, the network, the environment). A node host
// TRUSTS the extension's JS; its wasm is still limited to its imports and
// its capabilities. Run an extension you do not trust in a browser, or in
// a process of its own (`node --permission`).
import { Worker } from 'node:worker_threads';

const script = new URL('./node-worker.mjs', import.meta.url);

/// An instance's worker: a node worker thread in a Web Worker's shape.
export function spawn() {
  const w = new Worker(script);
  const shaped = {
    onmessage: null,
    onerror: null,
    postMessage: (m) => w.postMessage(m),
    terminate: () => w.terminate(),
    // The runner holds the process open only while it starts or runs a
    // call; idle, an extension does not keep node from exiting.
    ref: () => w.ref(),
    unref: () => w.unref(),
  };
  w.on('message', (data) => shaped.onmessage?.({ data }));
  w.on('error', (e) => shaped.onerror?.(e));
  return shaped;
}
