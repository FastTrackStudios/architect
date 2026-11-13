// What a share link's guest's view reads in the web app (#648, #669), in a
// real browser: the release bundle (ARCHITECT_WEB_BUNDLE) behind a front
// that does what production's proxy does (the app's files with its
// Content-Security-Policy from apps/web/sws.toml; `/_ext/` and `/s/` to the
// node), a node (`island_node`, ARCHITECT_ISLAND_NODE) sharing a gig's
// folder (a share note seeded in its vault) and drawing setlists with an
// island whose `list` view reports what it was handed.
//
// `/app/#share=<code>`, the app as the link's guest (the internal entry
// #646 builds on), at 390 px wide:
//
// - with `reads = ["documents", "files"]`: the setlist's view is handed its
//   text as the link's build wrote it (no `%%comments%%`, no link past the
//   link, no frontmatter, no vault path) and the link's art and lyrics
//   (an image as a `data:` URL, `.lrc` as text), nothing outside the link;
// - with `documents` alone: the text, no files;
// - declaring nothing: no note at all;
// - the build's `_note.json` is never a file of the link.
//
// Firefox over WebDriver BiDi (FIREFOX=… or `firefox` on the PATH), in a
// mount namespace whose runtime dir and /run/pipewire are empty (`unshare
// -r -m`), as app-ext-host.test.mjs runs it. SHOTS=<dir> writes the
// screenshots there. `just app-guest-content-test` builds and runs it.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync, readFileSync, existsSync, statSync } from 'node:fs';
import { createServer, request } from 'node:http';
import { createServer as createNetServer } from 'node:net';
import { tmpdir } from 'node:os';
import { crc32, deflateSync } from 'node:zlib';
import { dirname, join, extname, normalize } from 'node:path';

