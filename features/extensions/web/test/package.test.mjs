// @fasttrackstudio/architect through its public API under node: the lab's
// counter and the misbehaving component (features/extensions/tests/wasi-hazards),
// built for wasm32-wasip2 (as web.test.mjs finds them), made ready by
// `architect build-web`'s `build`, loaded with `loadExtension` and the
// node `spawn`. Typed runs, deny-by-default capabilities, destructive
// actions and `confirm`, an unhandled request, the time limit.
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { copyFileSync, cpSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { loadExtension, memoryHost, ArchitectError } from '../index.js';
import { spawn } from '../node.js';
import { build } from '../cli/cli.mjs';
import { guests } from './harness.mjs';

const here = fileURLToPath(new URL('.', import.meta.url));
const root = resolve(here, '../../../..');

const dir = guests();
const skip = dir ? false : 'the components are not built (cargo build --target wasm32-wasip2)';

/// Every extension a test loaded: stopped when the file's tests end, so a
/// failing test leaves no worker thread alive (#520).
const loaded = [];
after(() => loaded.forEach((e) => e.stop()));

const built = new Map();
const integrities = new Map();
function ready(name, opts, as = name) {
  if (!built.has(as)) {
    const out = join(root, 'target/wasi-web-package', as);
    const { integrity } = build(join(dir, `${name}.wasm`), out, opts);
    built.set(as, pathToFileURL(`${out}/`).href);
    integrities.set(as, integrity);
  }
  return built.get(as);
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

/// Loads it unpinned unless a test pins it: `integrity: false`, said.
async function load(source, options) {
  const ext = await loadExtension(source, { spawn, integrity: false, ...options });
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
  // Only what the .d.ts says; nothing of its insides.
  assert.deepEqual(Object.keys(ext).sort(), ['actions', 'capabilities', 'manifest', 'views', 'world']);
  for (const k of ['component', 'host', 'frame', 'panes']) assert.equal(ext[k], undefined, k);
  assert.ok(Object.isFrozen(ext.capabilities) && Object.isFrozen(ext.capabilities.actions));
  assert.throws(() => ext.capabilities.actions.push('*'));
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
  const newer = { ...metadata, world: '0.2.0', folder: new URL('.', at).href };
  await assert.rejects(
    loadExtension(newer, { spawn, integrity: false }),
    /built for architect:extension@0\.2\.0, newer than this host's 0\.1\.0: update @fasttrackstudio\/architect/,
  );
  const noWorld = { ...newer, world: undefined };
  await assert.rejects(loadExtension(noWorld, { spawn, integrity: false }), /does not say which architect:extension world/);
});

// P6 (review of #539): nothing tied the files a page loads to what was
// built; a compromised CDN got everything.
test('P6: pinned files load; a changed core module, glue or lock is refused before it runs', { skip }, async () => {
  const good = counterSource();
  const integrity = integrities.get('architect_sandbox_wasi_counter');
  assert.match(integrity, /^sha256-[A-Za-z0-9+/]+=*$/);
  const pinned = await load(good, { host: memoryHost(), integrity });
  assert.equal(await pinned.run('LAB_WASI_COUNTER_ADD', { by: 2 }), 2);
  const evil = join(root, 'target/wasi-web-package/evil-counter');
  const fresh = () => {
    cpSync(fileURLToPath(good), evil, { recursive: true });
    return pathToFileURL(`${evil}/`).href;
  };
  // A core module changed by one byte.
  let at = fresh();
  const core = join(evil, 'architect_sandbox_wasi_counter.core.wasm');
  const bytes = readFileSync(core);
  bytes[bytes.length - 1] ^= 1;
  writeFileSync(core, bytes);
  await assert.rejects(loadExtension(at, { spawn, integrity }), /architect_sandbox_wasi_counter\.core\.wasm is not the file its lock names/);
  // The glue swapped for the attacker's.
  at = fresh();
  writeFileSync(join(evil, 'architect_sandbox_wasi_counter.js'), 'globalThis.PWNED = true; export async function instantiate() {}');
  await assert.rejects(loadExtension(at, { spawn, integrity }), /architect_sandbox_wasi_counter\.js is not the file its lock names/);
  // A lock rewritten to match: not the lock pinned.
  at = fresh();
  writeFileSync(join(evil, 'architect.lock.json'), '{"lockfileVersion":1,"files":{}}\n');
  await assert.rejects(loadExtension(at, { spawn, integrity }), /its lock is not the one pinned/);
});

test('P1/P6: the module is a file inside the extension’s folder, nothing else', { skip }, async () => {
  const at = new URL('architect.json', counterSource());
  const metadata = JSON.parse(readFileSync(at, 'utf8'));
  for (const module of ['../evil.js', 'https://evil.example/glue.js', '/abs/glue.js', 'data:text/javascript,1', 'a/./b.js', '%2e%2e/x.js', 'x.js?y']) {
    await assert.rejects(
      loadExtension({ ...metadata, module, folder: new URL('.', at).href }, { spawn, integrity: false }),
      /is not a file inside its folder/,
      module,
    );
  }
});

test('floods: past its restarts a minute it is not started again', { skip, timeout: 60000 }, async () => {
  const ext = await load(hazardsSource(), { host: memoryHost(), limits: { call: 200, restarts: 2 } });
  for (let i = 0; i < 3; i++) await assert.rejects(ext.run('TEST_HAZARDS_SPIN'), /interrupted/);
  assert.match(ext.report().lastTrap, /it is not started again: 3 restarts in a minute/);
  await assert.rejects(ext.run('TEST_HAZARDS_SPIN'), /it is not running/);
});

test('node exits with an idle extension it did not stop', { skip, timeout: 60000 }, () => {
  const script = `
    import { loadExtension, memoryHost } from ${JSON.stringify(pathToFileURL(join(here, '../index.js')).href)};
    import { spawn } from ${JSON.stringify(pathToFileURL(join(here, '../node.js')).href)};
    const ext = await loadExtension(${JSON.stringify(counterSource())}, { spawn, host: memoryHost(), integrity: false });
    console.log(await ext.run('LAB_WASI_COUNTER_ADD', { by: 1 }));
  `;
  const t = performance.now();
  const file = join(root, 'target/wasi-web-package/exits.mjs');
  writeFileSync(file, script);
  const out = execFileSync(process.execPath, [file], { timeout: 30000 }).toString();
  assert.equal(out.trim(), '1');
  assert.ok(performance.now() - t < 30000, 'it exited by itself');
});

// N2 (second review of #539): a wrong digest was ignored for an object
// source, and leaving integrity out loaded unchecked.
test('N2: every source shape honours every integrity shape, and unpinned must be said', { skip }, async () => {
  const at = new URL('architect.json', counterSource());
  const folder = new URL('.', at).href;
  const metadata = JSON.parse(readFileSync(at, 'utf8'));
  const integrity = integrities.get('architect_sandbox_wasi_counter');
  const wrong = `sha256-${'A'.repeat(43)}=`;
  const lock = JSON.parse(readFileSync(new URL('architect.lock.json', folder), 'utf8'));
  // Left out: refused, whatever the source.
  await assert.rejects(loadExtension(counterSource(), { spawn }), /no integrity: pass the digest build-web printed/);
  await assert.rejects(loadExtension({ ...metadata, folder }, { spawn }), /no integrity/);
  // A wrong digest: refused, whatever the source.
  await assert.rejects(loadExtension(counterSource(), { spawn, integrity: wrong }), /its lock is not the one pinned/);
  await assert.rejects(loadExtension({ ...metadata, folder }, { spawn, integrity: wrong }), /its lock is not the one pinned/);
  // Not a digest, not a lock: refused.
  for (const bad of ['sha256-short', 'md5-x', 42, true, { files: [] }])
    await assert.rejects(loadExtension({ ...metadata, folder }, { spawn, integrity: bad }), /integrity|not a lock|digest/, JSON.stringify(bad));
  // Metadata given as an object that is not what the lock names: refused.
  await assert.rejects(
    loadExtension({ ...metadata, folder, manifest: { ...metadata.manifest, capabilities: { actions: ['*'] } } }, { spawn, integrity }),
    /the metadata given is not the architect.json its lock names/,
  );
  // The right ones: each source, each shape.
  for (const [source, pin] of [
    [counterSource(), integrity],
    [counterSource(), lock],
    [{ ...metadata, folder }, integrity],
    [{ ...metadata, folder }, lock],
  ]) {
    const ext = await load(source, { integrity: pin, host: memoryHost() });
    assert.equal(await ext.run('LAB_WASI_COUNTER_ADD', { by: 1 }), 1);
  }
});

// S1: an app action whose effect the embedder did not say ran as mutating.
test('S1: an action of handle.run’s, its effect unsaid, needs the destructive grant and a yes', { skip }, async () => {
  const ran = [];
  // The counter, asking for every effect on THEME_ (so only the grant and
  // the yes decide).
  const toml = (await import('smol-toml')).parse(readFileSync(join(root, 'apps/sandbox/wasi-counter/extension.toml'), 'utf8'));
  const wide = ready('architect_sandbox_wasi_counter', { manifest: { ...toml, capabilities: { actions: ['THEME_'], effect: 'destructive' } } }, 'counter-wide');
  const host = (confirm) => memoryHost({ handle: { run: (r) => (ran.push(r.action), ['light']) }, confirm });
  // The review's case: a mutating grant, confirm says no.
  const mutating = await load(wide, { host: host(() => false), grant: { actions: ['THEME_'], effect: 'mutating' } });
  await assert.rejects(mutating.run('LAB_WASI_COUNTER_ASK'), /refused: THEME_LIST is destructive/);
  const noYes = await load(wide, { host: host(() => false), grant: { actions: ['THEME_'], effect: 'destructive' } });
  await assert.rejects(noYes.run('LAB_WASI_COUNTER_ASK'), /confirm|confirmation/);
  assert.deepEqual(ran, [], 'it never ran');
  const yes = await load(wide, { host: host(() => true), grant: { actions: ['THEME_'], effect: 'destructive' } });
  assert.deepEqual(await yes.run('LAB_WASI_COUNTER_ASK'), ['light']);
  assert.deepEqual(ran, ['THEME_LIST']);
});

// S4: what its JS does between calls is the page's to end.
test('S4: idle past limits.idle it is stopped, and its next call starts it again', { skip, timeout: 30000 }, async () => {
  const ext = await load(hazardsSource(), { host: memoryHost({ confirm: () => true }), limits: { idle: 300 } });
  assert.deepEqual(await ext.run('TEST_HAZARDS_ECHO', 1), { echo: 1 });
  await new Promise((r) => setTimeout(r, 900));
  assert.equal(ext.report().alive, false, 'stopped while idle');
  assert.deepEqual(await ext.run('TEST_HAZARDS_ECHO', 2), { echo: 2 });
  assert.equal(ext.report().alive, true);
});
