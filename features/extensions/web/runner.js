// WASI component extensions in the browser: the main thread's side
// (docs/content/architecture/extensions.md, "WASI components", "On the
// web"; the native runner is features/extensions/src/wasi.rs and
// features/extensions/src/wasi/workers.rs).
//
// A component, transpiled by jco (transpile.mjs), runs in Web Workers:
// its instance in one (`instance.js`), each of its workers in one more.
// This side is the actor the native runner keeps on a thread: it sends one
// call at a time, times each against its limit (the instance's clock
// stops while the host answers it), terminates an instance past it (no
// epoch interruption in a browser: the Web Worker is the sandbox that can
// be stopped), starts it again fresh, and keeps its workers' queues, their
// back-pressure counted here since a browser's `postMessage` cannot block.
//
// Everything the component asks of the app goes to `host` as a host-service
// request (`hostcall::HostRequest` JSON), answered with `hostcall::HostAnswer`
// JSON: the embedder checks it against the extension's capabilities
// (features/extensions/src/web.rs `WebHost`).

/// Each way, a worker's queue holds this many messages (`wasi::QUEUE`).
export const QUEUE = 64;
/// An instance's workers running or waiting for the pool, at most (`wasi::MAX_WORKERS`).
export const MAX_WORKERS = 8;
/// News reaches an owner at most this often, in ms (`wasi::NEWS_EVERY`).
export const NEWS_EVERY = 16;
/// An instance's limits (`wasi::Limits`): 64 MiB, 5 s a call.
export const LIMITS = { memory: 64 << 20, call: 5000 };
/// A worker's: 64 MiB, 60 s for its whole run (`Limits::worker`).
export const WORKER_LIMITS = { memory: 64 << 20, call: 60000 };

const PAGE = 1 << 16;

// ---- the memory limit, in the module ---------------------------------------

function leb(bytes, at) {
  let value = 0;
  let shift = 0;
  for (;;) {
    const b = bytes[at++];
    value += (b & 0x7f) * 2 ** shift;
    shift += 7;
    if (!(b & 0x80)) return [value, at];
  }
}

function toLeb(n) {
  const out = [];
  do {
    let b = n % 128;
    n = Math.floor(n / 128);
    if (n) b |= 0x80;
    out.push(b);
  } while (n);
  return out;
}

/// `bytes` (a core module) with each memory it defines at most `pages`
/// pages: past them `memory.grow` fails, as a native `ResourceLimiter`
/// refuses it.
export function limitMemory(bytes, pages) {
  const b = new Uint8Array(bytes);
  let at = 8;
  while (at < b.length) {
    const id = b[at];
    const [size, body] = leb(b, at + 1);
    const end = body + size;
    if (id === 5) {
      let [count, p] = leb(b, body);
      const out = toLeb(count);
      while (count--) {
        const flags = b[p++];
        if (flags & 0x04) return bytes; // memory64: left alone
        let min;
        let max = Infinity;
        [min, p] = leb(b, p);
        if (flags & 0x01) [max, p] = leb(b, p);
        out.push((flags & 0x02) | 0x01, ...toLeb(min), ...toLeb(Math.max(min, Math.min(max, pages))));
      }
      const section = [5, ...toLeb(out.length), ...out];
      const patched = new Uint8Array(at + section.length + (b.length - end));
      patched.set(b.subarray(0, at), 0);
      patched.set(section, at);
      patched.set(b.subarray(end), at + section.length);
      return patched;
    }
    at = end;
  }
  return bytes;
}

/// Compiled core modules, shared by every instance of a component (the
/// native compile cache's part): by URL and memory limit.
const compiled = new Map();

async function coreModule(url, name, memory, load) {
  const key = `${url}#${name}#${memory}`;
  if (!compiled.has(key)) {
    compiled.set(
      key,
      load(new URL(name, url).href).then((bytes) =>
        WebAssembly.compile(limitMemory(bytes, Math.floor(memory / PAGE))),
      ),
    );
  }
  return compiled.get(key);
}

