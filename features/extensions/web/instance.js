// One instance of a WASI component in the browser: a Web Worker's script
// (features/extensions/web/runner.js starts it; the native runner's
// counterpart is features/extensions/src/wasi.rs `Actor`).
//
// The component was transpiled by jco (features/extensions/web/transpile.mjs)
// into core wasm and an `instantiate` function. This worker imports it,
// links the `architect:extension` world's imports and WASI 0.2 with
// nothing to reach, and runs the calls the main thread sends, one at a
// time. A host import that waits (the host service, a worker's `post`
// and `receive`, a sleep) suspends the component with JS Promise
// Integration (JSPI) until the main thread answers.
//
// Main -> instance: `start`, `call`, `answer`, `cancel`, `theme`.
// Instance -> main: `module` (a core module, compiled once by the main
// thread with the memory limit in it), `ready`, `reply`, `trap`, `host`
// (an import that waits: the main thread's clock stops), `log`,
// `progress`, `cancel`, `ended`.

let role = 'owner';
let own = new Set();
let theme = { id: '', scheme: '', revision: 0n, tokens: [] };
let cancelled = false;
let root = null;
let memoryLimit = 0;
let memories = [];
let lastAlloc = null;
let asks = 0;
const asked = new Map();

const post = (m) => globalThis.self.postMessage(m);

/// Asks the main thread; resolves with its answer.
function ask(m) {
  const id = ++asks;
  return new Promise((resolve) => {
    asked.set(id, resolve);
    post({ ...m, t: m.t ?? 'host', id });
  });
}

const json = (v) =>
  JSON.stringify(v, (_, x) => (typeof x === 'bigint' ? Number(x) : x));

class Exit extends Error {}

// ---- WASI 0.2, with nothing to reach -------------------------------------

class WasiError {
  constructor(text) {
    this.text = text;
  }
  toDebugString() {
    return this.text;
  }
}

class Pollable {
  constructor(at = 0) {
    this.at = at;
  }
  ready() {
    return performance.now() >= this.at;
  }
  // A sleep: suspends the component (JSPI) until it is due.
  block() {
    const ms = this.at - performance.now();
    return ms > 0 ? new Promise((r) => setTimeout(r, ms)) : undefined;
  }
}

async function poll(list) {
  for (;;) {
    const ready = [];
    list.forEach((p, i) => p.ready() && ready.push(i));
    if (ready.length) return new Uint32Array(ready);
    const next = Math.min(...list.map((p) => p.at)) - performance.now();
    await new Promise((r) => setTimeout(r, Math.max(0, next)));
  }
}

/// stdout or stderr: each line into the extension's log.
class OutputStream {
  constructor(source, level) {
    this.source = source;
    this.level = level;
    this.line = '';
    this.decoder = new TextDecoder();
  }
  checkWrite() {
    return 1n << 20n;
  }
  write(bytes) {
    this.line += this.decoder.decode(bytes, { stream: true });
    let nl;
    while ((nl = this.line.indexOf('\n')) >= 0) {
      const text = this.line.slice(0, nl);
      this.line = this.line.slice(nl + 1);
      const oom = /memory allocation of (\d+) bytes failed/.exec(text);
      if (oom) lastAlloc = Number(oom[1]);
      post({ t: 'log', level: this.level, source: this.source, text });
    }
  }
  blockingWriteAndFlush(bytes) {
    this.write(bytes);
  }
  flush() {}
  blockingFlush() {}
  subscribe() {
    return new Pollable();
  }
}

class InputStream {
  read() {
    throw { tag: 'closed' };
  }
  blockingRead() {
    throw { tag: 'closed' };
  }
  subscribe() {
    return new Pollable();
  }
}

class TerminalInput {}
class TerminalOutput {}

const stdout = new OutputStream('stdout', 'info');
const stderr = new OutputStream('stderr', 'warn');

function seed() {
  const b = new BigUint64Array(2);
  crypto.getRandomValues(b);
  return b;
}

