// @fasttrackstudio/architect/node: what running an extension under node
// (24 or later: JSPI) needs that a browser has built in, a worker for
// each instance. `loadExtension(source, { host, spawn })`.
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
  };
  w.on('message', (data) => shaped.onmessage?.({ data }));
  w.on('error', (e) => shaped.onerror?.(e));
  return shaped;
}
