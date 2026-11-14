// A share link opens the web app as its guest (#646), in a real browser:
// the release bundle (ARCHITECT_WEB_BUNDLE) behind a front that does what
// production's proxy does (the bundle's files, `/_ext/` and `/s/` to the
// node), a node (`island_node`, ARCHITECT_ISLAND_NODE, no island) sharing a
// gig: a setlist and its songs, as Studio shares one (the setlist first),
// laid out in two sets, each song with its cover and a recording, one
// with a picture in its text.
//
// `/s/<code>/` is the node's answer, the app's page with the app's own
// policy, not a page of the note; the app reads the link from its path:
// - the link's title, its notes in their sets (the setlist's headings),
//   each with its art and a play button for its recording;
// - the setlist's text; a song opened from the list, its words and its
//   picture loaded from below the link; the location follows;
// - Files: each file a download, "Download all" the link's archive;
// - `/s/<code>/_files/` opens on the files.
// At 390 px and at a desk's 1280 px; SHOTS=<dir> keeps the screenshots.
//
// A link for signed-in people (an issuer, `auth-server`,
// ARCHITECT_AUTH_SERVER): the app says to sign in, `/s/_signin` signs the
// person in at the issuer (approved there), the node's cookie comes back
// and the guest calls carry it: the link is shown. A session the app
// holds that the issuer no longer knows does not keep the person out.
// A node with no web app beside it says the link needs one.
//
// Firefox over WebDriver BiDi (FIREFOX=… or `firefox` on the PATH), muted,
// in a mount namespace whose runtime dir and /run/pipewire are empty
// (`unshare -r -m`). `just app-share-guest-test` builds and runs it.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync, readFileSync, existsSync, statSync } from 'node:fs';
import { randomBytes } from 'node:crypto';
import { createServer, request } from 'node:http';
import { createServer as createNetServer } from 'node:net';
import { tmpdir } from 'node:os';
import { crc32, deflateSync } from 'node:zlib';
import { dirname, join, extname, normalize } from 'node:path';

