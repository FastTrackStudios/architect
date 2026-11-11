import type { WorkerLike } from './index.js';

/** An instance's worker under node: a worker thread in a Web Worker's shape. Pass it as `loadExtension(source, { spawn })`. */
export declare function spawn(): WorkerLike;
