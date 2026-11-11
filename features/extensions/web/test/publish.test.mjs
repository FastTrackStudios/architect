// What the third review of #539 found before the first npm publish
// (MP1, D1–D4, SF1–SF6, notes). Each test failed on 769b4aa3.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { answerer } from '../host.js';
import { within } from '../check.js';

// MP1: the data cap was measured by String(key) but stored by the raw key:
// a number key subtracted a stored size and deleted nothing.
test('MP1: the data cap holds whatever type a key is', async () => {
  const MiB = 1 << 20;
  const ask = answerer({ manifest: { id: 'x' }, may: within({}, {}), log: () => {}, limits: { dataBytes: MiB + 16, dataTotal: 4 * MiB } });
  const a = async (r) => JSON.parse(await ask(JSON.stringify(r)));
  const big = 'y'.repeat(MiB - 8);
  for (let n = 0; n < 64; n++) {
    await a({ op: 'set_data', key: String(n), value: big });
    await a({ op: 'set_data', key: n, value: null });
  }
  // What is really stored, recomputed from what reads back.
  let total = 0;
  for (let n = 0; n < 64; n++) {
    const v = (await a({ op: 'data', key: String(n) })).ok;
    if (v !== null) total += JSON.stringify(v).length;
  }
  assert.ok(total <= 4 * MiB, `${(total / MiB).toFixed(1)} MiB stored under a 4 MiB cap`);
  assert.equal((await a({ op: 'set_data', key: 7, value: 1 })).error?.message, 'not a host request: set_data.key is not a string');
  assert.equal((await a({ op: 'data', key: { toString: 1 } })).error?.kind, 'failed');
  assert.equal((await a({ op: 'setting', key: ['x.a'] })).error?.kind, 'failed');
});

// SF6: without the components, the 13 component tests SKIPPED silently.
test('SF6: the component tests fail loudly when the components are not there, unless ALLOW_SKIP=1', async () => {
  const { spawnSync } = await import('node:child_process');
  const { fileURLToPath } = await import('node:url');
  const here = fileURLToPath(new URL('..', import.meta.url));
  const run = (env) => {
    const e = { ...process.env, ARCHITECT_WASI_GUESTS: '/nonexistent', ...env };
    delete e.NODE_TEST_CONTEXT; // a run of its own, not a subtest of this one
    return spawnSync(process.execPath, ['--test', '--test-reporter=tap', 'test/package.test.mjs'], { cwd: here, env: e, encoding: 'utf8', timeout: 60000 });
  };
  const loud = run({ ALLOW_SKIP: '' });
  assert.notEqual(loud.status, 0, 'it fails');
  assert.match(loud.stdout + loud.stderr, /the WASI components these tests run are not built/);
  const meant = run({ ALLOW_SKIP: '1' });
  assert.equal(meant.status, 0);
  assert.match(meant.stdout, /# skip(ped)? 13/);
});

// SF3: 20 never-closed 8 MiB streams held 180 MiB in the page; a timeout
// in the body came back as `{}`.
test('SF3: fetchHandler bounds open streams and their bytes, keeps them to their extension, words a timeout', async () => {
  const { fetchHandler } = await import('../host.js');
  const MiB = 1 << 20;
  const body = (n) => new Response(new Uint8Array(n), { status: 200 });
  const f = fetchHandler({ allow: ['api.example.com'], fetch: async () => body(8 * MiB) });
  const ctx = { extension: 'evil' };
  let opened = 0;
  for (let i = 0; i < 20; i++) {
    try {
      await f.fetch({ url: 'https://api.example.com/x', stream: true }, ctx);
      opened++;
    } catch (e) {
      assert.match(e.message, /streams open|would hold more than/);
    }
  }
  assert.ok(opened * 8 <= 32, `${opened} streams of 8 MiB held`);
  assert.throws(() => f.read_body({ stream: 's1' }, { extension: 'other' }), (e) => e.kind === 'not_found', 'not its stream');
  assert.equal(f.close_body({ stream: 's1' }, { extension: 'other' }), false, 'not its stream');
  assert.equal(f.close_body({ stream: 's1' }, ctx), true);
  // A body slower than the time it has: a message, not {}.
  const slow = fetchHandler({
    allow: ['api.example.com'],
    timeoutMs: 100,
    fetch: async (_, init) =>
      new Response(
        new ReadableStream({
          pull: () => new Promise((_, no) => init.signal.addEventListener('abort', () => no(init.signal.reason))),
        }),
      ),
  });
  await assert.rejects(slow.fetch({ url: 'https://api.example.com/slow' }, ctx), (e) => /took longer than 100 ms/.test(e.message));
});

// SF4: moduleBytes was checked only after the whole file was read, and
// the module cache was never emptied.
test('SF4: a module read stops at its cap', async () => {
  const { createServer } = await import('node:http');
  const { fetchBytes } = await import('../runner.js');
  // An endless "module".
  const server = createServer((_, res) => {
    res.writeHead(200, { 'content-type': 'application/wasm' });
    const chunk = Buffer.alloc(64 << 10);
    const more = () => {
      while (res.write(chunk));
    };
    res.on('drain', more);
    more();
  });
  await new Promise((ok) => server.listen(0, '127.0.0.1', ok));
  try {
    const t = performance.now();
    await assert.rejects(fetchBytes(`http://127.0.0.1:${server.address().port}/endless.wasm`, 1 << 20), /past 1048576 bytes/);
    assert.ok(performance.now() - t < 5000, 'it stopped reading at the cap');
  } finally {
    server.closeAllConnections();
    server.close();
  }
});

// D1: a memory64 memory was passed through unlimited even on the honest path.
test('D1: a memory64 module is refused by both copies of the parser', async () => {
  const { limitMemory } = await import('../runner.js');
  const { limitMemory: inWorker } = await import('../instance.js');
  // (module (memory i64 1)): flags 0x04.
  const m64 = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 5, 3, 1, 0x04, 1]);
  for (const f of [limitMemory, inWorker]) assert.throws(() => f(m64, 4), /memory64 module: its memory cannot be limited/);
});

// Notes: a glue at a site's root makes the whole origin its folder.
test('a site’s root is no extension’s folder: the runtime refuses it, build-web refuses to write there', async () => {
  const { besideModule } = await import('../runner.js');
  assert.equal(besideModule('https://app.example/glue.js', 'glue.core.wasm'), null);
  assert.equal(besideModule('https://app.example/ext/glue.js', 'glue.core.wasm'), 'https://app.example/ext/glue.core.wasm');
  const { loadExtension } = await import('../index.js');
  const meta = { module: 'glue.js', world: '0.1.0', manifest: { id: 'x', name: 'X', version: '0.1.0' }, folder: 'https://app.example/' };
  await assert.rejects(loadExtension(meta, { integrity: false }), /its folder is the site’s root/);
  const { build } = await import('../cli/cli.mjs');
  const { mkdtempSync, rmSync } = await import('node:fs');
  const { tmpdir } = await import('node:os');
  const { join } = await import('node:path');
  const dir = mkdtempSync(join(tmpdir(), 'architect-root-'));
  try {
    assert.throws(() => build('/nonexistent.wasm', join(dir, 'public'), { manifest: meta.manifest }), /looks like a site's root/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
