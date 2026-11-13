// The bottom slots as favourites in the web app (#607), in a real
// browser at a phone's 390 px: `/app/?demo` shows the web app's own set
// (Vault, Files, Search, New note); opening the demo vault's song note
// (`type: song`, a fixture) switches the slots to what a demo extension
// declares for that type (`[[favourites]]`, served by the node at
// `/_ext/favourites.json` with the extension that declared them, here
// `island_node`'s ARCHITECT_FAVOURITES), a tab the app cannot open (a share
// page's `player.now`) left out; a tab opens its island's view in a frame
// of its own, handed the note; a `{place}` keeps the set while its note is
// on show; a note of no declared type brings the default back; on a desk
// the sidebar's tabs are the same set, the current one marked. A
// declaration by an extension that names only what it may not (another
// extension's views, an action outside its capabilities) is no set at all.
//
// The release bundle (scripts/web-bundle.sh, ARCHITECT_WEB_BUNDLE) behind
// a front that does what production's proxy does, `island_node`
// (ARCHITECT_ISLAND_NODE) with an island whose views draw their props.
// Firefox over WebDriver BiDi (FIREFOX=… or `firefox` on the PATH), in a
// mount namespace whose runtime dir and /run/pipewire are empty (`unshare
// -r -m`). `just app-favourites-test` builds and runs it; with
// ARCHITECT_SHOTS=<dir> it keeps a screenshot of each step there.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync, readFileSync, existsSync, statSync } from 'node:fs';
import { createServer, request } from 'node:http';
import { createServer as createNetServer } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join, extname, normalize } from 'node:path';