function wasi() {
  const nanos = () => BigInt(Math.round(performance.now() * 1e6));
  const random = {
    getRandomBytes: (n) => crypto.getRandomValues(new Uint8Array(Number(n))),
    getRandomU64: () => seed()[0],
    getInsecureRandomBytes: (n) => crypto.getRandomValues(new Uint8Array(Number(n))),
    getInsecureRandomU64: () => seed()[0],
  };
  return {
    'wasi:cli/environment': {
      getEnvironment: () => [],
      getArguments: () => [],
      initialCwd: () => undefined,
    },
    'wasi:cli/exit': {
      exit: (status) => {
        throw new Exit(`it exited (${status.tag})`);
      },
    },
    'wasi:cli/stdin': { getStdin: () => new InputStream() },
    'wasi:cli/stdout': { getStdout: () => stdout },
    'wasi:cli/stderr': { getStderr: () => stderr },
    'wasi:cli/terminal-input': { TerminalInput },
    'wasi:cli/terminal-output': { TerminalOutput },
    'wasi:cli/terminal-stdin': { getTerminalStdin: () => undefined },
    'wasi:cli/terminal-stdout': { getTerminalStdout: () => undefined },
    'wasi:cli/terminal-stderr': { getTerminalStderr: () => undefined },
    'wasi:clocks/monotonic-clock': {
      now: nanos,
      resolution: () => 1000n,
      subscribeInstant: (at) => new Pollable(Number(at) / 1e6),
      subscribeDuration: (d) => new Pollable(performance.now() + Number(d) / 1e6),
    },
    'wasi:clocks/wall-clock': {
      now: () => {
        const ms = Date.now();
        return { seconds: BigInt(Math.floor(ms / 1000)), nanoseconds: (ms % 1000) * 1e6 };
      },
      resolution: () => ({ seconds: 0n, nanoseconds: 1e6 }),
    },
    'wasi:io/error': { Error: WasiError },
    'wasi:io/poll': { Pollable, poll },
    'wasi:io/streams': { InputStream, OutputStream },
    'wasi:random/random': random,
    'wasi:random/insecure': random,
    'wasi:random/insecure-seed': { insecureSeed: () => [seed()[0], seed()[1]] },
    // No files: no folders are opened.
    'wasi:filesystem/preopens': { getDirectories: () => [] },
  };
}

// ---- the world's imports ---------------------------------------------------

const ok = (val) => ({ tag: 'ok', val });
const err = (val) => ({ tag: 'err', val });
const failed = (message) => JSON.stringify({ kind: 'failed', message });
const WORKER_REFUSAL = failed('a worker reaches its owner only, by messages, not the app');

/// A host-service call (`hostcall::HostRequest`), answered by the main
/// thread's host (`hostcall::HostAnswer`).
async function service(request) {
  const answer = JSON.parse(await ask({ op: 'call', request: JSON.stringify(request) }));
  return 'ok' in answer ? ok(JSON.stringify(answer.ok ?? null)) : err(JSON.stringify(answer.error));
}

