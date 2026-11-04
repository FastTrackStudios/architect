// Runs instance.js in a node worker thread as a browser runs it in a Web
// Worker: `self.postMessage` and `self.onmessage`.
import { parentPort } from 'node:worker_threads';

globalThis.self = globalThis;
globalThis.postMessage = (m) => parentPort.postMessage(m);
await import('../instance.js');
parentPort.on('message', (data) => globalThis.onmessage({ data }));
