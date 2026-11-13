// A share link's page on a phone, measured in a real browser (Firefox,
// WebDriver BiDi, its viewport set exactly): a node (`island_node`,
// ARCHITECT_ISLAND_NODE) sharing a setlist note whose playlist is drawn as
// Studio draws one, the phone shell from a setlist's `[[favourites]]`
// (ARCHITECT_FAVOURITES) or the share's own tabs.
//
// At 390×844 and 360×740:
// - one title, at the top: the header shows the page's title, and exactly
//   one element on the page shows it;
// - the list's gutters: the number column at the 16 px gutter (three
//   digits fit), the tile about 46 px in, the "…" glyph on the right
//   gutter, nothing scrolling sideways; a set's label on the same gutter;
// - the card 8 px above the tab bar, on its gutters, nothing loaded or
//   playing, with or without the round button; scrolled to the end, the
//   last row and the "Files" line clear of the card;
// - contrast: a song with no recording's title, artist and number on the
//   page, and a key's white on its tile, at 4.5:1 or more.
// At 844×390 with a finger (a phone turned): the shell, and Charts a tab.
//
// Run: `just share-layout-test`. Prints PASS or FAIL per check, with what
// it measured.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

const NODE = process.env.ARCHITECT_ISLAND_NODE;
function which(cmd) {
  try { return execFileSync('sh', ['-c', `command -v ${cmd}`]).toString().trim() || null; } catch { return null; }
}
const browser = process.env.FIREFOX || which('firefox');
const skip = NODE && browser ? false : 'needs ARCHITECT_ISLAND_NODE (just share-layout-test) and a Firefox (FIREFOX=…)';
// CI (.github/workflows/browser.yml) has a browser: missing one there is a
// failure, never a skip.
if (process.env.ARCHITECT_BROWSER_REQUIRED && skip) throw new Error(`ARCHITECT_BROWSER_REQUIRED: ${skip}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const put = (root, rel, body) => { const p = join(root, rel); mkdirSync(dirname(p), { recursive: true }); writeFileSync(p, body); };
const freePort = () => new Promise((ok) => { const srv = createServer(); srv.listen(0, '127.0.0.1', () => { const { port } = srv.address(); srv.close(() => ok(port)); }); });
function wav(seconds) {
  const rate = 8000, n = Math.round(seconds * rate), b = Buffer.alloc(44 + n * 2);
  b.write('RIFF', 0); b.writeUInt32LE(36 + n * 2, 4); b.write('WAVE', 8); b.write('fmt ', 12); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22);
  b.writeUInt32LE(rate, 24); b.writeUInt32LE(rate * 2, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34); b.write('data', 36); b.writeUInt32LE(n * 2, 40);
  return b;
}

const CODE = 'gigabcdefhjkmnpq', L = `/s/${CODE}`;
const TITLE = 'Merry & Bright - CBU Christmas 2026';
const SETLIST = `[[favourites]]
note_type = "setlist"
[[favourites.tabs]]
id = "setlist"
label = "Setlist"
icon = "list-music"
target = { view = "page" }
[[favourites.tabs]]
id = "charts"
label = "Charts"
icon = "file-music"
target = { view = "player.now" }
[[favourites.tabs]]
id = "lyrics"
label = "Lyrics"
icon = "mic-vocal"
target = { view = "player.lyrics" }
[favourites.primary]
id = "play"
label = "Play"
icon = "play"
target = { action = "PLAYER_TOGGLE" }
state = "playing"
on = { label = "Pause", icon = "pause" }
`;

/// A row as Studio draws it (its number `n`, a long title).
const MORE = '<svg viewBox="0 0 24 24" width="20" height="20" fill="currentColor" aria-hidden="true"><circle cx="5" cy="12" r="2"/><circle cx="12" cy="12" r="2"/><circle cx="19" cy="12" r="2"/></svg>';
const row = (n) => `<li data-track="" data-media="${L}/media/song.wav" data-page="${L}/list/" data-title="Song ${n}" data-subtitle="Artist"><span data-track-mark="">${n}</span><span data-track-art="" data-hue="200" aria-hidden="true" style="background:hsl(200 42% 40%)">${n}</span><span data-track-text=""><span data-track-title="">Rockin' Around the Christmas Tree, number ${n}</span><span data-track-meta="">Artist</span></span><button type="button" data-track-more="" aria-label="More: Song ${n}">${MORE}</button></li>`;

async function node({ shell }) {
  const dir = mkdtempSync(join(tmpdir(), 'layout-'));
  const vault = join(dir, 'vault');
  const rows = Array.from({ length: 101 }, (_, i) => row(i + 1));
  rows.splice(3, 0, '<li data-playlist-section="">Tree Lighting Concert Set</li>');
  // A song with no recording, and a key on a tile of the lightest hue.
  rows.splice(4, 0, `<li data-page="${L}/list/" data-title="Feliz Navidad"><span data-track-mark="">4</span><span data-track-art="" data-hue="60" aria-hidden="true" style="background:hsl(60 42% 32%)">♪</span><span data-track-text=""><span data-track-title="">Feliz Navidad</span><span data-track-meta="">No recording</span></span><button type="button" data-track-more="" aria-label="More: Feliz Navidad">${MORE}</button></li>`);
  rows[0] = rows[0].replace('style="background:hsl(200 42% 40%)">1</span>', 'data-key="" data-key-size="xs" role="img" aria-label="Starts in E flat, ends in F sharp minor" style="background:hsl(60 42% 32%)"><span data-track-key="" aria-hidden="true">Eb→F#m</span></span>');
  rows.unshift('<li data-playlist-section="">Presidents Songs</li>');
  put(vault, 'Gig/list.md', `---\ntitle: ${TITLE}\ntype: setlist\n---\n<div class="studio-setlist" data-playlist="" data-playlist-join=""><ol style="list-style:none;margin:0;padding:0">${rows.join('')}</ol><p data-playlist-files=""><a href="${L}/_files/">Files</a> · <a href="${L}/_archive.zip">Download all</a></p></div>\n`);
  put(vault, 'Gig/media/song.wav', wav(30));
  const share = { code: CODE, vault: 'notes', scope: { Folder: 'Gig' }, title: 'The gig', audience: 'Link', access: 'Read', expires_at: null, revoked_at: null, created_at: 1, created_by: 'test', domain: null };
  put(vault, `_architect/shares/${CODE}.md`, `---\npublish: false\n---\n# Share ${CODE}\n\n\`\`\`json\n${JSON.stringify(share)}\n\`\`\`\n`);
  const env = { ...process.env, ARCHITECT_DATA_DIR: join(dir, 'data'), ARCHITECT_VAULTS: `notes=${vault}`, ARCHITECT_IROH: 'local', ARCHITECT_NAME: 'layout-test' };
  delete env.ARCHITECT_ISLAND_FOLDER;
  if (shell) { put(dir, 'favourites.toml', SETLIST); env.ARCHITECT_FAVOURITES = join(dir, 'favourites.toml'); }
  const port = await freePort();
  env.ARCHITECT_BIND = `127.0.0.1:${port}`;
  const proc = spawn(NODE, [], { stdio: 'ignore', env });
  const origin = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 160; i++) {
    await sleep(250);
    const html = await fetch(`${origin}${L}/list/`).then((r) => (r.ok ? r.text() : '')).catch(() => '');
    if (html.includes('data-playlist')) return { origin, close: () => { proc.kill(); rmSync(dir, { recursive: true, force: true }); } };
  }
  proc.kill();
  throw new Error('the node never served the list');
}

