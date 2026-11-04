// WASI component extensions in the browser's shape, under node: the lab's
// counter and the misbehaving component (features/extensions/tests/wasi-hazards),
// built for wasm32-wasip2, transpiled by jco, each instance in a worker
// thread standing in for a Web Worker (harness.mjs). The same checks as
// the native runner's (features/extensions/tests/wasi.rs): a view, actions,
// the host service, both limits, workers, their queues' back-pressure.
//
// Run by features/extensions/tests/wasi_web.rs (`cargo test`), or
// `npm test` here once the components are built.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Component, Pool, QUEUE, MAX_WORKERS, limitMemory } from '../runner.js';
import { guests, transpiled, spawn } from './harness.mjs';

const dir = guests();
const skip = dir ? false : 'the components are not built (cargo build --target wasm32-wasip2)';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function until(what, f, ms = 20000) {
  const end = performance.now() + ms;
  for (;;) {
    const v = await f();
    if (v) return v;
    if (performance.now() > end) assert.fail(`timed out waiting for ${what}`);
    await sleep(10);
  }
}

/// A component with a host that answers THEME_LIST and refuses the rest,
/// as a `WebHost` would outside its capabilities.
function component(name, opts = {}) {
  const logs = [];
  const frames = [];
  const asked = [];
  const c = new Component({
    url: transpiled(dir, name),
    instance: '',
    spawn,
    pool: new Pool(4),
    host: async (request) => {
      const r = JSON.parse(request);
      asked.push(r);
      if (r.op === 'run' && r.action === 'THEME_LIST') return JSON.stringify({ ok: ['light', 'dark', 'mocha'] });
      return JSON.stringify({ error: { kind: 'refused', capability: 'actions', why: `${r.action ?? r.op}: no` } });
    },
    log: (level, source, text) => logs.push({ level, source, text }),
    frame: (pane, f) => frames.push({ pane, ...f }),
    ...opts,
  });
  return { c, logs, frames, asked };
}

const counter = (opts) =>
  component('architect_lab_wasi_counter', { workers: { primes: { entry: 'count-primes' } }, ...opts });
const hazards = (opts) =>
  component('architect_extension_wasi_hazards', {
    workers: Object.fromEntries(['forever', 'hoard', 'chatty', 'deaf', 'echo'].map((w) => [w, { entry: w }])),
    ...opts,
  });

/// A view's edits applied to a little tree (the wire's stack machine,
/// as `RemoteDocument` and web.rs `Pane` apply it): element `cls`'s id.
function idOf(frames, cls) {
  const root = { kids: [], attrs: {} };
  const ids = new Map([[0, root]]);
  const stack = [];
  const node = (n) => ({ kids: [], attrs: {}, ...n });
  const place = (m) => stack.splice(stack.length - m, m);
  for (const f of frames) {
    for (const { tag, val } of f.edits ?? []) {
      const top = stack[stack.length - 1];
      if (tag === 'push-id') stack.push(ids.get(val));
      else if (tag === 'set-id') { ids.set(val, top); top.id = val; }
      else if (tag === 'child') stack[stack.length - 1] = top.kids[val];
      else if (tag === 'pop') stack.pop();
      else if (tag === 'create-element') stack.push(node({ tag: val.tag }));
      else if (tag === 'create-text') stack.push(node({ text: val }));
      else if (tag === 'clone') stack[stack.length - 1] = structuredClone(top);
      else if (tag === 'append-children') { const k = place(val); stack[stack.length - 1].kids.push(...k); }
      else if (tag === 'set-attribute') top.attrs[val.name] = val.value.val;
    }
  }
  const find = (n) => (n.attrs.class === cls ? n.id : n.kids.map(find).find((x) => x !== undefined));
  return find(root) ?? null;
}

const texts = (frames) =>
  frames.flatMap((f) => (f.edits ?? []).filter((e) => e.tag === 'set-text' || e.tag === 'create-text').map((e) => e.val));

const click = (element) => ({
  element,
  name: 'click',
  bubbles: true,
  data: {
    tag: 'mouse',
    val: {
      clientX: 0, clientY: 0, pageX: 0, pageY: 0, screenX: 0, screenY: 0, elementX: 0, elementY: 0,
      button: 0, buttons: 0, mods: { alt: false, ctrl: false, meta: false, shift: false },
    },
  },
});