async function fetchBytes(url) {
  if (url.startsWith('file:')) {
    const fs = await import('node:fs/promises');
    return fs.readFile(new URL(url));
  }
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${url}: ${r.status}`);
  return r.arrayBuffer();
}

// ---- the pool --------------------------------------------------------------

/// How many workers run at once (`wasi::Pool`): one fewer than the
/// machine has, at most four; the rest wait.
export class Pool {
  constructor(size) {
    const cores = globalThis.navigator?.hardwareConcurrency ?? 2;
    this.size = Math.max(1, size ?? Math.min(4, cores - 1));
    this.running = 0;
    this.waiting = [];
  }
  enter(go) {
    if (this.running < this.size) {
      this.running++;
      go();
    } else this.waiting.push(go);
  }
  leave() {
    const next = this.waiting.shift();
    if (next) next();
    else this.running--;
  }
  forget(go) {
    const i = this.waiting.indexOf(go);
    if (i >= 0) this.waiting.splice(i, 1);
    return i >= 0;
  }
}

const POOL = new Pool();

// ---- one instance: a Web Worker -------------------------------------------

function defaultSpawn(instance) {
  if (instance instanceof URL || /^(https?|file|blob):/.test(instance)) {
    return new Worker(instance, { type: 'module' });
  }
  const url = URL.createObjectURL(new Blob([instance], { type: 'text/javascript' }));
  return new Worker(url, { type: 'module' });
}

/// One instance in a Web Worker, its own time measured: the clock runs
/// while it runs, and stops while the host answers it.
class Instance {
  constructor(opts, role, limits, onAsk, onNote) {
    this.opts = opts;
    this.limits = limits;
    this.onAsk = onAsk;
    this.onNote = onNote;
    this.worker = (opts.spawn ?? defaultSpawn)(opts.instance);
    this.calls = new Map();
    this.nextCall = 0;
    this.spent = 0;
    this.since = null;
    this.timer = null;
    this.dead = null;
    this.ready = new Promise((resolve, reject) => {
      this.started = { resolve, reject };
    });
    this.worker.onmessage = (e) => this.message(e.data ?? e);
    this.worker.onerror = (e) => this.kill(`it failed: ${e.message ?? e}`);
    this.worker.postMessage({
      t: 'start',
      url: opts.url,
      role,
      memory: limits.memory,
      theme: opts.theme ? opts.theme() : null,
    });
  }

  /// Its own time so far, in ms.
  own() {
    return this.spent + (this.since === null ? 0 : performance.now() - this.since);
  }

  run() {
    if (this.since !== null || this.dead) return;
    this.since = performance.now();
    const left = Math.max(0, this.limits.call - this.spent);
    this.timer = setTimeout(() => {
      const ran = Math.round(this.own());
      this.kill(`interrupted: it ran for ${ran} ms, past its limit of ${this.limits.call} ms`);
    }, left);
  }

  pause() {
    if (this.since === null) return;
    this.spent += performance.now() - this.since;
    this.since = null;
    clearTimeout(this.timer);
  }

  /// A call, its clock running (a worker's run: one clock for it all;
  /// an owner's call: a clock each).
  call(iface, fn, args = [], extra = {}) {
    if (this.dead) return Promise.reject({ trap: this.dead });
    const id = ++this.nextCall;
    if (extra.fresh !== false) this.spent = 0;
    this.run();
    return new Promise((resolve, reject) => {
      this.calls.set(id, { resolve, reject });
      this.worker.postMessage({ t: 'call', id, iface, fn, args, ...extra });
    });
  }

  settle(id) {
    const c = this.calls.get(id);
    this.calls.delete(id);
    if (!this.calls.size) this.pause();
    return c;
  }

  async message(m) {
    switch (m.t) {
      case 'ready':
        this.started.resolve();
        break;
      case 'failed':
        this.started.reject(new Error(m.why));
        this.kill(m.why);
        break;
      case 'module': {
        const module = await coreModule(this.opts.url, m.name, this.limits.memory, this.opts.load ?? fetchBytes);
        this.worker.postMessage({ t: 'answer', id: m.id, value: { module } });
        break;
      }
      case 'reply': {
        const c = this.settle(m.id);
        if (!c) break;
        if (m.error !== undefined) c.reject({ error: m.error });
        else c.resolve(m.value === undefined ? null : JSON.parse(m.value));
        break;
      }
      case 'trap':
        this.kill(m.why);
        break;
      case 'host': {
        this.pause();
        let value;
        try {
          value = await this.onAsk(m);
        } catch (e) {
          value = m.op === 'call'
            ? JSON.stringify({ error: { kind: 'failed', message: String(e?.message ?? e) } })
            : { err: String(e?.message ?? e) };
        }
        if (this.dead) break;
        if (this.calls.size) this.run();
        this.worker.postMessage({ t: 'answer', id: m.id, value });
        break;
      }
      default:
        this.onNote(m);
    }
  }

  /// Ends it: a trap, its limit, a cancel or an unload.
  kill(why) {
    if (this.dead) return;
    this.dead = why;
    this.pause();
    this.worker.terminate();
    this.started.reject(new Error(why));
    for (const c of this.calls.values()) c.reject({ trap: why });
    this.calls.clear();
    this.onNote({ t: 'dead', why });
  }
}

// ---- a component: its instance, its workers --------------------------------

/// A WASI component's instance in the browser, with its workers.
///
/// `opts`: `url` (the transpiled component's module), `instance`
/// (instance.js: its URL or its source), `limits`, `workerLimits`,
/// `workers` (`{ id: { entry, url? } }`, the manifest's `[[workers]]`),
/// `host(requestJson) -> Promise<answerJson>`, `log(level, source, text)`,
/// `theme() -> {id, scheme, revision, tokens}`, `frame(pane, frame)` (a
/// view's edits, `{ edits }`, or `{ failed }`), `restarted(why)`, and for
/// tests `spawn(instance) -> Worker`, `load(url) -> bytes`, `pool`.
export class Component {
  constructor(opts) {
    this.opts = opts;
    this.limits = { ...LIMITS, ...opts.limits };
    this.workerLimits = { ...WORKER_LIMITS, ...opts.workerLimits };
    this.pool = opts.pool ?? POOL;
    this.queue = Promise.resolve();
    this.panes = new Set();
    this.crew = new Map();
    this.nextWorker = 0;
    this.newsDue = false;
    this.heardAt = 0;
    this.newsTimer = null;
    this.live = null;
    this.stopped = false;
    this.stats = { calls: 0, traps: 0, lastTrap: null, alive: false, generation: 0 };
    this.info = { actions: [], views: [] };
  }

  log(level, source, text) {
    this.opts.log?.(level, source, text);
  }

  /// Starts it (with what the copy it replaces saved): its actions and views.
  start(state = null) {
    return this.job(() => this.instantiate(state));
  }

  async instantiate(state) {
    const live = new Instance(
      this.opts,
      'owner',
      this.limits,
      (m) => this.ownerAsks(m),
      (m) => this.ownerNote(live, m),
    );
    await live.ready;
    await live.call('guest', 'init', [state]).catch((e) => {
      throw new Error(e.trap ?? e.error);
    });
    const actions = await live.call('guest', 'actions');
    const views = await live.call('guest', 'views');
    this.live = live;
    this.stats.alive = true;
    this.stats.generation++;
    this.info = { actions, views };
    return this.info;
  }

  /// One call at a time, as the native actor's queue: `f` after the last.
  job(f) {
    const run = this.queue.then(f, f);
    this.queue = run.then(
      () => this.afterJob(),
      () => this.afterJob(),
    );
    return run;
  }

  afterJob() {
    this.stats.calls++;
    if (this.newsDue) this.scheduleNews();
  }

  notRunning() {
    return this.stats.lastTrap ? `it is not running: ${this.stats.lastTrap}` : 'it is not running';
  }

  /// `f` on its live instance; a trap starts it again.
  async on(f) {
    if (!this.live) throw new Error(this.notRunning());
    try {
      return await f(this.live);
    } catch (e) {
      if (e && e.trap !== undefined) {
        await this.trapped(e.trap);
        throw new Error(e.trap);
      }
      throw new Error(e?.error ?? e?.message ?? String(e));
    }
  }

  /// Runs action `action` (JSON `args`, the invocation as JSON); then what
  /// its views changed.
  invoke(action, args, invocation) {
    return this.job(async () => {
      try {
        return await this.on((i) => i.call('guest', 'invoke', [action, args, invocation]));
      } finally {
        await this.timers();
      }
    });
  }

  /// Mounts view `view` as `pane`: its first frame to `frame`.
  render(pane, view) {
    return this.job(async () => {
      try {
        const edits = await this.on((i) => i.call('guest', 'render', [pane, view], { big: [0] }));
        this.panes.add(pane);
        this.opts.frame?.(pane, { edits });
      } catch (e) {
        this.opts.frame?.(pane, { failed: e.message });
      }
      // What it rendered may have set a timer.
      await this.timers();
    });
  }

  /// An event in `pane`'s view (the world's `view-event`, ids as numbers).
  event(pane, event) {
    return this.job(async () => {
      if (!this.live || !this.panes.has(pane)) return;
      try {
        const edits = await this.on((i) => i.call('guest', 'onEvent', [pane, event], { big: [0], event: true }));
        if (edits.length) this.opts.frame?.(pane, { edits });
      } catch (e) {
        if (this.panes.delete(pane)) this.opts.frame?.(pane, { failed: e.message });
      }
      await this.timers();
    });
  }

  close(pane) {
    return this.job(async () => {
      this.panes.delete(pane);
      if (this.live) await this.on((i) => i.call('guest', 'close', [pane], { big: [0] })).catch(() => {});
    });
  }

  saveState() {
    return this.job(() => (this.live ? this.on((i) => i.call('guest', 'saveState')).catch(() => null) : null));
  }

  /// Its views' timers that are due (the world's `tick`), then what they
  /// changed; the next, on a timer of the page's.
  async timers() {
    clearTimeout(this.timer);
    this.timer = null;
    if (!this.live) return;
    try {
      const next = await this.on((i) => i.call('guest', 'tick'));
      if (next != null && !this.stopped) {
        this.timer = setTimeout(() => this.job(() => this.timers()), Math.max(4, Number(next)));
      }
    } catch {
      return; // trapped: started again
    }
    await this.flush();
  }

  /// What its views changed by themselves, to their panes.
  async flush() {
    if (!this.panes.size || !this.live) return;
    try {
      const pending = await this.on((i) => i.call('guest', 'pending'));
      for (const p of pending) {
        if (p.edits.length && this.panes.has(p.pane)) this.opts.frame?.(p.pane, { edits: p.edits });
      }
    } catch {
      // trapped: started again
    }
  }

  async trapped(why) {
    this.stats.traps++;
    this.stats.lastTrap = why;
    this.log('error', 'wasi', `${why}; starting it again`);
    await this.restart(why);
  }

  async restart(why) {
    clearTimeout(this.timer);
    this.timer = null;
    this.live?.kill(why);
    this.live = null;
    this.stats.alive = false;
    const cancelled = this.cancelAll();
    if (cancelled) this.log('info', 'wasi', `${cancelled} worker(s) cancelled: it is starting again`);
    const panes = [...this.panes];
    this.panes.clear();
    for (const p of panes) this.opts.frame?.(p, { failed: why });
    if (this.stopped) return;
    try {
      await this.instantiate(null);
      this.opts.restarted?.(why);
    } catch (e) {
      this.log('error', 'wasi', `it did not start again: ${e.message}`);
    }
  }

  /// Unloads it: its instance and workers end.
  stop() {
    this.stopped = true;
    clearTimeout(this.newsTimer);
    clearTimeout(this.timer);
    this.cancelAll();
    this.live?.kill('it was unloaded');
    this.live = null;
    this.stats.alive = false;
  }

  /// What it runs: its instance and each worker.
  report() {
    return {
      ...this.stats,
      memory: this.limits.memory,
      workers: [...this.crew.values()].map((w) => ({
        id: w.id,
        name: w.name,
        status: w.status,
        progress: w.progress,
      })),
    };
  }

  // ---- its workers ----------------------------------------------------------

  /// The owner asked the host something that waits.
  async ownerAsks(m) {
    switch (m.op) {
      case 'call':
        return this.opts.host(m.request);
      case 'spawn':
        return this.spawn(m.name, m.args);
      case 'post':
        return this.toWorker(m.to, m.message);
      default:
        return { err: `no ${m.op}` };
    }
  }

  ownerNote(live, m) {
    if (m.t === 'log') this.log(m.level, m.source, m.text);
    else if (m.t === 'cancel') this.cancel(m.worker, 'its owner cancelled it');
    else if (m.t === 'dead' && this.live === live && !this.stopped && !live.calls.size) {
      // Ended between calls (it cannot: a call is how it runs).
    }
  }

  alive() {
    return [...this.crew.values()].filter((w) => w.status === 'waiting' || w.status === 'running').length;
  }

  spawn(name, args) {
    const decl = this.opts.workers?.[name];
    if (!decl) return { err: `it has no worker ${name} (its manifest's [[workers]])` };
    if (this.alive() >= MAX_WORKERS)
      return { err: `it has ${MAX_WORKERS} workers running or waiting: as many as it may` };
    const id = ++this.nextWorker;
    const w = {
      id,
      name,
      entry: decl.entry,
      url: decl.url ?? this.opts.url,
      args,
      status: 'waiting',
      inbox: [],
      receiving: null,
      news: [],
      blocked: [],
      progress: null,
      outcome: null,
      instance: null,
      go: null,
    };
    this.crew.set(id, w);
    w.go = () => this.begin(w);
    this.pool.enter(w.go);
    return { ok: id };
  }

  begin(w) {
    if (w.status !== 'waiting') return this.pool.leave();
    w.status = 'running';
    const opts = { ...this.opts, url: w.url };
    w.instance = new Instance(
      opts,
      'worker',
      this.workerLimits,
      (m) => this.workerAsks(w, m),
      (m) => this.workerNote(w, m),
    );
    const entry = w.url === this.opts.url ? w.entry : w.name;
    w.instance.ready
      .then(() => w.instance.call('guest', 'init', [null]))
      .then(() => w.instance.call('work', 'run', [entry, w.id, w.args], { big: [1], fresh: false }))
      .then(
        (result) => this.ended(w, { tag: 'done', val: result }),
        (e) => this.ended(w, w.cancelled ? { tag: 'cancelled' } : { tag: 'failed', val: String(e?.trap ?? e?.error ?? e?.message ?? e) }),
      );
  }

  /// The owner's `post` to worker `to`: refused at once when its queue is
  /// full (a view never waits) or it ended.
  toWorker(to, message) {
    const w = this.crew.get(to);
    if (!w || w.outcome) return { err: `worker ${to} ended` };
    if (w.inbox.length >= QUEUE)
      return { err: `worker ${to}'s queue is full (${QUEUE} messages it has not read)` };
    if (w.receiving) {
      const r = w.receiving;
      w.receiving = null;
      r({ message });
    } else w.inbox.push(message);
    return {};
  }

  async workerAsks(w, m) {
    switch (m.op) {
      case 'post':
        // To its owner: it waits while the owner's queue is full.
        if (w.outcome || w.cancelled) return { err: 'cancelled' };
        if (w.news.length < QUEUE) {
          w.news.push(m.message);
          this.nudge();
          return {};
        }
        return new Promise((resolve) => w.blocked.push({ message: m.message, resolve }));
      case 'receive':
        if (w.inbox.length) return { message: w.inbox.shift() };
        if (!m.wait) return {};
        return new Promise((resolve) => {
          const timer = setTimeout(() => {
            w.receiving = null;
            resolve({});
          }, m.wait);
          w.receiving = (a) => {
            clearTimeout(timer);
            resolve(a);
          };
        });
      default:
        return m.op === 'call' ? JSON.stringify({ error: { kind: 'failed', message: 'a worker reaches its owner only' } }) : { err: 'a worker reaches its owner only' };
    }
  }

  workerNote(w, m) {
    if (m.t === 'log') this.log(m.level, `worker ${w.name}`, m.text);
    else if (m.t === 'progress') {
      w.progress = { done: m.done, total: m.total, note: m.note };
      w.progressNew = true;
      this.nudge();
    }
  }

  ended(w, outcome) {
    if (w.outcome) return;
    w.outcome = outcome;
    w.status = outcome.tag;
    w.instance?.kill(outcome.tag === 'done' ? 'it ended' : (outcome.val ?? 'cancelled'));
    for (const b of w.blocked) b.resolve({ err: 'cancelled' });
    w.blocked = [];
    if (w.receiving) w.receiving({});
    if (w.instance) this.pool.leave();
    else this.pool.forget(w.go);
    if (outcome.tag === 'failed') this.log('warn', `worker ${w.name}`, outcome.val);
    w.doneNew = true;
    this.nudge();
  }

  /// Stops worker `id` at once, whether it looks or not: its Web Worker
  /// is terminated.
  cancel(id, why) {
    const w = this.crew.get(id);
    if (!w || w.outcome) return;
    w.cancelled = true;
    w.instance?.kill(why);
    this.ended(w, { tag: 'cancelled' });
  }

  cancelAll() {
    let n = 0;
    for (const w of this.crew.values()) {
      if (!w.outcome) {
        this.cancel(w.id, 'its owner went');
        w.doneNew = false;
        n++;
      }
    }
    this.crew.clear();
    return n;
  }

  // ---- news ------------------------------------------------------------------

  nudge() {
    this.newsDue = true;
    this.scheduleNews();
  }

  scheduleNews() {
    if (this.newsTimer || this.stopped) return;
    const wait = Math.max(0, NEWS_EVERY - (performance.now() - this.heardAt));
    this.newsTimer = setTimeout(() => {
      this.newsTimer = null;
      this.job(() => this.hear());
    }, wait);
  }

  /// Its workers' news, between its calls: every message in order, the
  /// latest progress, the end once; then what its views changed.
  async hear() {
    this.newsDue = false;
    this.heardAt = performance.now();
    if (!this.live) return;
    let heard = false;
    try {
      for (const w of [...this.crew.values()]) {
        while (w.news.length) {
          const message = w.news.shift();
          const b = w.blocked.shift();
          if (b) {
            w.news.push(b.message);
            b.resolve({});
          }
          heard = true;
          await this.on((i) => i.call('work', 'onMessage', [w.id, message], { big: [0] }));
        }
        if (w.progressNew) {
          w.progressNew = false;
          heard = true;
          await this.on((i) => i.call('work', 'onProgress', [w.id, w.progress], { big: [0] }));
        }
        if (w.doneNew) {
          w.doneNew = false;
          heard = true;
          await this.on((i) => i.call('work', 'onDone', [w.id, w.outcome], { big: [0] }));
          this.crew.delete(w.id);
        }
      }
    } catch {
      return; // trapped: started again
    }
    if (heard) await this.flush();
  }
}
