// The runner against what a hostile extension's JS sends the page
// (second review of #539): module names (M1), core modules built to make
// a parser loop (N3), log lines passing for the host's (S2), oversized
// messages and data (S4), prototype worker names (S5).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { besideModule, Component, limitMemory, LIMITS } from '../runner.js';
import { limitMemory as inWorker } from '../instance.js';
import { answerer } from '../host.js';
import { within } from '../check.js';

const GLUE = 'https://app.example/ext/counter/glue.js';

test('M1: a module the extension asks for is a file beside its glue, nothing else', () => {
  assert.equal(besideModule(GLUE, 'glue.core.wasm'), 'https://app.example/ext/counter/glue.core.wasm');
  assert.equal(besideModule(GLUE, 'workers/a.core.wasm'), 'https://app.example/ext/counter/workers/a.core.wasm');
  for (const name of [
    '/private/secret.txt',
    '../../private/secret.txt',
    'a/../../secret.txt',
    './glue.core.wasm',
    'https://app.example/private/secret.txt',
    '//app.example/private/secret.txt',
    'file:///etc/passwd',
    'data:application/wasm;base64,AA',
    'blob:https://app.example/1',
    '%2e%2e/secret.txt',
    'a\\..\\secret.txt',
    'x?y',
    'x#y',
    '',
    'a//b',
  ])
    assert.equal(besideModule(GLUE, name), null, name);
});

/// (module (memory 1)): 13 bytes.
const ONE = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 5, 3, 1, 0, 1]);
/// The review's 14 bytes: a memory section claiming 16M memories.
const SIXTEEN_M = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 5, 4, 0x80, 0x80, 0x80, 0x08]);
/// One claiming 2^32 - 1.
const FOUR_G = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 5, 6, 0xff, 0xff, 0xff, 0xff, 0x0f, 0]);

test('N3: a core module whose counts lie is refused at once, by both copies of the parser', () => {
  for (const f of [limitMemory, inWorker]) {
    assert.deepEqual([...f(ONE, 4).subarray(8)], [5, 4, 1, 1, 1, 4]);
    for (const [bytes, why] of [
      [SIXTEEN_M, /more memories than its section holds/],
      [FOUR_G, /more memories than its section holds/],
      [new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 5, 0xff, 0xff, 0xff, 0xff, 0x0f]), /runs past|section/],
      [new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 5, 9, 1, 0, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0]), /more than 5 bytes|past/],
      [new Uint8Array([1, 2, 3]), /not a wasm module/],
      [new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 5, 3, 1, 1]), /runs past/],
    ]) {
      const t = performance.now();
      assert.throws(() => f(bytes, 4), why);
      assert.ok(performance.now() - t < 20, `${f === inWorker ? 'worker' : 'runner'}: ${performance.now() - t} ms`);
    }
  }
  // A big real-looking one: linear time.
  const body = [0xe8, 0x07]; // 1000 memories, 4002 bytes
  for (let i = 0; i < 1000; i++) body.push(1, 0, 1, 0);
  const mod = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 5, 0xa2, 0x1f, ...body]);
  const t = performance.now();
  for (const f of [limitMemory, inWorker]) assert.ok(f(mod, 4) instanceof Uint8Array);
  assert.ok(performance.now() - t < 50);
});

function component(opts = {}) {
  const lines = [];
  const c = new Component({ url: GLUE, instance: '', spawn: () => ({ postMessage() {}, terminate() {} }), host: async () => '{}', log: (...l) => lines.push(l), ...opts });
  c.limits = { ...LIMITS, ...opts.limits };
  return { c, lines };
}

test('S2: its lines are at most logLines a second, their level one of four, their source never the host', () => {
  const { c, lines } = component();
  c.ownerNote(null, { t: 'log', level: 'evil', source: 'host', text: 'refused (actions): fake' });
  c.ownerNote(null, { t: 'log', level: 'warn', source: 'stderr', text: 'ok' });
  assert.deepEqual(lines, [
    ['info', 'log', 'refused (actions): fake'],
    ['warn', 'stderr', 'ok'],
  ]);
  for (let i = 0; i < 3000; i++) c.ownerNote(null, { t: 'log', level: 'info', source: 'log', text: `line ${i}` });
  assert.equal(lines.length, LIMITS.logLines, 'the rest dropped');
});

test('S2: the host-service log takes a known level only', async () => {
  const lines = [];
  const ask = answerer({ manifest: { id: 'x' }, may: within({}, {}), log: (...l) => lines.push(l) });
  await ask(JSON.stringify({ op: 'log', text: 'hi', level: '<script>' }));
  assert.deepEqual(lines, [['info', 'log', 'hi']]);
});

test('S4: an oversized host request, worker message or data value is refused', async () => {
  const { c } = component({ limits: { messageBytes: 1000 } });
  const big = 'x'.repeat(2000);
  const a = JSON.parse(await c.ownerAsks({ op: 'call', request: big }));
  assert.match(a.error.message, /a host request past 1000 bytes/);
  assert.match(c.toWorker(1, big).err, /a message past 1000 bytes/);
  const ask = answerer({ manifest: { id: 'x' }, may: within({}, {}), log: () => {}, limits: { dataBytes: 100, dataTotal: 250 } });
  const set = async (key, value) => JSON.parse(await ask(JSON.stringify({ op: 'set_data', key, value })));
  assert.match((await set('a', 'y'.repeat(200))).error.message, /past its limit of 100/);
  assert.deepEqual(await set('a', 'y'.repeat(90)), { ok: null });
  assert.deepEqual(await set('b', 'y'.repeat(90)), { ok: null });
  assert.match((await set('c', 'y'.repeat(90))).error.message, /would be \d+ bytes: past its limit of 250/);
  assert.deepEqual(await set('a', null), { ok: null }, 'freed');
  assert.deepEqual(await set('c', 'y'.repeat(90)), { ok: null });
});

test('S5: a worker name is one its manifest declares, not a prototype key', () => {
  const { c } = component({ workers: { primes: { entry: 'count-primes' } } });
  for (const name of ['constructor', '__proto__', 'toString', 'hasOwnProperty'])
    assert.match(c.spawn(name).err, /has no worker/, name);
});