test('the counter renders, a click and its actions change it, it asks the app', { skip }, async () => {
  const { c, frames, asked, logs } = counter();
  const info = await c.start();
  assert.deepEqual(info.views, ['counter']);
  assert.ok(info.actions.includes('LAB_WASI_COUNTER_ADD'));
  await c.render(1, 'counter');
  assert.equal(frames[0].pane, 1);
  assert.ok(texts(frames).includes('0'), 'it shows 0');

  const add = idOf(frames, 'wasi-add');
  assert.ok(add !== null, 'the Add button has an id');
  await c.event(1, click(add));
  assert.ok(texts(frames).includes('1'), 'a click made it 1');

  assert.equal(await c.invoke('LAB_WASI_COUNTER_ADD', '{"by":5}', '{}'), '6');
  assert.ok(texts(frames).includes('6'), 'its view followed the action');

  const said = await c.invoke('LAB_WASI_COUNTER_ASK', '', '{}');
  assert.deepEqual(JSON.parse(said), ['light', 'dark', 'mocha']);
  assert.deepEqual(asked[0], { op: 'run', action: 'THEME_LIST', args: null });
  assert.ok(logs.some((l) => l.text === 'The app has 3 themes'));
  c.stop();
});

test('a refused host call is the host service error, as JSON', { skip }, async () => {
  const refuse = async () =>
    JSON.stringify({ error: { kind: 'refused', capability: 'actions', why: 'THEME_LIST is not in a namespace this extension may run' } });
  const { c, frames } = counter({ host: refuse });
  await c.start();
  await c.render(1, 'counter');
  await assert.rejects(c.invoke('LAB_WASI_COUNTER_ASK', '', '{}'), /refused.*THEME_LIST is not in a namespace/);
  assert.ok(texts(frames).some((t) => t.startsWith('The app said no: refused')), JSON.stringify(texts(frames)));
  c.stop();
});

test('stdout is a line in its log', { skip }, async () => {
  const { c, logs } = hazards();
  await c.start();
  await c.invoke('TEST_HAZARDS_ECHO', '7', '{}');
  assert.ok(logs.some((l) => l.source === 'stdout' && l.text === 'echo 7'), JSON.stringify(logs));
  c.stop();
});

test('a call that never ends is interrupted at its limit; it starts again', { skip }, async () => {
  const { c, logs } = hazards({ limits: { call: 300 } });
  await c.start();
  const t = performance.now();
  await assert.rejects(c.invoke('TEST_HAZARDS_SPIN', '', '{}'), /interrupted: it ran for \d+ ms, past its limit of 300 ms/);
  assert.ok(performance.now() - t < 3000);
  assert.equal(c.report().traps, 1);
  assert.equal(c.report().generation, 2, 'started again');
  assert.equal(await c.invoke('TEST_HAZARDS_ECHO', '1', '{}'), '{"echo":1}');
  assert.ok(logs.some((l) => /starting it again/.test(l.text)));
  c.stop();
});

test('memory stops at its limit; it starts again', { skip }, async () => {
  const { c } = hazards({ limits: { memory: 32 << 20 } });
  await c.start();
  await assert.rejects(c.invoke('TEST_HAZARDS_HOARD', '', '{}'), /out of memory: it asked for \d+ MiB, past its limit of 32 MiB/);
  assert.equal(c.report().traps, 1);
  assert.equal(await c.invoke('TEST_HAZARDS_ECHO', '2', '{}'), '{"echo":2}');
  c.stop();
});

test('the memory limit is written into the module', () => {
  // (module (memory 1)) and (memory 1 100000)
  const one = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 5, 3, 1, 0, 1]);
  const limited = limitMemory(one, 4);
  assert.deepEqual([...limited.subarray(8)], [5, 4, 1, 1, 1, 4]);
  const big = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 5, 6, 1, 1, 1, 0xa0, 0x8d, 0x06]);
  assert.deepEqual([...limitMemory(big, 4).subarray(8)], [5, 4, 1, 1, 1, 4]);
  assert.ok(WebAssembly.validate(limited));
});

test('a primes job in a worker: the counter keeps answering', { skip }, async () => {
  const { c, frames } = counter();
  await c.start();
  await c.render(1, 'counter');
  const add = idOf(frames, 'wasi-add');
  await c.invoke('LAB_WASI_COUNTER_PRIMES', '{"n":2000000}', '{}');
  const slowest = [];
  for (let i = 0; i < 20; i++) {
    const t = performance.now();
    await c.event(1, click(add));
    slowest.push(performance.now() - t);
    await sleep(20);
  }
  slowest.sort((a, b) => a - b);
  // Each click within a frame budget, with the job running elsewhere.
  assert.ok(slowest[18] < 50, `p95 ${slowest[18]} ms`);
  assert.ok(texts(frames).includes('20'));
  const job = await until('the job', async () => {
    const s = JSON.parse(await c.invoke('LAB_WASI_COUNTER_JOB', '', '{}'));
    return s.status === 'done' ? s : null;
  }, 120000);
  assert.equal(job.result.found, 148933);
  assert.ok(texts(frames).some((t) => /148933 primes below 2000000/.test(t)));
  c.stop();
});

