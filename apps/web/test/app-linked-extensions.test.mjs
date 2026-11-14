// A distribution's linked extensions in the web app (#495, W1), in a real
// browser: the release bundle (ARCHITECT_WEB_BUNDLE), whose wasm is the
// Architect app's web build (`architect_web::run(architect())`), served
// as production serves it (its policy from apps/web/sws.toml), no node.
//
// In `/app/?demo`:
// - the Architect app's linked extensions are on ⌘K: Projects' "Open
//   Projects", the Calendar's "Open the calendar", the Time Tracker's,
//   the theme picker's "Choose a theme…";
// - ⌘K's "Open Calendar" opens the Calendar's own view as a pane of the
//   workspace (`calendar/calendar`), drawn by its Dioxus component in the
//   app's document, not in a frame;
// - "Open Projects" opens Projects' list the same way (its rows need the
//   vault's `BASES_*`, which the browser gets with documents, #495 H1);
// - the settings page lists the extensions' sections (Calendar, Tasks,
//   Day plans) beside Appearance.
//
// At 390 px and 1280 px, light and dark; SHOTS=<dir> keeps the
// screenshots. Firefox over WebDriver BiDi (FIREFOX=… or `firefox` on the
// PATH), muted, in a mount namespace whose runtime dir and /run/pipewire
// are empty (`unshare -r -m`). `just app-linked-extensions-test` builds
// and runs it.
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
const skip = BUNDLE && browser ? false : 'needs ARCHITECT_WEB_BUNDLE (just app-linked-extensions-test) and a Firefox (FIREFOX=…)';
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