function jsonArg(text) {
  if (!text.trim()) return null;
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

function world() {
  const worker = role !== 'owner';
  // A call answered with nothing (`result<_, string>`).
  const unit = async (request) => {
    if (worker) return err(WORKER_REFUSAL);
    const r = await service(request);
    return r.tag === 'ok' ? ok(undefined) : r;
  };
  // A call from its fields as JSON text, answered as JSON text.
  const fields = (op, text) => {
    if (worker) return err(WORKER_REFUSAL);
    const f = jsonArg(text);
    return service(Object.assign({}, f && typeof f === 'object' ? f : {}, { op }));
  };
  return {
    'architect:extension/types': {},
    'architect:extension/host': {
      runAction: (action, args) => {
        if (worker) return err(WORKER_REFUSAL);
        if (own.has(action))
          return err(failed(`${action} is its own: it runs it itself, not through the host`));
        return service({ op: 'run', action, args: jsonArg(args) });
      },
      setting: (key) => (worker ? err(WORKER_REFUSAL) : service({ op: 'setting', key })),
      setSetting: async (key, value) => {
        if (worker) return err(WORKER_REFUSAL);
        const r = await service({ op: 'set_setting', key, value: jsonArg(value) });
        return r.tag === 'ok' ? ok(undefined) : r;
      },
      theme: () => theme,
      log: (level, text) => post({ t: 'log', level, source: 'log', text }),
      call: async (request) => {
        if (worker) return `{"error": ${WORKER_REFUSAL}}`;
        return ask({ op: 'call', request });
      },
      callService: (name, method, args) =>
        worker ? err(WORKER_REFUSAL) : service({ op: 'service', service: name, method, args: jsonArg(args) }),
      // The app's interface (`hostcall`'s notify … decorate): the browser's
      // host answers them (`WebHost`: not in the browser yet).
      notify: (text, kind) => unit({ op: 'notify', text, kind: kind || null }),
      clipboardRead: async () => {
        if (worker) return err(WORKER_REFUSAL);
        const r = await service({ op: 'clipboard_read' });
        return r.tag === 'ok' ? ok(JSON.parse(r.val) ?? undefined) : r;
      },
      clipboardWrite: (text) => unit({ op: 'clipboard_write', text }),
      pick: (request) => fields('pick', request),
      prompt: (request) => fields('prompt', request),
      menu: (request) => fields('menu', request),
      decorate: (kind, id, decoration) =>
        unit({ op: 'decorate', target: { kind, id }, decoration: decoration == null ? null : jsonArg(decoration) }),
      // Time and events: the page's clock; hooks made at run time, the
      // browser's host answers (`WebHost`: not in the browser yet).
      now: () => BigInt(Date.now()),
      subscribe: async (hook) => {
        const r = await fields('subscribe', hook);
        return r.tag === 'ok' ? ok(JSON.parse(r.val)) : r;
      },
      unsubscribe: async (key) => {
        if (worker) return err(WORKER_REFUSAL);
        const r = await service({ op: 'unsubscribe', key });
        return r.tag === 'ok' ? ok(Boolean(JSON.parse(r.val))) : r;
      },
      // The app's documents: the browser's host has none to reach.
      listDocuments: (under) => fields('list_documents', JSON.stringify({ under: under || null })),
      readDocument: async (path) => {
        const r = await fields('read_document', JSON.stringify({ path }));
        return r.tag === 'ok' ? ok(JSON.parse(r.val)) : r;
      },
      createDocument: (path, text) => fields('create_document', JSON.stringify({ path, text })),
      editDocument: async (path, edits, name) => {
        const r = await fields('edit_document', JSON.stringify({ path, edits: jsonArg(edits), name: name ?? null }));
        return r.tag === 'ok' ? ok(JSON.parse(r.val)) : r;
      },
      renameDocument: (path, to) => fields('rename_document', JSON.stringify({ path, to })),
      moveDocument: (path, folder) => fields('move_document', JSON.stringify({ path, folder })),
      deleteDocument: (path) => unit({ op: 'delete_document', path }),
      watchDocuments: (hook) => fields('watch_documents', hook),
      // The network, through the browser host (`WebHost`): the browser's
      // fetch, where CORS lets it.
      fetch: (request) => fields('fetch', request),
      readBody: (stream, max) => fields('read_body', JSON.stringify({ stream, max: max || null })),
      closeBody: async (stream) => {
        if (worker) return false;
        const r = await service({ op: 'close_body', stream });
        return r.tag === 'ok' && Boolean(JSON.parse(r.val));
      },
      secretSet: async (name) => {
        const r = await fields('secret', JSON.stringify({ name }));
        return r.tag === 'ok' ? ok(Boolean(JSON.parse(r.val).set)) : r;
      },
      askSecret: async (name) => {
        const r = await fields('ask_secret', JSON.stringify({ name }));
        return r.tag === 'ok' ? ok(Boolean(JSON.parse(r.val))) : r;
      },
      // The editor being typed in, and MIDI and OSC ports: whatever the
      // browser's host answers (`WebHost`: not in the browser yet).
      editorState: () => fields('editor_state', ''),
      editEditor: (command) => fields('edit_editor', command),
      decorateEditor: (decorations) => unit({ op: 'decorate_editor', decorations: jsonArg(decorations) ?? [] }),
      ports: (kind) => fields('ports', JSON.stringify({ kind: kind || null })),
      openPort: (request) => fields('open_port', request),
      portSend: (port, message) => unit({ op: 'port_send', port, message: jsonArg(message) }),
      portReceive: (port, max) => fields('port_receive', JSON.stringify({ port, max: max || null })),
      closePort: async (port) => {
        if (worker) return false;
        const r = await service({ op: 'close_port', port });
        return r.tag === 'ok' && Boolean(JSON.parse(r.val));
      },
    },
    'architect:extension/worker': {
      spawn: async (name, args) => {
        if (worker) return err('a worker cannot start workers');
        const a = await ask({ op: 'spawn', name, args });
        return a.ok !== undefined ? ok(BigInt(a.ok)) : err(a.err);
      },
      post: async (to, message) => {
        const a = await ask({ op: 'post', to: Number(to), message });
        return a.err === undefined ? ok(undefined) : err(a.err);
      },
      cancel: (w) => {
        if (!worker) post({ t: 'cancel', worker: Number(w) });
      },
      progress: (done, total, note) => {
        if (worker) post({ t: 'progress', done, total, note });
      },
      cancelled: () => worker && cancelled,
      receive: async (waitMs) => {
        if (!worker) return undefined;
        const a = await ask({ op: 'receive', wait: waitMs });
        return a.message ?? undefined;
      },
    },
  };
}

// ---- the memory limit ------------------------------------------------------
//
// A sandbox's page hands this worker a core module's bytes unread: the
// memory limit is written in here, off the page's thread. The same code
// as runner.js `limitMemory` (test/limits.test.mjs keeps them equal):
// every size and count checked against the bytes there before a loop.

/// A LEB128 u32 at `at`, inside `end`: `[value, next]`; throws past it.
function leb(b, at, end) {
  let value = 0;
  for (let shift = 0; shift < 35; shift += 7) {
    if (at >= end) throw new Error('not a wasm module: a number runs past its section');
    const byte = b[at++];
    value += (byte & 0x7f) * 2 ** shift;
    if (!(byte & 0x80)) {
      if (value > 0xffffffff) throw new Error('not a wasm module: a number past 32 bits');
      return [value, at];
    }
  }
  throw new Error('not a wasm module: a number of more than 5 bytes');
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
/// refuses it. Throws when it is not a module whose sizes add up.
export function limitMemory(bytes, pages) {
  const b = new Uint8Array(bytes);
  if (b.length < 8 || b[0] !== 0 || b[1] !== 0x61 || b[2] !== 0x73 || b[3] !== 0x6d)
    throw new Error('not a wasm module');
  let at = 8;
  while (at < b.length) {
    const id = b[at];
    const [size, body] = leb(b, at + 1, b.length);
    const end = body + size;
    if (end > b.length) throw new Error('not a wasm module: a section runs past its end');
    if (id === 5) {
      let [count, p] = leb(b, body, end);
      // Each memory is at least two bytes: a count past that is a lie.
      if (count > (end - p) / 2) throw new Error('not a wasm module: more memories than its section holds');
      const out = toLeb(count);
      while (count--) {
        if (p >= end) throw new Error('not a wasm module: a memory runs past its section');
        const flags = b[p++];
        if (flags & 0x04) return b; // memory64: left alone
        let min;
        let max = Infinity;
        [min, p] = leb(b, p, end);
        if (flags & 0x01) [max, p] = leb(b, p, end);
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
  return b;
}

// ---- instantiating, calling ------------------------------------------------

function memoryNow() {
  return memories.reduce((n, m) => n + m.buffer.byteLength, 0);
}

/// Why a call that threw trapped.
function trapWhy(e) {
  if (e instanceof Exit) return e.message;
  const used = memoryNow();
  if (lastAlloc !== null || used >= memoryLimit) {
    const asked = lastAlloc === null ? used : used + lastAlloc;
    return `out of memory: it asked for ${asked >> 20} MiB, past its limit of ${memoryLimit >> 20} MiB`;
  }
  return `it trapped: ${e && e.message ? e.message : String(e)}`;
}

async function start(m) {
  role = m.role;
  memoryLimit = m.memory;
  if (m.theme) theme = { ...m.theme, revision: BigInt(m.theme.revision ?? 0) };
  // The glue as text (a sandbox's page read it): a module of this
  // worker's own origin; under node, a data: URL.
  let url = m.url;
  if (m.glue != null) {
    url = globalThis.process?.versions?.node
      ? `data:text/javascript;base64,${globalThis.Buffer.from(m.glue).toString('base64')}`
      : URL.createObjectURL(new Blob([m.glue], { type: 'text/javascript' }));
  }
  const { instantiate } = await import(url);
  const imports = { ...wasi(), ...world() };
  let refused = null;
  const guarded = new Proxy(imports, {
    get(t, k) {
      if (typeof k === 'string' && /^wasi:(sockets|http)\//.test(k)) refused ??= k;
      return t[k];
    },
  });
  root = await instantiate(
    // A compiled module from the page, or (from a sandbox's page, another
    // origin) its bytes, compiled here.
    (name) => ask({ t: 'module', name }).then((a) => a.module ?? WebAssembly.compile(limitMemory(a.bytes, a.memory))),
    guarded,
    async (module, linked) => {
      const instance = await WebAssembly.instantiate(module, linked);
      for (const v of Object.values(instance.exports))
        if (v instanceof WebAssembly.Memory) memories.push(v);
      return instance;
    },
  );
  if (refused) throw new Error(`it imports ${refused}: no capability grants the network`);
}

/// Runs `fn` of interface `iface`: its answer, an `Err` it returned, or a trap.
async function call(m) {
  const f = root?.[m.iface]?.[m.fn];
  if (!f) return post({ t: 'reply', id: m.id, error: `it has no ${m.iface}.${m.fn}` });
  const args = (m.args ?? []).map((a, i) => (m.big?.includes(i) ? BigInt(a) : a));
  if (m.event) args[1] = { ...args[1], element: BigInt(args[1].element) };
  let value;
  try {
    value = await f(...args);
  } catch (e) {
    if (e && Object.hasOwn(e, 'payload')) {
      return post({ t: 'reply', id: m.id, error: String(e.payload) });
    }
    return post({ t: 'trap', id: m.id, why: trapWhy(e) });
  }
  if (memoryNow() > memoryLimit) return post({ t: 'trap', id: m.id, why: trapWhy(null) });
  if (m.fn === 'actions') own = new Set(value);
  post({ t: 'reply', id: m.id, value: json(value ?? null) });
}

// (Imported by a test, outside a worker, it only defines its functions.)
(globalThis.self ?? globalThis).onmessage = async ({ data: m }) => {
  switch (m.t) {
    case 'start':
      try {
        await start(m);
        post({ t: 'ready' });
      } catch (e) {
        post({ t: 'failed', why: e && e.message ? e.message : String(e) });
      }
      break;
    case 'call':
      await call(m);
      break;
    case 'answer': {
      const resolve = asked.get(m.id);
      asked.delete(m.id);
      resolve?.(m.value);
      break;
    }
    case 'cancel':
      cancelled = true;
      break;
    case 'theme':
      theme = { ...m.theme, revision: BigInt(m.theme.revision ?? 0) };
      break;
  }
};