async function firefox({ coarse = false } = {}) {
  const profile = mkdtempSync(join(tmpdir(), 'layout-ff-'));
  // A finger (pointer: coarse), as a phone, when asked.
  const finger = coarse ? 'user_pref("ui.primaryPointerCapabilities", 1);\nuser_pref("ui.allPointerCapabilities", 1);\n' : '';
  writeFileSync(join(profile, 'user.js'), 'user_pref("media.autoplay.default", 0);\nuser_pref("media.autoplay.blocking_policy", 0);\n' + finger);
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
  await send('browsingContext.navigate', { context, url: 'about:blank', wait: 'complete' }).catch(() => {});
  return {
    size: (width, height) => send('browsingContext.setViewport', { context, viewport: { width, height } }),
    go: (url) => send('browsingContext.navigate', { context, url, wait: 'complete' }),
    // Read-only measures (and a click), out of the page's CSP (BiDi).
    eval: async (expression) => {
      const r = await send('script.evaluate', { expression, target: { context }, awaitPromise: true, userActivation: true });
      if (r.type === 'exception') throw new Error(r.exceptionDetails.text);
      return r.result.value;
    },
    close: () => { ws.close(); proc.kill(); try { rmSync(profile, { recursive: true, force: true }); } catch {} },
  };
}