const NODE = process.env.ARCHITECT_ISLAND_NODE;
const BUNDLE = process.env.ARCHITECT_WEB_BUNDLE;
const AUTH = process.env.ARCHITECT_AUTH_SERVER;
const SHOTS = process.env.SHOTS;
function which(cmd) {
  try { return execFileSync('sh', ['-c', `command -v ${cmd}`]).toString().trim() || null; } catch { return null; }
}
const browser = process.env.FIREFOX || which('firefox');
const skip = NODE && BUNDLE && browser ? false : 'needs ARCHITECT_ISLAND_NODE, ARCHITECT_WEB_BUNDLE (just app-share-guest-test) and a Firefox (FIREFOX=…)';
if (process.env.ARCHITECT_BROWSER_REQUIRED && skip) throw new Error(`ARCHITECT_BROWSER_REQUIRED: ${skip}`);
const skipSignIn = skip || (AUTH ? false : 'needs ARCHITECT_AUTH_SERVER (just app-share-guest-test)');
if (process.env.ARCHITECT_BROWSER_REQUIRED && skipSignIn) throw new Error(`ARCHITECT_BROWSER_REQUIRED: ${skipSignIn}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const put = (root, rel, body) => { const p = join(root, rel); mkdirSync(dirname(p), { recursive: true }); writeFileSync(p, body); };
const freePort = () => new Promise((ok) => { const srv = createNetServer(); srv.listen(0, '127.0.0.1', () => { const { port } = srv.address(); srv.close(() => ok(port)); }); });
const CODE = 'merrybrightshare';

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

/// A second of silence as a WAV: a recording that plays nothing.
function wav() {
  const rate = 8000, n = rate;
  const b = Buffer.alloc(44 + n);
  b.write('RIFF', 0); b.writeUInt32LE(36 + n, 4); b.write('WAVE', 8); b.write('fmt ', 12);
  b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22); b.writeUInt32LE(rate, 24);
  b.writeUInt32LE(rate, 28); b.writeUInt16LE(1, 32); b.writeUInt16LE(8, 34); b.write('data', 36); b.writeUInt32LE(n, 40);
  b.fill(128, 44);
  return b;
}

const SONGS = [
  ['joy', 'Joy to the World', [196, 40, 40], [240, 190, 60]],
  ['silent', 'Silent Night', [30, 40, 110], [140, 160, 230]],
  ['sleigh', 'Sleigh Ride', [20, 110, 70], [200, 230, 120]],
  ['noel', 'The First Noel', [110, 50, 140], [240, 180, 220]],
];

/// A node sharing `Gig/` by link `CODE`: to anyone with the link, or to
/// `invited` (emails) signed in at `issuer`; the web app beside it
/// (ARCHITECT_WEB_APP, the bundle's folder) unless `webApp` is false.
async function node({ invited = null, issuer = null, webApp = true } = {}) {
  const dir = mkdtempSync(join(tmpdir(), 'share-guest-'));
  const vault = join(dir, 'vault');
  put(vault, 'Gig/set.md', '---\ntitle: Merry and Bright\ntype: setlist\n---\nChristmas Eve, two sets.\n\n'
    + '## Set 1\n\n1. [[Gig/joy|Joy to the World]]\n2. [[Gig/silent|Silent Night]]\n\n'
    + '## Set 2\n\n1. [[Gig/sleigh|Sleigh Ride]]\n2. [[Gig/noel|The First Noel]]\n');
  for (const [slug, title, a, b] of SONGS) {
    put(vault, `Gig/${slug}.md`, `---\ntitle: ${title}\ntype: song\nart: "[[Gig/art/${slug}.png]]"\n---\nKey of D, capo 2.\n\n![[Gig/media/${slug}.wav]]\n`
      + (slug === 'joy' ? '\nThe stage plot:\n\n![[Gig/art/stage.png]]\n' : ''));
    put(vault, `Gig/art/${slug}.png`, png(64, a, b));
    put(vault, `Gig/media/${slug}.wav`, wav());
  }
  put(vault, 'Gig/art/stage.png', png(240, [60, 60, 70], [220, 200, 160]));
  const audience = invited ? { Invited: invited } : 'Link';
  const share = { code: CODE, vault: 'notes', scope: { Notes: ['Gig/set.md', ...SONGS.map(([slug]) => `Gig/${slug}.md`)] }, title: 'Merry and Bright', audience, access: 'Read', expires_at: null, revoked_at: null, created_at: 1, created_by: 'test', domain: null };
  put(vault, `_architect/shares/${CODE}.md`, `---\npublish: false\n---\n# Share ${CODE}\n\n\`\`\`json\n${JSON.stringify(share)}\n\`\`\`\n`);
  const env = {
    ...process.env, ARCHITECT_DATA_DIR: join(dir, 'data'), ARCHITECT_VAULTS: `notes=${vault}`,
    ARCHITECT_IROH: 'local', ARCHITECT_NAME: 'share-guest-test',
  };
  for (const k of Object.keys(env)) if (k.startsWith('ARCHITECT_ISLAND') || k === 'ARCHITECT_FAVOURITES' || k === 'ARCHITECT_WEB_APP' || k === 'ARCHITECT_ISSUER') delete env[k];
  // The web bundle beside the node, as the front below serves it: its
  // folder, which the node checks at start.
  if (webApp) env.ARCHITECT_WEB_APP = BUNDLE;
  if (issuer) env.ARCHITECT_ISSUER = issuer;
  const port = await freePort();
  env.ARCHITECT_BIND = `127.0.0.1:${port}`;
  const proc = spawn(NODE, [], { stdio: 'ignore', env });
  const origin = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 160; i++) {
    await sleep(250);
    // A signed-in link: up once it is built (its page answers 401, not 503).
    const ok = invited
      ? await fetch(`${origin}/s/${CODE}/`).then((r) => r.status === 401, () => false)
      : await fetch(`${origin}/s/${CODE}/_guest/link`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ code: CODE, token: '' }) })
        .then((r) => (r.ok ? r.json() : null), () => null)
        .then((l) => !!l && JSON.stringify(l).split('"page":"/s/').length > SONGS.length + 1, () => false);
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
  const profile = mkdtempSync(join(tmpdir(), 'share-guest-ff-'));
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
    size: (width, height) => send('browsingContext.setViewport', { context, viewport: { width, height } }),
    shot: async (file) => { const r = await send('browsingContext.captureScreenshot', { context, origin: 'document' }); writeFileSync(file, Buffer.from(r.data, 'base64')); },
    eval: (e) => evalIn(context, e),
    evalIn,
    cookies: async () => (await send('storage.getCookies', { partition: { type: 'context', context } })).cookies,
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


const q = (sel) => `document.querySelector(${JSON.stringify(sel)})`;

