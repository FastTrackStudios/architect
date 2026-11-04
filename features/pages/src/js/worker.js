// A worker's runtime: what `import … from "architect/worker"` gives a
// script spawned by `worker.spawn("job.js")` (features/pages/src/js/mod.rs).
// It has no app: no actions, no theme, no elements — only messages.
(() => {
  "use strict";
  const host = globalThis.__host;
  delete globalThis.__host;
  const handlers = [];
  const show = (v) => (typeof v === "string" ? v : v instanceof Error ? v.message : JSON.stringify(v));
  globalThis.console = Object.freeze({
    log: (...a) => host.log("info", a.map(show).join(" ")),
    info: (...a) => host.log("info", a.map(show).join(" ")),
    warn: (...a) => host.log("warn", a.map(show).join(" ")),
    error: (...a) => host.log("error", a.map(show).join(" ")),
  });
  const json = (v) => JSON.stringify(v === undefined ? null : v);
  globalThis.__worker = Object.freeze({
    api: Object.freeze({
      args: JSON.parse(host.args),
      post: (m) => host.post(json(m)),
      onMessage: (fn) => void handlers.push(fn),
    }),
    message(text) {
      const v = JSON.parse(text);
      for (const fn of handlers) fn(v);
    },
    serving: () => handlers.length > 0,
    json,
  });
})();
