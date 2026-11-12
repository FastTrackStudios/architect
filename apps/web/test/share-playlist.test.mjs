// A share link's playlist and now-playing view in a real browser
// (studio#445, architect#594; `just share-player-test` runs it with
// share-player.test.mjs): a headless node (`architect-desktop serve`,
// ARCHITECT_DESKTOP names the binary) on a vault of its own, in a headless
// Firefox over WebDriver BiDi at a phone's width (FIREFOX=… names one).
//
// - A share with no playlist keeps the bar's song a link to its page.
// - A row's `data-page` is a note's to write: the view is fetched only from
//   a page the link built (a raw `.html` attachment, the node's own
//   `/s/_signout`, a vault `index.html` are never fetched or imported).
// - The view: the position bar at the top, the chart, the controls under
//   it; focus moves in, the page behind is inert, Escape gives it back.
// - A page that does not load says so and is asked for again next time.
// - A recording the node refuses stops the queue and says so.
// - Every playlist of a page is one queue; the page's own media are not.
//
// Run with `env -u LD_LIBRARY_PATH` from a dev shell: its libstdc++ stops
// Firefox starting. Nothing leaves the machine.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

const desktop = process.env.ARCHITECT_DESKTOP;
function firefox() {
  if (process.env.FIREFOX) return process.env.FIREFOX;
  try { return execFileSync('sh', ['-c', 'command -v firefox']).toString().trim() || null; } catch { return null; }
}
const skip = desktop && firefox() ? false : 'needs ARCHITECT_DESKTOP (just share-player-test) and a Firefox (FIREFOX=…)';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function put(root, rel, body) {
  const path = join(root, rel);
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, body);
}

/// `seconds` of a quiet 220 Hz tone: 8 kHz, 16-bit mono PCM.
function wav(seconds) {
  const rate = 8000;
  const n = Math.round(seconds * rate);
  const buf = Buffer.alloc(44 + n * 2);
  buf.write('RIFF', 0); buf.writeUInt32LE(36 + n * 2, 4); buf.write('WAVE', 8);
  buf.write('fmt ', 12); buf.writeUInt32LE(16, 16); buf.writeUInt16LE(1, 20); buf.writeUInt16LE(1, 22);
  buf.writeUInt32LE(rate, 24); buf.writeUInt32LE(rate * 2, 28); buf.writeUInt16LE(2, 32); buf.writeUInt16LE(16, 34);
  buf.write('data', 36); buf.writeUInt32LE(n * 2, 40);
  for (let i = 0; i < n; i++) buf.writeInt16LE(Math.round(800 * Math.sin((2 * Math.PI * 220 * i) / rate)), 44 + i * 2);
  return buf;
}

const CODE = 'gigabcdefhjkmnpq';
const L = `/s/${CODE}`;

/// A row of a playlist: its recording and the page that shows it.
const row = (title, media, page) =>
  `<li data-track data-media="${L}/Gig/media/${media}" data-page="${page}" data-title="${title}" data-subtitle="${title} by us">${title} row</li>`;

