// P1 (review of #539): an extension's JS ran with the embedder's origin.
// A store or URL extension ships its own "jco glue"; the worker that ran
// it was a blob: of the page, so the glue read the page's cookie-bearing
// URLs, its IndexedDB and its BroadcastChannels. Here a fake glue tries
// all of it under the guide's CSP, in headless Firefox, and the server
// records what reached it.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { firefox, open, serve } from './firefox.mjs';

const skip = firefox() ? false : 'needs a Firefox (FIREFOX=…)';
const pkg = fileURLToPath(new URL('..', import.meta.url));

/// The guide's CSP (docs/content/guide/extensions-in-your-web-app.md).
async function guideCsp() {
  const { SANDBOX_HASH } = await import('../runner.js').catch(() => ({ SANDBOX_HASH: '' }));
  return `default-src 'none'; script-src 'self' 'wasm-unsafe-eval' blob: '${SANDBOX_HASH}'; worker-src blob:; frame-src 'none'; connect-src 'self'; img-src 'self' data:; media-src 'none'; style-src 'self'; base-uri 'none'; object-src 'none'; form-action 'none'`;
}

function site(dir, origin, csp) {
  symlinkSync(pkg, join(dir, 'pkg'));
  mkdirSync(join(dir, 'fake'));
  writeFileSync(join(dir, 'secret.txt'), 'SECRET');
  writeFileSync(join(dir, 'evil-import.js'), 'globalThis.imported = true;');
  writeFileSync(
    join(dir, 'fake/architect.json'),
    JSON.stringify({ module: 'glue.js', world: '0.1.0', manifest: { id: 'evil.glue', name: 'Evil', version: '0.1.0', api: '0.1' } }),
  );
  // The fake glue: everything a worker with the page's origin could do.
  writeFileSync(
    join(dir, 'fake/glue.js'),
    `const O = ${JSON.stringify(origin)};
const out = { origin: self.origin, location: String(self.location).slice(0, 10) };
const tryIt = async (k, f) => { try { out[k] = await f(); } catch (e) { out[k] = 'blocked: ' + (e && e.name); } };
await tryIt('fetch', async () => { const r = await fetch(O + '/secret.txt', { credentials: 'include' }); return r.status + ' ' + (await r.text()); });
await tryIt('xhr', () => new Promise((ok, no) => { const x = new XMLHttpRequest(); x.open('GET', O + '/secret.txt'); x.withCredentials = true; x.onload = () => ok(x.responseText); x.onerror = () => no(new Error('xhr')); x.send(); }));
await tryIt('idb', () => new Promise((ok, no) => { const q = indexedDB.open('page-db'); q.onsuccess = () => ok([...q.result.objectStoreNames]); q.onerror = () => no(q.error); }));
await tryIt('channel', () => { new BroadcastChannel('page-channel').postMessage('from the glue'); return 'posted'; });
await tryIt('caches', async () => (await caches.keys()).length);
await tryIt('beacon', () => navigator.sendBeacon(O + '/beacon', 'x'));
await tryIt('import', async () => { await import(O + '/evil-import.js'); return 'imported'; });
await tryIt('ws', () => { new WebSocket(O.replace('http', 'ws') + '/ws'); return 'created'; });
self.postMessage({ t: 'log', level: 'info', source: 'glue', text: JSON.stringify(out) });
export async function instantiate() { throw new Error('a fake'); }
`,
  );
  writeFileSync(
    join(dir, 'page.js'),
    `import { loadExtension } from './pkg/index.js';
document.cookie = 'session=s3cret; path=/';
const heard = [];
new BroadcastChannel('page-channel').onmessage = (e) => heard.push(e.data);
await new Promise((ok) => { const q = indexedDB.open('page-db'); q.onupgradeneeded = () => q.result.createObjectStore('secrets'); q.onsuccess = ok; });
const logs = [];
let error = null;
try { await loadExtension('/fake/', { integrity: false, log: (l) => logs.push(l) }); } catch (e) { error = e.message; }
await new Promise((r) => setTimeout(r, 1000));
// Its own line: the host names its source ('log'), whatever it said.
const glue = logs.find((l) => l.text.startsWith('{"origin"'));
const claimed = logs.some((l) => l.source === 'glue');
window.results = { error, heard, claimed, glueSource: glue ? glue.source : null, glue: glue ? JSON.parse(glue.text) : null, frames: document.querySelectorAll('iframe[sandbox]').length };
`,
  );
  writeFileSync(join(dir, 'index.html'), `<!doctype html><meta http-equiv="Content-Security-Policy" content="${csp}"><body><script type="module" src="/page.js"></script>`);
}

test('P1: the extension’s glue runs in an opaque origin and reaches nothing of the page’s', { skip, timeout: 90000 }, async () => {
  const dir = mkdtempSync(join(tmpdir(), 'architect-glue-'));
  const requests = [];
  const server = await serve(dir, (req) => requests.push({ url: req.url, cookie: req.headers.cookie ?? null }));
  const { tab, close } = await open();
  try {
    site(dir, server.origin, await guideCsp());
    await tab.go(`${server.origin}/`);
    const r = JSON.parse(await tab.until('window.results && JSON.stringify(window.results)', 'the page', 40000));
    assert.ok(r.glue, `the glue ran and said what it reached: ${JSON.stringify(r)}`);
    assert.equal(r.glue.origin, 'null', 'an opaque origin');
    for (const k of ['fetch', 'xhr', 'idb', 'caches', 'import', 'ws']) assert.match(String(r.glue[k]), /^blocked/, `${k}: ${r.glue[k]}`);
    assert.deepEqual(r.heard, [], 'the page’s BroadcastChannel heard nothing');
    const reached = requests.filter((q) => /secret|beacon|evil-import|\/ws/.test(q.url));
    assert.deepEqual(reached, [], 'no request of the glue’s reached the page’s origin');
    assert.match(r.error, /did not start/);
    assert.equal(r.frames, 0, 'its sandbox frame was taken down');
    assert.equal(r.glueSource, 'log', 'the source of its line is the host’s to name');
    assert.equal(r.claimed, false);
  } finally {
    close();
    server.close();
    rmSync(dir, { recursive: true, force: true });
  }
});
