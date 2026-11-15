// A Luau extension's run in a browser (#812): one action or hook's script
// in a Web Worker of its own (`features/pages/src/web/luau.rs`), on
// luau-web's Luau (apps/web/assets/luau-web, scripts/vendor-luau-web.sh),
// loaded only when a Luau extension runs.
//
// Its globals are `luau_runtime.luau`'s: every host, ext and actions call
// is a message to the page, which answers it with the host service; a
// call waits by suspending the script (Asyncify), so nothing here needs
// `Atomics.wait` or cross-origin isolation.
//
// From the page: `start` ({lib, prelude, source, args, ctx, id, host,
// ext, workers}), then `answer`s. To it: `loaded`, `call` ({id, g, name,
// args}), `log`, `worker` ({on}) and `done` ({ok, text}).

const post = globalThis.postMessage.bind(globalThis);
const listen = globalThis.addEventListener.bind(globalThis);

// What a script's runtime may not reach: the browser's own network,
// storage and workers. A script reaches no JavaScript at all (its globals
// are Luau's and the prelude's); this is so for the runtime too.
const HIDDEN = [
  "fetch", "XMLHttpRequest", "WebSocket", "WebSocketStream", "EventSource", "WebTransport",
  "importScripts", "indexedDB", "caches", "Worker", "SharedWorker", "BroadcastChannel",
  "navigator", "postMessage",
];
function hide(names) {
  for (const name of names)
    for (let o = globalThis; o; o = Object.getPrototypeOf(o))
      try {
        if (Object.prototype.hasOwnProperty.call(o, name)) delete o[name];
      } catch {}
}

const show = (e) => (e instanceof Error ? e.message : String(e));
const waiting = new Map();
let nextCall = 0;

// One call to the page: `1<json>` answered, `0<why>` refused.
function call(g, name, args) {
  const id = nextCall++;
  post({ t: "call", id, g, name, args });
  return new Promise((resolve) => waiting.set(id, resolve));
}

// A state with the prelude's globals for `start` (the run's, or a
// worker's), read-only; and its `__run`.
async function state(LuauState, prelude, start, fns) {
  const s = await LuauState.createAsync({});
  const make = s.loadstring(prelude, "=prelude", true);
  const [globals] = await make(fns.call, fns.say, fns.runWorker, start);
  let run = null;
  for (const key of globals.keys()) {
    const value = globals.get(key);
    if (key === "__run") run = value;
    else s.env.set(key, value, true);
  }
  return { s, run };
}

listen("message", async (ev) => {
  const m = ev.data;
  if (m.t === "answer") {
    const resolve = waiting.get(m.id);
    waiting.delete(m.id);
    if (resolve) resolve((m.ok ? "1" : "0") + m.text);
    return;
  }
  if (m.t !== "start") return;
  try {
    const { LuauState } = await import(m.lib);
    const say = (text) => post({ t: "log", text: String(text) });
    // A worker: its script in a state of its own here, to its end, its
    // report one JSON text (it calls no JavaScript: its owner's call is
    // in flight).
    const runWorker = async (name, args, inbox) => {
      post({ t: "worker", on: true });
      let out;
      try {
        const start = JSON.stringify({ mode: "worker", name, id: m.id })
          .replace(/}$/, `,"args":${args},"inbox":${inbox}}`);
        const w = await state(LuauState, m.prelude, start, {});
        const main = w.s.loadstring(m.workers[name], "=script", true);
        [out] = await w.run(main);
      } catch (e) {
        out = JSON.stringify({ status: "failed", error: show(e) });
      }
      post({ t: "worker", on: false });
      return out;
    };
    const start = JSON.stringify({
      mode: "run", id: m.id, host: m.host, ext: m.ext, workers: m.workers,
    }).replace(/}$/, `,"args":${m.args},"ctx":${m.ctx}}`);
    // luau-web 1.4.0 is built for `web` and `node` only, and asserts on a
    // worker: it is told it is on a page while it starts (its checks are
    // `typeof window` and `typeof WorkerGlobalScope`, read once), which is
    // all it asks of either (its wasm is inline, it reads no document).
    const scope = globalThis.WorkerGlobalScope;
    globalThis.window = globalThis;
    globalThis.WorkerGlobalScope = undefined;
    let run;
    try {
      run = await state(LuauState, m.prelude, start, { call, say, runWorker });
    } finally {
      delete globalThis.window;
      globalThis.WorkerGlobalScope = scope;
    }
    // Its runtime is loaded (the wasm is inlined): from here, none of the
    // browser's network, storage or workers.
    hide(HIDDEN);
    post({ t: "loaded" });
    const main = run.s.loadstring(m.source, "=script", true);
    const [text] = await run.run(main);
    post({ t: "done", ok: true, text });
  } catch (e) {
    post({ t: "done", ok: false, text: show(e) });
  }
});