/// A node serving a link share of `Gig/`: two songs with pages, and a
/// setlist page of two playlists whose rows point at good, missing and
/// hostile pages.
async function node(dir) {
  const vault = join(dir, 'vault');
  put(vault, 'Gig/01-joy.md', '---\ntitle: Joy\n---\n![[media/joy.wav]]\n\n![[media/joy.lrc]]\n\n```chart\n| C | G | Am | F | C | G | C |\n```\n');
  // An extension draws the ```chart fence as a song's now-playing view:
  // an SVG with a transparent rect per bar, its time on it.
  const ext = join(dir, 'data', 'extensions', 'demo.chart');
  put(ext, 'extension.toml', 'id = "demo.chart"\nname = "Chart"\nversion = "0.1.0"\nkind = "data"\n\n[[embeds]]\nid = "chart"\npage = "pages/chart.tsx"\nclaims = { fence = ["chart"] }\n');
  put(ext, 'pages/chart.tsx', `import { h, useEmbed } from "architect";
export default function Chart() {
  const bars = (useEmbed()?.source ?? "").split("|").map((b) => b.trim()).filter((b) => b !== "");
  const w = 120;
  return (
    <div data-now-playing="">
      <svg class="demo-chart" viewBox={\`0 0 \${bars.length * w} 60\`} width="100%" data-follow-playhead="">
        {bars.map((b, i) => (
          <g key={\`\${i}\`}>
            <rect class="demo-bar" data-at={\`\${i * 0.5}\`} x={\`\${i * w}\`} y="0" width={\`\${w}\`} height="60" fill="transparent" />
            <text x={\`\${i * w + 8}\`} y="36">{b}</text>
          </g>
        ))}
      </svg>
    </div>
  );
}
`);
  put(vault, 'Gig/02-noel.md', '---\ntitle: Noel\n---\n![[media/noel.wav]]\n');
  put(vault, 'Gig/03-list.md', '---\ntitle: List\n---\n' +
    '<div data-playlist data-title="First set"><button data-playlist-play>Play all</button><ul>' +
    row('Joy', 'joy.wav', `${L}/01-joy/`) +
    row('Evil', 'evil.wav', `${L}/Gig/evil.html`) +
    row('Signout', 'signout.wav', '/s/_signout?to=/') +
    '</ul></div>\n\n' +
    '<div data-playlist data-title="Second set"><ul>' +
    row('Raw', 'raw.wav', `${L}/Gig/raw/`) +
    row('Nope', 'nope.wav', `${L}/nope/`) +
    row('Missing', 'missing.wav', `${L}/02-noel/`) +
    '</ul></div>\n\n![[media/noel.wav]]\n\n[[evil.html]] [[raw/index.html]] [[media/evil.wav]] [[media/signout.wav]] [[media/raw.wav]] [[media/nope.wav]]\n');
  put(vault, 'Gig/evil.html', `<!doctype html><html><body><div class="site-block"><div data-now-playing data-media="${L}/Gig/media/joy.wav"><p id="pwn">RAW ATTACHMENT</p><style>html body{outline:9px solid red !important}</style><img src="data:," onerror="window.__xss=1"><meta http-equiv="refresh" content="1;url=${L}/02-noel/"></div></div></body></html>`);
  put(vault, 'Gig/raw/index.html', `<!doctype html><html><body><div data-now-playing><p id="pwn">RAW INDEX</p><meta http-equiv="refresh" content="1;url=${L}/02-noel/"></div></body></html>`);
  put(vault, 'Gig/media/joy.lrc', '[00:00.00]Joy to the world\n[00:00.60]The Lord is come\n[00:01.20]Let earth\n');
  put(vault, 'Gig/media/joy.wav', wav(3.5));
  for (const name of ['noel', 'evil', 'signout', 'raw', 'nope']) put(vault, `Gig/media/${name}.wav`, wav(3));
  const share = {
    code: CODE, vault: 'notes', scope: { Folder: 'Gig' }, title: 'The gig', audience: 'Link', access: 'Read',
    expires_at: null, revoked_at: null, created_at: 1, created_by: 'test', domain: null,
  };
  put(vault, `_architect/shares/${CODE}.md`, `---\npublish: false\n---\n# Share ${CODE}\n\n\`\`\`json\n${JSON.stringify(share)}\n\`\`\`\n`);
  const port = 19900 + Math.floor(Math.random() * 90);
  const config = join(dir, 'node.toml');
  writeFileSync(config, `name = "playlist-test"\nbind = "127.0.0.1:${port}"\ndata_dir = "${join(dir, 'data')}"\niroh = "local"\nsweep_secs = 3600\n\n[vaults]\nnotes = "${vault}"\n`);
  const proc = spawn(desktop, ['serve', '--config', config], { stdio: 'ignore' });
  const origin = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 160; i++) {
    await sleep(250);
    const html = await fetch(`${origin}${L}/03-list/`).then((r) => (r.ok ? r.text() : '')).catch(() => '');
    if (html.includes('data-playlist')) return { proc, origin };
  }
  proc.kill();
  throw new Error('the node never served the list');
}

