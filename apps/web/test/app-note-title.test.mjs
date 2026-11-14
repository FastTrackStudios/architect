// A note says its name once, at the top (#710), in a real browser: in
// `/app/?showcase` the showcase's start note (`Showcase.md`) opens with
// its name in the bar at the top, and its text starts with its content,
// not a `# Showcase` heading repeating it (the seeded vaults lost theirs).
//
// At 390 px and 1280 px, light and dark; SHOTS=<dir> keeps the
// screenshots. The release bundle (scripts/web-bundle.sh,
// ARCHITECT_WEB_BUNDLE) served with the app page's policy; Firefox over
// WebDriver BiDi (FIREFOX=… or `firefox` on the PATH), muted, in a mount
// namespace whose runtime dir and /run/pipewire are empty (`unshare -r
// -m`). Run with `env -u LD_LIBRARY_PATH node --test`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync, readFileSync, existsSync, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { createServer as createNetServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join, extname, normalize } from 'node:path';

const BUNDLE = process.env.ARCHITECT_WEB_BUNDLE;
const SHOTS = process.env.SHOTS;
function which(cmd) {
  try { return execFileSync('sh', ['-c', `command -v ${cmd}`]).toString().trim() || null; } catch { return null; }
}
const browser = process.env.FIREFOX || which('firefox');
const skip = BUNDLE && browser ? false : 'needs ARCHITECT_WEB_BUNDLE (scripts/web-bundle.sh) and a Firefox (FIREFOX=…)';
if (process.env.ARCHITECT_BROWSER_REQUIRED && skip) throw new Error(`ARCHITECT_BROWSER_REQUIRED: ${skip}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const freePort = () => new Promise((ok) => { const srv = createNetServer(); srv.listen(0, '127.0.0.1', () => { const { port } = srv.address(); srv.close(() => ok(port)); }); });

/// The app page's policy, as production's server sends it (sws.toml).
function appPolicy() {
  const sws = readFileSync(new URL('../sws.toml', import.meta.url), 'utf8');
  const m = sws.match(/Content-Security-Policy = "([^"]+)"/);
  if (!m) throw new Error('no Content-Security-Policy in apps/web/sws.toml');
  return m[1];
}

const TYPES = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' };

/// The bundle's files, the app's page with its policy.
async function front() {
  const policy = appPolicy();
  const port = await freePort();
  const srv = createServer((req, res) => {
    const path = decodeURIComponent(new URL(req.url, 'http://x').pathname);
    let file = normalize(join(BUNDLE, path));
    if (!file.startsWith(normalize(BUNDLE))) { res.statusCode = 403; res.end(); return; }
    if (existsSync(file) && statSync(file).isDirectory()) file = join(file, 'index.html');
    if (!existsSync(file)) { res.statusCode = 404; res.end(); return; }
    const headers = { 'content-type': TYPES[extname(file)] || 'application/octet-stream', 'cache-control': 'no-store' };
    if (path === '/app/' || path === '/app/index.html') headers['content-security-policy'] = policy;
    res.writeHead(200, headers);
    res.end(readFileSync(file));
  });
  await new Promise((ok) => srv.listen(port, '127.0.0.1', ok));
  return { origin: `http://127.0.0.1:${port}`, close: () => srv.close() };
}

/// A headless Firefox, muted, light or `dark`, at `width`.
async function firefox(dark, width) {
  const profile = mkdtempSync(join(tmpdir(), 'app-title-ff-'));
  const port = await freePort();
  const runtime = process.env.XDG_RUNTIME_DIR || `/run/user/${process.getuid()}`;
  const env = { ...process.env, XDG_RUNTIME_DIR: runtime, MOZ_HEADLESS: '1' };
  delete env.LD_LIBRARY_PATH; delete env.PIPEWIRE_REMOTE; delete env.PULSE_SERVER;
  const hide = 'mount -t tmpfs none "$XDG_RUNTIME_DIR" && { [ ! -d /run/pipewire ] || mount -t tmpfs none /run/pipewire; } && exec "$0" "$@"';
  writeFileSync(join(profile, 'user.js'), [
    'user_pref("media.volume_scale", "0.0");',
    `user_pref("ui.systemUsesDarkTheme", ${dark ? 1 : 0});`,
    `user_pref("layout.css.prefers-color-scheme.content-override", ${dark ? 0 : 1});`,
  ].join('\n') + '\n');
  const args = ['--headless', '--no-remote', '--profile', profile, '--remote-debugging-port', String(port)];
  const proc = process.platform === 'darwin'
    ? spawn(browser, args, { stdio: 'ignore', env, detached: true })
    : spawn('unshare', ['-r', '-m', 'sh', '-c', hide, browser, ...args], { stdio: 'ignore', env, detached: true });
  let ws;
  for (let i = 0; i < 80 && !ws; i++) {
    await sleep(250);
    try { const s = new WebSocket(`ws://127.0.0.1:${port}/session`); await new Promise((ok, no) => { s.onopen = ok; s.onerror = no; }); ws = s; } catch {}
  }
  if (!ws) { try { process.kill(-proc.pid, 'SIGKILL'); } catch {} throw new Error('firefox did not start (unshare -r -m needed)'); }
  let id = 0;
  const waiting = new Map();
  ws.onmessage = (e) => { const m = JSON.parse(e.data); if (m.id && waiting.has(m.id)) { const w = waiting.get(m.id); waiting.delete(m.id); m.type === 'error' ? w.no(new Error(`${m.error}: ${m.message}`)) : w.ok(m.result); } };
  const send = (method, params = {}) => new Promise((ok, no) => { const i = ++id; waiting.set(i, { ok, no }); ws.send(JSON.stringify({ id: i, method, params })); });
  await send('session.new', { capabilities: {} });
  const context = (await send('browsingContext.create', { type: 'tab' })).context;
  await send('browsingContext.setViewport', { context, viewport: { width, height: 844 } });
  return {
    go: (url) => send('browsingContext.navigate', { context, url, wait: 'complete' }),
    eval: async (expression) => {
      const r = await send('script.evaluate', { expression, target: { context }, awaitPromise: true, userActivation: true });
      if (r.type === 'exception') throw new Error(r.exceptionDetails.text);
      return r.result.value;
    },
    shot: async (name) => {
      if (!SHOTS) return;
      const { data } = await send('browsingContext.captureScreenshot', { context });
      mkdirSync(SHOTS, { recursive: true });
      writeFileSync(join(SHOTS, `${name}.png`), Buffer.from(data, 'base64'));
    },
    close: async () => { try { ws.close(); } catch {} try { process.kill(-proc.pid, 'SIGKILL'); } catch {} await sleep(300); try { rmSync(profile, { recursive: true, force: true }); } catch {} },
  };
}

