// The third review of #539's items that need the built components (SF2,
// SF4, D4): they run with the other component tests (test:package,
// features/extensions/tests/wasi_web.rs). Each failed on 769b4aa3.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { guests } from './harness.mjs';

const dir = guests();
const skip = dir ? false : 'ALLOW_SKIP=1: the components are not built';

// SF2: the idle watchdog counted the extension's own timers and workers as
// use, so a guest could keep itself alive forever.
test('SF2: only the embedder’s calls keep it from idling: not its workers, not its timers', { skip }, async () => {
  const { join } = await import('node:path');
  const { pathToFileURL, fileURLToPath } = await import('node:url');
  const { build } = await import('../cli/cli.mjs');
  const { Component } = await import('../runner.js');
  const { spawn } = await import('../node.js');
  const root = fileURLToPath(new URL('../../../..', import.meta.url));
  const out = join(root, 'target/wasi-web-package/sf2-hazards');
  build(join(dir, 'architect_extension_wasi_hazards.wasm'), out, {
    manifest: { id: 'test.hazards', name: 'Hazards', version: '0.1.0', api: '0.1', actions: [{ id: 'TEST_HAZARDS_WORK', name: 'Work' }] },
  });
  const c = new Component({
    url: pathToFileURL(join(out, 'architect_extension_wasi_hazards.js')).href,
    instance: '',
    spawn,
    host: async () => JSON.stringify({ error: { kind: 'unsupported', what: 'no' } }),
    limits: { idle: 300 },
    workers: { forever: { entry: 'forever' } },
  });
  try {
    await c.start();
    // A worker of its own that never ends.
    await c.invoke('TEST_HAZARDS_WORK', '{"worker":"forever"}', '{}');
    // Its view's timer, mounted and closed again: no view now.
    await c.render(1, 'ticker');
    await c.close(1);
    await new Promise((r) => setTimeout(r, 1200));
    assert.equal(c.report().alive, false, 'idle: stopped, its worker with it');
    assert.equal(c.report().workers.length, 0);
  } finally {
    c.stop();
  }
});

// SF4: the module cache was never emptied.
test('SF4: a component’s cache is emptied when it stops', { skip }, async () => {
  const { Component } = await import('../runner.js');
  const { join } = await import('node:path');
  const { pathToFileURL, fileURLToPath } = await import('node:url');
  const { build } = await import('../cli/cli.mjs');
  const { spawn } = await import('../node.js');
  const root = fileURLToPath(new URL('../../../..', import.meta.url));
  const out = join(root, 'target/wasi-web-package/sf4-counter');
  build(join(dir, 'architect_sandbox_wasi_counter.wasm'), out, { manifestPath: join(root, 'apps/sandbox/wasi-counter/extension.toml') });
  const c = new Component({ url: pathToFileURL(join(out, 'architect_sandbox_wasi_counter.js')).href, instance: '', spawn, host: async () => '{}' });
  await c.start();
  assert.ok(c.cache.size > 0, 'its modules, cached');
  c.stop();
  assert.equal(c.cache.size, 0, 'emptied');
});

// D4: integrity: false warned only into an embedder's log.
test('D4: integrity: false is said on the console when the embedder gave no log', { skip }, async () => {
  const { join } = await import('node:path');
  const { pathToFileURL, fileURLToPath } = await import('node:url');
  const { build } = await import('../cli/cli.mjs');
  const { loadExtension } = await import('../index.js');
  const { spawn } = await import('../node.js');
  const root = fileURLToPath(new URL('../../../..', import.meta.url));
  const out = join(root, 'target/wasi-web-package/d4-counter');
  build(join(dir, 'architect_sandbox_wasi_counter.wasm'), out, { manifestPath: join(root, 'apps/sandbox/wasi-counter/extension.toml') });
  const warned = [];
  const warn = console.warn;
  console.warn = (...a) => warned.push(a.join(' '));
  try {
    const ext = await loadExtension(pathToFileURL(`${out}/`).href, { spawn, integrity: false });
    ext.stop();
  } finally {
    console.warn = warn;
  }
  assert.ok(warned.some((w) => /loaded with integrity: false, its files unchecked/.test(w)), JSON.stringify(warned));
});

