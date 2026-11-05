// A TSX/JS page's Web Worker in a browser (features/pages/src/web/mod.rs).
// The browser's own engine runs the page: this file makes `__host`, the
// bridge the page runtime speaks to (features/pages/src/js/prelude.js,
// put in at `@prelude` below), as messages to the main thread; links the
// page's modules (prepared by features/pages/src/modules.rs) as blob:
// URLs; and answers the main thread one unit of work at a time.
//
// To the main thread: `busy` when a unit starts (it times the unit and
// terminates this worker past its budget), `idle` when it is done (with
// the page's tree when it rendered, or why its render threw), `failed`
// when the page did not load, and the page's `call`s, `set_shared`,
// `notify` and `log`. From it: `start`, `event`, `answer`, `shared`,
// `theme`.
//
// What the prelude's `sync` answers at once is kept here: shared state
// (each change comes from the main thread), the embed's input. A page
// reaches nothing else of the browser: its network, storage and workers
// are taken away before it loads (`HIDDEN`), so a fetch is `net.fetch`,
// checked against its extension's capabilities.

const post = globalThis.postMessage.bind(globalThis);
const listen = globalThis.addEventListener.bind(globalThis);
const later = globalThis.setTimeout.bind(globalThis);
const unlater = globalThis.clearTimeout.bind(globalThis);
const now = () => performance.now();
const Spawn = globalThis.Worker;
const blobOf = (code) => URL.createObjectURL(new Blob([code], { type: "text/javascript" }));

// What a page may not reach: the browser's own network, storage and
// workers (native pages have none either).
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

const shared = new Map();
let embed = "null";
let nextCall = 0;
let nextJob = 1;
let start = null;

// Module names, for what a page throws: each blob: URL said as
// `<root>/<name>`, which the main thread maps back to the source.
const names = new Map();
const named = (text) => {
  let out = String(text);
  for (const [url, name] of names) out = out.split(url).join(start.root + "/" + name);
  return out;
};
const show = (e) =>
  e instanceof Error ? (e.stack && e.stack.includes(e.message) ? e.stack : e.message + "\n" + (e.stack || "")) : String(e);

globalThis.__host = {
  call(kind, a, b) {
    const id = nextCall++;
    post({ t: "call", id, kind, a, b });
    return id;
  },
  sync(kind, a, b) {
    switch (kind) {
      case "shared":
        return shared.has(a) ? shared.get(a) : "null";
      case "embed":
        return embed;
      case "set_shared":
        JSON.parse(b);
        post({ t: "set_shared", key: a, value: b });
        if (b === "null") shared.delete(a);
        else shared.set(a, b);
        return null;
      case "notify":
        post({ t: "notify", text: a, kind: b });
        return null;
      default:
        throw new Error("no host call " + kind);
    }
  },
  log(level, text) {
    post({ t: "log", level, text: named(text) });
  },
  now,
  spawn: (file, args) => spawnJob(file, args),
  post(id, text) {
    const j = jobs.get(id);
    if (j) j.w.postMessage({ t: "message", text });
  },
  terminate: (id) => stopJob(id, null),
};

// @prelude

const architect = globalThis.__architect;

// ── modules ─────────────────────────────────────────────────────────
const linked = new Map();
const linking = new Set();
/// Module `name`'s URL, its imports linked first; `role` is "page" or
/// "worker" (which of "architect" and "architect/worker" it may import).
function link(name, role, from) {
  const ours = role === "page" ? "architect" : "architect/worker";
  const key = role + ":" + name;
  if (linked.has(key)) return linked.get(key);
  let code;
  if (name === "architect" || name === "architect/worker") {
    if (name !== ours)
      throw new Error(`${from}: a ${role} imports "${ours}" and files of its own folder ("./x.tsx"), nothing else`);
    code = name === "architect" ? start.architect : start.architectWorker;
  } else {
    const m = start.modules[name];
    if (!m) throw new Error(`${name}: no such file in the page's folder`);
    if (m.error) throw new Error(m.error);
    if (linking.has(name)) throw new Error(`${name}: an import cycle, which a page in a browser cannot load`);
    linking.add(name);
    try {
      code = m.code;
      for (const [specifier, to, why] of m.imports) {
        if (why != null) throw new Error(`${name}: import "${specifier}": ${why}`);
        code = code.split(JSON.stringify(start.link + to)).join(JSON.stringify(link(to, role, name)));
      }
    } finally {
      linking.delete(name);
    }
  }
  const url = blobOf(code);
  linked.set(key, url);
  names.set(url, name);
  return url;
}

