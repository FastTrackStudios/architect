// check.js against the native host's decisions: the cases of
// features/extensions/src/capabilities.rs's and hostcall.rs's tests, the
// same capability names and words. And world.js against extension.wit.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { actionPrefix, capabilities, check, mayRun, noCapabilities, within } from '../check.js';
import { answerer, memoryHost } from '../host.js';
import { actionPrefix as cliPrefix } from '../cli/manifest.mjs';
import { newer, refusal, WORLD, worldsIn } from '../world.js';

const caps = (actions, effect, reads) => capabilities({ actions, effect, reads });

test('what it may do is the narrower of ask and grant (capabilities.rs)', () => {
  const asks = caps(['*'], 'destructive', ['org', 'selection']);
  const granted = caps(['TRANSPORT_'], 'mutating', ['selection']);
  const may = within(asks, granted);
  assert.deepEqual(may.actions, ['TRANSPORT_']);
  assert.equal(may.effect, 'mutating');
  assert.deepEqual(may.reads, ['selection']);
  assert.equal(mayRun(may, 'FTS_M_', 'FTS_M_GO', 'destructive'), null, 'its own');
  assert.equal(mayRun(may, 'FTS_M_', 'TRANSPORT_PLAY', 'mutating'), null);
  assert.deepEqual(mayRun(may, 'FTS_M_', 'WORKSPACE_CLOSE', 'read_only'), [
    'actions',
    'WORKSPACE_CLOSE is not in a namespace this extension may run',
  ]);
  assert.deepEqual(mayRun(may, 'FTS_M_', 'TRANSPORT_WIPE', 'destructive'), [
    'effect',
    'TRANSPORT_WIPE is destructive: this extension may run mutating actions at most',
  ]);
  assert.deepEqual(within(asks, noCapabilities()), noCapabilities(), 'nothing granted');
});

test('the interface it uses is granted one by one', () => {
  const may = within({ ui: ['notify', 'clipboard'] }, { ui: ['notify'] });
  const manifest = { id: 'fts.m' };
  assert.equal(check(manifest, may, { op: 'notify', text: 'x' }), null);
  assert.deepEqual(check(manifest, may, { op: 'clipboard_read' }), [
    'ui.clipboard',
    "this extension may not use the app's clipboard (capabilities.ui)",
  ]);
});

test('each request is checked as hostcall::check checks it', () => {
  const manifest = {
    id: 'fts.net',
    capabilities: { hosts: ['api.example.com'] },
    secrets: [{ id: 'TOKEN', hosts: ['api.example.com'] }],
  };
  const may = within(
    { reads: ['documents'], hosts: ['api.example.com', '*.cdn.net'], events: ['transport.*'], ports: ['midi:IAC*', 'osc:127.0.0.1:9000'] },
    { reads: ['documents'], hosts: ['*'], events: ['transport.*'], ports: ['midi:IAC*', 'osc:127.0.0.1:9000'] },
  );
  assert.equal(check(manifest, may, { op: 'read_document', path: 'a.md' }), null);
  assert.deepEqual(check(manifest, may, { op: 'create_document', path: 'a.md' }), [
    'writes.documents',
    "this extension may not change the app's documents (capabilities.writes)",
  ]);
  assert.deepEqual(check(manifest, may, { op: 'org' }), ['reads.org', "this extension may not read the app's org (capabilities.reads)"]);
  assert.equal(check(manifest, may, { op: 'setting', key: 'fts.net.mode' }), null);
  assert.deepEqual(check(manifest, may, { op: 'setting', key: 'other.mode' }), [
    'settings',
    'other.mode is not one of its settings: an extension reads and writes only its own (fts.net.…)',
  ]);
  assert.equal(check(manifest, may, { op: 'fetch', url: 'https://api.example.com/x', headers: { auth: '{{secret:TOKEN}}' } }), null);
  assert.equal(check(manifest, may, { op: 'fetch', url: 'https://a.cdn.net/x' }), null);
  assert.deepEqual(check(manifest, may, { op: 'fetch', url: 'https://evil.com/x' }), [
    'hosts',
    'this extension may not reach evil.com (capabilities.hosts)',
  ]);
  assert.deepEqual(check(manifest, may, { op: 'fetch', url: 'file:///etc/passwd' }), ['hosts', 'file:///etc/passwd: only http and https are fetched']);
  assert.deepEqual(check(manifest, may, { op: 'fetch', url: 'https://api.example.com/{{secret:TOKEN}}' }), [
    'secrets',
    "a secret goes in a header's value, not the URL or the body",
  ]);
  assert.deepEqual(check(manifest, may, { op: 'fetch', url: 'https://api.example.com/', headers: { a: '{{secret:NOPE}}' } }), [
    'secrets',
    'NOPE is not one of its secrets ([[secrets]])',
  ]);
  assert.equal(check(manifest, may, { op: 'subscribe', on: 'app:transport.changed' }), null);
  assert.deepEqual(check(manifest, may, { op: 'subscribe', on: 'app:mixer.changed' }), [
    'events.mixer.changed',
    "this extension may not hear the app's mixer.changed events (capabilities.events)",
  ]);
  assert.equal(check(manifest, may, { op: 'open_port', kind: 'midi', name: 'IAC Bus 1' }), null);
  assert.equal(check(manifest, may, { op: 'open_port', kind: 'osc', name: '127.0.0.1:9000' }), null);
  assert.deepEqual(check(manifest, may, { op: 'open_port', kind: 'osc', name: '127.0.0.1:9000', listen: 9001 }), [
    'ports.osc:127.0.0.1:9000',
    'this extension may not open osc port 127.0.0.1:9000, hearing on 9001, (capabilities.ports)',
  ]);
  assert.equal(check(manifest, may, { op: 'data', key: 'k' }), null, 'its own data');
  assert.equal(actionPrefix('lab.wasi-counter'), 'LAB_WASI_COUNTER_');
});

