// A distribution's linked views for a share link's guest (#805), in a real
// browser: the release bundle (ARCHITECT_WEB_BUNDLE) behind a front that
// does what production's proxy does (the app's files with its
// Content-Security-Policy from apps/web/sws.toml; `/_ext/` and `/s/` to
// the node), a node (`island_node`, ARCHITECT_ISLAND_NODE) sharing a
// folder whose notes are of the view probe's type, `probe-note`
// (apps/web/src/app/view_probe.rs: a linked extension compiled into the
// app, as a distribution's are). The node's island claims the same type:
// the linked view wins, and no frame is mounted for it.
//
// `/s/<code>/page/?probe=views`, the app as the link's guest, at 390 px:
//
// - the note is drawn by the probe's linked view in the app's document,
//   handed its text as the link's node gave it (no `%%comments%%`, no
//   link past the link, no frontmatter) and the link's art and lyrics,
//   never a vault path or a property, in the shape an island's view gets;
// - the view plays its own source on the playhead, as a guest;
// - the guest's host refuses it every vault request (documents, vault
//   queries, kept data);
// - its `open` (NOTE_OPEN) opens a note the link lists and refuses one it
//   does not;
// - the type's favourites tab "More" opens its `more` view, handed the
//   link's root note;
// - `probe=views,silent` (its manifest declares no read): the view is
//   drawn and handed no note at all.
//
// Firefox over WebDriver BiDi (FIREFOX=… or `firefox` on the PATH), in a
// mount namespace whose runtime dir and /run/pipewire are empty (`unshare
// -r -m`). SHOTS=<dir> keeps the screenshots. `just
// app-guest-linked-test` builds and runs it.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync, readFileSync, existsSync, statSync } from 'node:fs';
import { createServer, request } from 'node:http';
import { createServer as createNetServer } from 'node:net';
import { tmpdir } from 'node:os';
import { crc32, deflateSync } from 'node:zlib';
import { dirname, join, extname, normalize } from 'node:path';
import { firefox, until, sleep, shotDir, appPolicy, BUNDLE, browser } from './lib/app-bidi.mjs';

const NODE = process.env.ARCHITECT_ISLAND_NODE;
const skip = NODE && BUNDLE && browser ? false : 'needs ARCHITECT_ISLAND_NODE, ARCHITECT_WEB_BUNDLE (just app-guest-linked-test) and a Firefox (FIREFOX=…)';
if (process.env.ARCHITECT_BROWSER_REQUIRED && skip) throw new Error(`ARCHITECT_BROWSER_REQUIRED: ${skip}`);
const put = (root, rel, body) => { const p = join(root, rel); mkdirSync(dirname(p), { recursive: true }); writeFileSync(p, body); };
const freePort = () => new Promise((ok) => { const srv = createNetServer(); srv.listen(0, '127.0.0.1', () => { const { port } = srv.address(); srv.close(() => ok(port)); }); });
const WASM = Buffer.from([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]);
const CODE = 'probeviewsshared';

/// A `size`×`size` PNG: the note's art.
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
    for (let x = 0; x < size; x++) { row[1 + x * 3] = 40; row[2 + x * 3] = Math.round((x + y) * 160 / (2 * size)); row[3 + x * 3] = 200; }
    rows.push(row);
  }
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk('IHDR', head), chunk('IDAT', deflateSync(Buffer.concat(rows))), chunk('IEND', Buffer.alloc(0))]);
}
const PNG = png(16);

/// The island's glue: it would report what it was handed, were it mounted.
const GLUE = `
export default async function init() {}
export function mount(view) {
  document.body.dataset.view = view;
  globalThis.architectPage.subscribe((p) => { document.body.dataset.report = JSON.stringify(p.note); });
}`;