test('a share link opens the web app as its guest: the list in sets, a note, its picture, the files', { skip }, async () => {
  const n = await node();
  const f = await front(n.origin);
  const b = await firefox();
  const shots = SHOTS || '/tmp/share-web-guest';
  mkdirSync(shots, { recursive: true });
  try {
    // The node's answer: the app's page and policy, not the note.
    const res = await fetch(`${n.origin}/s/${CODE}/`);
    assert.equal(res.status, 200);
    const html = await res.text();
    assert.match(html, /<script type="module" src="\/app\/start\.js"><\/script>/);
    assert.match(html, /<title>Merry and Bright · Architect<\/title>/);
    assert.ok(!html.includes('Christmas Eve'), 'no note on the page');
    assert.match(res.headers.get('content-security-policy'), /^default-src 'none'; script-src 'self' 'wasm-unsafe-eval';/);
    assert.match(res.headers.get('content-security-policy'), /frame-ancestors 'none'$/);
    // The node's share contract and version, and the line a too-old app
    // would leave showing.
    assert.match(html, /<meta name="architect-share-shell" content="\d+">/);
    assert.match(html, /<meta name="architect-node-version" content="[^"]+">/);
    assert.match(html, /<p id="share-stall"/);

    for (const [width, height, tag] of [[390, 844, '390'], [1280, 860, 'desk']]) {
      await b.size(width, height);
      await b.go(`${f.origin}/s/${CODE}/`);
      // The link's title, the setlist opened, its songs in their sets.
      await until(() => b.eval(`(${q('[data-testid=guest-title]')} || {}).textContent === 'Merry and Bright'`), 60000, 'the link\'s title');
      await until(() => b.eval(`(${q('[data-testid=guest-note] h2')} || {}).textContent === 'Merry and Bright'`), 20000, 'the setlist').catch(async (e) => {
        await b.shot(join(shots, `failed-${tag}.png`)).catch(() => {});
        const page = await b.eval(`(${q('[data-testid=guest-page]')} || document.body).innerText.slice(0, 1500)`).catch((x) => x.message);
        throw new Error(`${e.message}\n${page}\n${f.proxied.join('\n')}`);
      });
      await until(() => b.eval(`document.querySelectorAll('[data-guest-set]').length === 2`), 20000, 'two sets');
      // The app is drawing the link: the node's "still opening?" line is gone.
      assert.equal(await b.eval(`!!document.getElementById('share-stall')`), false);
      const list = JSON.parse(await b.eval(`JSON.stringify([...document.querySelectorAll('[data-guest-set], [data-guest-note]')].map((e) => e.dataset.guestSet ? 'set:' + e.dataset.guestSet : e.dataset.guestNote))`));
      assert.deepEqual(list, ['set:Set 1', 'Joy to the World', 'Silent Night', 'set:Set 2', 'Sleigh Ride', 'The First Noel']);
      assert.ok(await b.eval(`document.querySelector('[data-testid=guest-note]').textContent.includes('Christmas Eve, two sets.')`));
      // Each song's cover, and its recording's play button.
      await until(() => b.eval(`[...document.querySelectorAll('[data-guest-note] img')].filter((i) => i.complete && i.naturalWidth === 64).length === 4`), 20000, 'four covers').catch(async (e) => {
        const imgs = await b.eval(`JSON.stringify([...document.querySelectorAll('[data-guest-note] img')].map((i) => [i.src, i.complete, i.naturalWidth]))`);
        const link = await fetch(`${n.origin}/s/${CODE}/_guest/link`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ code: CODE, token: '' }) }).then((r) => r.text());
        throw new Error(`${e.message}\n${imgs}\n${link}`);
      });
      assert.equal(await b.eval(`document.querySelectorAll('[data-guest-play]').length`), 4);
      // Nothing scrolls sideways.
      assert.ok(await b.eval(`document.documentElement.scrollWidth <= window.innerWidth`), 'no sideways scroll');
      await sleep(400);
      await b.shot(join(shots, `share-guest-list-${tag}.png`));

      // A song from the list: its words and its picture, the location its page.
      await b.eval(`[...document.querySelectorAll('[data-guest-note="Joy to the World"] button')][0].click()`);
      await until(() => b.eval(`(${q('[data-testid=guest-note] h2')} || {}).textContent === 'Joy to the World'`), 20000, 'the song').catch(async (e) => {
        await b.shot(join(shots, `failed-song-${tag}.png`)).catch(() => {});
        const why = await b.eval(`JSON.stringify({ at: location.pathname, h2: [...document.querySelectorAll('[data-testid=guest-note] h2')].map((h) => h.textContent), row: (${q('[data-guest-note="Joy to the World"]')} || {}).outerHTML })`).catch((x) => x.message);
        throw new Error(`${e.message}\n${why}`);
      });
      await until(() => b.eval(`(() => { const i = ${q('[data-guest-image]')}; return !!i && i.complete && i.naturalWidth === 240; })()`), 20000, 'the stage plot');
      assert.ok(await b.eval(`document.querySelector('[data-testid=guest-note]').textContent.includes('Key of D, capo 2.')`));
      assert.equal(await b.eval('location.pathname'), `/s/${CODE}/gig/joy/`);
      await sleep(300);
      await b.shot(join(shots, `share-guest-note-${tag}.png`));

      // The files: each a download, all of them as the link's archive.
      await b.eval(`${q('[data-testid=guest-files-tab]')}.click()`);
      await until(() => b.eval(`!!${q('[data-testid=guest-download-all]')}`), 10000, 'the files');
      const all = await b.eval(`${q('[data-testid=guest-download-all]')}.getAttribute('href')`);
      assert.equal(all, `/s/${CODE}/_archive.zip?by=note`);
      assert.equal(await b.eval('location.pathname'), `/s/${CODE}/_files/`);
      const files = JSON.parse(await b.eval(`JSON.stringify([...document.querySelectorAll('[data-guest-file]')].map((a) => a.dataset.guestFile))`));
      assert.ok(files.includes('Gig/media/joy.wav') && files.includes('Gig/art/stage.png'), files.join(', '));
      const zip = await fetch(`${f.origin}${all}`);
      assert.equal(zip.status, 200);
      assert.equal(zip.headers.get('content-type'), 'application/zip');
      await sleep(300);
      await b.shot(join(shots, `share-guest-files-${tag}.png`));
    }

    // Opened at the files page: the files.
    await b.go(`${f.origin}/s/${CODE}/_files/`);
    await until(() => b.eval(`!!${q('[data-testid=guest-download-all]')}`), 60000, 'the files, opened at _files/');
    // Every call went to this origin's node, below the link.
    assert.ok(f.proxied.some((p) => p === `POST /s/${CODE}/_guest/link`), f.proxied.join('\n'));
  } finally { await b.close(); f.close(); n.close(); }
});

