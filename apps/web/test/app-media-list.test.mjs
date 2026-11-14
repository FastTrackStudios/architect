// A collection note's native list (#668), in a real browser: the release
// bundle (ARCHITECT_WEB_BUNDLE) behind a front that does what production's
// proxy does (the bundle's files, `/_ext/` and `/s/` to a node when there
// is one).
//
// In `/app/?demo`, the demo's Christmas setlist (`Setlists/Merry &
// Bright.md`: Studio's `sets`/`songs` shape, no island claiming it) draws
// as the core list at the note's head, not as raw frontmatter:
// - both sets by name, nine rows, four greyed (no recording), each with
//   its cover, title over artist and length;
// - the `sets`/`songs` rows gone from the property table, the
//   ```studio-setlist fence folded out of the reading;
// - a row pressed plays on the Now Playing card, the row marked; the end
//   of a song moves on to the next song in the list.
//
// As a share link's guest (a node, `island_node` with no island, sharing
// a copy of the demo's setlist and its songs with PNG covers and the
// demo's silent recording at each recording's path; the setlist also
// links a song outside the share): the same list from the link's listed
// notes, the outside song never listed, a row plays, the end advances.
//
// What the node hands a guest of the setlist's words (`_guest/read`)
// names no song outside the link: the setlist's ```studio-setlist fence
// carries a row for the outside song, as Studio writes it, and the node
// cuts it (#687 review).
//
// The same guest's setlist with an island claiming setlists: its view
// first, then the link's notes to open, under "Notes on this link".
//
// The text area of a collection note is one element whether the list is
// drawn or not: typed into, then the extension listing arriving late
// (claiming the note's type, so the list gives way), then typed into
// again, nothing typed is lost and it keeps its focus.
//
// At 390 px and 1280 px, light and dark; SHOTS=<dir> keeps the
// screenshots. Firefox over WebDriver BiDi (FIREFOX=… or `firefox` on the
// PATH), muted, in a mount namespace whose runtime dir and /run/pipewire
// are empty (`unshare -r -m`). `just app-media-list-test` builds and runs
// it.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync, readFileSync, existsSync, statSync, copyFileSync } from 'node:fs';
import { createServer, request } from 'node:http';
import { createServer as createNetServer } from 'node:net';
import { tmpdir } from 'node:os';
import { crc32, deflateSync } from 'node:zlib';
import { dirname, join, extname, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';

const NODE = process.env.ARCHITECT_ISLAND_NODE;
const BUNDLE = process.env.ARCHITECT_WEB_BUNDLE;
const SHOTS = process.env.SHOTS;
function which(cmd) {
  try { return execFileSync('sh', ['-c', `command -v ${cmd}`]).toString().trim() || null; } catch { return null; }
}
const browser = process.env.FIREFOX || which('firefox');
const skip = BUNDLE && browser ? false : 'needs ARCHITECT_WEB_BUNDLE (just app-media-list-test) and a Firefox (FIREFOX=…)';
const skipGuest = skip || (NODE ? false : 'needs ARCHITECT_ISLAND_NODE (just app-media-list-test)');
if (process.env.ARCHITECT_BROWSER_REQUIRED && skipGuest) throw new Error(`ARCHITECT_BROWSER_REQUIRED: ${skipGuest}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const put = (root, rel, body) => { const p = join(root, rel); mkdirSync(dirname(p), { recursive: true }); writeFileSync(p, body); };
const freePort = () => new Promise((ok) => { const srv = createNetServer(); srv.listen(0, '127.0.0.1', () => { const { port } = srv.address(); srv.close(() => ok(port)); }); });
const REPO = fileURLToPath(new URL('../../../', import.meta.url));
const DEMO = join(REPO, 'apps/ui/demo-vault');
const SETLIST = 'Setlists/Merry & Bright.md';
const CODE = 'merrybrighttrack';

const SETS = [
  ['Opening Set', ['Sleigh Ride', 'Silent Night', 'Joy to the World (Unspeakable Joy)']],
  ['Concert Set', ['Feliz Navidad', 'What Christmas Means', 'This Christmas', "Rockin' Around the Christmas Tree", 'Last Christmas', 'All I Want for Christmas Is You']],
];
const GREYED = ['Feliz Navidad', 'What Christmas Means', "Rockin' Around the Christmas Tree", 'Last Christmas'];

/// A `size`×`size` PNG, a gradient from `a` to `b` (RGB): a cover.
function png(size, a, b) {
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
    for (let x = 0; x < size; x++) {
      const t = (x + y) / (2 * size);
      for (let c = 0; c < 3; c++) row[1 + x * 3 + c] = Math.round(a[c] + (b[c] - a[c]) * t);
    }
    rows.push(row);
  }
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk('IHDR', head), chunk('IDAT', deflateSync(Buffer.concat(rows))), chunk('IEND', Buffer.alloc(0))]);
}

/// A node sharing a copy of the demo's setlist and its nine songs (and
/// not a tenth song the setlist links), each cover a PNG, each recording
/// the demo's silent one.
async function node(island = null) {
  const dir = mkdtempSync(join(tmpdir(), 'media-list-'));
  const vault = join(dir, 'vault');
  const songs = [];
  let set = readFileSync(join(DEMO, SETLIST), 'utf8');
  // A song outside the share, linked from the setlist: never a guest's.
  set = set.replace('    - "[[Songs/Christmas/Last Christmas/Last Christmas|Last Christmas]]"\n',
    '    - "[[Songs/Christmas/Last Christmas/Last Christmas|Last Christmas]]"\n    - "[[Private/Secret Song|Secret Song]]"\n');
  // ... and its row in the setlist's fence, as Studio writes one.
  set = set.replace('## Concert Set\n', '## Concert Set\n10 | Secret Song | Private/Secret Song.md | E · 99 bpm | Private/secret-take.mp3 | Unreleased Artist\n');
  if (!set.includes('Private/Secret Song.md | E')) throw new Error('the fence row was not added');
  put(vault, SETLIST, set);
  put(vault, 'Private/Secret Song.md', '---\ntitle: Secret Song\ntype: song\nrecording: Recordings/Christmas/Sleigh Ride.mp3\n---\nNot shared.\n');
  const colours = [[196, 40, 40], [30, 40, 110], [20, 110, 70], [110, 50, 140], [200, 120, 30], [40, 120, 160], [160, 40, 90], [70, 70, 70], [180, 150, 40]];
  SETS.flatMap(([, s]) => s).forEach((title, i) => {
    const folder = `Songs/Christmas/${title}`;
    const text = readFileSync(join(DEMO, folder, `${title}.md`), 'utf8').replace(/^cover: .*$/m, `cover: "${folder}/cover.png"`);
    put(vault, `${folder}/${title}.md`, text);
    put(vault, `${folder}/cover.png`, png(64, colours[i], [240, 230, 210]));
    const rec = text.match(/^recording: (.*)$/m);
    if (rec) {
      mkdirSync(dirname(join(vault, rec[1])), { recursive: true });
      copyFileSync(join(REPO, 'apps/web/app/demo-recording.mp3'), join(vault, rec[1]));
    }
    songs.push(`${folder}/${title}.md`);
  });
  const share = { code: CODE, vault: 'notes', scope: { Notes: [SETLIST, ...songs] }, title: 'Merry & Bright', audience: 'Link', access: 'Read', expires_at: null, revoked_at: null, created_at: 1, created_by: 'test', domain: null };
  put(vault, `_architect/shares/${CODE}.md`, `---\npublish: false\n---\n# Share ${CODE}\n\n\`\`\`json\n${JSON.stringify(share)}\n\`\`\`\n`);
  const env = { ...process.env, ARCHITECT_DATA_DIR: join(dir, 'data'), ARCHITECT_VAULTS: `notes=${vault}`, ARCHITECT_IROH: 'local', ARCHITECT_NAME: 'media-list-test', ARCHITECT_WEB_APP: BUNDLE };
  for (const k of Object.keys(env)) if (k.startsWith('ARCHITECT_ISLAND') || k === 'ARCHITECT_FAVOURITES' || k === 'ARCHITECT_ISSUER') delete env[k];
  // An island claiming setlists, when asked: a view that only says it drew.
  if (island) {
    const folder = join(dir, 'island');
    put(folder, 'web-view.js', 'export default async function init() {}\nexport function mount(view) { document.body.textContent = "drawn: " + view; }\n');
    put(folder, 'web-view_bg.wasm', Buffer.from([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]));
    put(folder, 'architect.json', JSON.stringify({ module: 'web-view.js', manifest: { id: 'demo.setlist', capabilities: {} } }));
    Object.assign(env, { ARCHITECT_ISLAND_FOLDER: folder, ARCHITECT_ISLAND_ID: 'demo.setlist', ARCHITECT_ISLAND_VIEW: 'progress', ARCHITECT_ISLAND_EMBEDS: 'list=setlist=date', ARCHITECT_ISLAND_READS: 'documents' });
  }
  const port = await freePort();
  env.ARCHITECT_BIND = `127.0.0.1:${port}`;
  const proc = spawn(NODE, [], { stdio: ['ignore', 'ignore', process.env.NODE_LOG ? 'inherit' : 'ignore'], env });
  const origin = `http://127.0.0.1:${port}`;
  const link = () => fetch(`${origin}/s/${CODE}/_guest/link`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ code: CODE, token: '' }) }).then((r) => (r.ok ? r.json() : null), () => null);
  const read = (path) => fetch(`${origin}/s/${CODE}/_guest/read`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ code: CODE, token: '', path }) }).then(async (r) => ({ status: r.status, text: await r.text() }));
  let l;
  for (let i = 0; i < 160; i++) {
    await sleep(250);
    l = await link();
    if (l && l.notes && l.notes.length === 10 && l.notes.every((n) => n.page)) return { origin, link, read, close: () => { proc.kill(); rmSync(dir, { recursive: true, force: true }); } };
  }
  const raw = await fetch(`${origin}/s/${CODE}/_guest/link`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ code: CODE, token: '' }) }).then(async (r) => `${r.status} ${(await r.text()).slice(0, 300)}`, (e) => e.message);
  proc.kill();
  throw new Error(`the node never built the link: ${raw} ${JSON.stringify(l)?.slice(0, 2000)}`);
}