const NODE = process.env.ARCHITECT_ISLAND_NODE;
const BUNDLE = process.env.ARCHITECT_WEB_BUNDLE;
const SHOTS = process.env.ARCHITECT_SHOTS;
function which(cmd) {
  try { return execFileSync('sh', ['-c', `command -v ${cmd}`]).toString().trim() || null; } catch { return null; }
}
const browser = process.env.FIREFOX || which('firefox');
const skip = NODE && BUNDLE && browser ? false : 'needs ARCHITECT_ISLAND_NODE, ARCHITECT_WEB_BUNDLE (just app-favourites-test) and a Firefox (FIREFOX=…)';
if (process.env.ARCHITECT_BROWSER_REQUIRED && skip) throw new Error(`ARCHITECT_BROWSER_REQUIRED: ${skip}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const put = (root, rel, body) => { const p = join(root, rel); mkdirSync(dirname(p), { recursive: true }); writeFileSync(p, body); };
const freePort = () => new Promise((ok) => { const srv = createNetServer(); srv.listen(0, '127.0.0.1', () => { const { port } = srv.address(); srv.close(() => ok(port)); }); });
const WASM = Buffer.from([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]);

/// Each view draws its name and what it is handed: the note's title and
/// body, and its playhead's time.
const glue = `
export default async function init() {}
export function mount(view) {
  document.body.dataset.view = view;
  document.body.style.cssText = "margin:0;font:15px system-ui;color:var(--foreground, #222);background:var(--background, #fff)";
  globalThis.architectPage.subscribe((p) => {
    const note = p.note || {};
    document.body.dataset.note = note.path || "";
    document.body.dataset.keys = Object.keys(p).sort().join(",");
    const box = document.createElement("div");
    box.style.cssText = "padding:12px 16px";
    const h = document.createElement("h2");
    h.style.cssText = "margin:0 0 8px;font-size:17px";
    h.textContent = "Demo extension: " + view;
    box.appendChild(h);
    const b = document.createElement("p"); b.style.margin = "4px 0"; b.textContent = (note.title || "") + ": " + (note.body || "").slice(0, 120); box.appendChild(b);
    const t = document.createElement("p"); t.style.cssText = "margin:10px 0 0;opacity:.7"; t.textContent = "at " + Number(p.time || 0).toFixed(1) + "s"; box.appendChild(t);
    document.body.replaceChildren(box);
  });
}`;

/// What the demo extension (the island's own, `demo.view`) declares for
/// the demo vault's song notes: the note, a view of its island's, a share
/// page's player view this app does not have (left out), a note beside
/// the song, and the navigator; its primary action is quick open.
const FAVOURITES = `
[by]
extension = "demo.view"

[[favourites]]
note_type = "song"

[[favourites.tabs]]
id = "song"
label = "Song"
icon = "music"
target = { view = "page" }

[[favourites.tabs]]
id = "now"
label = "Now"
icon = "play"
target = { view = "player.now" }

[[favourites.tabs]]
id = "lyrics"
label = "Lyrics"
icon = "mic-vocal"
target = { view = "lyrics" }

[[favourites.tabs]]
id = "chords"
label = "Chords"
icon = "guitar"
target = { place = "Chords" }

[[favourites.tabs]]
id = "files"
label = "Songs"
icon = "list-music"
target = { view = "browse" }

[favourites.primary]
id = "find"
label = "Find"
icon = "search"
target = { view = "search" }
state = "playing"
on = { label = "Pause", icon = "pause" }
`;

/// Another extension's declaration for the same type: the island's views
/// (bare, and by the island's id), an app action its capabilities do not
/// reach. None of it is its own to name, so it is no set at all.
const FOREIGN = `
[by]
extension = "acme.other"

[[favourites]]
note_type = "song"

[[favourites.tabs]]
id = "words"
label = "Words"
icon = "mic-vocal"
target = { view = "lyrics" }

[[favourites.tabs]]
id = "deck"
label = "Deck"
icon = "sliders-horizontal"
target = { view = "demo.view/transport" }

[[favourites.tabs]]
id = "write"
label = "Write"
icon = "square-pen"
target = { action = "VAULT_NEW_NOTE" }

[favourites.primary]
id = "settings"
label = "Settings"
icon = "settings"
target = { action = "APP_SETTINGS" }
`;

async function node(favourites = FAVOURITES) {
  const dir = mkdtempSync(join(tmpdir(), 'app-fav-'));
  const vault = join(dir, 'vault');
  put(vault, 'Welcome.md', '# Welcome\n');
  const folder = join(dir, 'island');
  put(folder, 'web-view.js', glue);
  put(folder, 'web-view_bg.wasm', WASM);
  put(folder, 'architect.json', JSON.stringify({ module: 'web-view.js', manifest: { id: 'demo.view', capabilities: {} } }));
  put(dir, 'favourites.toml', favourites);
  const port = await freePort();
  const proc = spawn(NODE, [], {
    stdio: 'ignore',
    env: {
      ...process.env, ARCHITECT_BIND: `127.0.0.1:${port}`, ARCHITECT_DATA_DIR: join(dir, 'data'), ARCHITECT_VAULTS: `notes=${vault}`,
      ARCHITECT_IROH: 'local', ARCHITECT_NAME: 'app-favourites-test', ARCHITECT_ISLAND_FOLDER: folder, ARCHITECT_ISLAND_ID: 'demo.view',
      ARCHITECT_ISLAND_VIEW: 'progress', ARCHITECT_ISLAND_TRANSPORT: 'transport', ARCHITECT_ISLAND_LYRICS: 'lyrics',
      ARCHITECT_ISLAND_EMBEDS: 'progress=song', ARCHITECT_FAVOURITES: join(dir, 'favourites.toml'),
    },
  });
  const origin = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 160; i++) {
    await sleep(250);
    const ok = await fetch(`${origin}/_ext/favourites.json`).then((r) => r.ok && r.json()).then((j) => j && j.favourites.length === 1, () => false);
    if (ok) return { origin, close: () => { proc.kill(); rmSync(dir, { recursive: true, force: true }); } };
  }
  proc.kill();
  throw new Error('the node never listed its favourites');
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

// ── The browser ─────────────────────────────────────────────────────

async function firefox() {
  const profile = mkdtempSync(join(tmpdir(), 'app-fav-ff-'));
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
  await send('browsingContext.setViewport', { context, viewport: { width: 390, height: 844 } });
  const evalIn = async (ctx, expression) => {
    const r = await send('script.evaluate', { expression, target: { context: ctx }, awaitPromise: true, userActivation: true });
    if (r.type === 'exception') throw new Error(r.exceptionDetails.text);
    return r.result.value;
  };
  return {
    go: (url) => send('browsingContext.navigate', { context, url, wait: 'complete' }),
    press: (key) => send('input.performActions', { context, actions: [{ type: 'key', id: 'keys', actions: [{ type: 'keyDown', value: key }, { type: 'keyUp', value: key }] }] }),
    viewport: (width, height) => send('browsingContext.setViewport', { context, viewport: { width, height } }),
    shot: async (name) => {
      if (!SHOTS) return;
      const { data } = await send('browsingContext.captureScreenshot', { context });
      mkdirSync(SHOTS, { recursive: true });
      writeFileSync(join(SHOTS, `${name}.png`), Buffer.from(data, 'base64'));
    },
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


const SLOTS = `JSON.stringify({
  tabs: [...document.querySelectorAll('button[data-tab]')].map((b) => b.getAttribute('aria-label')),
  current: (document.querySelector('button[data-tab][aria-current=page]') || {}).dataset?.tab || null,
  primary: (document.querySelector('button[data-primary]') || { getAttribute: () => null }).getAttribute('aria-label'),
})`;
const slots = async (b) => JSON.parse(await b.eval(SLOTS));
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

/// Presses the last leaf element whose text is `txt` (a navigator row).
const click = (b, txt) => b.eval(`(() => { const all = [...document.querySelectorAll('*')].filter((e) => e.children.length === 0 && e.textContent.trim() === ${JSON.stringify(txt)}); const e = all[all.length - 1]; if (!e) return false; (e.closest('button,[role=treeitem],[role=button],a,li,div') || e).click(); return true; })()`);
const tab = (b, label) => b.eval(`(() => { const t = [...document.querySelectorAll('button[data-tab]')].find((x) => x.getAttribute('aria-label') === ${JSON.stringify(label)}); if (!t) return false; t.click(); return true; })()`);

/// A note from the navigator, in a phone's drawer: the web app's Files
/// tab, or the declared set's (`browse`, "Songs").
async function openNote(b, folder, name) {
  await until(async () => (await tab(b, 'Files')) || tab(b, 'Songs'), 10000, 'a Files tab');
  await sleep(400);
  if (folder) { await until(() => click(b, folder), 10000, `the ${folder} folder`); await sleep(300); }
  await until(() => click(b, name), 10000, `the note ${name}`);
  await sleep(400);
}

test('the slots follow the open note\'s type at 390 px, and a tab opens its view', { skip }, async () => {
  const n = await node();
  const f = await front(n.origin);
  const b = await firefox();
  try {
    await b.go(`${f.origin}/app/?demo`);
    await until(() => b.eval(`!!document.querySelector('[data-testid=note-text]')`), 60000, 'the demo');
    // The web app's own: what it can open, and New note.
    const own = { tabs: ['Vault', 'Files', 'Search'], current: 'vault', primary: 'New note' };
    await until(async () => same(await slots(b), own), 10000, 'the default slots');
    await b.shot('phone-default');

    // The song note: the type's declared set, `player.now` left out; the
    // view its island claims the type with at the note's head.
    await openNote(b, 'Songs', 'Amazing Grace');
    const declared = { tabs: ['Song', 'Lyrics', 'Chords', 'Songs'], current: 'song:song', primary: 'Find' };
    await until(async () => same(await slots(b), declared), 15000, 'the declared slots');
    await until(() => b.eval(`!!document.querySelector('[data-testid=note-embed][data-view=progress] [data-testid=ext-frame][data-ext-state=ready]')`), 20000, 'the claimed view at the note\'s head');
    await b.shot('phone-declared');

    // Lyrics: the island's lyrics view in a frame of its own, handed the
    // note and its playhead as data; the slots stay the type's, Lyrics
    // current.
    assert.ok(await tab(b, 'Lyrics'));
    await until(() => b.eval(`!!document.querySelector('[data-testid=ext-view-pane][data-ext-view-of=lyrics] [data-testid=ext-frame][data-ext-state=ready]')`), 20000, 'the lyrics frame ready');
    await until(async () => (await slots(b)).current === 'song:lyrics', 10000, 'Lyrics current');
    assert.deepEqual((await slots(b)).tabs, declared.tabs);
    const lyrics = await until(async () => { for (const c of await b.frames()) { if ((await b.evalIn(c, 'document.body && document.body.dataset.view').catch(() => null)) === 'lyrics') return c; } return null; }, 10000, 'the lyrics frame');
    const handed = await until(() => b.evalIn(lyrics, 'document.body.dataset.note && JSON.stringify([document.body.dataset.note, document.body.dataset.keys])'), 10000, 'its props');
    const [notePath, keys] = JSON.parse(handed);
    assert.equal(notePath, 'Songs/Amazing Grace.md');
    for (const k of ['note', 'playing', 'time', 'queue', 'source']) assert.ok(keys.split(',').includes(k), `${k} in ${keys}`);
    assert.ok(!keys.split(',').includes('lyrics'), 'no song-shaped props');
    const frames = JSON.parse(await b.eval(`JSON.stringify([...document.querySelectorAll('iframe')].map((f) => [f.getAttribute('sandbox'), new URL(f.src).pathname]))`));
    assert.ok(frames.length >= 1);
    for (const [sandbox, path] of frames) {
      assert.equal(sandbox, 'allow-scripts');
      assert.match(path, /^\/_ext\/webview\.[0-9a-f]+\.html$/);
    }
    await sleep(800);
    await b.shot('phone-lyrics');

    // Song: back to the note itself.
    assert.ok(await tab(b, 'Song'));
    await until(async () => (await slots(b)).current === 'song:song', 10000, 'Song current');

    // Chords, a note beside the song with no type: it opens, and the set
    // stays the song's while it is on show, Chords current.
    assert.ok(await tab(b, 'Chords'));
    await until(() => b.eval(`!!document.querySelector('[data-note="Songs/Chords.md"]')`), 10000, 'the Chords note');
    await until(async () => same(await slots(b), { ...declared, current: 'song:chords' }), 10000, 'the song\'s set kept, Chords current');
    await b.shot('phone-place');
    assert.ok(await tab(b, 'Song'));
    await until(async () => same(await slots(b), declared), 10000, 'back on the song');

    // The primary action: quick open.
    assert.ok(await b.eval(`(() => { const p = document.querySelector('button[data-primary]'); p.click(); return true; })()`));
    await until(() => b.eval(`!!document.activeElement && document.activeElement.tagName === 'INPUT'`), 10000, 'quick open, its field focused');
    await b.press('\uE00C');
    // Its field was a typing field: the shell's Hide the keyboard brings
    // the bar back, as on a phone.
    await sleep(300);
    await b.eval(`(() => { const h = document.querySelector('[aria-label="Hide the keyboard"]'); if (h) h.click(); return true; })()`);
    await until(async () => (await slots(b)).tabs.length > 0 && !(await b.eval(`document.activeElement && document.activeElement.tagName === 'INPUT'`)), 10000, 'quick open closed');

    // A note of no declared type: the default again.
    await openNote(b, null, 'Welcome');
    await until(async () => same(await slots(b), own), 15000, 'the default slots again');

    // A desk: the same sets, in the sidebar.
    await b.viewport(1280, 900);
    await b.go(`${f.origin}/app/?demo`);
    await until(() => b.eval(`!!document.querySelector('[data-testid=note-text]')`), 60000, 'the demo on a desk');
    await sleep(800);
    await b.shot('desk-default');
    await until(() => click(b, 'Songs'), 10000, 'the Songs folder on a desk');
    await sleep(300);
    await until(() => click(b, 'Amazing Grace'), 10000, 'the song note on a desk');
    await until(() => b.eval(`!!document.querySelector('[data-testid=note-embed]')`), 15000, 'the song note on a desk');
    // The sidebar's tabs are the declared set, the note's tab current.
    await until(async () => { const s = await slots(b); return s.tabs.includes('Lyrics') && s.current === 'song:song'; }, 15000, 'the declared set on a desk, Song current');
    await sleep(1500);
    await b.shot('desk-declared');
  } finally { await b.close(); f.close(); n.close(); }
});

test('a set naming only what its extension may not is no set, and the listing is read-only', { skip }, async () => {
  const n = await node(FOREIGN);
  const f = await front(n.origin);
  const b = await firefox();
  try {
    // The node lists it, with who declared it; HEAD answers, POST does not.
    const listed = await fetch(`${n.origin}/_ext/favourites.json`).then((r) => r.json());
    assert.equal(listed.favourites[0].by.extension, 'acme.other');
    const head = await fetch(`${n.origin}/_ext/favourites.json`, { method: 'HEAD' });
    assert.equal(head.status, 200);
    assert.equal(head.headers.get('content-type'), 'application/json');
    assert.equal((await head.text()).length, 0);
    assert.equal((await fetch(`${n.origin}/_ext/favourites.json`, { method: 'POST' })).status, 405);

    await b.go(`${f.origin}/app/?demo`);
    await until(() => b.eval(`!!document.querySelector('[data-testid=note-text]')`), 60000, 'the demo');
    const own = { tabs: ['Vault', 'Files', 'Search'], current: 'vault', primary: 'New note' };
    await until(async () => same(await slots(b), own), 10000, 'the default slots');
    await openNote(b, 'Songs', 'Amazing Grace');
    await until(() => b.eval(`!!document.querySelector('[data-note="Songs/Amazing Grace.md"]')`), 10000, 'the song note');
    // Another extension's views and an action it may not run: every tab
    // left out, so the song keeps the default set and its round button.
    await sleep(1500);
    assert.deepEqual(await slots(b), own);
    await b.shot('phone-foreign');
  } finally { await b.close(); f.close(); n.close(); }
});