// ── units of work ───────────────────────────────────────────────────
// A unit is a message's or a timer's: its function, the promise jobs it
// made, the timers due, and renders until the page is no longer dirty —
// each step a task of its own, so what a promise job does is in it too.
// `busy` goes before it, `idle` after: a unit that never ends (a loop,
// in a handler or a render or a promise job) is stopped by the main
// thread, which terminates this worker.
let working = false;
let loaded = false;
let tree = null;
let failure = null;
let timer = null;
const tasks = new MessageChannel();
tasks.port1.onmessage = settle;
const step = () => tasks.port2.postMessage(0);

function report(e) {
  post({ t: "log", level: "error", text: named(show(e)) });
}

function work(fn) {
  if (!working) {
    working = true;
    tree = null;
    failure = null;
    post({ t: "busy" });
    step();
  }
  try {
    fn();
  } catch (e) {
    report(e);
  }
}

function settle() {
  let next = -1;
  try {
    next = architect.tick();
  } catch (e) {
    report(e);
  }
  if (loaded && architect.dirty()) {
    try {
      tree = architect.render();
      failure = null;
    } catch (e) {
      failure = named(show(e));
    }
    step();
    return;
  }
  working = false;
  post(failure != null ? { t: "idle", error: failure } : tree != null ? { t: "idle", tree } : { t: "idle" });
  tree = null;
  failure = null;
  if (timer != null) unlater(timer);
  timer = next >= 0 ? later(() => ((timer = null), work(() => {})), next) : null;
}

// ── its workers: each a nested Web Worker ───────────────────────────
const jobs = new Map();
const OUT_OF_TIME = (ms) => `ran past its time of ${ms} ms: hand heavy work to a worker (worker.spawn)`;

function spawnJob(file, args) {
  const id = nextJob++;
  file = String(file);
  let url;
  try {
    url = link(file.replace(/^\.\//, ""), "worker", file);
  } catch (e) {
    const why = named(show(e));
    later(() => work(() => architect.worker(id, "failed", JSON.stringify(why))));
    return id;
  }
  const w = new Spawn(start.jobUrl, { type: "module" });
  const j = { w, result: null, timer: later(() => stopJob(id, OUT_OF_TIME(start.workerTime)), start.workerTime) };
  jobs.set(id, j);
  w.onmessage = (ev) => {
    const m = ev.data;
    if (m.t === "log") post({ t: "log", level: m.level, text: `worker ${id}: ${named(m.text)}` });
    else if (m.t === "message") work(() => architect.worker(id, "message", m.text));
    else if (m.t === "done") {
      if (m.serving) j.result = m.text;
      else finish(id, "done", m.text);
    } else if (m.t === "failed") finish(id, "failed", JSON.stringify(named(m.why)));
  };
  w.onerror = (e) => {
    e.preventDefault();
    finish(id, "failed", JSON.stringify(named(e.message || "it failed to start")));
  };
  w.postMessage({ t: "start", url, args: String(args), hidden: HIDDEN });
  return id;
}

function finish(id, kind, value) {
  const j = jobs.get(id);
  if (!j) return;
  jobs.delete(id);
  unlater(j.timer);
  j.w.terminate();
  work(() => architect.worker(id, kind, value));
}

/// Stops worker `id`: `why` it failed, or (asked by its page) with what
/// it returned when it was serving messages.
function stopJob(id, why) {
  const j = jobs.get(id);
  if (!j) return;
  if (why != null) finish(id, "failed", JSON.stringify(why));
  else if (j.result != null) finish(id, "done", j.result);
  else finish(id, "failed", JSON.stringify("it was stopped"));
}

// ── the main thread's messages ──────────────────────────────────────
listen("message", (ev) => {
  const m = ev.data;
  switch (m.t) {
    case "start": {
      start = m;
      for (const [k, v] of m.shared) shared.set(k, v);
      embed = m.embed;
      start.jobUrl = blobOf(m.job);
      hide(HIDDEN);
      try {
        architect.setTheme(m.theme);
        const url = link(m.entry, "page", m.entry);
        import(url).then(
          (ns) => {
            try {
              architect.mount(ns.default);
              loaded = true;
              work(() => {});
            } catch (e) {
              post({ t: "failed", why: named(show(e)) });
            }
          },
          (e) => post({ t: "failed", why: named(show(e)) }),
        );
      } catch (e) {
        // A module that does not parse or link: its message says where.
        post({ t: "failed", why: named(e instanceof Error ? e.message : String(e)) });
      }
      break;
    }
    case "event":
      work(() => architect.event(m.text));
      break;
    case "answer":
      work(() => architect.answer(m.id, m.ok, m.text));
      break;
    case "shared":
      if (m.value === "null") shared.delete(m.key);
      else shared.set(m.key, m.value);
      work(() => architect.shared(m.key, m.value));
      break;
    case "theme":
      work(() => architect.setTheme(m.text));
      break;
  }
});
