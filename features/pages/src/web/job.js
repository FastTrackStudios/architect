// A page's worker in a browser (`worker.spawn("job.js", args)`): a Web
// Worker its page's Web Worker starts (features/pages/src/web/page.js).
// It has the worker runtime native workers have
// (features/pages/src/js/worker.js, put in at `@prelude` below) — no app,
// only messages — and none of the browser's network, storage or workers.
//
// From its page: `start` (its module's URL, its args as JSON), then
// `message`s. To it: `log`, `message`, `done` (what its default export
// returned, and whether it serves messages still) or `failed`.

const post = globalThis.postMessage.bind(globalThis);
const listen = globalThis.addEventListener.bind(globalThis);
const show = (e) =>
  e instanceof Error ? (e.stack && e.stack.includes(e.message) ? e.stack : e.message + "\n" + (e.stack || "")) : String(e);

function prelude() {
  // @prelude
}

let started = false;
const waiting = [];

function message(text) {
  try {
    globalThis.__worker.message(text);
  } catch (e) {
    post({ t: "failed", why: show(e) });
  }
}

listen("message", async (ev) => {
  const m = ev.data;
  if (m.t === "message") {
    if (started) message(m.text);
    else waiting.push(m.text);
    return;
  }
  if (m.t !== "start") return;
  globalThis.__host = {
    args: m.args,
    log: (level, text) => post({ t: "log", level, text: String(text) }),
    post: (text) => post({ t: "message", text }),
  };
  try {
    prelude();
    for (const name of m.hidden)
      for (let o = globalThis; o; o = Object.getPrototypeOf(o))
        try {
          if (Object.prototype.hasOwnProperty.call(o, name)) delete o[name];
        } catch {}
    const w = globalThis.__worker;
    const ns = await import(m.url);
    const main = ns.default;
    const result = typeof main === "function" ? await main(w.api.args) : main;
    const serving = w.serving();
    post({ t: "done", text: w.json(result), serving });
    started = true;
    for (const text of waiting.splice(0)) message(text);
  } catch (e) {
    post({ t: "failed", why: show(e) });
  }
});
