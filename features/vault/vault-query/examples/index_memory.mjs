// A browser's copy of the vault index in V8: run after
//   cargo build -p vault-query --release --example index_memory --target wasm32-unknown-unknown
//   wasm-bindgen --target nodejs --out-dir <out> target/wasm32-unknown-unknown/release/examples/index_memory.wasm
// as: node examples/index_memory.mjs <out>/index_memory.js [notes]
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const m = require(process.argv[2]);
const n = Number(process.argv[3] ?? 10000);
const snap = m.build(n);
const held = m.load();
m.drop_bytes();
const t0 = performance.now();
const [total, peak, kept] = m.query(50);
const first = performance.now() - t0;
const t1 = performance.now();
m.query(50);
const again = performance.now() - t1;
const kb = (b) => (b / 1024 / 1024).toFixed(2) + " MiB";
console.log(`wasm (V8), ${n} notes: snapshot ${snap} B (${Math.round(snap / n)} B a note); replica holds ${kb(held)} (${Math.round(held / n)} B a note); a query (limit 50, total ${total}) peaks at ${kb(peak)}, keeps ${kb(kept)}; first ${first.toFixed(1)} ms, again ${again.toFixed(1)} ms; heap now ${kb(m.live())}`);