test('the answerer: unknown op, unknown app action, refusal, an unhandled op, a throwing handler', async () => {
  const lines = [];
  const manifest = { id: 'fts.x', sections: [{ settings: [{ key: 'fts.x.mode', default: 'a' }] }] };
  const may = within({ actions: ['APP_'], ui: ['notify', 'menus'] }, { actions: ['APP_'], ui: ['notify', 'menus'] });
  const ask = answerer({
    manifest,
    may,
    host: {
      actions: { APP_GO: { effect: 'read_only', run: (args) => ({ went: args }) }, APP_WIPE: { effect: 'destructive', run: () => 'wiped' } },
      handle: { menu: () => { throw new Error('boom'); } },
    },
    log: (level, source, text) => lines.push({ level, source, text }),
  });
  const a = async (r) => JSON.parse(await ask(JSON.stringify(r)));
  assert.deepEqual(await a({ op: 'nope' }), { error: { kind: 'failed', message: 'not a host request: "nope"' } });
  assert.deepEqual(JSON.parse(await ask('{')).error.kind, 'failed');
  assert.deepEqual(await a({ op: 'run', action: 'APP_GO', args: 1 }), { ok: { went: 1 } });
  assert.deepEqual(await a({ op: 'run', action: 'APP_NONE' }), { error: { kind: 'not_found', what: 'action APP_NONE' } });
  assert.deepEqual(await a({ op: 'run', action: 'APP_WIPE' }), {
    error: { kind: 'refused', capability: 'effect', why: 'APP_WIPE is destructive: this extension may run read_only actions at most' },
  });
  assert.deepEqual(await a({ op: 'notify', text: 'hi' }), { error: { kind: 'unsupported', what: 'notify is not available in this host' } });
  assert.deepEqual(await a({ op: 'menu', items: [] }), { error: { kind: 'failed', message: 'boom' } });
  assert.deepEqual(await a({ op: 'setting', key: 'fts.x.mode' }), { ok: 'a' });
  assert.deepEqual(await a({ op: 'set_setting', key: 'fts.x.mode', value: 'b' }), { ok: null });
  assert.deepEqual(await a({ op: 'setting', key: 'fts.x.mode' }), { ok: 'b' });
  assert.deepEqual(await a({ op: 'set_setting', key: 'fts.x.other', value: 1 }), { error: { kind: 'not_found', what: 'setting fts.x.other in its manifest' } });
  assert.deepEqual(await a({ op: 'set_data', key: 'k', value: [1] }), { ok: null });
  assert.deepEqual(await a({ op: 'data', key: 'k' }), { ok: [1] });
  assert.ok(lines.some((l) => l.text === 'menu: boom'));
});