/// A headless Firefox that may play sound unasked, one tab at a phone's
/// width (set after a first navigation: before one, it does not hold).
async function open() {
  const profile = mkdtempSync(join(tmpdir(), 'architect-playlist-'));
  writeFileSync(join(profile, 'user.js'), 'user_pref("media.autoplay.default", 0);\nuser_pref("media.autoplay.blocking_policy", 0);\n');
  const port = 19000 + Math.floor(Math.random() * 900);
  const proc = spawn(firefox(), ['--headless', '--no-remote', '--profile', profile, '--remote-debugging-port', String(port)], { stdio: 'ignore' });
  let ws;
  for (let i = 0; i < 80 && !ws; i++) {
    await sleep(250);
    try {
      const s = new WebSocket(`ws://127.0.0.1:${port}/session`);
      await new Promise((ok, no) => { s.onopen = ok; s.onerror = no; });
      ws = s;
    } catch {}
  }
  if (!ws) throw new Error('firefox did not start');
  let id = 0;
  const waiting = new Map();
  ws.onmessage = (e) => {
    const m = JSON.parse(e.data);
    if (m.id && waiting.has(m.id)) {
      const w = waiting.get(m.id);
      waiting.delete(m.id);
      m.type === 'error' ? w.no(new Error(`${m.error}: ${m.message}`)) : w.ok(m.result);
    }
  };
  const send = (method, params = {}) => new Promise((ok, no) => {
    const i = ++id;
    waiting.set(i, { ok, no });
    ws.send(JSON.stringify({ id: i, method, params }));
  });
  await send('session.new', { capabilities: {} });
  const context = (await send('browsingContext.getTree', {})).contexts[0].context;
  let sized = false;
  const tab = {
    go: async (url) => {
      if (!sized) {
        await send('browsingContext.navigate', { context, url: 'about:blank', wait: 'complete' }).catch(() => {});
        await send('browsingContext.setViewport', { context, viewport: { width: 390, height: 844 } });
        sized = true;
      }
      return send('browsingContext.navigate', { context, url, wait: 'complete' });
    },
    eval: async (expression) => {
      const r = await send('script.evaluate', { expression, target: { context }, awaitPromise: true, userActivation: true });
      if (r.type === 'exception') throw new Error(r.exceptionDetails.text);
      return r.result.value;
    },
    async until(expression, what, ms = 10000) {
      const started = Date.now();
      for (;;) {
        const v = await tab.eval(expression).catch(() => null);
        if (v) return v;
        if (Date.now() - started > ms) throw new Error(`waited ${ms} ms for ${what}`);
        await sleep(50);
      }
    },
  };
  return {
    tab,
    close: () => {
      ws.close();
      proc.kill();
      try { rmSync(profile, { recursive: true, force: true }); } catch {}
    },
  };
}

/// The URLs the page fetched (not its media or assets).
const FETCHED = 'JSON.stringify(performance.getEntriesByType("resource").filter((e) => e.initiatorType === "fetch").map((e) => new URL(e.name).pathname))';
/// The row named `title`, clicked.
const tap = (title) => `[...document.querySelectorAll("[data-track]")].find((r) => r.dataset.title === ${JSON.stringify(title)}).click(), true`;
const OPEN = 'document.querySelector(".site-player-title").click(), true';

let shared;
async function setup() {
  if (!shared) {
    const dir = mkdtempSync(join(tmpdir(), 'architect-playlist-gig-'));
    shared = { dir, ...(await node(dir)) };
  }
  return shared;
}
test.after(() => {
  if (shared) {
    shared.proc.kill();
    rmSync(shared.dir, { recursive: true, force: true });
  }
});

test('a share with no playlist keeps the song a link to its page', { skip, timeout: 90000 }, async () => {
  const { origin } = await setup();
  const { tab, close } = await open();
  try {
    await tab.go(`${origin}${L}/02-noel/`);
    await tab.until('document.querySelector(".site-player")', 'the bar');
    assert.equal(await tab.eval('document.querySelector(".site-player-title").tagName'), 'A');
    assert.equal(await tab.eval('!!document.querySelector(".site-now")'), false, 'no view without a playlist');
    await tab.eval('architect.playhead.seek(0, { src: "' + L + '/Gig/media/joy.wav", play: false }), true');
    await tab.until('document.querySelector(".site-player-title").getAttribute("href")', 'the song linked');
    assert.equal(await tab.eval('new URL(document.querySelector(".site-player-title").href).pathname'), `${L}/01-joy/`);
    assert.equal(await tab.eval('document.querySelector(".site-player-title").hasAttribute("aria-haspopup")'), false);
    await tab.eval(OPEN);
    await tab.until(`location.pathname === "${L}/01-joy/"`, 'the song\'s page');
  } finally { close(); }
});