/// The bundle's files, the app's page with its policy; no node (404).
async function front() {
  const policy = appPolicy();
  const port = await freePort();
  const srv = createServer((req, res) => {
    const path = new URL(req.url, 'http://x').pathname;
    let file = normalize(join(BUNDLE, decodeURIComponent(path)));
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

/// A headless Firefox, muted, light or `dark`.
async function firefox(dark = false) {
  const profile = mkdtempSync(join(tmpdir(), 'linked-ext-ff-'));
  writeFileSync(join(profile, 'user.js'), [
    'user_pref("media.volume_scale", "0.0");',
    `user_pref("ui.systemUsesDarkTheme", ${dark ? 1 : 0});`,
    `user_pref("layout.css.prefers-color-scheme.content-override", ${dark ? 0 : 1});`,
  ].join('\n') + '\n');
  const port = await freePort();
  const runtime = process.env.XDG_RUNTIME_DIR || `/run/user/${process.getuid()}`;
  const env = { ...process.env, XDG_RUNTIME_DIR: runtime, MOZ_HEADLESS: '1' };
  delete env.LD_LIBRARY_PATH; delete env.PIPEWIRE_REMOTE; delete env.PULSE_SERVER;
  const hide = 'mount -t tmpfs none "$XDG_RUNTIME_DIR" && { [ ! -d /run/pipewire ] || mount -t tmpfs none /run/pipewire; } && exec "$0" "$@"';
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
  const logs = [];
  ws.onmessage = (e) => {
    const m = JSON.parse(e.data);
    if (m.method === 'log.entryAdded') logs.push(`${m.params.level}: ${m.params.text}`);
    if (m.id && waiting.has(m.id)) { const w = waiting.get(m.id); waiting.delete(m.id); m.type === 'error' ? w.no(new Error(`${m.error}: ${m.message}`)) : w.ok(m.result); }
  };
  const send = (method, params = {}) => new Promise((ok, no) => { const i = ++id; waiting.set(i, { ok, no }); ws.send(JSON.stringify({ id: i, method, params })); });
  await send('session.new', { capabilities: {} });
  await send('session.subscribe', { events: ['log.entryAdded'] });
  const context = (await send('browsingContext.create', { type: 'tab' })).context;
  await send('browsingContext.setViewport', { context, viewport: { width: 1280, height: 900 } });
  return {
    logs,
    go: (url) => send('browsingContext.navigate', { context, url, wait: 'complete' }),
    size: (width, height) => send('browsingContext.setViewport', { context, viewport: { width, height } }),
    shot: async (file) => { const r = await send('browsingContext.captureScreenshot', { context, origin: 'viewport' }); writeFileSync(file, Buffer.from(r.data, 'base64')); },
    eval: async (expression) => {
      const r = await send('script.evaluate', { expression, target: { context }, awaitPromise: true, userActivation: true });
      if (r.type === 'exception') throw new Error(r.exceptionDetails.text);
      return r.result.value;
    },
    keys: (value) => send('input.performActions', { context, actions: [{ type: 'key', id: 'kb', actions: [...value].flatMap((k) => [{ type: 'keyDown', value: k }, { type: 'keyUp', value: k }]) }] }),
    chord: (mod, key) => send('input.performActions', { context, actions: [{ type: 'key', id: 'kb', actions: [{ type: 'keyDown', value: mod }, { type: 'keyDown', value: key }, { type: 'keyUp', value: key }, { type: 'keyUp', value: mod }] }] }),
    close: async () => { try { ws.close(); } catch {} try { process.kill(-proc.pid, 'SIGKILL'); } catch {} await sleep(300); try { rmSync(profile, { recursive: true, force: true }); } catch {} },
  };
}

async function until(fn, ms = 20000, what = 'a condition') {
  const end = Date.now() + ms;
  let last;
  while (Date.now() < end) { try { last = await fn(); if (last) return last; } catch (e) { last = e.message; } await sleep(200); }
  throw new Error(`timed out waiting for ${what}; last: ${JSON.stringify(last)}`);
}

const CTRL = '';
const ENTER = '';
const ESC = '';
const shotDir = () => { const d = SHOTS || join(tmpdir(), 'linked-ext-shots'); mkdirSync(d, { recursive: true }); return d; };
/// Text on show anywhere in the page.
const shows = (b, text) => b.eval(`document.body.innerText.includes(${JSON.stringify(text)})`);

/// Whether the palette's text (spaces taken out) has `name`.
const lists = (rows, name) => rows.includes(name.replace(/\s+/g, ''));

/// ⌘K, `query` typed: the palette's rows' text.
async function palette(b, query) {
  await b.chord(CTRL, 'k');
  await until(() => b.eval('document.activeElement?.tagName === "INPUT"'), 10000, 'the palette');
  await b.keys(query);
  await sleep(400);
  // A row's matched letters are spans of their own, its spaces lost
  // between them: the text without any, as `lists` compares it.
  return (await b.eval('document.body.innerText')).replace(/\s+/g, '');
}

/// ⌘K, `name` typed and run.
async function run(b, name) {
  const rows = await palette(b, name);
  assert.ok(lists(rows, name), `⌘K lists ${name}`);
  await b.keys(ENTER);
  await sleep(500);
}

test("the Architect app's linked extensions run in /app/?demo: ⌘K, their views as panes, their settings", { skip }, async () => {
  const f = await front();
  const shots = shotDir();
  try {
    for (const dark of [false, true]) {
      const b = await firefox(dark);
      const theme = dark ? 'dark' : 'light';
      try {
        await b.go(`${f.origin}/app/?demo`);
        await until(() => b.eval(`!!document.querySelector('[data-testid=note-text]')`), 90000, 'the demo');
        // The Architect app's linked extensions are running (the page's
        // `data-extensions`), none refused.
        const ids = await until(async () => { const v = await b.eval('document.documentElement.dataset.extensions || ""'); return v && v.split(' '); }, 20000, 'the extensions to load');
        for (const id of ['projects', 'tasks', 'time', 'calendar', 'dayplan', 'theme.picker']) {
          assert.ok(ids.includes(id), `${id} runs: ${ids.join(' ')}; ${b.logs.filter((l) => l.includes('extensions:')).join(' | ')}`);
        }
        // Their actions on ⌘K.
        for (const name of ['Open Projects', 'Open the calendar', 'Choose a theme…']) {
          const rows = await palette(b, name);
          assert.ok(lists(rows, name), `⌘K lists ${name}: ${rows.slice(-400)}`);
          await b.keys(ESC);
          await sleep(200);
        }
        // The Calendar's view, as a pane, drawn in the app's document.
        // How many places say "Calendar" now (the navigator's folder).
        const labels = `[...document.querySelectorAll('*')].filter((e) => e.children.length === 0 && e.textContent.trim() === 'Calendar' && e.getClientRects().length).length`;
        const before = await b.eval(labels);
        // Its view, opened from ⌘K's views (`CALENDAR_OPEN` reads the
        // vault's bases first, which the browser host gets with documents).
        await run(b, 'Open Calendar');
        await until(async () => (await b.eval(labels)) > before, 15000, 'the Calendar pane (its tab)');
        assert.equal(await b.eval('document.querySelectorAll("iframe").length'), 0, 'a linked view is no frame');
        for (const [w, h, tag] of [[1280, 900, 'desk-1280'], [390, 844, 'phone-390']]) {
          await b.size(w, h);
          await sleep(800);
          assert.ok(await b.eval('document.documentElement.scrollWidth <= window.innerWidth'), `${tag}: no sideways scroll`);
          await b.shot(join(shots, `linked-calendar-${tag}-${theme}.png`));
        }
        await b.size(1280, 900);
        // Projects' list, the same way.
        await run(b, 'Open Projects');
        await until(() => shows(b, 'Projects'), 15000, 'the Projects pane');
        await b.shot(join(shots, `linked-projects-desk-1280-${theme}.png`));
        // Their settings sections beside the app's.
        await run(b, 'Settings');
        await until(() => b.eval(`!!document.querySelector('[data-testid=settings-layer]')`), 10000, 'the settings page');
        for (const section of ['Appearance', 'Calendar', 'Tasks']) {
          await until(() => shows(b, section), 10000, `the ${section} section`);
        }
        await b.shot(join(shots, `linked-settings-desk-1280-${theme}.png`));
        const panics = b.logs.filter((l) => /panicked|RuntimeError|unreachable/.test(l));
        assert.deepEqual(panics, [], 'no panic in the page');
      } finally { await b.close(); }
    }
  } finally { f.close(); }
});
