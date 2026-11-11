// @fasttrackstudio/architect through its public API under node: the lab's
// counter and the misbehaving component (features/extensions/tests/wasi-hazards),
// built for wasm32-wasip2 (as web.test.mjs finds them), made ready by
// `architect build-web`'s `build`, loaded with `loadExtension` and the
// node `spawn`. Typed runs, deny-by-default capabilities, destructive
// actions and `confirm`, an unhandled request, the time limit.
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, mkdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { loadExtension, memoryHost, ArchitectError } from '../index.js';
import { spawn } from '../node.js';
import { build } from '../cli.mjs';

const here = fileURLToPath(new URL('.', import.meta.url));
const root = resolve(here, '../../../..');

function guests() {
  for (const dir of [
    process.env.ARCHITECT_WASI_GUESTS,
    join(root, 'target/wasi-guest/wasm32-wasip2/debug'),
    join(root, 'target/wasm32-wasip2/debug'),
  ])
    if (dir && existsSync(join(dir, 'architect_sandbox_wasi_counter.wasm'))) return dir;
  return null;
}

const dir = guests();
const skip = dir ? false : 'the components are not built (cargo build --target wasm32-wasip2)';

/// Every extension a test loaded: stopped when the file's tests end, so a
/// failing test leaves no worker thread alive (#520).
const loaded = [];
after(() => loaded.forEach((e) => e.stop()));

const built = new Map();
function ready(name, opts) {
  if (!built.has(name)) {
    const out = join(root, 'target/wasi-web-package', name);
    mkdirSync(out, { recursive: true });
    build(join(dir, `${name}.wasm`), out, opts);
    built.set(name, pathToFileURL(`${out}/`).href);
  }
  return built.get(name);
}

const counterSource = () =>
  ready('architect_sandbox_wasi_counter', { manifestPath: join(root, 'apps/sandbox/wasi-counter/extension.toml') });

/// The hazards component has no manifest of its own: this one, ECHO
/// destructive so `confirm` has something to say no to.
const HAZARDS = {
  id: 'test.hazards',
  name: 'Hazards',
  version: '0.1.0',
  kind: 'wasm',
  api: '0.1',
  capabilities: { ui: ['notify', 'clipboard'], reads: ['documents'], writes: ['documents'] },
  actions: [
    { id: 'TEST_HAZARDS_SPIN', name: 'Spin', effect: 'read_only' },
    { id: 'TEST_HAZARDS_ECHO', name: 'Echo', effect: 'destructive' },
    { id: 'TEST_HAZARDS_ASK', name: 'Ask', effect: 'read_only' },
    { id: 'TEST_HAZARDS_DOCS', name: 'Docs', effect: 'mutating', params: '{"type":"object","properties":{"path":{"type":"string"}}}' },
  ],
};
const hazardsSource = () => ready('architect_extension_wasi_hazards', { manifest: HAZARDS });

async function load(source, options) {
  const ext = await loadExtension(source, { spawn, ...options });
  loaded.push(ext);
  return ext;
}

test('load the counter and run an action with typed arguments', { skip }, async () => {
  /** @type {import('../index.js').Extension<import('../../../../target/wasi-web-package/architect_sandbox_wasi_counter/actions.js').Actions>} */
  const ext = await load(counterSource(), { host: memoryHost() });
  assert.equal(ext.manifest.id, 'lab.wasi-counter');
  assert.equal(ext.world, '0.1.0');
  assert.deepEqual(ext.views.map((v) => v.id), ['counter', 'bars']);
  const add = ext.actions.find((a) => a.id === 'LAB_WASI_COUNTER_ADD');
  assert.deepEqual(add.schema, { type: 'object', properties: { by: { type: 'integer' } } });
  assert.equal(add.effect, 'mutating');
  assert.equal(await ext.run('LAB_WASI_COUNTER_ADD', { by: 5 }), 5);
  assert.equal(await ext.run('LAB_WASI_COUNTER_ADD'), 6);
  const { bind } = await import(new URL('actions.js', counterSource()).href);
  assert.equal(await bind(ext).add({ by: 4 }), 10);
});

test('a capability the embedder did not grant is refused, as the native host refuses it', { skip }, async () => {
  const host = memoryHost({ actions: { THEME_LIST: { effect: 'read_only', run: () => ['light', 'dark'] } } });
  const denied = await load(counterSource(), { host });
  assert.deepEqual(denied.capabilities.actions, [], 'nothing granted: nothing reached');
  await assert.rejects(denied.run('LAB_WASI_COUNTER_ASK'), (e) => {
    assert.ok(e instanceof ArchitectError);
    assert.match(e.message, /refused: THEME_LIST is not in a namespace this extension may run/);
    return true;
  });
  const granted = await load(counterSource(), { host, grant: { actions: ['THEME_'] } });
  assert.deepEqual(await granted.run('LAB_WASI_COUNTER_ASK'), ['light', 'dark']);
  // A grant wider than it asked for is not more than it asked for.
  const wide = await load(counterSource(), { host, grant: { actions: ['*'], effect: 'destructive' } });
  assert.deepEqual(wide.capabilities.actions, ['THEME_LIST']);
  assert.equal(wide.capabilities.effect, 'read_only');
});