/// A node sharing `Shared/` by link `CODE`, its island claiming
/// `probe-note` too (view `list`, reading the note and its files).
async function node() {
  const dir = mkdtempSync(join(tmpdir(), 'guest-linked-'));
  const vault = join(dir, 'vault');
  put(vault, 'Shared/page.md', '---\ntitle: Probe page\ntype: probe-note\nsecret: the alarm code\n---\n'
    + 'The shared words.\n\n- [[Shared/second|Second]]\n- [[Private/plan|the plan]]\n%%call the venue first%%\n\n'
    + '![[Shared/art.png]]\n\n![[Shared/words.lrc]]\n\n![[Private/cover.png]]\n');
  put(vault, 'Shared/second.md', '---\ntitle: Second\n---\nThe second note.\n');
  put(vault, 'Shared/art.png', PNG);
  put(vault, 'Shared/words.lrc', '[00:01.00]The words');
  put(vault, 'Private/plan.md', '# Plan\n\nThe secret plan.\n');
  put(vault, 'Private/cover.png', 'private png');
  const share = { code: CODE, vault: 'notes', scope: { Folder: 'Shared' }, title: 'Probe page', audience: 'Link', access: 'Read', expires_at: null, revoked_at: null, created_at: 1, created_by: 'test', domain: null };
  put(vault, `_architect/shares/${CODE}.md`, `---\npublish: false\n---\n# Share ${CODE}\n\n\`\`\`json\n${JSON.stringify(share)}\n\`\`\`\n`);
  const folder = join(dir, 'island');
  put(folder, 'web-view.js', GLUE);
  put(folder, 'web-view_bg.wasm', WASM);
  put(folder, 'architect.json', JSON.stringify({ module: 'web-view.js', manifest: { id: 'demo.probe-island', capabilities: {} } }));
  const env = {
    ...process.env, ARCHITECT_DATA_DIR: join(dir, 'data'), ARCHITECT_VAULTS: `notes=${vault}`,
    ARCHITECT_IROH: 'local', ARCHITECT_NAME: 'guest-linked-test',
    ARCHITECT_ISLAND_FOLDER: folder, ARCHITECT_ISLAND_ID: 'demo.probe-island',
    ARCHITECT_ISLAND_VIEW: 'progress', ARCHITECT_ISLAND_TRANSPORT: 'transport',
    ARCHITECT_ISLAND_EMBEDS: 'list=probe-note', ARCHITECT_ISLAND_READS: 'documents,files',
    ARCHITECT_WEB_APP: BUNDLE,
  };
  delete env.ARCHITECT_ISLAND2_FOLDER;
  delete env.ARCHITECT_FAVOURITES;
  const port = await freePort();
  env.ARCHITECT_BIND = `127.0.0.1:${port}`;
  const proc = spawn(NODE, [], { stdio: 'ignore', env });
  const origin = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 160; i++) {
    await sleep(250);
    const ok = await fetch(`${origin}/s/${CODE}/page/`).then((r) => r.ok, () => false);
    if (ok) return { origin, close: () => { proc.kill(); rmSync(dir, { recursive: true, force: true }); } };
  }
  proc.kill();
  throw new Error('the node never built the link');
}

const TYPES = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' };