/// What the page looks like, measured: rects as [left, top, right, bottom].
const MEASURE = `JSON.stringify((() => {
  const r = (e) => { if (!e) return null; const b = e.getBoundingClientRect(); return [b.left, b.top, b.right, b.bottom].map((x) => Math.round(x * 10) / 10); };
  // Shown: laid out, not hidden, and on the screen (the navigator waits
  // off it, moved aside, until its tab opens it).
  const shown = (e) => { if (!e || !e.getClientRects().length) return false; const s = getComputedStyle(e); const b = e.getBoundingClientRect(); return s.visibility !== 'hidden' && s.display !== 'none' && Number(s.opacity) > 0 && b.right > 0 && b.left < innerWidth && b.width > 0; };
  const title = ${JSON.stringify(TITLE)};
  // Every element whose own text is the title, shown: one title at the top
  // (the player's card naming the setlist with nothing loaded is the
  // player, not a second title).
  const showing = [...document.querySelectorAll('body *')].filter((e) => shown(e) && !e.closest('.site-player') && [...e.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim() === title)).map((e) => e.tagName + '.' + e.className);
  const first = document.querySelector('[data-track]');
  const hundred = [...document.querySelectorAll('[data-track-mark]')].find((m) => m.textContent.trim() === '100');
  const more = first.querySelector('[data-track-more]');
  const glyph = more.querySelector('svg');
  const g = glyph.getBoundingClientRect();
  const rows = [...document.querySelectorAll('[data-playlist] li')];
  const card = document.querySelector('.site-player');
  // WCAG contrast of two computed colours (rgb()/rgba(), alpha on the page).
  // Any CSS colour (the theme's are oklch) as sRGB [r, g, b, a], by a
  // canvas: it says a colour back as #rrggbb, or rgba() when see-through.
  const ctx = document.createElement('canvas').getContext('2d');
  const rgb = (c) => {
    ctx.fillStyle = '#000'; ctx.fillStyle = c;
    const v = String(ctx.fillStyle);
    if (v.startsWith('#')) return [1, 3, 5].map((i) => parseInt(v.slice(i, i + 2), 16)).concat([1]);
    return (v.match(/[\d.]+/g) || []).map(Number);
  };
  const lum = ([r, g, b]) => { const ch = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b); };
  const over = (fg, bg) => { const [r, g, b, a = 1] = rgb(fg); const [R, G, B] = rgb(bg); return [r * a + R * (1 - a), g * a + G * (1 - a), b * a + B * (1 - a)]; };
  const ratio = (fg, bg) => { const l1 = lum(over(fg, bg)), l2 = lum(rgb(bg)); return Math.round(((Math.max(l1, l2) + 0.05) / (Math.min(l1, l2) + 0.05)) * 100) / 100; };
  const page = getComputedStyle(document.body).backgroundColor.startsWith('rgba(0, 0, 0, 0') ? getComputedStyle(document.documentElement).backgroundColor : getComputedStyle(document.body).backgroundColor;
  const quiet = document.querySelector('[data-playlist] li[data-page]:not([data-track])');
  const keyed = document.querySelector('[data-track-art][data-key]');
  return {
    vw: document.documentElement.clientWidth, vh: window.innerHeight,
    scrollWidth: document.documentElement.scrollWidth,
    header: (document.querySelector('.site-brand') || {}).innerText,
    showing,
    row: r(first),
    mark: r(first.querySelector('[data-track-mark]')),
    tile: r(first.querySelector('[data-track-art]')),
    titleBox: r(first.querySelector('[data-track-title]')),
    more: r(more),
    // The dots' right edge: the last circle (cx 19, r 2 of 24) in the svg.
    dots: Math.round((g.left + g.width * 21 / 24) * 10) / 10,
    hundredFits: hundred ? hundred.scrollWidth <= hundred.clientWidth + 0.5 : null,
    section: r(document.querySelector('[data-playlist-section]')),
    card: shown(card) ? r(card) : null,
    pill: r(document.querySelector('.site-tabs-pill')),
    tabs: r(document.querySelector('.site-tabs')),
    round: shown(document.querySelector('.site-tabs-round')),
    last: r(rows[rows.length - 1]),
    files: r(document.querySelector('[data-playlist-files]')),
    contrast: {
      quietTitle: ratio(getComputedStyle(quiet.querySelector('[data-track-title]')).color, page),
      quietMeta: ratio(getComputedStyle(quiet.querySelector('[data-track-meta]')).color, page),
      quietMark: ratio(getComputedStyle(quiet.querySelector('[data-track-mark]')).color, page),
      key: ratio(getComputedStyle(keyed.querySelector('[data-track-key]')).color, getComputedStyle(keyed).backgroundColor),
    },
  };
})())`;

function check(name, fn) {
  return async () => {
    try { await fn(); console.log(`PASS ${name}`); } catch (e) { console.log(`FAIL ${name}: ${e.message}`); throw e; }
  };
}
const near = (a, b, tol, what, m) => assert.ok(Math.abs(a - b) <= tol, `${what}: ${a} vs ${b} (±${tol}) in ${JSON.stringify(m)}`);