/// The app page's policy, as production's server sends it (sws.toml).
function appPolicy() {
  const sws = readFileSync(new URL('../sws.toml', import.meta.url), 'utf8');
  const m = sws.match(/Content-Security-Policy = "([^"]+)"/);
  if (!m) throw new Error('no Content-Security-Policy in apps/web/sws.toml');
  return m[1];
}

const TYPES = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml', '.woff2': 'font/woff2', '.mp3': 'audio/mpeg' };

/// The front: `/_ext/` and `/s/` to the node (none: 404), the rest the
/// bundle's files (ranged, as the release's server does), the app's page
/// with its policy.
async function front(nodeOrigin, islands = null) {
  const policy = appPolicy();
  const port = await freePort();
  const srv = createServer((req, res) => {
    const path = new URL(req.url, 'http://x').pathname;
    // A listing held until the test lets it go (`islands.held`).
    if (islands && path === '/_ext/islands.json') {
      islands.held.then(() => { res.writeHead(200, { 'content-type': 'application/json', 'cache-control': 'no-store' }); res.end(JSON.stringify(islands.body)); });
      return;
    }
    if (path.startsWith('/_ext/') || path.startsWith('/s/')) {
      if (!nodeOrigin) { res.statusCode = 404; res.end(); return; }
      const target = new URL(nodeOrigin);
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
    const body = readFileSync(file);
    const headers = { 'content-type': TYPES[extname(file)] || 'application/octet-stream', 'cache-control': 'no-store', 'accept-ranges': 'bytes' };
    if (path === '/app/' || path === '/app/index.html') headers['content-security-policy'] = policy;
    const range = /^bytes=(\d+)-(\d*)$/.exec(req.headers.range || '');
    if (range) {
      const start = Number(range[1]);
      const end = range[2] ? Math.min(Number(range[2]), body.length - 1) : body.length - 1;
      headers['content-range'] = `bytes ${start}-${end}/${body.length}`;
      res.writeHead(206, headers);
      res.end(body.subarray(start, end + 1));
      return;
    }
    res.writeHead(200, headers);
    res.end(body);
  });
  await new Promise((ok) => srv.listen(port, '127.0.0.1', ok));
  return { origin: `http://127.0.0.1:${port}`, close: () => srv.close() };
}

// ── The browser ─────────────────────────────────────────────────────

/// A headless Firefox, muted, light or `dark`.
async function firefox(dark = false) {
  const profile = mkdtempSync(join(tmpdir(), 'media-list-ff-'));
  writeFileSync(join(profile, 'user.js'), [
    'user_pref("media.volume_scale", "0.0");',
    'user_pref("media.autoplay.default", 0);',
    'user_pref("media.autoplay.blocking_policy", 0);',
    `user_pref("ui.systemUsesDarkTheme", ${dark ? 1 : 0});`,
    `user_pref("layout.css.prefers-color-scheme.content-override", ${dark ? 0 : 1});`,
  ].join('\n') + '\n');
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
  return {
    go: (url) => send('browsingContext.navigate', { context, url, wait: 'complete' }),
    size: (width, height) => send('browsingContext.setViewport', { context, viewport: { width, height } }),
    shot: async (file) => { const r = await send('browsingContext.captureScreenshot', { context, origin: 'document' }); writeFileSync(file, Buffer.from(r.data, 'base64')); },
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
  while (Date.now() < end) { try { last = await fn(); if (last) return last; } catch (e) { last = e.message; } await sleep(200); }
  throw new Error(`timed out waiting for ${what}; last: ${JSON.stringify(last)}`);
}

/// The list as drawn: its sets, and each row's title, subtitle, length,
/// whether it plays, its state and whether its cover loaded.
const LIST = `JSON.stringify((() => { const l = document.querySelector('[data-testid=media-list]'); if (!l) return null; return {
  source: l.dataset.source,
  sets: [...l.querySelectorAll('[data-media-set]')].map((s) => s.dataset.mediaSet),
  rows: [...l.querySelectorAll('[data-media-row]')].map((r) => ({ title: r.dataset.mediaRow, plays: r.dataset.plays === 'true', state: r.dataset.state, current: r.getAttribute('aria-current') === 'true',
    meta: r.querySelector('.media-list-meta')?.textContent || '', length: r.querySelector('.media-list-length')?.textContent || '',
    art: (() => { const i = r.querySelector('.media-list-art img'); return !!i && i.complete && i.naturalWidth > 0; })() })),
  sum: l.querySelector('[data-testid=media-list-sum]')?.textContent }; })())`;
const list = async (b) => JSON.parse(await b.eval(LIST));
const bar = async (b) => JSON.parse(await b.eval(`JSON.stringify((() => { const t = document.querySelector('[data-testid=now-playing-title]'); return t && { title: t.textContent, subtitle: document.querySelector('[data-testid=now-playing-subtitle]')?.textContent, playing: document.querySelector('[data-testid=now-playing-toggle]')?.dataset.playing === 'true' }; })())`));
const pressRow = (b, title) => b.eval(`(() => { const r = [...document.querySelectorAll('[data-media-row]')].find((r) => r.dataset.mediaRow === ${JSON.stringify(title)}); if (!r) return false; r.querySelector('.media-list-main').click(); return true; })()`);
const press = (b, testid) => b.eval(`(() => { const e = document.querySelector('[data-testid=${testid}]'); if (!e) return false; e.click(); return true; })()`);

/// The list's sets, its nine rows in order, four greyed, every cover.
async function checkList(b, what) {
  const l = await until(async () => { const l = await list(b); return l && l.rows.length === 9 && l.rows.every((r) => r.art) && l; }, 20000, `${what}: nine rows with their covers`).catch(async (e) => {
    throw new Error(`${e.message}\n${JSON.stringify(await list(b))}`);
  });
  assert.deepEqual(l.sets, SETS.map(([name]) => name));
  assert.deepEqual(l.rows.map((r) => r.title), SETS.flatMap(([, s]) => s));
  assert.deepEqual(l.rows.filter((r) => !r.plays).map((r) => r.title), GREYED);
  assert.equal(l.rows.find((r) => r.title === 'Sleigh Ride').meta, 'Tori Kelly');
  return l;
}

/// A row pressed plays on the card and is marked; the end of the song
/// (the position line's End) moves on to the next song that plays.
async function checkPlays(b, what) {
  // A greyed row offers nothing to play (pressed, it opens its note).
  assert.match(await b.eval(`[...document.querySelectorAll('[data-media-row]')].find((r) => r.dataset.mediaRow === 'Feliz Navidad').querySelector('.media-list-main').getAttribute('aria-label')`), /nothing to play/);
  assert.ok(await pressRow(b, 'Silent Night'));
  const s = await until(async () => { const s = await bar(b); return s && s.title === 'Silent Night' && s.playing && s; }, 10000, `${what}: the card playing Silent Night`);
  assert.match(s.subtitle, /for KING & COUNTRY/);
  await until(async () => (await list(b)).rows.find((r) => r.current)?.title === 'Silent Night', 5000, `${what}: its row marked`);
  // The end of the song: the next in the list.
  await until(() => press(b, 'now-playing-open'), 5000, 'the card opened');
  await until(() => b.eval(`(() => { const l = document.querySelector('[data-testid=now-playing-line]'); if (!l || l.getAttribute('aria-disabled') === 'true') return false; l.focus(); l.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true })); return true; })()`), 10000, 'the position line');
  await until(async () => { const s = await bar(b); return s && s.title === 'Joy to the World (Unspeakable Joy)' && s; }, 20000, `${what}: the next song after the end`);
  await until(async () => (await list(b)).rows.find((r) => r.current)?.title === 'Joy to the World (Unspeakable Joy)', 5000, `${what}: the next row marked`);
  await press(b, 'now-playing-open');
}

const shotDir = () => { const d = SHOTS || join(tmpdir(), 'media-list-shots'); mkdirSync(d, { recursive: true }); return d; };

/// Opens the demo's setlist: its folder in the navigator, then the note.
async function openSetlist(b, origin) {
  await b.go(`${origin}/app/?demo`);
  await until(() => b.eval(`!!document.querySelector('[data-testid=note-text]')`), 60000, 'the demo');
  const click = (txt) => b.eval(`(() => { const all = [...document.querySelectorAll('*')].filter((e) => e.children.length === 0 && e.textContent.trim() === ${JSON.stringify(txt)}); const e = all[all.length - 1]; if (!e) return false; (e.closest('button,[role=treeitem],[role=button],a,li,div') || e).click(); return true; })()`);
  await until(() => click('Setlists'), 10000, 'the Setlists folder');
  await sleep(300);
  await until(() => click('Merry & Bright'), 10000, 'the setlist');
  await until(() => b.eval(`document.querySelector('[data-note]')?.dataset.note === ${JSON.stringify(SETLIST)}`), 10000, 'the setlist open');
}

test('the demo setlist draws as the core list and plays on the card', { skip }, async () => {
  const f = await front(null);
  const shots = shotDir();
  try {
    for (const dark of [false, true]) {
      const b = await firefox(dark);
      const theme = dark ? 'dark' : 'light';
      try {
        await openSetlist(b, f.origin);
        await checkList(b, `demo (${theme})`);
        // Not raw frontmatter: no `sets`/`songs` rows, the fence folded.
        const props = await b.eval(`[...document.querySelectorAll('[data-testid=note-properties] dt')].map((d) => d.textContent).join(',')`);
        assert.ok(!/\b(sets|songs)\b/.test(props), props);
        assert.ok(props.includes('title'), props);
        const reading = await b.eval(`document.querySelector('[data-testid=note-reading]')?.textContent || ''`);
        assert.ok(reading.includes('A Christmas set in two parts.'), reading);
        assert.ok(!reading.includes('Sleigh Ride.mp3') && !reading.includes(' | '), 'the studio-setlist fence folded');
        assert.equal(await b.eval(`document.querySelector('[data-testid=note-text-fold]').open`), false);
        if (!dark) await checkPlays(b, 'demo');
        else { await pressRow(b, 'This Christmas'); await until(async () => (await bar(b))?.title === 'This Christmas', 10000, 'dark: the card'); }
        for (const [w, h, tag] of [[1280, 900, 'desk-1280'], [390, 844, 'phone-390']]) {
          await b.size(w, h);
          await sleep(800);
          assert.ok(await b.eval('document.documentElement.scrollWidth <= window.innerWidth'), `${tag}: no sideways scroll`);
          await b.shot(join(shots, `demo-list-${tag}-${theme}.png`));
        }
        await press(b, 'now-playing-toggle');
      } finally { await b.close(); }
    }
  } finally { f.close(); }
});

test('a share link\'s guest gets the same list from the link\'s notes, never one outside it', { skip: skipGuest }, async () => {
  const n = await node();
  const f = await front(n.origin);
  const shots = shotDir();
  try {
    // The node lists the sets with only the link's notes: the outside
    // song is in no set, and named nowhere in what a guest is given.
    const l = await n.link();
    const root = l.notes.find((x) => x.path === SETLIST);
    assert.deepEqual(root.sets.map((s) => [s.name, s.notes.length]), [['Opening Set', 3], ['Concert Set', 6]]);
    assert.ok(!JSON.stringify(l).includes('Secret'), 'nothing of the outside song');
    const sleigh = l.notes.find((x) => x.title === 'Sleigh Ride');
    assert.equal(sleigh.media, 'Recordings/Christmas/Sleigh Ride.mp3');
    assert.equal(sleigh.subtitle, 'Tori Kelly');
    assert.equal(sleigh.duration_ms, 186000);
    assert.equal(sleigh.art, 'Songs/Christmas/Sleigh Ride/cover.png');
    // The setlist's words as the node hands them: its fence keeps the
    // link's songs and names nothing outside it.
    const set = await n.read(SETLIST);
    assert.equal(set.status, 200, set.text);
    assert.ok(set.text.includes('Songs/Christmas/Sleigh Ride/Sleigh Ride.md'), set.text.slice(0, 2000));
    for (const never of ['Secret', 'Private/', 'secret-take', 'Unreleased Artist']) {
      assert.ok(!set.text.includes(never), `${never} in ${set.text.slice(0, 4000)}`);
    }
    for (const dark of [false, true]) {
      const b = await firefox(dark);
      const theme = dark ? 'dark' : 'light';
      try {
        await b.go(`${f.origin}/s/${CODE}/`);
        await until(() => b.eval(`(document.querySelector('[data-testid=guest-title]') || {}).textContent === 'Merry & Bright'`), 60000, 'the link');
        await checkList(b, `guest (${theme})`);
        assert.equal((await list(b)).source, 'app:share');
        assert.equal(await b.eval(`document.body.textContent.includes('Secret Song')`), false);
        // The fence the list repeats is folded out of the guest's reading.
        assert.ok(!(await b.eval(`document.querySelector('[data-testid=guest-note]')?.textContent || ''`)).includes('Sleigh Ride.mp3'));
        if (!dark) await checkPlays(b, 'guest');
        for (const [w, h, tag] of [[1280, 900, 'desk-1280'], [390, 844, 'phone-390']]) {
          await b.size(w, h);
          await sleep(800);
          assert.ok(await b.eval('document.documentElement.scrollWidth <= window.innerWidth'), `${tag}: no sideways scroll`);
          await b.shot(join(shots, `guest-list-${tag}-${theme}.png`));
        }
        await press(b, 'now-playing-toggle');
      } finally { await b.close(); }
    }
  } finally { f.close(); n.close(); }
});

test('a guest\'s setlist an island claims: its view first, then the link\'s notes under their heading', { skip: skipGuest }, async () => {
  const n = await node(true);
  const f = await front(n.origin);
  const b = await firefox(false);
  try {
    for (const [w, h, tag] of [[390, 844, 'phone-390'], [1280, 900, 'desk-1280']]) {
      await b.size(w, h);
      await b.go(`${f.origin}/s/${CODE}/`);
      await until(() => b.eval(`!!document.querySelector('[data-testid=ext-frame]') && !!document.querySelector('[data-testid=guest-list]')`), 60000, `${tag}: the view and the notes`);
      // The island draws the list: no core list, and its view comes before
      // the notes, which sit under their own heading.
      assert.equal(await list(b), null, 'no core list under a claim');
      assert.ok(await b.eval(`!!(document.querySelector('[data-testid=ext-frame]').compareDocumentPosition(document.querySelector('[data-testid=guest-list]')) & Node.DOCUMENT_POSITION_FOLLOWING)`), 'the view before the notes');
      assert.equal(await b.eval(`document.querySelector('[data-testid=guest-notes] h2')?.textContent`), 'Notes on this link');
      await sleep(800);
      await b.shot(join(shotDir(), `guest-claimed-${tag}.png`));
    }
  } finally { await b.close(); f.close(); n.close(); }
});

test('a collection note\'s text area keeps what was typed when the extension listing arrives late', { skip }, async () => {
  let release;
  const held = new Promise((ok) => { release = ok; });
  const pin = `sha256-${'A'.repeat(43)}=`;
  // An island claiming `setlist`: once listed, the list gives way to it.
  const body = { islands: [{ id: 'demo.late', folder: '/_ext/island/demo.late.abcdef12/', web: { entry: 'web-late.js', webview: '/_ext/webview.abcdef12.html', files: { 'web-late.js': pin, 'web-late.wasm': pin } }, embeds: [{ view: 'late', note_type: ['setlist'] }] }] };
  const f = await front(null, { held, body });
  const b = await firefox(false);
  try {
    await openSetlist(b, f.origin);
    await until(async () => (await list(b))?.rows.length === 9, 20000, 'the list');
    // Unfolded, typed into at its end.
    assert.ok(await b.eval(`(() => { const d = document.querySelector('[data-testid=note-text-fold]'); d.querySelector('summary').click(); return d.open; })()`));
    const type = (words) => b.eval(`(() => { const t = document.querySelector('[data-testid=note-text]'); t.focus(); t.setSelectionRange(t.value.length, t.value.length); return document.execCommand('insertText', false, ${JSON.stringify(words)}); })()`);
    await until(() => b.eval(`document.querySelector('[data-testid=note-text]').dataset.live === 'ready'`), 20000, 'the text area live');
    assert.ok(await type('\nfirst words'));
    await b.eval(`window.__area = document.querySelector('[data-testid=note-text]'); true`);
    await sleep(500);
    // The listing arrives: the list gives way, the text area stays.
    release();
    await until(async () => !(await list(b)), 20000, 'the list giving way to the island');
    assert.ok((await b.eval(`document.querySelector('[data-testid=note-text]').value`)).endsWith('\nfirst words'), 'what was typed, kept through the change');
    assert.equal(await b.eval(`document.querySelector('[data-testid=note-text-fold]').open`), true);
    assert.equal(await b.eval(`document.querySelector('[data-testid=note-text]') === window.__area`), true, 'the same text area');
    assert.equal(await b.eval(`document.activeElement === window.__area`), true, 'it keeps its focus');
    assert.ok(await type(' then more'));
    await sleep(1500);
    const value = await b.eval(`document.querySelector('[data-testid=note-text]').value`);
    assert.ok(value.includes('first words then more'), value.slice(-300));
  } finally { await b.close(); f.close(); }
});
