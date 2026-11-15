// Shared by the web app's own browser tests (app-linked-extensions,
// app-documents): the release bundle served as production serves it (its
// policy from apps/web/sws.toml, no node), and a headless Firefox over
// WebDriver BiDi, muted, in a mount namespace whose runtime dir and
// /run/pipewire are empty (`unshare -r -m`).
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync, readFileSync, existsSync, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { createServer as createNetServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join, extname, normalize } from 'node:path';

export const BUNDLE = process.env.ARCHITECT_WEB_BUNDLE;
export const SHOTS = process.env.SHOTS;
export function which(cmd) {
  try { return execFileSync('sh', ['-c', `command -v ${cmd}`]).toString().trim() || null; } catch { return null; }
}
export const browser = process.env.FIREFOX || which('firefox');
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const freePort = () => new Promise((ok) => { const srv = createNetServer(); srv.listen(0, '127.0.0.1', () => { const { port } = srv.address(); srv.close(() => ok(port)); }); });

/// The app page's policy, as production's server sends it (sws.toml).
export function appPolicy() {
  const sws = readFileSync(new URL('../../sws.toml', import.meta.url), 'utf8');
  const m = sws.match(/Content-Security-Policy = "([^"]+)"/);
  if (!m) throw new Error('no Content-Security-Policy in apps/web/sws.toml');
  return m[1];
}

const TYPES = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' };

/// The bundle's files, the app's page with its policy; no node (404).
export async function front() {
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
export async function firefox(dark = false) {
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

export async function until(fn, ms = 20000, what = 'a condition') {
  const end = Date.now() + ms;
  let last;
  while (Date.now() < end) { try { last = await fn(); if (last) return last; } catch (e) { last = e.message; } await sleep(200); }
  throw new Error(`timed out waiting for ${what}; last: ${JSON.stringify(last)}`);
}

/// Why a test cannot run here (`recipe` builds what it needs), or false.
export function skipWhy(recipe) {
  const skip = BUNDLE && browser ? false : `needs ARCHITECT_WEB_BUNDLE (just ${recipe}) and a Firefox (FIREFOX=…)`;
  if (process.env.ARCHITECT_BROWSER_REQUIRED && skip) throw new Error(`ARCHITECT_BROWSER_REQUIRED: ${skip}`);
  return skip;
}

/// A folder for screenshots: SHOTS, or one under the temp dir named `name`.
export const shotDir = (name) => { const d = SHOTS || join(tmpdir(), name); mkdirSync(d, { recursive: true }); return d; };