test('a destructive action does not run unless confirm says yes', { skip }, async () => {
  const none = await load(hazardsSource(), { host: memoryHost() });
  await assert.rejects(none.run('TEST_HAZARDS_ECHO', 1), /refused \(confirm\): TEST_HAZARDS_ECHO is destructive/);
  const asked = [];
  const no = await load(hazardsSource(), { host: memoryHost({ confirm: (a) => (asked.push(a.id), false) }) });
  await assert.rejects(no.run('TEST_HAZARDS_ECHO', 2), /refused \(confirm\)/);
  assert.deepEqual(asked, ['TEST_HAZARDS_ECHO']);
  const yes = await load(hazardsSource(), { host: memoryHost({ confirm: () => true }) });
  assert.deepEqual(await yes.run('TEST_HAZARDS_ECHO', 3), { echo: 3 });
});

test('a request the host has no handler for answers "not available in this host"; the component runs on', { skip }, async () => {
  const logs = [];
  const host = memoryHost({ log: (l) => logs.push(l) });
  const ext = await load(hazardsSource(), { host, grant: { ui: ['notify', 'clipboard'] } });
  const t = performance.now();
  // The guest words it by its kind; the host's log says what.
  await assert.rejects(ext.run('TEST_HAZARDS_ASK'), /^ArchitectError: unsupported$|unsupported/);
  assert.ok(
    logs.some((l) => l.source === 'host' && l.text === 'clipboard_write: unsupported here: clipboard_write is not available in this host'),
    JSON.stringify(logs),
  );
  assert.ok(performance.now() - t < 3000, 'answered, not hung');
  assert.deepEqual(host.notifications, [{ text: 'hello from WASI', kind: 'success', title: null }]);
  assert.equal(ext.report().traps, 0, 'an error it can show, not a trap');
  // Documents in memory, as granted.
  const docs = await load(hazardsSource(), { host, grant: { reads: ['documents'], writes: ['documents'] } });
  const out = await docs.run('TEST_HAZARDS_DOCS', { path: 'notes/a.md' });
  assert.equal(out.text, 'one two');
  assert.equal(out.read, 'one two');
  assert.deepEqual(out.listed, [{ path: 'notes/a.md', name: 'a.md', space: 'notes' }]);
  assert.equal(host.documents.get('notes/a.md'), 'one two');
  // Without the grant, the same call is refused before it reaches the host.
  const bare = await load(hazardsSource(), { host });
  await assert.rejects(bare.run('TEST_HAZARDS_DOCS', { path: 'notes/b.md' }), /refused: this extension may not change the app's documents/);
  assert.ok(!host.documents.has('notes/b.md'));
});

test('a component past its time limit is stopped, starts again, and can be loaded again', { skip }, async () => {
  const ext = await load(hazardsSource(), { host: memoryHost({ confirm: () => true }), limits: { call: 300 } });
  const t = performance.now();
  await assert.rejects(ext.run('TEST_HAZARDS_SPIN'), /interrupted: it ran for \d+ ms, past its limit of 300 ms/);
  assert.ok(performance.now() - t < 3000);
  assert.equal(ext.report().traps, 1);
  assert.equal(ext.report().generation, 2, 'started again');
  assert.deepEqual(await ext.run('TEST_HAZARDS_ECHO', 1), { echo: 1 });
  ext.stop();
  await assert.rejects(ext.run('TEST_HAZARDS_ECHO', 1), /not loaded/);
  const again = await load(hazardsSource(), { host: memoryHost({ confirm: () => true }), limits: { call: 300 } });
  assert.deepEqual(await again.run('TEST_HAZARDS_ECHO', 2), { echo: 2 });
});

test('a component built for a newer world is refused, with why', { skip }, async () => {
  const at = new URL('architect.json', counterSource());
  const metadata = JSON.parse(await (await import('node:fs/promises')).readFile(at, 'utf8'));
  const newer = { ...metadata, world: '0.2.0', module: new URL(metadata.module, at).href };
  await assert.rejects(
    loadExtension(newer, { spawn }),
    /built for architect:extension@0\.2\.0, newer than this host's 0\.1\.0: update @fasttrackstudio\/architect/,
  );
  const noWorld = { ...newer, world: undefined };
  await assert.rejects(loadExtension(noWorld, { spawn }), /does not say which architect:extension world/);
});