test("the world's version is extension.wit's, and a newer one is refused", () => {
  const wit = readFileSync(new URL('../../wit/extension.wit', import.meta.url), 'utf8');
  assert.equal(/^package architect:extension@([\d.]+);/m.exec(wit)[1], WORLD);
  const manifestRs = readFileSync(new URL('../../src/manifest.rs', import.meta.url), 'utf8');
  assert.match(manifestRs, /pub const API: &str = "0\.1";/);
  assert.ok(newer('0.2.0', '0.1.0') && newer('0.1.1', '0.1.0') && newer('1.0.0', '0.9.9'));
  assert.ok(!newer('0.1.0', '0.1.0') && !newer('0.0.9', '0.1.0'));
  assert.equal(refusal('0.1.0', '0.1', 'x'), null);
  assert.match(refusal('0.2.0', '0.1', 'x'), /built for architect:extension@0\.2\.0, newer than this host's 0\.1\.0/);
  assert.match(refusal('0.1.0', '0.2', 'x'), /written for extension API 0\.2, newer than this host's 0\.1/);
  assert.deepEqual(worldsIn("'architect:extension/guest@0.1.0#init' 'architect:extension/host@0.1.0'"), ['0.1.0']);
});

test('an action the embedder offers is never the extension’s own, whatever its name', async () => {
  // The review of #539: an extension `vault`, granted nothing, ran the
  // page's mutating VAULT_EMPTY_TRASH.
  const ran = [];
  const ask = answerer({
    manifest: { id: 'vault' },
    may: within({}, {}),
    host: { actions: { VAULT_EMPTY_TRASH: { effect: 'mutating', run: () => ran.push('emptied') } } },
    log: () => {},
  });
  const a = JSON.parse(await ask(JSON.stringify({ op: 'run', action: 'VAULT_EMPTY_TRASH' })));
  assert.deepEqual(a, { error: { kind: 'refused', capability: 'actions', why: 'VAULT_EMPTY_TRASH is not in a namespace this extension may run' } });
  assert.deepEqual(ran, []);
});

test('a destructive app action needs confirm, even through handle.run', async () => {
  const ran = [];
  const make = (confirm) =>
    answerer({
      manifest: { id: 'x' },
      may: within({ actions: ['APP_'], effect: 'destructive' }, { actions: ['APP_'], effect: 'destructive' }),
      host: {
        actions: { APP_WIPE: { effect: 'destructive', run: () => 'unused' } },
        handle: { run: (r) => ran.push(r.action) },
        confirm,
      },
      log: () => {},
    });
  const req = JSON.stringify({ op: 'run', action: 'APP_WIPE' });
  assert.equal(JSON.parse(await make(undefined)(req)).error.kind, 'confirmation_required');
  assert.equal(JSON.parse(await make(() => 'yes')(req)).error.kind, 'confirmation_required', 'only true is a yes');
  assert.deepEqual(ran, []);
  assert.deepEqual(JSON.parse(await make(() => true)(req)), { ok: 1 });
  assert.deepEqual(ran, ['APP_WIPE']);
});

test('handlers see the capabilities; memoryHost keeps files to the granted folders', async () => {
  let seen;
  const may = within({ folders: ['/data'], reads: ['org'] }, { folders: ['/data'], reads: ['org'] });
  const host = memoryHost({ files: { '/data/a.txt': 'A', '/etc/passwd': 'root' }, handle: { org: (_, ctx) => ((seen = ctx), 'o') } });
  const ask = answerer({ manifest: { id: 'x' }, may, host, log: () => {} });
  const a = async (r) => JSON.parse(await ask(JSON.stringify(r)));
  assert.deepEqual(await a({ op: 'org' }), { ok: 'o' });
  assert.deepEqual(seen.capabilities.folders, ['/data']);
  assert.ok(Object.isFrozen(seen) && Object.isFrozen(seen.capabilities.folders));
  assert.deepEqual(await a({ op: 'read_file', path: '/data/a.txt' }), { ok: 'A' });
  assert.equal((await a({ op: 'read_file', path: '/etc/passwd' })).error.capability, 'folders');
  assert.equal((await a({ op: 'write_file', path: '/etc/passwd', text: 'x' })).error.capability, 'folders');
  assert.equal(host.files.get('/etc/passwd'), 'root');
});

test('prototype keys are no actions or handlers; IPv6 hosts keep their brackets, as the Rust check', async () => {
  const ask = answerer({ manifest: { id: 'x' }, may: within({ actions: ['*'] }, { actions: ['*'] }), host: { actions: {} }, log: () => {} });
  for (const action of ['constructor', '__proto__', 'toString'])
    assert.equal(JSON.parse(await ask(JSON.stringify({ op: 'run', action }))).error.kind, 'not_found', action);
  assert.equal(JSON.parse(await ask(JSON.stringify({ op: 'hasOwnProperty' }))).error.kind, 'failed');
  const may = within({ hosts: ['[::1]'] }, { hosts: ['[::1]'] });
  assert.equal(check({ id: 'x' }, may, { op: 'fetch', url: 'http://[::1]:8080/' }), null);
  assert.deepEqual(check({ id: 'x' }, within({ hosts: ['::1'] }, { hosts: ['::1'] }), { op: 'fetch', url: 'http://[::1]/' }), [
    'hosts',
    'this extension may not reach [::1] (capabilities.hosts)',
  ]);
});

test('the CLI’s action namespace is the runtime’s', () => {
  for (const id of ['lab.wasi-counter', 'vault', 'a..b', 'x-y_z.9']) assert.equal(cliPrefix(id), actionPrefix(id));
});