test('a link that is nowhere says so, from the node and in the app', { skip }, async () => {
  const n = await node();
  try {
    const res = await fetch(`${n.origin}/s/abcdefghjkmnpqrs/`);
    assert.equal(res.status, 404);
    assert.ok(!(await res.text()).includes('start.js'));
    const page = await fetch(`${n.origin}/s/${CODE}/nowhere/`);
    assert.equal(page.status, 404);
  } finally { n.close(); }
});

test('a node with no web app beside it says the link needs one', { skip }, async () => {
  const n = await node({ webApp: false });
  try {
    const res = await fetch(`${n.origin}/s/${CODE}/`);
    assert.equal(res.status, 503);
    assert.match(res.headers.get('content-type'), /^text\/plain/);
    assert.equal(res.headers.get('cache-control'), 'no-store');
    const text = await res.text();
    assert.match(text, /opens in Architect's web app, which this server does not serve/);
    assert.ok(!text.includes('start.js') && !text.includes('Merry'), text);
  } finally { n.close(); }
});

/// An issuer of its own (`auth-server`, ARCHITECT_AUTH_SERVER) on a free
/// port, its database in a temporary folder, its key made here and never
/// shown; and `email` signed up at it: its session token there.
async function issuer(email) {
  const dir = mkdtempSync(join(tmpdir(), 'share-guest-auth-'));
  const port = await freePort();
  const env = { ...process.env, AUTH_BIND_ADDR: `127.0.0.1:${port}`, AUTH_SECRET: randomBytes(32).toString('hex'), AUTH_DATABASE_URL: `sqlite://${join(dir, 'auth.db')}?mode=rwc`, RUST_LOG: 'warn' };
  for (const k of ['AUTH_SECRET_FILE', 'AUTH_DATABASE_URL_FILE', 'AUTH_DEV_SEED', 'AUTH_IMPORT_SNAPSHOT']) delete env[k];
  const proc = spawn(AUTH, [], { stdio: 'ignore', env });
  const origin = `http://127.0.0.1:${port}`;
  const close = () => { proc.kill(); rmSync(dir, { recursive: true, force: true }); };
  let token;
  for (let i = 0; i < 160 && !token; i++) {
    await sleep(250);
    token = await fetch(`${origin}/auth/sign-up/email`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ input: { email, password: 'correct horse battery staple' } }) })
      .then((r) => (r.ok ? r.json() : null), () => null).then((j) => j && j.token, () => null);
  }
  if (!token) { close(); throw new Error('the issuer never signed anyone up'); }
  // Approve a device sign-in's code, as the person does at the issuer.
  const approve = async (userCode) => {
    const r = await fetch(`${origin}/auth/device/approve`, { method: 'POST', headers: { 'content-type': 'application/x-www-form-urlencoded', cookie: `architect-auth.session=${token}` }, body: `user_code=${encodeURIComponent(userCode)}` });
    assert.equal(r.status, 200, 'the issuer approved the code');
  };
  return { origin, approve, close };
}

