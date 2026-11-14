// The extension showcase in the web app (#710): `/app/?showcase` opens the
// showcase vault (`apps/ui/showcase-vault`) in the tab's memory, the app
// itself on dioxus-web, in a real browser. Its folders (one per group of
// capabilities, R to W) are in the navigator, and its index note and a
// capability's note open. The web app hosts no extensions yet (#495): the
// notes are what it shows.
//
// The release bundle (scripts/web-bundle.sh, ARCHITECT_WEB_BUNDLE) served
// by a small front with the app page's own policy (apps/web/sws.toml), no
// node. Firefox over WebDriver BiDi (FIREFOX=… or `firefox` on the PATH), in
// a mount namespace whose runtime dir and /run/pipewire are empty (`unshare
// -r -m`). With ARCHITECT_SHOTS=<dir> it writes the showcase's web
// screenshots there: <note>-<390|1280>-<light|dark>.png, the scheme the
// browser's (`prefers-color-scheme`, set per launch). `just
// showcase-web-shots` builds the bundle and runs it into
// target/showcase/web.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync, readFileSync, existsSync, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { createServer as createNetServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join, extname, normalize, relative, isAbsolute } from 'node:path';

const BUNDLE = process.env.ARCHITECT_WEB_BUNDLE;
const SHOTS = process.env.ARCHITECT_SHOTS;
function which(cmd) {
  try { return execFileSync('sh', ['-c', `command -v ${cmd}`]).toString().trim() || null; } catch { return null; }
}
const browser = process.env.FIREFOX || which('firefox');
const skip = BUNDLE && browser ? false : 'needs ARCHITECT_WEB_BUNDLE (just showcase-web-shots) and a Firefox (FIREFOX=…)';
if (process.env.ARCHITECT_BROWSER_REQUIRED && skip) throw new Error(`ARCHITECT_BROWSER_REQUIRED: ${skip}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
/// How long one WebDriver BiDi command may take.
const SEND_MS = 60000;
const freePort = () => new Promise((ok) => { const srv = createNetServer(); srv.listen(0, '127.0.0.1', () => { const { port } = srv.address(); srv.close(() => ok(port)); }); });

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
    // Inside the bundle, not merely a path that starts like it
    // (`/x/web-bundle-other`).
    const rel = relative(normalize(BUNDLE), file);
    if (rel.startsWith('..') || isAbsolute(rel)) { res.statusCode = 403; res.end(); return; }
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

/// Firefox, headless, its content in `scheme` (light or dark).
async function firefox(scheme) {
  const profile = mkdtempSync(join(tmpdir(), 'app-showcase-ff-'));
  const port = await freePort();
  const runtime = process.env.XDG_RUNTIME_DIR || `/run/user/${process.getuid()}`;
  const env = { ...process.env, XDG_RUNTIME_DIR: runtime, MOZ_HEADLESS: '1' };
  delete env.LD_LIBRARY_PATH; delete env.PIPEWIRE_REMOTE; delete env.PULSE_SERVER;
  const hide = 'mount -t tmpfs none "$XDG_RUNTIME_DIR" && { [ ! -d /run/pipewire ] || mount -t tmpfs none /run/pipewire; } && exec "$0" "$@"';
  // 0: dark, 1: light, whatever the system says.
  writeFileSync(join(profile, 'user.js'), [
    'user_pref("media.volume_scale", "0.0");',
    `user_pref("layout.css.prefers-color-scheme.content-override", ${scheme === 'dark' ? 0 : 1});`,
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
  const kill = () => { try { process.kill(-proc.pid, 'SIGKILL'); } catch {} try { rmSync(profile, { recursive: true, force: true }); } catch {} };
  if (!ws) { kill(); throw new Error('firefox did not start (unshare -r -m needed)'); }
  let id = 0;
  const waiting = new Map();
  ws.onmessage = (e) => { const m = JSON.parse(e.data); if (m.id && waiting.has(m.id)) { const w = waiting.get(m.id); waiting.delete(m.id); m.type === 'error' ? w.no(new Error(`${m.error}: ${m.message}`)) : w.ok(m.result); } };
  // Every command answers in SEND_MS, or fails: a browser that stops
  // answering ends the test rather than hangs it.
  const send = (method, params = {}) => new Promise((ok, no) => {
    const i = ++id;
    const timer = setTimeout(() => { waiting.delete(i); no(new Error(`${method}: no answer in ${SEND_MS} ms`)); }, SEND_MS);
    waiting.set(i, { ok: (v) => { clearTimeout(timer); ok(v); }, no: (e) => { clearTimeout(timer); no(e); } });
    ws.send(JSON.stringify({ id: i, method, params }));
  });
  let context;
  try {
    await send('session.new', { capabilities: {} });
    context = (await send('browsingContext.create', { type: 'tab' })).context;
  } catch (e) {
    try { ws.close(); } catch {}
    kill();
    throw e;
  }
  return {
    go: (url) => send('browsingContext.navigate', { context, url, wait: 'complete' }),
    viewport: (width, height) => send('browsingContext.setViewport', { context, viewport: { width, height } }),
    shot: async (name) => {
      if (!SHOTS) return;
      const { data } = await send('browsingContext.captureScreenshot', { context });
      mkdirSync(SHOTS, { recursive: true });
      writeFileSync(join(SHOTS, `${name}.png`), Buffer.from(data, 'base64'));
    },
    eval: async (expression) => {
      const r = await send('script.evaluate', { expression, target: { context }, awaitPromise: true, userActivation: true });
      if (r.type === 'exception') throw new Error(r.exceptionDetails.text);
      return r.result.value;
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

/// What the page shows: its text and its text areas' (a note is edited
/// as Markdown in one).
const text = (b) => b.eval(`document.body.innerText + '\\n' + [...document.querySelectorAll('textarea')].map((t) => t.value).join('\\n')`);
/// Presses the last leaf element whose text is `txt` (a navigator row).
const click = (b, txt) => b.eval(`(() => { const all = [...document.querySelectorAll('*')].filter((e) => e.children.length === 0 && e.textContent.trim() === ${JSON.stringify(txt)}); const e = all[all.length - 1]; if (!e) return false; (e.closest('button,[role=treeitem],[role=button],a,li,div') || e).click(); return true; })()`);
const tab = (b, label) => b.eval(`(() => { const t = [...document.querySelectorAll('button[data-tab]')].find((x) => x.getAttribute('aria-label') === ${JSON.stringify(label)}); if (!t) return false; t.click(); return true; })()`);

/// Opens note `name` in folder `folder` (none: the vault's top) from the
/// navigator: a phone's Files tab first.
async function openNote(b, folder, name, phone) {
  if (phone) { await until(() => tab(b, 'Files'), 10000, 'a Files tab'); await sleep(400); }
  if (folder) { await until(() => click(b, folder), 10000, `the ${folder} folder`); await sleep(300); }
  await until(() => click(b, name), 10000, `the note ${name}`);
  await sleep(600);
}

const GROUPS = ['R Runtimes', 'L Install and lifecycle', 'C Contributions', 'H Host services', 'U Drawing and input', 'W The web app'];

for (const scheme of ['light', 'dark']) {
  test(`/app/?showcase opens the showcase vault (${scheme})`, { skip, timeout: 300000 }, async () => {
    const f = await front();
    let b;
    try {
      b = await firefox(scheme);
      for (const [width, height, phone] of [[1280, 844, false], [390, 844, true]]) {
        await b.viewport(width, height);
        await b.go(`${f.origin}/app/?showcase`);
        // The app, on the showcase's vault: its root is named Showcase,
        // its folders are the capability groups.
        // A phone's navigator is behind its Files tab.
        if (phone) await until(() => tab(b, 'Files'), 60000, 'a Files tab');
        try {
          await until(async () => (await text(b)).includes('Showcase'), 60000, 'the showcase');
        } catch (e) {
          await b.shot(`failed-${width}-${scheme}`);
          throw new Error(`${e.message}; the page says: ${JSON.stringify((await text(b)).slice(0, 400))}`);
        }
        // The folders, one per group of capabilities (a desk's tree; a
        // phone's drawer lists the vault's top notes, the index among them).
        const want = phone ? ['Showcase'] : [...GROUPS, 'Samples'];
        try {
          await until(async () => { const t = await text(b); return want.every((g) => t.includes(g)); }, 15000, 'the capability folders');
        } catch (e) {
          await b.shot(`failed-${width}-${scheme}`);
          throw new Error(`${e.message}; the page says: ${JSON.stringify((await text(b)).slice(0, 600))}`);
        }
        assert.equal(
          await b.eval(`matchMedia('(prefers-color-scheme: dark)').matches`),
          scheme === 'dark',
          'the browser shows the scheme asked for',
        );
        // The index note, then a capability's.
        await openNote(b, null, 'Showcase', false);
        await until(async () => (await text(b)).includes('One note per capability'), 10000, 'the index note');
        await b.shot(`showcase-${width}-${scheme}`);
        // A capability's note, from the desk's navigator (a phone's drawer
        // lists a folder at a time; the note is a text area there, its
        // links not links).
        if (phone) continue;
        await openNote(b, 'C Contributions', 'C11 The editor', false);
        await until(async () => (await text(b)).includes('Advanced Tables'), 10000, 'C11\'s note');
        await b.shot(`c11-the-editor-${width}-${scheme}`);
      }
    } finally {
      if (b) await b.close();
      f.close();
    }
  });
}
