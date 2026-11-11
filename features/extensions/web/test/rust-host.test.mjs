// The runner as Architect's own web app drives it (features/extensions/src/web.rs
// `WebComponent::start`: a module URL, instance.js as source, no `load`,
// no `spawn`), in headless Firefox, against a hostile glue (second review
// of #539):
// - M1: it asks the page for "module" /private/secret.txt while the page
//   holds a session cookie. Before: the page fetched it with the cookie
//   and handed SECRET-OF-THE-PAGE into the sandbox.
// - N3: it sends five 14-byte "core modules" whose memory section claims
//   16M entries. Before: the page's main thread parsed them and froze.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { firefox, open, serve } from './firefox.mjs';

const skip = firefox() ? false : 'needs a Firefox (FIREFOX=…)';
const pkg = fileURLToPath(new URL('..', import.meta.url));
const LIAR = [0, 97, 115, 109, 1, 0, 0, 0, 5, 4, 0x80, 0x80, 0x80, 0x08];

function site(dir, { secret, liars }) {
  symlinkSync(pkg, join(dir, 'pkg'));
  mkdirSync(join(dir, 'private'));
  mkdirSync(join(dir, 'ext'));
  writeFileSync(join(dir, 'private/secret.txt'), 'SECRET-OF-THE-PAGE');
  for (let i = 1; i <= 5; i++) writeFileSync(join(dir, `ext/evil${i}.bin`), Buffer.from(LIAR));
  writeFileSync(
    join(dir, 'ext/glue.js'),
    `const got = {};
self.addEventListener('message', (e) => {
  const m = e.data;
  if (m && m.t === 'answer' && m.id >= 4242) got[m.id] = m.value && m.value.bytes ? new TextDecoder().decode(m.value.bytes).slice(0, 40) : 'no bytes';
});
${secret ? "self.postMessage({ t: 'module', id: 4242, name: '/private/secret.txt' });" : ''}
${liars ? "for (let i = 1; i <= 5; i++) self.postMessage({ t: 'module', id: 5000 + i, name: 'evil' + i + '.bin' });" : ''}
await new Promise((r) => setTimeout(r, 2500));
self.postMessage({ t: 'log', level: 'info', source: 'log', text: JSON.stringify(got) });
export async function instantiate() { await new Promise(() => {}); }
`,
  );
  writeFileSync(
    join(dir, 'page.js'),
    `import { Component, LIMITS } from './pkg/runner.js';
document.cookie = 'session=s3cret; path=/';
// The page's main thread: the longest gap between ticks while it runs.
let last = performance.now(), worst = 0;
const tick = setInterval(() => { const now = performance.now(); worst = Math.max(worst, now - last); last = now; }, 10);
const instance = await (await fetch('/pkg/instance.js')).text();
const logs = [];
const c = new Component({
  url: new URL('/ext/glue.js', location.href).href,
  instance,
  limits: { ...LIMITS, start: 6000 },
  workers: {},
  host: async () => JSON.stringify({ error: { kind: 'unsupported', what: 'no' } }),
  log: (level, source, text) => logs.push({ level, source, text }),
  frame: () => {},
  restarted: () => {},
});
let error = null;
try { await Promise.race([c.start(), new Promise((_, no) => setTimeout(() => no(new Error('the page stopped waiting')), 8000))]); } catch (e) { error = String(e && e.message || e); }
await new Promise((r) => setTimeout(r, 3000));
clearInterval(tick);
c.stop();
window.results = { error, logs, worst };
`,
  );
  writeFileSync(join(dir, 'index.html'), '<!doctype html><body><script type="module" src="/page.js"></script>');
}

async function run(flags) {
  const dir = mkdtempSync(join(tmpdir(), 'architect-rusthost-'));
  const requests = [];
  const server = await serve(dir, (req) => requests.push({ url: req.url, cookie: req.headers.cookie ?? null }));
  const { tab, close } = await open();
  try {
    site(dir, flags);
    await tab.go(`${server.origin}/`);
    const r = JSON.parse(await tab.until('window.results && JSON.stringify(window.results)', 'the page', 100000));
    return { r, requests };
  } finally {
    close();
    server.close();
    rmSync(dir, { recursive: true, force: true });
  }
}

test('M1: the glue reads no page URL with the page’s cookie, through the setup web.rs uses', { skip, timeout: 150000 }, async () => {
  const { r, requests } = await run({ secret: true });
  assert.deepEqual(requests.filter((q) => q.url.includes('private')), [], 'the page fetched nothing outside the glue’s folder');
  assert.ok(!JSON.stringify(r).includes('SECRET-OF-THE-PAGE'), `nothing of the page’s reached the glue: ${JSON.stringify(r)}`);
  assert.match(r.error ?? '', /not a file beside its module/, JSON.stringify(r));
  for (const q of requests.filter((q) => q.url.startsWith('/ext/'))) assert.equal(q.cookie, null, `${q.url} without the page's cookie`);
});

test('N3: lying core modules do not freeze the page’s main thread', { skip, timeout: 150000 }, async () => {
  const { r, requests } = await run({ liars: true });
  assert.equal(requests.filter((q) => /evil\d\.bin/.test(q.url)).length, 5, 'they were fetched and handed in');
  assert.ok(r.worst < 400, `the page's main thread went ${Math.round(r.worst)} ms without a tick`);
});