async function until(fn, ms = 20000, what = 'a condition') {
  const end = Date.now() + ms;
  let last;
  while (Date.now() < end) { try { last = await fn(); if (last) return last; } catch (e) { last = e.message; } await sleep(150); }
  throw new Error(`timed out waiting for ${what}; last: ${JSON.stringify(last)}`);
}

/// The open note's text and the leaf elements saying its name.
const LOOK = `JSON.stringify({
  note: (document.querySelector('[data-note]') || {}).dataset?.note || null,
  text: (document.querySelector('[data-testid=note-text]') || {}).value || '',
  named: [...document.querySelectorAll('body *')].filter((e) => e.children.length === 0 && e.textContent.trim() === 'Showcase' && e.getClientRects().length > 0).length,
  headings: [...document.querySelectorAll('[data-note] h1')].map((h) => h.textContent.trim()),
})`;

/// Presses the last visible leaf element whose text is `txt` (a navigator row).
const click = (b, txt) => b.eval(`(() => { const all = [...document.querySelectorAll('*')].filter((e) => e.children.length === 0 && e.textContent.trim() === ${JSON.stringify(txt)} && e.getClientRects().length > 0); const e = all[all.length - 1]; if (!e) return false; (e.closest('button,[role=treeitem],[role=button],a,li,div') || e).click(); return true; })()`);
const tab = (b, label) => b.eval(`(() => { const t = [...document.querySelectorAll('button[data-tab], button[aria-label], [role=button][aria-label]')].find((x) => x.getAttribute('aria-label') === ${JSON.stringify(label)} && x.getClientRects().length > 0); if (!t) return false; t.click(); return true; })()`);

/// The showcase's start note, from the navigator (a phone's Files drawer).
async function openShowcase(b) {
  await until(() => b.eval(`!!document.querySelector('button[aria-label=Files]')`), 60000, 'the app');
  await sleep(800);
  await tab(b, 'Files');
  await sleep(500);
  // A desk's sidebar starts folded: unfolded, the navigator shows.
  if (!(await click(b, 'Showcase'))) { await tab(b, 'Toggle the sidebar'); await sleep(600); }
  await until(() => click(b, 'Showcase'), 10000, 'the Showcase row');
  await sleep(600);
}

test('the showcase note says its name at the top and starts with its content', { skip }, async () => {
  const f = await front();
  try {
    for (const width of [390, 1280]) {
      for (const dark of [false, true]) {
        const b = await firefox(dark, width);
        try {
          await b.go(`${f.origin}/app/?showcase`);
          await openShowcase(b);
          const look = JSON.parse(await until(async () => {
            const l = JSON.parse(await b.eval(LOOK));
            return l.note === 'Showcase.md' && l.text ? JSON.stringify(l) : null;
          }, 20000, 'the showcase note'));
          const body = look.text.replace(/^---\n[\s\S]*?\n---\n/, '');
          const first = body.split('\n').find((l) => l.trim()) || '';
          assert.notEqual(first.trim(), '# Showcase', `the text starts with its content: ${body.slice(0, 120)}`);
          assert.deepEqual(look.headings, [], 'no heading repeats the name');
          assert.ok(look.named >= 1, 'the name is at the top');
          await sleep(600);
          await b.shot(`showcase-note-${width}-${dark ? 'dark' : 'light'}`);
        } finally {
          await b.close();
        }
      }
    }
  } finally {
    f.close();
  }
});