test('the view is fetched only from a page the link built', { skip, timeout: 90000 }, async () => {
  const { origin } = await setup();
  // The node: a vault index.html is not published; a page is marked built.
  assert.equal((await fetch(`${origin}${L}/Gig/raw/index.html`)).status, 404, 'a vault index.html is not copied');
  const raw = await fetch(`${origin}${L}/Gig/raw/`);
  assert.equal(raw.status, 404, 'so its folder is no page');
  assert.notEqual(raw.headers.get('x-architect-page'), 'built');
  const page = await fetch(`${origin}${L}/01-joy/`);
  assert.equal(page.headers.get('x-architect-page'), 'built');
  const evil = await fetch(`${origin}${L}/Gig/evil.html`);
  assert.equal(evil.headers.get('x-architect-page'), null, 'an attachment is not a page');

  const { tab, close } = await open();
  try {
    await tab.go(`${origin}${L}/03-list/`);
    for (const title of ['Evil', 'Signout']) {
      await tab.eval(tap(title));
      await tab.until(`architect.playhead.src && document.querySelector(".site-player-title").textContent === ${JSON.stringify(title)}`, title);
      // Not a page the link built: no view to open, no page to link.
      assert.equal(await tab.eval('document.querySelector(".site-player-title").hasAttribute("href")'), false, title);
      assert.equal(await tab.eval('document.querySelector(".site-player-title").hasAttribute("aria-haspopup")'), false, title);
      await tab.eval(OPEN);
      await sleep(300);
      assert.equal(await tab.eval('document.querySelector(".site-now").hidden'), true, `${title} opens nothing`);
    }
    // Under the link and ending in `/`, but not built: fetched, refused.
    await tab.eval(tap('Raw'));
    await tab.until('document.querySelector(".site-player-title").textContent === "Raw"', 'Raw');
    await tab.eval(OPEN);
    await tab.until('/could not be loaded|not one this link built/.test(document.querySelector(".site-now-body").textContent)', 'the refusal said');
    await sleep(1500);
    const fetched = JSON.parse(await tab.eval(FETCHED));
    assert.ok(!fetched.some((p) => p.includes('evil.html') || p.includes('_signout')), JSON.stringify(fetched));
    assert.equal(await tab.eval('!!document.querySelector("#pwn")'), false, 'nothing imported');
    assert.equal(await tab.eval('!!document.querySelector(".site-now meta, .site-now style:not(svg style), .site-now script")'), false);
    assert.equal(await tab.eval('getComputedStyle(document.body).outlineStyle'), 'none', 'the page not restyled');
    assert.equal(await tab.eval('String(window.__xss)'), 'undefined');
    assert.equal(await tab.eval('location.pathname'), `${L}/03-list/`, 'no refresh took the page away');
  } finally { close(); }
});