/// The front: `/_ext/` and `/s/` to the node, the rest the bundle's
/// files, the app's page with its policy.
async function front(nodeOrigin) {
  const policy = appPolicy();
  const port = await freePort();
  const target = new URL(nodeOrigin);
  const srv = createServer((req, res) => {
    const path = new URL(req.url, 'http://x').pathname;
    if (path.startsWith('/_ext/') || path.startsWith('/s/')) {
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
  return { origin: `http://127.0.0.1:${port}`, close: () => srv.close() };
}

/// What the probe's element `testid` under `scope` was handed.
const handed = (b, scope, testid) => b.eval(`(() => { const e = document.querySelector('${scope} [data-testid=${testid}]'); return e ? JSON.stringify({ claim: e.dataset.claim, props: JSON.parse(e.dataset.props), playing: e.dataset.playing, source: (e.querySelector('[data-testid=probe-source]') || {}).textContent }) : null; })()`).then((v) => v && JSON.parse(v));
/// One host-service call as the probe (`window.architectViewProbe`).
const ask = (b, request) => b.eval(`window.architectViewProbe(${JSON.stringify(JSON.stringify(request))})`).then(JSON.parse);
const tab = (b, label) => b.eval(`(() => { const t = [...document.querySelectorAll('button[data-tab]')].find((x) => x.getAttribute('aria-label') === ${JSON.stringify(label)}); if (!t) return false; t.click(); return true; })()`);
const NOTE = '[data-testid=guest-note]';

/// The link opened as its guest at 390 px with `probe`; `run(b)` then.
async function asGuest(probe, run) {
  const n = await node();
  const f = await front(n.origin);
  const b = await firefox().catch((e) => { f.close(); n.close(); throw e; });
  try {
    await b.size(390, 844);
    await b.go(`${f.origin}/s/${CODE}/page/?probe=${probe}`);
    await until(() => b.eval(`!!document.querySelector('${NOTE}')`), 60000, 'the guest note');
    await run(b);
    const panics = b.logs.filter((l) => /panicked|RuntimeError|unreachable/.test(l));
    assert.deepEqual(panics, [], 'no panic in the page');
  } finally { await b.close(); f.close(); n.close(); }
}

test('a guest\'s note is drawn by the linked view, handed what the link admits', { skip }, async () => {
  const shots = shotDir('guest-linked-shots');
  await asGuest('views', async (b) => {
    const ids = await until(async () => { const v = await b.eval('document.documentElement.dataset.extensions || ""'); return v && v.split(' '); }, 20000, 'the extensions to load');
    assert.ok(ids.includes('web.views'), `the probe runs for a guest: ${ids.join(' ')}`);
    // The linked view is the note's page: no frame for the island's claim.
    const got = await until(() => handed(b, '[data-testid=guest-note-view]', 'probe-note'), 30000, 'the linked view as the note');
    assert.equal(await b.eval(`document.querySelectorAll('[data-testid=ext-frame]').length`), 0, 'the linked view wins: no island frame');
    assert.equal(got.claim, 'note_type:probe-note');
    const note = got.props.note;
    assert.equal(note.title, 'Probe page');
    assert.equal(note.type, 'probe-note');
    assert.ok(!('path' in note), 'never a vault path');
    assert.deepEqual(note.properties, {}, 'no property a linked claim did not declare');
    assert.match(note.body, /The shared words\./);
    for (const never of ['call the venue', 'Private/', 'secret', 'alarm', '---']) assert.ok(!note.body.includes(never), `${never} in ${note.body}`);
    assert.equal(got.source, note.body, 'use_embed().source is the text it reads');
    const files = Object.fromEntries((note.files || []).map((x) => [x.path, x]));
    assert.deepEqual(Object.keys(files).sort(), ['Shared/art.png', 'Shared/words.lrc']);
    assert.equal(files['Shared/art.png'].data, `data:image/png;base64,${PNG.toString('base64')}`);
    assert.equal(files['Shared/words.lrc'].text, '[00:01.00]The words');
    assert.equal(await b.eval(`document.querySelector('${NOTE}').dataset.files`), '2');
    await b.shot(join(shots, 'guest-linked-note-390.png'));

    // It plays its own source, as a guest.
    await b.eval(`document.querySelector('[data-testid=probe-play]').click()`);
    await until(async () => (await handed(b, '[data-testid=guest-note-view]', 'probe-note'))?.playing === 'true', 10000, 'its own source playing');

    // A guest's host refuses it every vault request.
    for (const request of [
      { op: 'read_document', path: 'Shared/page.md' },
      { op: 'list_documents' },
      { op: 'query_vault', source: 'views: []' },
      { op: 'data', key: 'k' },
      { op: 'set_data', key: 'k', value: 1 },
    ]) {
      const answer = await ask(b, request);
      assert.match(answer.error || '', /^refused \(guest\)/, `${JSON.stringify(request)}: ${JSON.stringify(answer)}`);
    }

    // The type's favourites: "More" opens its `more` view, handed the
    // link's root note.
    await until(() => tab(b, 'More'), 15000, 'the declared tab More');
    const more = await until(() => handed(b, '[data-testid=page-view]', 'probe-more'), 15000, 'the More view');
    assert.equal(more.props.note?.title, 'Probe page');
    assert.ok(!('path' in (more.props.note || {})), 'never a vault path');
    await b.shot(join(shots, 'guest-linked-more-390.png'));

    // NOTE_OPEN: a note the link lists opens, one it does not is refused.
    const listed = await ask(b, { op: 'open', path: 'Shared/second.md' });
    assert.equal(listed.ok?.opened, 'Shared/second.md', JSON.stringify(listed));
    await until(() => b.eval(`document.body.innerText.includes('The second note.')`), 15000, 'the listed note opened');
    const unlisted = await ask(b, { op: 'open', path: 'Private/plan.md' });
    assert.match(unlisted.error || '', /not a note/, JSON.stringify(unlisted));
    assert.ok(!(await b.eval(`document.body.innerText.includes('The secret plan.')`)), 'nothing of the unlisted note');
  });
});

test('a linked view that declares no read is handed no note for a guest', { skip }, async () => {
  await asGuest('views,silent', async (b) => {
    // Drawn (a card at the note's head: with no text to draw it is not
    // the note's page), and handed nothing of it.
    const got = await until(() => handed(b, NOTE, 'probe-note'), 30000, 'the silent linked view');
    assert.deepEqual(got.props, { note: null });
    assert.equal(got.source, '');
    assert.equal(await b.eval(`document.querySelector('${NOTE}').dataset.text`), 'false');
    assert.equal(await b.eval(`document.querySelector('${NOTE}').dataset.files`), '0');
    assert.equal(await b.eval(`document.querySelectorAll('[data-testid=ext-frame]').length`), 0, 'the linked view still wins over the island that reads');
  });
});