const NODE = process.env.ARCHITECT_ISLAND_NODE;
const BUNDLE = process.env.ARCHITECT_WEB_BUNDLE;
const SHOTS = process.env.SHOTS;
function which(cmd) {
  try { return execFileSync('sh', ['-c', `command -v ${cmd}`]).toString().trim() || null; } catch { return null; }
}
const browser = process.env.FIREFOX || which('firefox');
const skip = NODE && BUNDLE && browser ? false : 'needs ARCHITECT_ISLAND_NODE, ARCHITECT_WEB_BUNDLE (just app-guest-content-test) and a Firefox (FIREFOX=…)';
if (process.env.ARCHITECT_BROWSER_REQUIRED && skip) throw new Error(`ARCHITECT_BROWSER_REQUIRED: ${skip}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const put = (root, rel, body) => { const p = join(root, rel); mkdirSync(dirname(p), { recursive: true }); writeFileSync(p, body); };
const freePort = () => new Promise((ok) => { const srv = createNetServer(); srv.listen(0, '127.0.0.1', () => { const { port } = srv.address(); srv.close(() => ok(port)); }); });
const WASM = Buffer.from([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]);
const CODE = 'merrybrightstack';
/// A `size`×`size` PNG, a red-to-gold gradient: the song's art.
function png(size) {
  const chunk = (type, data) => {
    const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
    const body = Buffer.concat([Buffer.from(type), data]);
    const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(body) >>> 0);
    return Buffer.concat([len, body, crc]);
  };
  const head = Buffer.alloc(13);
  head.writeUInt32BE(size, 0); head.writeUInt32BE(size, 4); head[8] = 8; head[9] = 2;
  const rows = [];
  for (let y = 0; y < size; y++) {
    const row = Buffer.alloc(1 + size * 3);
    for (let x = 0; x < size; x++) { row[1 + x * 3] = 200; row[2 + x * 3] = Math.round((x + y) * 160 / (2 * size)); row[3 + x * 3] = 40; }
    rows.push(row);
  }
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk('IHDR', head), chunk('IDAT', deflateSync(Buffer.concat(rows))), chunk('IEND', Buffer.alloc(0))]);
}
const PNG = png(32);

// ── The island ──────────────────────────────────────────────────────

/// Its `list` view: draws what it is handed (the text's lines, the art,
/// the lyrics) and reports it on its body for the test to read.
const LIST = `
  const root = document.createElement("div");
  root.style.cssText = "font: 14px/1.5 system-ui, sans-serif; color: var(--foreground, #222); padding: 8px;";
  document.body.appendChild(root);
  globalThis.architectPage.subscribe((p) => {
    const note = p.note;
    document.body.dataset.report = JSON.stringify(note);
    root.textContent = "";
    const h = document.createElement("div");
    h.style.cssText = "font-weight: 600; margin-bottom: 6px;";
    h.textContent = note ? "Setlist view: " + note.title + " (" + (note.files || []).length + " files)" : "Setlist view: no note";
    root.appendChild(h);
    if (!note) return;
    const pre = document.createElement("pre");
    pre.style.cssText = "white-space: pre-wrap; margin: 0 0 8px; font: 13px/1.5 ui-monospace, monospace;";
    pre.textContent = note.body || "(no text)";
    root.appendChild(pre);
    for (const f of note.files || []) {
      const row = document.createElement("div");
      row.style.cssText = "display: flex; gap: 8px; align-items: center; margin: 4px 0;";
      if (f.kind === "image") {
        const img = document.createElement("img");
        img.src = f.data; img.width = 48; img.height = 48;
        img.style.cssText = "image-rendering: pixelated; border-radius: 6px;";
        row.appendChild(img);
      }
      const t = document.createElement("span");
      t.textContent = f.path + (f.kind === "lyrics" ? ": " + f.text : "");
      row.appendChild(t);
      root.appendChild(row);
    }
  });
`;

const GLUE = `
export default async function init() {}
export function mount(view) {
  document.body.dataset.view = view;
  if (view === "list") {
${LIST}
  }
}`;

// ── The node and the front ──────────────────────────────────────────

/// A node sharing `Gig/` by link `CODE`, its island claiming setlists
/// with view `list`, reading them, its extension declaring `reads`.
async function node(reads) {
  const dir = mkdtempSync(join(tmpdir(), 'guest-content-'));
  const vault = join(dir, 'vault');
  put(vault, 'Gig/set.md', '---\ntitle: Merry and Bright\ntype: setlist\ndate: 2026-12-24\nsecret: the alarm code\n---\n'
    + '1. [[Gig/joy|Joy to the World]]\n2. [[Private/plan|the plan]]\n%%call the venue first%%\n\n'
    + '![[Gig/art/joy.png]]\n\n![[Gig/joy.lrc]]\n\n![[Private/cover.png]]\n');
  put(vault, 'Gig/joy.md', '---\ntitle: Joy to the World\ntype: song\n---\nJoy.\n');
  put(vault, 'Gig/art/joy.png', PNG);
  put(vault, 'Gig/joy.lrc', '[00:01.00]Joy to the world');
  put(vault, 'Private/plan.md', '# Plan\n\nThe secret plan.\n');
  put(vault, 'Private/cover.png', 'private png');
  const share = { code: CODE, vault: 'notes', scope: { Folder: 'Gig' }, title: 'Merry and Bright', audience: 'Link', access: 'Read', expires_at: null, revoked_at: null, created_at: 1, created_by: 'test', domain: null };
  put(vault, `_architect/shares/${CODE}.md`, `---\npublish: false\n---\n# Share ${CODE}\n\n\`\`\`json\n${JSON.stringify(share)}\n\`\`\`\n`);
  const folder = join(dir, 'island');
  put(folder, 'web-view.js', GLUE);
  put(folder, 'web-view_bg.wasm', WASM);
  put(folder, 'architect.json', JSON.stringify({ module: 'web-view.js', manifest: { id: 'demo.setlist', capabilities: {} } }));
  const env = {
    ...process.env, ARCHITECT_DATA_DIR: join(dir, 'data'), ARCHITECT_VAULTS: `notes=${vault}`,
    ARCHITECT_IROH: 'local', ARCHITECT_NAME: 'guest-content-test',
    ARCHITECT_ISLAND_FOLDER: folder, ARCHITECT_ISLAND_ID: 'demo.setlist',
    ARCHITECT_ISLAND_VIEW: 'progress', ARCHITECT_ISLAND_TRANSPORT: 'transport',
    ARCHITECT_ISLAND_EMBEDS: 'list=setlist=date', ARCHITECT_ISLAND_READS: reads,
  };
  delete env.ARCHITECT_ISLAND2_FOLDER;
  const port = await freePort();
  env.ARCHITECT_BIND = `127.0.0.1:${port}`;
  const proc = spawn(NODE, [], { stdio: 'ignore', env });
  const origin = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 160; i++) {
    await sleep(250);
    const ok = await fetch(`${origin}/s/${CODE}/set/`).then((r) => r.ok, () => false);
    if (ok) return { origin, close: () => { proc.kill(); rmSync(dir, { recursive: true, force: true }); } };
  }
  proc.kill();
  throw new Error('the node never built the link');
}