test('a link for signed-in people: the sign-in, its cookie, the guest calls, the link', { skip: skipSignIn }, async () => {
  const email = 'ada@local.test';
  const auth = await issuer(email);
  const n = await node({ invited: [email], issuer: auth.origin });
  const f = await front(n.origin);
  const b = await firefox();
  const shots = SHOTS || '/tmp/share-web-guest';
  mkdirSync(shots, { recursive: true });
  try {
    // Nobody signed in: the app's page names nothing of the link, and the
    // app offers the node's sign-in.
    const res = await fetch(`${n.origin}/s/${CODE}/`);
    assert.equal(res.status, 401);
    assert.ok(!(await res.text()).includes('Merry and Bright'));
    await b.go(`${f.origin}/s/${CODE}/`);
    await until(() => b.eval(`!!${q('[data-testid=guest-sign-in]')}`), 60000, 'the Sign in button');
    assert.equal(await b.eval(`!!document.getElementById('share-stall')`), false);
    // A session this browser's app keeps that the issuer does not know
    // (signed out elsewhere, not yet expired here): it must not keep the
    // person at "Sign in" once the node's cookie is there.
    await b.eval(`localStorage.setItem('architect.session', JSON.stringify({ token: 'a-session-the-issuer-does-not-know', expires_at_unix: Math.floor(Date.now() / 1000) + 3600 })), true`);

    // Sign in: the node starts the issuer's device sign-in and waits.
    await b.eval(`${q('[data-testid=guest-sign-in]')}.click(), true`);
    await until(() => b.eval(`location.pathname === '/s/_signin' && !!document.querySelector('b')`), 20000, 'the sign-in page');
    const userCode = await b.eval(`document.querySelector('b').textContent`);
    assert.ok(await b.eval(`!!${q('a.button')} && ${q('a.button')}.href.startsWith(${JSON.stringify(auth.origin)})`), 'the approval is at the issuer');
    await b.shot(join(shots, 'share-guest-signin-wait.png'));
    await auth.approve(userCode);

    // The page goes on by itself: the node's cookie, back at the link, the
    // link shown through guest calls that carry the cookie.
    await until(() => b.eval(`(${q('[data-testid=guest-title]')} || {}).textContent === 'Merry and Bright'`), 60000, 'the link, signed in').catch(async (e) => {
      await b.shot(join(shots, 'failed-signed-in.png')).catch(() => {});
      const page = await b.eval(`location.href + '\\n' + document.body.innerText.slice(0, 800)`).catch((x) => x.message);
      throw new Error(`${e.message}\n${page}\n${f.proxied.join('\n')}`);
    });
    assert.equal(await b.eval('location.pathname'), `/s/${CODE}/`);
    await until(() => b.eval(`document.querySelectorAll('[data-guest-note]').length === ${SONGS.length + 1} || document.querySelectorAll('[data-guest-note]').length === ${SONGS.length}`), 20000, 'the link\'s notes');
    const cookie = (await b.cookies()).find((c) => c.name === 'architect_session');
    assert.ok(cookie, 'the node set its session cookie');
    assert.equal(cookie.httpOnly, true);
    assert.equal(cookie.sameSite, 'lax');
    assert.ok(!(await b.eval('document.cookie')).includes('architect_session'), 'never readable by script');
    assert.ok(f.proxied.includes('GET /s/_signin/wait'), f.proxied.join('\n'));
    assert.ok(f.proxied.filter((p) => p === `POST /s/${CODE}/_guest/link`).length >= 2, f.proxied.join('\n'));
    // The same guest call without the cookie is still refused: it was the
    // cookie that opened it.
    const bare = await fetch(`${n.origin}/s/${CODE}/_guest/link`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ code: CODE, token: '' }) });
    assert.equal(bare.status, 401);
    await b.shot(join(shots, 'share-guest-signed-in-390.png'));
  } finally { await b.close(); f.close(); n.close(); auth.close(); }
});
