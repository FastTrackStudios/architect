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
// The probes the node-rendered share page's suite ran (retired with the
// page, #646), in the app, and the app's own rules (#495 Constraints):
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
// The Now Playing bar (#589) on the same stack: the song's folder is the
// island's queue on the playhead, the bar shows it, its presses reach the
// frames' props, a frame's asks move the bar, a note's recording is a
// second source the bar switches to, and a frame moves only its own
// extension's source, within the rate limit. The app's own audio element
// streams the demo's silent recording with `Range`.
//
// Firefox over WebDriver BiDi (FIREFOX=… or `firefox` on the PATH), its
// volume at zero and autoplay allowed (the profile's prefs). On Linux in a
// mount namespace whose runtime dir and /run/pipewire are empty (`unshare
// -r -m`): a headless Firefox otherwise opens a stream on the machine's
// live audio graph; on a Mac, headless as it is (its output at zero and the
// file silent). `just app-ext-host-test` builds and runs it; SHOTS=<dir>
// writes the bar's screenshots (390 px and desk width) there.
//
// The demo's Christmas setlist (`Setlists/Merry & Bright.md`, made by
// `apps/ui/demo-vault.py --christmas`): a setlist view's queue carries each
// song's cover (a vault path), the frame is handed it as `art_src` (a
// `data:` URL) and the card draws it, title over artist; a song note's
// recording does the same from the note. With STUDIO_ISLAND=<folder>
// (Studio's `just island-web <folder>`, `island.css` beside it), Studio's
// own setlist view draws the set, its songs with no recording greyed, a
// row pressed plays on the card.
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
  // Once ready (asks before it are refused): its source, a seek past it,
  // then the flood.
  setTimeout(() => globalThis.architectPage.ask("PLAYHEAD_SET", { name: "Look", queue: [{ id: "a", title: "A", duration: 10 }] }), 300);
  setTimeout(() => globalThis.architectPage.ask("PLAYHEAD_SEEK", { t: 999 }), 600);
  setTimeout(() => { for (let i = 0; i < 200; i++) globalThis.architectPage.ask("PLAYHEAD_SEEK", { t: (i % 10) + 1 }); }, 1100);
  document.body.dataset.report = JSON.stringify(r);