test('the view: position at the top, the chart, the controls; focus, inert, Escape', { skip, timeout: 90000 }, async () => {
  const { origin } = await setup();
  const { tab, close } = await open();
  try {
    await tab.go(`${origin}${L}/03-list/`);
    await tab.eval(tap('Joy'));
    await tab.until('architect.playhead.playing', 'Joy playing');
    assert.equal(await tab.eval('document.querySelector(".site-player-title").getAttribute("aria-haspopup")'), 'dialog');
    await tab.eval(OPEN);
    await tab.until('document.querySelector(".site-now-body .demo-chart")', 'the chart in the view');
    // Top to bottom: where it is, the chart, the controls.
    const tops = JSON.parse(await tab.eval('JSON.stringify([".site-now-progress", ".site-now-body", ".site-now-controls"].map((s) => document.querySelector(s).getBoundingClientRect().top))'));
    assert.ok(tops[0] < tops[1] && tops[1] < tops[2], JSON.stringify(tops));
    // It follows there.
    await tab.until('document.querySelector(".site-now .demo-bar[aria-current=time]")', 'a bar marked in the view');
    // Focus in, the page behind inert, modal at a phone's width.
    assert.equal(await tab.eval('document.activeElement.className'), 'site-now-close');
    assert.equal(await tab.eval('document.querySelector(".site-now").getAttribute("aria-modal")'), 'true');
    assert.equal(await tab.eval('document.querySelector(".site-player").hasAttribute("inert")'), true);
    // Lyrics a tap away; the song's page linked.
    assert.equal(await tab.eval('document.querySelector(".site-now-lyrics").hidden'), false);
    await tab.eval('document.querySelector(".site-now-lyrics").click(), true');
    assert.equal(await tab.eval('getComputedStyle(document.querySelector(".site-now-words")).display'), 'block');
    // They follow there too.
    await tab.until('document.querySelector(".site-now-words [aria-current=time]")', 'a lyric line marked in the view');
    assert.equal(await tab.eval('new URL(document.querySelector(".site-now-page").href).pathname'), `${L}/01-joy/`);
    // The toggle is not redrawn while it plays.
    await tab.eval('architect.playhead.seek(0), window.__m = 0, new MutationObserver((l) => (window.__m += l.length)).observe(document.querySelector(".site-now .site-player-toggle"), { childList: true }), true');
    await sleep(1000);
    assert.equal(await tab.eval('window.__m'), 0, 'no redraw while the state holds');
    // The lock screen knows the song.
    assert.equal(await tab.eval('navigator.mediaSession && navigator.mediaSession.metadata && navigator.mediaSession.metadata.title'), 'Joy');
    // Escape: closed, focus back on the bar's song, still playing.
    await tab.eval('document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })), true');
    await tab.until('document.querySelector(".site-now").hidden', 'closed');
    assert.equal(await tab.eval('document.activeElement.className'), 'site-player-title');
    assert.equal(await tab.eval('document.querySelector(".site-player").hasAttribute("inert")'), false);
    assert.equal(await tab.eval('architect.playhead.playing'), true);
  } finally { close(); }
});

test('a page that does not load says so, and is asked for again', { skip, timeout: 90000 }, async () => {
  const { origin } = await setup();
  const { tab, close } = await open();
  try {
    await tab.go(`${origin}${L}/03-list/`);
    await tab.eval(tap('Nope'));
    await tab.until('document.querySelector(".site-player-title").textContent === "Nope"', 'Nope');
    await tab.eval(OPEN);
    await tab.until('/\\(404\\)/.test(document.querySelector(".site-now-body").textContent)', 'the 404 said');
    await tab.eval('document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })), true');
    await tab.until('document.querySelector(".site-now").hidden', 'closed');
    await tab.eval(OPEN);
    await tab.until(`JSON.parse(${FETCHED}).filter((p) => p === "${L}/nope/").length === 2`, 'asked again');
  } finally { close(); }
});

test('the queue: every playlist of the page in order, its own media not; a refused file stops it', { skip, timeout: 90000 }, async () => {
  const { origin } = await setup();
  const { tab, close } = await open();
  try {
    await tab.go(`${origin}${L}/03-list/`);
    // Six rows over two playlists: the page's own `![[media/noel.wav]]`
    // is not queued.
    assert.equal(await tab.eval('architect.playhead.snapshot().tracks'), 6);
    await tab.eval(tap('Signout'));
    await tab.until('architect.playhead.playing', 'the first set\'s last playing');
    await tab.eval('architect.playhead.next(), true');
    await tab.until(`architect.playhead.src === "${L}/Gig/media/raw.wav"`, 'the second set\'s first next');
    await tab.eval(tap('Nope'));
    await tab.until(`architect.playhead.src === "${L}/Gig/media/nope.wav"`, 'Nope');
    await tab.eval('architect.playhead.next(), true');
    await tab.until(`architect.playhead.src === "${L}/Gig/media/missing.wav"`, 'Missing next');
    // The node refuses it: said, stopped, not skipped past.
    await tab.until('/could not be played/.test(document.querySelector(".site-player-note").textContent)', 'the refusal said');
    await sleep(800);
    assert.equal(await tab.eval('architect.playhead.playing'), false);
    assert.equal(await tab.eval('architect.playhead.src'), `${L}/Gig/media/missing.wav`);
    // A repaint keeps what it said.
    await tab.eval('architect.playhead.seek(0), true');
    await sleep(200);
    assert.match(await tab.eval('document.querySelector(".site-player-note").textContent'), /could not be played/);
  } finally { close(); }
});