for (const [shell, w, h, theme] of [[true, 390, 844, 'light'], [true, 360, 740, 'light'], [false, 390, 844, 'light'], [false, 360, 740, 'light'], [true, 390, 844, 'dark']]) {
  {
    const name = `${w}×${h}, ${shell ? 'a setlist\'s tabs and the round button' : 'the share\'s own tabs'}, ${theme}`;
    test(name, { skip, timeout: 120000 }, check(name, async () => {
      const { origin, close } = await node({ shell });
      const tab = await firefox();
      try {
        await tab.size(w, h);
        await tab.go(`${origin}${L}/list/`);
        await tab.eval(`try { localStorage.setItem("site-theme", "${theme}"); } catch (e) {} true`);
        await tab.go(`${origin}${L}/list/`);
        await sleep(500);
        let m = JSON.parse(await tab.eval(MEASURE));
        // One title, at the top: the page's.
        assert.equal(String(m.header).trim(), TITLE, `the header shows the page's title: ${JSON.stringify(m)}`);
        assert.equal(m.showing.length, 1, `exactly one element shows it: ${JSON.stringify(m.showing)}`);
        // The list's gutters.
        assert.ok(m.scrollWidth <= m.vw, `nothing scrolls sideways: ${m.scrollWidth} > ${m.vw}`);
        // The number column from the left gutter, its digits right-aligned
        // 8 px before the tile.
        near(m.row[0], 16, 0.6, 'the row (its number column) on the left gutter', m);
        near(m.mark[2], m.tile[0] - 8, 0.6, 'the number right-aligned before the tile', m);
        assert.ok(m.tile[0] >= 44 && m.tile[0] <= 48, `the tile 44–48 px in: ${m.tile[0]}`);
        near(m.dots, m.vw - 16, 3, 'the "…" on the right gutter', m);
        assert.ok(m.more[2] <= m.vw - 16 + 0.6, `the button inside the gutter: ${m.more[2]}`);
        assert.equal(m.hundredFits, true, 'three digits fit their column');
        near(m.section[0], 16, 0.6, 'a set\'s label on the gutter', m);
        assert.equal(m.round, shell, 'the round button when the type has one');
        // Contrast: the quiet row's words and a key on its tile.
        console.log(`contrast ${theme}: ${JSON.stringify(m.contrast)}`);
        for (const [what, value] of Object.entries(m.contrast)) assert.ok(value >= 4.5, `${what} at ${value}:1 (4.5 wanted): ${JSON.stringify(m.contrast)}`);
        // The card: nothing loaded, then playing.
        assert.ok(m.card, `the card shows: ${JSON.stringify(m)}`);
        near(m.card[3] + 8, m.pill[1], 1, 'the card 8 px above the tab bar', m);
        near(m.card[0], m.pill[0], 0.6, 'the card on the tab bar\'s left gutter', m);
        near(m.card[2], m.tabs[2] - 16, 0.6, 'the card on the tab bar\'s right gutter', m);
        await tab.eval('document.querySelector("[data-track] [data-track-title]").click(), true');
        await sleep(600);
        m = JSON.parse(await tab.eval(MEASURE));
        near(m.card[3] + 8, m.pill[1], 1, 'playing: the card 8 px above the tab bar', m);
        // Scrolled to the end: the last row and the files line clear of it.
        await tab.eval('window.scrollTo(0, document.documentElement.scrollHeight), true');
        await sleep(300);
        m = JSON.parse(await tab.eval(MEASURE));
        assert.ok(m.last[3] <= m.card[1], `the last row clear of the card: ${m.last[3]} > ${m.card[1]}`);
        assert.ok(m.files[3] <= m.card[1], `the files line clear of the card: ${m.files[3]} > ${m.card[1]}`);
      } finally {
        tab.close();
        close();
      }
    }));
  }
}

const turned = '844×390 with a finger: a phone turned keeps the shell, and Charts is a tab';
test(turned, { skip, timeout: 120000 }, check(turned, async () => {
  const { origin, close } = await node({ shell: true });
  const tab = await firefox({ coarse: true });
  try {
    await tab.size(844, 390);
    await tab.go(`${origin}${L}/list/`);
    await sleep(500);
    const seen = JSON.parse(await tab.eval('JSON.stringify({ coarse: matchMedia("(pointer: coarse)").matches, tabs: getComputedStyle(document.querySelector(".site-tabs")).display })'));
    assert.equal(seen.coarse, true, 'the browser has a finger');
    assert.equal(seen.tabs, 'flex', `the tab bar shows: ${JSON.stringify(seen)}`);
    await tab.eval('document.querySelector("[data-track] [data-track-title]").click(), true');
    await sleep(400);
    await tab.eval('document.querySelector("[data-tab=charts]").click(), true');
    await sleep(600);
    const sheet = JSON.parse(await tab.eval('JSON.stringify((() => { const s = document.querySelector(".site-now"); return { shown: !!s && !s.hidden, tab: !!s && s.classList.contains("site-now-tab") }; })())'));
    assert.deepEqual(sheet, { shown: true, tab: true }, 'Charts a tab, not the desk\'s side panel');
  } finally {
    tab.close();
    close();
  }
}));