/// The app page's policy, as production's server sends it (sws.toml).
function appPolicy() {
  const sws = readFileSync(new URL('../sws.toml', import.meta.url), 'utf8');
  const m = sws.match(/Content-Security-Policy = "([^"]+)"/);
  if (!m) throw new Error('no Content-Security-Policy in apps/web/sws.toml');
  return m[1];
}

const TYPES = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' };

/// The front: `/_ext/` and `/s/` to the node, the rest the bundle's files,
/// the app's page with its policy.
async function front(nodeOrigin) {
  const policy = appPolicy();
  const port = await freePort();
  const target = new URL(nodeOrigin);
  const proxied = [];
  const srv = createServer((req, res) => {
    const path = new URL(req.url, 'http://x').pathname;
    if (path.startsWith('/_ext/') || path.startsWith('/s/')) {
      proxied.push(`${req.method} ${path}`);
      const up = request({ host: target.hostname, port: target.port, path: req.url, method: req.method, headers: { ...req.headers, host: target.host } }, (r) => {
        res.writeHead(r.statusCode, r.headers); r.pipe(res);
      });
      up.on('error', () => { res.statusCode = 502; res.end(); });
      req.pipe(up);
      return;
    }
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
  return { origin: `http://127.0.0.1:${port}`, policy, proxied, close: () => srv.close() };
}

// ── The browser ─────────────────────────────────────────────────────

async function firefox() {
  const profile = mkdtempSync(join(tmpdir(), 'guest-content-ff-'));
  writeFileSync(join(profile, 'user.js'), 'user_pref("media.volume_scale", "0.0");\n');
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
  await send('browsingContext.setViewport', { context, viewport: { width: 390, height: 844 } });
  const evalIn = async (ctx, expression) => {
    const r = await send('script.evaluate', { expression, target: { context: ctx }, awaitPromise: true, userActivation: true });
    if (r.type === 'exception') throw new Error(r.exceptionDetails.text);
    return r.result.value;
  };
  return {
    go: (url) => send('browsingContext.navigate', { context, url, wait: 'complete' }),
    shot: async (file) => { const r = await send('browsingContext.captureScreenshot', { context, origin: 'document' }); writeFileSync(file, Buffer.from(r.data, 'base64')); },
    eval: (e) => evalIn(context, e),
    evalIn,
    frames: async () => ((await send('browsingContext.getTree', { root: context })).contexts[0].children || []).map((c) => c.context),
    close: async () => { try { ws.close(); } catch {} try { process.kill(-proc.pid, 'SIGKILL'); } catch {} await sleep(300); try { rmSync(profile, { recursive: true, force: true }); } catch {} },
  };
}

async function until(fn, ms = 20000, what = 'a condition') {
  const end = Date.now() + ms;
  let last;
  while (Date.now() < end) { try { last = await fn(); if (last) return last; } catch (e) { last = e.message; } await sleep(200); }
  throw new Error(`timed out waiting for ${what}; last: ${JSON.stringify(last)}`);
}

/// The `list` view's report: what it was handed of the note.
async function report(b) {
  for (const c of await b.frames()) {
    const got = await b.evalIn(c, 'document.body && document.body.dataset.view === "list" ? (document.body.dataset.report || "") : null').catch(() => null);
    if (got) return JSON.parse(got);
  }
  return undefined;
}

/// The link opened as its guest at 390 px; what the setlist's view was
/// handed, once it was.
async function asGuest(reads, shot) {
  const n = await node(reads);
  const f = await front(n.origin);
  const b = await firefox();
  try {
    await b.go(`${f.origin}/app/#share=${CODE}`);
    await until(() => b.eval(`!!document.querySelector('[data-testid=guest-note]')`), 60000, 'the guest note');
    // The setlist, from the link's notes.
    await until(() => b.eval(`(() => { const e = document.querySelector('[data-guest-note="Merry and Bright"]'); if (!e) return false; e.click(); return true; })()`), 10000, 'the setlist in the link\'s notes');
    await until(() => b.eval(`(document.querySelector('[data-testid=guest-note] h2') || {}).textContent === 'Merry and Bright'`), 15000, 'the setlist note');
    const got = await until(async () => { const r = await report(b); return r === undefined ? null : { r }; }, 30000, 'the view\'s report').catch(async (e) => {
      const why = await b.eval(`JSON.stringify({ frames: [...document.querySelectorAll('[data-testid=ext-frame]')].map((x) => ({ view: x.dataset.extView, state: x.dataset.extState, why: x.dataset.extWhy })), embed: !!document.querySelector('[data-testid=note-embed]'), note: (document.querySelector('[data-testid=guest-note]') || {}).outerHTML?.slice(0, 600) })`).catch((x) => x.message);
      if (SHOTS) { mkdirSync(SHOTS, { recursive: true }); await b.shot(join(SHOTS, `failed-${shot}`)); }
      throw new Error(`${e.message}\n${why}\n${f.proxied.join('\n')}`);
    });
    // Its art drawn, when it has any.
    if ((got.r?.files || []).length) await sleep(500);
    if (SHOTS && shot) { mkdirSync(SHOTS, { recursive: true }); await b.shot(join(SHOTS, shot)); }
    const page = await b.eval(`JSON.stringify({ files: document.querySelector('[data-testid=guest-note]').dataset.files, text: document.querySelector('[data-testid=guest-note]').dataset.text, title: (document.querySelector('[data-testid=guest-title]') || {}).textContent })`);
    const note = await fetch(`${f.origin}/s/${CODE}/set/_note.json`);
    return { note: got.r, page: JSON.parse(page), noteFile: note.status, proxied: f.proxied };
  } finally { await b.close(); f.close(); n.close(); }
}

test('a guest\'s setlist view is handed its text and the link\'s art and lyrics', { skip }, async () => {
  const { note, page, noteFile, proxied } = await asGuest('documents,files', 'guest-setlist-390.png');
  assert.equal(page.title, 'Merry and Bright');
  assert.equal(note.title, 'Merry and Bright');
  assert.equal(note.type, 'setlist');
  assert.deepEqual(note.properties, {}, 'a guest gets no property the node did not give');
  assert.ok(!('path' in note), 'never a vault path');
  assert.match(note.body, /1\. \[\[Gig\/joy\|Joy to the World\]\]/);
  assert.match(note.body, /2\. the plan\n/);
  for (const never of ['call the venue', 'Private/', 'secret', 'alarm', '---']) assert.ok(!note.body.includes(never), `${never} in ${note.body}`);
  const files = Object.fromEntries(note.files.map((x) => [x.path, x]));
  assert.deepEqual(Object.keys(files).sort(), ['Gig/art/joy.png', 'Gig/joy.lrc']);
  assert.equal(files['Gig/art/joy.png'].kind, 'image');
  assert.equal(files['Gig/art/joy.png'].data, `data:image/png;base64,${PNG.toString('base64')}`);
  assert.equal(files['Gig/joy.lrc'].text, '[00:01.00]Joy to the world');
  assert.equal(page.files, '2');
  assert.equal(noteFile, 404, 'the build\'s _note.json is no file of the link');
  // Read through the guest service and the link's own files, same origin.
  assert.ok(proxied.some((p) => p.startsWith(`POST /s/${CODE}/_guest/read`)), proxied.join('\n'));
  assert.ok(proxied.some((p) => p === `GET /s/${CODE}/Gig/art/joy.png`), proxied.join('\n'));
});

test('a view that declares documents alone gets the text, no files', { skip }, async () => {
  const { note, page } = await asGuest('documents', 'guest-setlist-no-files-390.png');
  assert.match(note.body, /Joy to the World/);
  assert.ok(!('files' in note), JSON.stringify(note));
  assert.equal(page.files, '0');
});

test('a view that declares no read gets no note', { skip }, async () => {
  const { note, page } = await asGuest('', 'guest-setlist-no-read-390.png');
  assert.equal(note, null);
  assert.equal(page.text, 'false');
  assert.equal(page.files, '0');
});