test('a worker is cancelled when it unloads, or when asked', { skip }, async () => {
  const { c } = hazards();
  await c.start();
  const id = JSON.parse(await c.invoke('TEST_HAZARDS_WORK', '{"worker":"forever"}', '{}'));
  await until('it runs', () => c.report().workers.some((w) => w.status === 'running'));
  c.stop();
  assert.equal(c.report().workers.length, 0);
  assert.equal(c.pool.running, 0, 'its thread is free');
  assert.ok(id > 0);
});

test('a worker past its limits fails alone; its owner runs on', { skip }, async () => {
  const { c } = hazards({ workerLimits: { call: 300, memory: 32 << 20 } });
  await c.start();
  const state = async (id) => JSON.parse(await c.invoke('TEST_HAZARDS_WORKER', JSON.stringify({ id }), '{}'));
  const spin = JSON.parse(await c.invoke('TEST_HAZARDS_WORK', '{"worker":"forever"}', '{}'));
  const hoard = JSON.parse(await c.invoke('TEST_HAZARDS_WORK', '{"worker":"hoard"}', '{}'));
  const a = await until('forever fails', async () => ((await state(spin)).status === 'failed' ? state(spin) : null));
  assert.match(a.error, /interrupted: it ran for \d+ ms, past its limit of 300 ms/);
  const b = await until('hoard fails', async () => ((await state(hoard)).status === 'failed' ? state(hoard) : null));
  assert.match(b.error, /out of memory: .* past its limit of 32 MiB/);
  assert.equal(c.report().traps, 0, 'the owner did not trap');
  c.stop();
});

test('back-pressure: an owner post to a full queue fails at once', { skip }, async () => {
  const { c } = hazards();
  await c.start();
  const id = JSON.parse(await c.invoke('TEST_HAZARDS_WORK', '{"worker":"deaf"}', '{}'));
  await until('it runs', () => c.report().workers.some((w) => w.status === 'running'));
  const flood = JSON.parse(await c.invoke('TEST_HAZARDS_FLOOD', JSON.stringify({ id, n: 100 }), '{}'));
  assert.equal(flood.accepted, QUEUE);
  assert.match(flood.error, /queue is full/);
  c.stop();
});

test("back-pressure: a worker's post waits; every message arrives, in order", { skip }, async () => {
  const { c } = hazards();
  await c.start();
  const id = JSON.parse(await c.invoke('TEST_HAZARDS_WORK', '{"worker":"chatty","args":{"n":300}}', '{}'));
  // The owner busy: the worker fills the owner's queue and waits.
  await c.invoke('TEST_HAZARDS_STALL', '{"ms":300}', '{}');
  const s = await until('it is done', async () => {
    const s = JSON.parse(await c.invoke('TEST_HAZARDS_WORKER', JSON.stringify({ id }), '{}'));
    return s.status === 'done' ? s : null;
  });
  assert.deepEqual(s.messages, [...Array(300).keys()]);
  assert.equal(s.result, 300);
  c.stop();
});

test('a worker receives, and answers', { skip }, async () => {
  const { c } = hazards();
  await c.start();
  const id = JSON.parse(await c.invoke('TEST_HAZARDS_WORK', '{"worker":"echo"}', '{}'));
  await c.invoke('TEST_HAZARDS_FLOOD', JSON.stringify({ id, n: 3 }), '{}');
  const s = await until('three echoes', async () => {
    const s = JSON.parse(await c.invoke('TEST_HAZARDS_WORKER', JSON.stringify({ id }), '{}'));
    return s.messages.length === 3 ? s : null;
  });
  assert.deepEqual(s.messages, [{ echo: 0 }, { echo: 1 }, { echo: 2 }]);
  c.stop();
});

test('at most MAX_WORKERS running or waiting', { skip }, async () => {
  const { c } = hazards();
  await c.start();
  for (let i = 0; i < MAX_WORKERS; i++) await c.invoke('TEST_HAZARDS_WORK', '{"worker":"deaf"}', '{}');
  await assert.rejects(c.invoke('TEST_HAZARDS_WORK', '{"worker":"deaf"}', '{}'), /as many as it may/);
  assert.equal(c.pool.running, 4, 'four run; the rest wait for the pool');
  c.stop();
});