`;

/// The share suite's FLOOD: the frame's port captured, then hellos and
/// readies, a source of one minute-long item, sizes and asks by the thousand;
/// after the rate limit refills it asks for 20 + how many times it was
/// booted again.
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
  // Its source, once it says it is ready (asks before it are refused).
  orig.call(port, { t: "ask", action: "PLAYHEAD_SET", args: { name: "Flood", queue: [{ id: "a", title: "A", duration: 60 }] } });
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
    // A built island's folder as it is (Studio's `just island-web`), or
    // one written here from `bundle`.
    const folder = one.folder || join(dir, `island-${n}`);
    if (!one.folder) {
      put(folder, 'web-view.js', one.bundle);
      put(folder, 'web-view_bg.wasm', WASM);
      put(folder, 'architect.json', JSON.stringify({ module: 'web-view.js', manifest: { id: one.id, capabilities: {} } }));
    }
    if (one.sheet) env[`${prefix}_SHEET`] = one.sheet;
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
  const media = [];
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
    // A recording, ranged as the node's files are (`206`, `Content-Range`).
    if (extname(file) === '.mp3') {
      const body = readFileSync(file);
      const m = /^bytes=(\d*)-(\d*)$/.exec(req.headers.range || '');
      const h = { 'content-type': 'audio/mpeg', 'accept-ranges': 'bytes', 'cache-control': 'no-store' };
      if (!m) { media.push(`200 ${path}`); res.writeHead(200, { ...h, 'content-length': body.length }); res.end(body); return; }
      const start = m[1] ? Number(m[1]) : Math.max(0, body.length - Number(m[2]));
      const end = m[1] && m[2] ? Math.min(Number(m[2]), body.length - 1) : body.length - 1;
      media.push(`206 ${path} ${start}-${end}`);
      res.writeHead(206, { ...h, 'content-range': `bytes ${start}-${end}/${body.length}`, 'content-length': end - start + 1 });
      res.end(body.subarray(start, end + 1));
      return;
    }
    const headers = { 'content-type': TYPES[extname(file)] || 'application/octet-stream', 'cache-control': 'no-store' };
    if (path === '/app/' || path === '/app/index.html') headers['content-security-policy'] = policy;
    res.writeHead(200, headers);
    res.end(readFileSync(file));
  });
  await new Promise((ok) => srv.listen(port, '127.0.0.1', ok));
  return { origin: `http://127.0.0.1:${port}`, policy, fetched, media, close: () => srv.close() };
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
  // Nothing is heard: its output at zero; autoplay allowed, so the app's
  // audio element can be seen playing.
  writeFileSync(join(profile, 'user.js'), 'user_pref("media.volume_scale", "0.0");\nuser_pref("media.autoplay.default", 0);\nuser_pref("media.autoplay.blocking_policy", 0);\nuser_pref("ui.systemUsesDarkTheme", 0);\n');
  const port = await freePort();
  const runtime = process.env.XDG_RUNTIME_DIR || `/run/user/${process.getuid()}`;
  const env = { ...process.env, XDG_RUNTIME_DIR: runtime, MOZ_HEADLESS: '1' };
  delete env.LD_LIBRARY_PATH; delete env.PIPEWIRE_REMOTE; delete env.PULSE_SERVER;
  // Away from the audio graph, or not at all.
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
    size: (width, height) => send('browsingContext.setViewport', { context, viewport: { width, height } }),
    shot: async (file) => { const r = await send('browsingContext.captureScreenshot', { context }); writeFileSync(file, Buffer.from(r.data, 'base64')); },
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
    // At the note's head: held at a share of the window, scrolling inside
    // (`ext_host::head_height`), however tall it says it is.
    const cap = Math.max(240, (await w.b.eval('innerHeight')) * 0.6);
    const held = await until(async () => {
      const h = parseFloat((await mark(w.b, 'transport')).height);
      return Math.abs(h - cap) < 1 && h;
    }, 5000, 'the frame to be held at the cap');
    assert.ok(Math.abs(held - cap) < 1, `${held} px, the cap ${cap}`);
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
    // At the note's head it is at most a share of the window, and scrolls
    // inside its own box (#657, #660): the note is not pushed away.
    const tall = JSON.parse(await b.eval(`JSON.stringify([document.querySelector('[data-testid=note-embed] iframe').getBoundingClientRect().height, innerHeight])`));
    assert.ok(tall[0] <= Math.max(240, tall[1] * 0.6) + 1, `a frame ${tall[0]} px tall in a window ${tall[1]} px`);
    assert.match(full.current, /^Setlists\/Merry\.md#\d+$/);
    const refused = JSON.parse(await b.eval('JSON.stringify(window.__refused)'));
    for (const why of ['not ready', 'media must be a vault path', 'at most 500 items']) assert.ok(refused.some((r) => r.includes(why)), `${why} in ${JSON.stringify(refused)}`);
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
// ── The Now Playing bar (#589) ──────────────────────────────────────

/// A queue view, as Studio's (studio#490 `queue.rs`) speaks #589 v0.1: it
/// sets its source and makes its song current, and writes what it was
/// handed last to its body.
const QUEUE = `
  let set = false;
  globalThis.architectPage.subscribe((p) => {
    document.body.dataset.props = JSON.stringify({ source: p.source, name: p.name, active: p.active, title: p.title,
      playing: p.playing, index: p.index, tracks: p.tracks, time: p.time, clock: p.clock, item: p.item && p.item.id, note: p.note && p.note.path });
    // Once shown (its first props come after ready): an ask before is
    // refused.
    if (set || p.tracks) return;
    set = true;
    globalThis.architectPage.ask("PLAYHEAD_SET", { name: "Demo", queue: [
      { id: "a", title: "Blue Moon Walk", subtitle: "Cody Wright", duration: 180 },
      { id: "b", title: "Slow River", subtitle: "Traditional", duration: 90 },
      { id: "c", title: "Amazing Grace", subtitle: "John Newton", media: "Recordings/Amazing Grace.mp3" },
    ] });
    globalThis.architectPage.ask("PLAYHEAD_ITEM", { id: "c" });
  });
`;

const BAR = `JSON.stringify((() => { const e = document.querySelector('[data-testid=now-playing]'); if (!e) return null;
  const q = (id) => (e.querySelector('[data-testid=' + id + ']') || {}).textContent || '';
  return { source: e.dataset.source, playing: e.dataset.playing === 'true', time: Number(e.dataset.time), index: Number(e.dataset.index),
    title: q('now-playing-title'), subtitle: q('now-playing-subtitle'), name: q('now-playing-source'), unheard: q('now-playing-unheard') }; })())`;
const bar = async (b) => JSON.parse(await b.eval(BAR));
const press = (b, id) => b.eval(`(() => { const e = document.querySelector('[data-testid=${id}]'); if (!e) return false; e.click(); return true; })()`);
const propsOf = async (b, view) => { const c = await frameOf(b, view); return c && JSON.parse((await b.evalIn(c, 'document.body.dataset.props || "null"')) || 'null'); };
const ask = async (b, view, action, args) => { const c = await frameOf(b, view); return b.evalIn(c, `globalThis.architectPage.ask(${JSON.stringify(action)}, ${JSON.stringify(args ?? null)}); true`); };
const clickText = (b, txt) => b.eval(`(() => { const all = [...document.querySelectorAll('*')].filter((e) => e.children.length === 0 && !e.closest('[data-testid=now-playing]') && e.textContent.trim() === ${JSON.stringify(txt)}); const e = all[all.length - 1]; if (!e) return false; (e.closest('button,[role=treeitem],[role=button],a,li,div') || e).click(); return true; })()`);
const ready1 = (b) => until(async () => (await marks(b)).filter((m) => m.state === 'ready').length === 1, 20000, 'the claimed frame ready');

test('the Now Playing bar shows the queue, its presses reach the frames and a frame\'s asks move it', { skip }, async () => {
  const w = await world({ progress: QUIET, transport: QUEUE, lyrics: QUIET }, 'transport');
  const { b } = w;
  try {
    const r = await fetch(`${w.f.origin}/app/`);
    assert.match(r.headers.get('content-security-policy'), /media-src 'self'/);
    await openSong(b, w.f.origin);
    await ready1(b);
    // The view's queue, its song current, in the bar.
    const shown = await until(async () => { const s = await bar(b); return s && s.title === 'Amazing Grace' && s; }, 10000, 'the bar');
    assert.equal(shown.source, 'demo.view');
    assert.match(shown.subtitle, /John Newton/);
    assert.match(shown.name, /Demo/);
    assert.equal(shown.playing, false);
    let p = await until(() => propsOf(b, 'transport'), 10000, 'the transport frame\'s props');
    p = await until(async () => { const q = await propsOf(b, 'transport'); return q && q.item === 'c' && q; }, 10000, 'its song current');
    assert.deepEqual([p.source, p.title, p.index, p.tracks, p.playing, p.note], ['demo.view', 'Amazing Grace', 2, 3, false, 'Songs/Amazing Grace.md']);

    // The bar's play reaches the frames, and the app's audio element
    // streams the recording with ranges.
    assert.ok(await press(b, 'now-playing-toggle'));
    await until(async () => (await propsOf(b, 'transport')).playing === true, 5000, 'the frame to hear play');
    // The bar reads the deck on its own beat: waited for, never assumed
    // (#660: this raced the frame's props in 2 runs of 3).
    await until(async () => (await bar(b)).playing === true, 5000, 'the bar playing');
    await until(() => b.eval(`(() => { const a = document.querySelector('[data-testid=now-playing-audio]'); return !!a && a.src.endsWith('/app/demo-recording.mp3') && !a.paused && a.currentTime > 0.3; })()`), 10000, 'the recording to play');
    assert.ok(w.f.media.some((m) => m.startsWith('206 /app/demo-recording.mp3')), `ranged: ${w.f.media.join(', ')}`);
    assert.equal(await b.eval('navigator.mediaSession && navigator.mediaSession.metadata ? navigator.mediaSession.metadata.title : null'), 'Amazing Grace');
    // What keeps time: the media element where the browser has a sound
    // output (macOS), the app's silent clock where the suite's isolation
    // leaves Firefox none (Linux: the sink fails and the app says the
    // song is not heard here, which is right).
    const clock = process.platform === 'linux' ? 'silent' : 'media';
    await until(async () => (await propsOf(b, 'transport')).clock === clock, 5000, `the ${clock} clock`);
    // Only the page's own element plays: no frame has one.
    for (const c of await b.frames()) assert.equal(await b.evalIn(c, 'document.querySelectorAll("audio,video").length'), 0);

    // A frame's asks move the bar.
    await ask(b, 'transport', 'PLAYHEAD_ITEM', { index: 0, play: true });
    await until(async () => (await bar(b)).title === 'Blue Moon Walk', 5000, 'the bar on the first song');
    // The bar's next and previous reach the frames.
    assert.ok(await press(b, 'now-playing-next'));
    await until(async () => (await propsOf(b, 'transport')).title === 'Slow River', 5000, 'the frames on the next song');
    assert.ok(await press(b, 'now-playing-previous'));
    await until(async () => (await propsOf(b, 'transport')).title === 'Blue Moon Walk', 5000, 'the frames on the previous song');
    await ask(b, 'transport', 'PLAYHEAD_PAUSE');
    await until(async () => (await bar(b)).playing === false, 5000, 'the bar paused');
    await ask(b, 'transport', 'PLAYHEAD_SEEK', { t: 30 });
    await until(async () => Math.abs((await bar(b)).time - 30) < 0.5, 5000, 'the bar at 30 s');
    assert.equal((await mark(b, 'transport')).refused, '');
    // A seek before its item's length is known is refused, past it clamped.
    await ask(b, 'transport', 'PLAYHEAD_SEEK', { t: 1e9 });
    await until(async () => Math.abs((await bar(b)).time - 180) < 0.5, 5000, 'the bar clamped at the end');

    // Media nobody can hear here and no length: not "playing" in silence.
    // Passed over to the next, or stopped at the last, and said so (#660 M4).
    await ask(b, 'transport', 'PLAYHEAD_SET', { name: 'Demo', queue: [{ id: 'u', title: 'Unheard', media: 'Private/elsewhere.mp3' }], play: true });
    const stopped = await until(async () => { const s = await bar(b); return s.title === 'Unheard' && !s.playing && s.unheard && s; }, 10000, 'the unplayable item stopped and named');
    assert.equal(stopped.unheard, "Can't be played");
    // Back to the queue for what follows.
    await ask(b, 'transport', 'PLAYHEAD_SET', { name: 'Demo', queue: [
      { id: 'a', title: 'Blue Moon Walk', subtitle: 'Cody Wright', duration: 180 },
      { id: 'b', title: 'Slow River', subtitle: 'Traditional', duration: 90 },
      { id: 'c', title: 'Amazing Grace', subtitle: 'John Newton', media: 'Recordings/Amazing Grace.mp3' },
    ], index: 2 });
    await until(async () => (await bar(b)).title === 'Amazing Grace', 5000, 'the queue back');

    if (process.env.SHOTS) {
      mkdirSync(process.env.SHOTS, { recursive: true });
      await sleep(400);
      await b.shot(join(process.env.SHOTS, 'now-playing-desk-firefox.png'));
      await b.size(390, 844);
      await until(async () => !!(await bar(b)), 5000, 'the bar at 390 px');
      await sleep(600);
      await b.shot(join(process.env.SHOTS, 'now-playing-390-firefox.png'));
      await press(b, 'now-playing-title');
      await sleep(400);
      await b.shot(join(process.env.SHOTS, 'now-playing-390-open-firefox.png'));
    }
  } finally { await w.close(); }
});

// #667 H1, M2, M3: a far time refused at the contract; `^` drawn whatever
// a source asks, a button the keyboard reaches; a frame's element in the
// scheme its document is handed when the person's theme is not the
// system's (the system light here, the person's choice dark).
test('a far time is refused, ^ is always there, and a frame keeps its document\'s scheme', { skip }, async () => {
  // A line of the frame's own text on its transparent page: what an
  // opaque backdrop would put a box behind.
  const shown = `${QUEUE}\n  document.body.style.margin = "0"; const p = document.createElement("p"); p.style.margin = "8px 0"; p.textContent = "The transport view, in its frame (transparent)"; document.body.appendChild(p);`;
  const w = await world({ progress: QUIET, transport: shown, lyrics: QUIET }, 'transport');
  const { b } = w;
  try {
    await openSong(b, w.f.origin);
    await ready1(b);
    await until(async () => (await bar(b))?.title === 'Amazing Grace', 10000, 'the bar');
    // The review's probe: 2e15 s long, at 1.36e15 s, 400 bpm, playing.
    await ask(b, 'transport', 'PLAYHEAD_SET', { name: 'Demo', play: true, time: 1.36e15,
      queue: [{ id: 'x', title: 'Forever', duration: 2e15, tempo: { bpm: 400 } }] });
    await until(async () => /seconds/.test((await mark(b, 'transport')).refused), 5000, 'the far time refused');
    assert.equal((await bar(b)).title, 'Amazing Grace');

    // Only Click asked: `^` is drawn after it, a real button.
    await ask(b, 'transport', 'PLAYHEAD_SET', { name: 'Demo', slots: ['click'], queue: [
      { id: 'a', title: 'Blue Moon Walk', subtitle: 'Cody Wright', duration: 180, tempo: { bpm: 96 } },
      { id: 'c', title: 'Amazing Grace', subtitle: 'John Newton', media: 'Recordings/Amazing Grace.mp3' },
    ] });
    await until(() => b.eval(`!!document.querySelector('[data-testid=now-playing-click]')`), 5000, 'Click drawn');
    const open = await until(() => b.eval(`(() => { const e = document.querySelector('[data-testid=now-playing-open]'); if (!e) return null;
      e.focus(); return JSON.stringify({ tag: e.tagName, focused: document.activeElement === e, after: !!e.previousElementSibling && e.previousElementSibling.dataset.testid }); })()`), 5000, '^ drawn').then(JSON.parse);
    assert.deepEqual(open, { tag: 'BUTTON', focused: true, after: 'now-playing-click' });

    // The person's theme dark, the system light: the frame's element and
    // its document both dark.
    const schemesNow = async () => {
      const c = await frameOf(b, 'transport');
      const el = await b.eval(`getComputedStyle(document.querySelector('[data-testid=ext-frame] iframe')).colorScheme`);
      const doc = await b.evalIn(c, 'getComputedStyle(document.documentElement).colorScheme');
      return `${el}|${doc}`;
    };
    await until(async () => (await schemesNow()) === 'light|light', 10000, 'the frame light with the system');
    // Picked by hand while the frame is up: Settings, Appearance, Dark.
    assert.ok(await b.eval(`(() => { const e = document.querySelector('[aria-label=Settings]'); if (!e) return false; e.click(); return true; })()`));
    await until(() => clickText(b, 'Appearance'), 10000, 'the Appearance page');
    await sleep(300);
    await until(() => b.eval(`(() => { const e = [...document.querySelectorAll('button span')].find((s) => s.children.length === 0 && s.textContent.trim() === 'Dark'); if (!e) return false; e.closest('button').click(); return true; })()`), 10000, 'the Dark mode card');
    await until(() => b.eval(`(() => { const d = document.querySelector('[aria-label="Close settings"]'); if (!d) return false; d.click(); return true; })()`), 5000, 'settings closed');
    assert.equal(await b.eval('matchMedia("(prefers-color-scheme: dark)").matches'), false, 'the system is light');
    await until(async () => (await schemesNow()) === 'dark|dark', 10000, 'the frame dark, element and document');

    if (process.env.SHOTS) {
      mkdirSync(process.env.SHOTS, { recursive: true });
      await until(async () => (await bar(b))?.title === 'Amazing Grace', 10000, 'the bar');
      const frameInView = `(() => { const e = document.querySelector('[data-testid=note-embed]'); if (e) e.scrollIntoView({ block: 'center' }); return !!e; })()`;
      await b.eval(frameInView);
      await sleep(600);
      await b.shot(join(process.env.SHOTS, 'm3-desk-dark-on-light-system.png'));
      await b.size(390, 844);
      await sleep(800);
      await b.eval(frameInView);
      await sleep(300);
      await b.shot(join(process.env.SHOTS, 'm2-m3-390-dark-on-light-system.png'));
      await press(b, 'now-playing-open');
      await sleep(500);
      await b.shot(join(process.env.SHOTS, 'm2-390-detail-open.png'));
    }
  } finally { await w.close(); }
});

test('two sources coexist, the bar switches between them, and a frame moves only its own', { skip }, async () => {
  const w = await world({ progress: QUIET, transport: QUEUE, lyrics: QUIET }, 'transport');
  const { b } = w;
  try {
    await openSong(b, w.f.origin);
    await ready1(b);
    await until(async () => (await bar(b))?.source === 'demo.view', 10000, 'the song in the bar');
    // A note's recording: the app's own source, played from the note.
    await until(() => clickText(b, 'Inbox'), 10000, 'the Inbox folder');
    await sleep(300);
    await until(async () => (await clickText(b, 'Voice memo: chorus melody')) || clickText(b, 'Voice memo chorus melody'), 10000, 'the voice memo');
    await until(() => press(b, 'note-recording-play'), 10000, 'its recording\'s play');
    const memo = await until(async () => { const s = await bar(b); return s.source === 'app:notes' && s.playing && s; }, 5000, 'the memo playing in the bar');
    assert.match(memo.title, /Voice memo/);
    assert.match(memo.name, /Notes/);
    // The switcher lists both; the song comes back paused where it was.
    assert.ok(await press(b, 'now-playing-source'));
    const listed = await until(() => b.eval(`JSON.stringify([...document.querySelectorAll('[data-testid=now-playing-sources] [data-source]')].map((e) => e.dataset.source))`).then(JSON.parse).then((l) => l.length && l), 5000, 'the sources');
    assert.deepEqual(listed.sort(), ['app:notes', 'demo.view']);
    assert.ok(await b.eval(`(() => { document.querySelector('[data-testid=now-playing-sources] [data-source="demo.view"]').click(); return true; })()`));
    await until(async () => { const s = await bar(b); return s.source === 'demo.view' && !s.playing; }, 5000, 'the song shown, paused');

    // Back on the song: its frames move their own source only.
    await until(() => clickText(b, 'Songs'), 10000, 'the Songs folder');
    await sleep(300);
    await until(() => clickText(b, 'Amazing Grace'), 10000, 'the song note');
    await ready1(b);
    await ask(b, 'transport', 'PLAYHEAD_PLAY', { source: 'app:notes' });
    await until(async () => (await mark(b, 'transport')).refused === 'not your source', 5000, 'another\'s source refused');
    await ask(b, 'transport', 'PLAYHEAD_ACTIVATE', { source: 'demo.view' });
    await until(async () => (await mark(b, 'transport')).refused === 'not an ask a frame makes', 5000, 'switching refused');
    await ask(b, 'transport', 'PLAYHEAD_SET', { queue: [{ id: 'x', title: 'X', media: 'https://evil.test/x.mp3' }] });
    await until(async () => /vault path/.test((await mark(b, 'transport')).refused), 5000, 'a URL refused as media');
    await ask(b, 'transport', 'PLAYHEAD_PLAY');
    await until(async () => { const s = await bar(b); return s.source === 'demo.view' && s.playing; }, 5000, 'its own playing');
    // The memo was not moved by any of it, and is paused now the song plays.
    assert.ok(await press(b, 'now-playing-source'));
    await until(() => b.eval(`(() => { const e = document.querySelector('[data-testid=now-playing-sources] [data-source="app:notes"]'); if (!e) return false; e.click(); return true; })()`), 5000, 'the memo in the switcher');
    await until(async () => { const s = await bar(b); return s.source === 'app:notes' && !s.playing && /Voice memo/.test(s.title); }, 5000, 'the memo shown, paused');
    // The rate limit holds for the new asks too.
    const c = await frameOf(b, 'transport');
    await b.evalIn(c, 'for (let i = 0; i < 100; i++) globalThis.architectPage.ask("PLAYHEAD_ITEM", { index: i % 3 }); true');
    await until(async () => (await mark(b, 'transport')).refused === 'too many requests', 5000, 'the flood refused');
    if (process.env.SHOTS) {
      mkdirSync(process.env.SHOTS, { recursive: true });
      assert.ok(await press(b, 'now-playing-source'));
      await sleep(400);
      await b.shot(join(process.env.SHOTS, 'now-playing-desk-sources-firefox.png'));
    }
  } finally { await w.close(); }
});

// ── The demo's Christmas setlist ────────────────────────────────────

/// A setlist view: the note's ```studio-setlist rows with a recording as
/// its source, each with its cover (the row's tenth field) as `art`; what
/// it was handed of their art written to its body.
const SETLIST = `
  let set = false;
  const svg = (i) => !!i && typeof i.art_src === 'string' && i.art_src.startsWith('data:image/svg+xml;base64,');
  globalThis.architectPage.subscribe((p) => {
    document.body.dataset.props = JSON.stringify({ tracks: p.tracks, item: p.item && p.item.id,
      itemArt: svg(p.item), queueArt: (p.queue || []).filter(svg).length });
    if (set || p.tracks || !p.note || !p.note.body) return;
    set = true;
    const rows = p.note.body.split('\\n').map((l) => l.split('|').map((f) => f.trim()))
      .filter((f) => f.length >= 10 && /^[0-9]+$/.test(f[0]) && f[4]);
    globalThis.architectPage.ask("PLAYHEAD_SET", { name: p.note.title, queue: rows.map((f) => (
      { id: f[2], title: f[1], subtitle: f[5], duration: Number(f[6]), media: f[4], art: f[9] })) });
  });
`;

/// Opens the navigator's folder `name` in its tree (its chevron: a press
/// on its name selects it).
const expand = (b, name) => b.eval(`(() => { const row = [...document.querySelectorAll('[role=treeitem]')].find((r) => [...r.querySelectorAll('*')].some((e) => e.children.length === 0 && e.textContent.trim() === ${JSON.stringify(name)})); const c = row && row.querySelector('.nv-chevron'); if (!c) return false; if (row.getAttribute('aria-expanded') !== 'true') c.click(); return true; })()`);

const artSrc = (b) => b.eval(`document.querySelector('[data-testid=now-playing-art] img')?.getAttribute('src') || ''`);

test('the demo setlist: each song\'s cover reaches its frame as data and the card draws it', { skip }, async () => {
  const n = await node(null, null, [{ id: 'demo.view', bundle: glue({ progress: QUIET, transport: SETLIST, lyrics: QUIET }), embeds: 'transport=setlist=', reads: 'documents' }]);
  const f = await front(n.origin);
  const b = await firefox();
  try {
    await openNote(b, f.origin, ['Setlists'], 'Merry & Bright');
    // Five of its nine songs have a recording; each queued with its cover.
    const p = await until(async () => { const q = await propsOf(b, 'transport'); return q && q.tracks === 5 && q; }, 15000, 'the five songs queued');
    assert.equal(p.queueArt, 5, 'every queued song handed its cover');
    // The island claims the setlist's type: it draws, not the core list
    // (#668), and the list's own rows stay in the property table.
    assert.equal(await b.eval(`!!document.querySelector('[data-testid=media-list]')`), false, 'no core list under a claim');
    assert.ok((await b.eval(`[...document.querySelectorAll('[data-testid=note-properties] dt')].map((d) => d.textContent).join(',')`)).includes('sets'));
    await ask(b, 'transport', 'PLAYHEAD_ITEM', { id: 'Libraries/Christmas/Silent Night/Silent Night.md', play: true });
    const s = await until(async () => { const s = await bar(b); return s && s.title === 'Silent Night' && s; }, 10000, 'the card on Silent Night');
    assert.match(s.subtitle, /for KING & COUNTRY/);
    assert.match(await artSrc(b), /^data:image\/svg\+xml;base64,/);
    await until(async () => (await propsOf(b, 'transport')).itemArt, 5000, 'the current song\'s cover in the props');
    await until(() => b.eval(`document.querySelector('[data-testid=now-playing-audio]')?.src.endsWith('/app/demo-recording.mp3')`), 5000, 'the demo recording');
    // Art that is not the demo vault's image is never drawn.
    await ask(b, 'transport', 'PLAYHEAD_SET', { name: 'X', queue: [{ id: 'x', title: 'X', duration: 10, art: 'Attachments/Brief.pdf' }], play: true });
    await until(async () => (await bar(b)).title === 'X', 5000, 'the card on X');
    assert.equal(await artSrc(b), '');
  } finally { await b.close(); f.close(); n.close(); }
});

test('a demo song note plays its recording on the card under its artist, with its cover', { skip }, async () => {
  const w = await world({ progress: QUIET, transport: QUIET, lyrics: QUIET }, 'lyrics');
  const { b } = w;
  try {
    await b.go(`${w.f.origin}/app/?demo`);
    await until(() => b.eval(`!!document.querySelector('[data-testid=note-text]')`), 60000, 'the demo');
    for (const folder of ['Libraries', 'Christmas']) await until(() => expand(b, folder), 10000, `the folder ${folder} expanded`);
    await openNote(b, w.f.origin, ['Sleigh Ride'], 'Sleigh Ride', false);
    await until(() => press(b, 'note-recording-play'), 10000, 'the note\'s Play recording');
    const s = await until(async () => { const s = await bar(b); return s && s.title === 'Sleigh Ride' && s; }, 10000, 'the card on Sleigh Ride');
    assert.match(s.subtitle, /Tori Kelly/);
    assert.match(await artSrc(b), /^data:image\/svg\+xml;base64,/);
    if (process.env.SHOTS) {
      mkdirSync(process.env.SHOTS, { recursive: true });
      for (const [width, name] of [[390, 'phone-390'], [1280, 'desk-1280']]) {
        await b.size(width, width === 390 ? 844 : 900);
        await sleep(700);
        await b.shot(join(process.env.SHOTS, `song-note-recording-${name}.png`));
      }
    }
  } finally { await w.close(); }
});

const STUDIO = process.env.STUDIO_ISLAND;
const ROWS = `JSON.stringify([...document.querySelectorAll('.sl-row')].map((r) => ({ name: r.querySelector('.sl-name')?.textContent, disabled: r.getAttribute('aria-disabled') === 'true', current: r.hasAttribute('data-current') })))`;

test('Studio\'s setlist view draws the demo setlist and a row pressed plays on the card', { skip: skip || (STUDIO ? false : 'needs STUDIO_ISLAND=<Studio `just island-web` folder>') }, async () => {
  const n = await node(null, null, [{ id: 'studio.transport', folder: STUDIO, sheet: 'island.css', embeds: 'setlist.playlist=setlist=date', reads: 'documents' }]);
  const f = await front(n.origin);
  const b = await firefox();
  const shots = process.env.SHOTS;
  try {
    // Opened on a desk (a phone's navigator is a drawer), then seen at
    // 390 px too.
    await openNote(b, f.origin, ['Setlists'], 'Merry & Bright');
    // Its one frame, once the host says it is ready.
    await until(async () => (await mark(b, 'setlist.playlist'))?.state === 'ready', 20000, 'Studio\'s setlist view ready');
    // The frame drawing the rows (its context, found again if it is
    // rebuilt).
    const view = async () => { for (const f of await b.frames()) { const r = JSON.parse(await b.evalIn(f, ROWS).catch(() => '[]')); if (r.length) return { f, r }; } return null; };
    const { r: rows } = await until(async () => { const v = await view(); return v && v.r.length === 9 && v; }, 20000, 'nine rows');
    // A press on the row named `name`, in the frame as it is now.
    const pressRow = async (name) => { const v = await until(view, 10000, 'the view'); return b.evalIn(v.f, `[...document.querySelectorAll('.sl-row')].find((r) => r.querySelector('.sl-name').textContent === ${JSON.stringify(name)}).click(); true`); };
    assert.deepEqual(rows.map((r) => r.name), ['Sleigh Ride', 'Silent Night', 'Joy to the World (Unspeakable Joy)', 'Feliz Navidad', 'What Christmas Means', 'This Christmas', "Rockin' Around the Christmas Tree", 'Last Christmas', 'All I Want for Christmas Is You']);
    assert.deepEqual(rows.filter((r) => r.disabled).map((r) => r.name), ['Feliz Navidad', 'What Christmas Means', "Rockin' Around the Christmas Tree", 'Last Christmas']);
    const shot = async (file) => { if (!shots) return; mkdirSync(shots, { recursive: true }); await sleep(900); await b.shot(join(shots, file)); };
    await shot('studio-setlist-desk-1280.png');
    await b.size(390, 844);
    await shot('studio-setlist-phone-390.png');
    // A greyed row asks nothing; a row with a recording plays.
    await until(async () => (await view())?.r.length === 9, 20000, 'the rows at 390 px');
    await pressRow('Feliz Navidad');
    await sleep(600);
    assert.notEqual((await bar(b))?.title, 'Feliz Navidad');
    await pressRow('This Christmas');
    const s = await until(async () => { const s = await bar(b); return s && s.title === 'This Christmas' && s.playing && s; }, 10000, 'the card playing This Christmas');
    assert.match(s.subtitle, /Donny Hathaway/);
    await until(async () => (await view())?.r.find((r) => r.current)?.name === 'This Christmas', 5000, 'its row marked');
    await shot('studio-setlist-playing-phone-390.png');
    await b.size(1280, 900);
    await shot('studio-setlist-playing-desk-1280.png');
    await press(b, 'now-playing-toggle');
  } finally { await b.close(); f.close(); n.close(); }
});
