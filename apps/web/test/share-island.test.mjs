// An island in a real browser (studio#445): a node with a linked-in WASI
// component (`apps/desktop/examples/island_node.rs`, the sandbox's counter
// built with `architect-ext build-web`), a share whose setlist plays, in a
// headless Firefox at a phone's width over WebDriver BiDi.
//
// - The sandbox frame is the node's own document with its own policy
//   (`sandbox allow-scripts`, no network, `frame-ancestors 'self'`,
//   nosniff, immutable), and only that policy; the share page's is
//   unchanged.
// - Opened at the top level it is still an opaque origin: no cookies.
// - The island's view takes the now-playing view's top and bottom slots;
//   the player's bar and controls give way only then.
// - Its stylesheet keeps no overlay, no beacon, no resize of its box.
// - It reaches nothing but the playhead intents: its own ask is refused.
//
// `just share-island-test` builds and runs it. Run with
// `env -u LD_LIBRARY_PATH` from a dev shell (its libstdc++ stops Firefox).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { cpSync, mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

const islandNode = process.env.ARCHITECT_ISLAND_NODE;
const islandFolder = process.env.ARCHITECT_ISLAND_FOLDER;
function firefox() {
  if (process.env.FIREFOX) return process.env.FIREFOX;
  try { return execFileSync('sh', ['-c', 'command -v firefox']).toString().trim() || null; } catch { return null; }
}
const skip = islandNode && islandFolder && firefox() ? false : 'needs ARCHITECT_ISLAND_NODE and ARCHITECT_ISLAND_FOLDER (just share-island-test) and a Firefox (FIREFOX=…)';
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


/// The island node (`apps/desktop/examples/island_node.rs`) serving a
/// link share of `Gig/`: a setlist page whose rows play two songs, Joy's
/// page drawing its chart as its now-playing view; the WASI counter as
/// the island, with a stylesheet that tries everything a sheet may not.
async function node(dir, sheet) {
  const vault = join(dir, 'vault');
  put(vault, 'Gig/01-joy.md', '---\ntitle: Joy\n---\n![[media/joy.wav]]\n\n```chart\n| C | G | Am | F | C | G | C |\n```\n');
  const ext = join(dir, 'data', 'extensions', 'demo.chart');
  put(ext, 'extension.toml', 'id = "demo.chart"\nname = "Chart"\nversion = "0.1.0"\nkind = "data"\n\n[[embeds]]\nid = "chart"\npage = "pages/chart.tsx"\nclaims = { fence = ["chart"] }\n');
  put(ext, 'pages/chart.tsx', `import { h, useEmbed } from "architect";
export default function Chart() {
  const bars = (useEmbed()?.source ?? "").split("|").map((b) => b.trim()).filter((b) => b !== "");
  return (
    <div data-now-playing="" data-sections='[{"name":"Verse","start":0,"end":2,"color":"#22c55e"},{"name":"Chorus","start":2,"end":3.5,"color":"#3b82f6"}]'>
      <svg class="demo-chart" viewBox={\`0 0 \${bars.length * 120} 60\`} width="100%" data-follow-playhead="">
        {bars.map((b, i) => (<rect key={\`\${i}\`} class="demo-bar" data-at={\`\${i * 0.5}\`} x={\`\${i * 120}\`} y="0" width="120" height="60" fill="transparent" />))}
      </svg>
    </div>
  );
}
`);
  put(vault, 'Gig/02-noel.md', '---\ntitle: Noel\n---\n![[media/noel.wav]]\n');
  put(vault, 'Gig/03-list.md', '---\ntitle: List\n---\n<div data-playlist data-title="Set"><ul>' +
    `<li data-track data-media="${L}/Gig/media/joy.wav" data-page="${L}/01-joy/" data-title="Joy">Joy</li>` +
    `<li data-track data-media="${L}/Gig/media/noel.wav" data-page="${L}/02-noel/" data-title="Noel">Noel</li>` +
    '</ul></div>\n');
  put(vault, 'Gig/media/joy.wav', wav(3.5));
  put(vault, 'Gig/media/noel.wav', wav(3));
  const share = {
    code: CODE, vault: 'notes', scope: { Folder: 'Gig' }, title: 'The gig', audience: 'Link', access: 'Read',
    expires_at: null, revoked_at: null, created_at: 1, created_by: 'test', domain: null,
  };
  put(vault, `_architect/shares/${CODE}.md`, `---\npublish: false\n---\n# Share ${CODE}\n\n\`\`\`json\n${JSON.stringify(share)}\n\`\`\`\n`);
  // The island: the counter's built web folder, and a sheet that tries an
  // overlay, a beacon and a resize of its box.
  const folder = join(dir, 'island');
  cpSync(islandFolder, folder, { recursive: true });
  writeFileSync(join(folder, 'island.css'), sheet);
  const port = 19800 + Math.floor(Math.random() * 90);
  const env = {
    ...process.env,
    ARCHITECT_BIND: `127.0.0.1:${port}`,
    ARCHITECT_DATA_DIR: join(dir, 'data'),
    ARCHITECT_VAULTS: `notes=${vault}`,
    ARCHITECT_IROH: 'local',
    ARCHITECT_NAME: 'island-test',
    ARCHITECT_ISLAND_FOLDER: folder,
    ARCHITECT_ISLAND_VIEW: 'counter',
    ARCHITECT_ISLAND_SHEET: 'island.css',
  };
  const proc = spawn(islandNode, [], { stdio: 'ignore', env });
  const origin = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 160; i++) {
    await sleep(250);
    const html = await fetch(`${origin}${L}/03-list/`).then((r) => (r.ok ? r.text() : '')).catch(() => '');
    if (html.includes('site-islands')) return { proc, origin };
  }
  proc.kill();
  throw new Error('the island node never served the list');
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
  const listeners = [];
  ws.onmessage = (e) => {
    const m = JSON.parse(e.data);
    if (m.id && waiting.has(m.id)) {
      const w = waiting.get(m.id);
      waiting.delete(m.id);
      m.type === 'error' ? w.no(new Error(`${m.error}: ${m.message}`)) : w.ok(m.result);
    } else if (m.method) listeners.forEach((f) => f(m));
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
    send,
    /// Hears the browser's events (`session.subscribe` first).
    on: (f) => listeners.push(f),
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


/// A sheet the sanitiser keeps that still tries to leave its slot: its
/// own theme, an element thrown far off and blown up, everything moved.
const AGGRESSIVE = ':root,:host{--island-mark:1}.wasi-count{color:rgb(1, 2, 3);position:absolute;top:-3000px;left:-3000px;width:9000px;height:9000px;background:rgb(255, 0, 0);transform:scale(20)}*{margin:-400px}';
/// The review's payloads (architect#601): refused whole.
const PAYLOAD = String.raw`:\68 ost{position:fixed!important;inset:0!important;width:100vw!important;height:100vh!important;max-height:none!important;z-index:2147483647!important;background:rgb(255,0,0)!important}.wasi-count::before{content:\75 rl(https://e.vil/x) "{";}}`;

const nodes = {};
async function setup(sheet = AGGRESSIVE) {
  if (!nodes[sheet]) {
    const dir = mkdtempSync(join(tmpdir(), 'architect-island-'));
    nodes[sheet] = { dir, ...(await node(dir, sheet)) };
  }
  return nodes[sheet];
}
test.after(() => {
  for (const n of Object.values(nodes)) {
    n.proc.kill();
    rmSync(n.dir, { recursive: true, force: true });
  }
});

/// The page's points (a 20 px grid over the viewport) where the island
/// shows: none may be outside its slots.
const ESCAPES = `(() => {
  const slots = [...document.querySelectorAll(".site-now-island")].filter((s) => !s.hidden).map((s) => s.getBoundingClientRect());
  const hosts = [...document.querySelectorAll(".site-now-island .architect-view-host")];
  const out = [];
  for (let y = 0; y < innerHeight; y += 20) for (let x = 0; x < innerWidth; x += 20) {
    const at = document.elementFromPoint(x, y);
    if (!at || !hosts.some((h) => h === at || h.contains(at))) continue;
    if (!slots.some((r) => x >= r.left && x <= r.right && y >= r.top && y <= r.bottom)) out.push([x, y]);
  }
  return JSON.stringify(out);
})()`;

/// What the node put on the setlist page for its player.
async function islands(origin) {
  const html = await fetch(`${origin}${L}/03-list/`).then((r) => r.text());
  return JSON.parse(/<script type="application\/json" id="site-islands">([^<]*)<\/script>/.exec(html)[1]);
}

test('the sandbox is the node’s own document, with its own policy and only that', { skip, timeout: 90000 }, async () => {
  const { origin } = await setup();
  const ext = await islands(origin);
  assert.match(ext.sandbox, /^\/_ext\/sandbox\.[0-9a-f]+\.html$/);
  const r = await fetch(`${origin}${ext.sandbox}`);
  assert.equal(r.status, 200);
  const csp = r.headers.get('content-security-policy');
  assert.ok(!csp.includes(','), `one policy, not the share's beside it: ${csp}`);
  assert.match(csp, /^sandbox allow-scripts;/);
  for (const never of ['allow-same-origin', 'allow-top-navigation', 'allow-popups', 'allow-forms', 'allow-modals', "'unsafe-inline'", "'self';"]) {
    assert.ok(!csp.includes(never), `${never} in ${csp}`);
  }
  assert.match(csp, /connect-src 'none'/);
  assert.match(csp, /frame-ancestors 'self'$/);
  assert.equal(r.headers.get('x-content-type-options'), 'nosniff');
  assert.match(r.headers.get('cache-control'), /immutable/);
  // The share page's own policy is as it was: no wasm, no blob:, no
  // sandbox hash.
  const page = await fetch(`${origin}${L}/03-list/`);
  const policy = page.headers.get('content-security-policy');
  assert.ok(!/wasm|blob:|unsafe-eval/.test(policy), policy);
  // The runtime and the island's files: nosniff and immutable, from /_ext/ only.
  const index = await fetch(`${origin}${ext.runtime}index.js`);
  assert.equal(index.headers.get('x-content-type-options'), 'nosniff');
  assert.match(index.headers.get('content-type'), /javascript/);
  assert.equal((await fetch(`${origin}/_ext/sandbox.html`)).status, 404, 'only by its content’s name');
  for (const method of ['POST', 'PUT', 'DELETE']) {
    const w = await fetch(`${origin}${ext.sandbox}`, { method });
    assert.equal(w.status, 405, method);
  }
  assert.equal((await fetch(`${origin}${ext.sandbox}`, { method: 'HEAD' })).status, 200);
  // The stylesheet is pinned by its digest beside the lock's.
  assert.match(ext.islands[0].stylesheetIntegrity, /^sha256-/);
  assert.equal((await fetch(`${origin}${L}/_ext/x`)).status, 404);
});

test('opened at the top level, the sandbox is still an opaque origin', { skip, timeout: 90000 }, async () => {
  const { origin } = await setup();
  const ext = await islands(origin);
  const { tab, close } = await open();
  try {
    await tab.go(`${origin}${ext.sandbox}`);
    assert.equal(await tab.eval('origin'), 'null');
    assert.equal(await tab.eval('(() => { try { return String(document.cookie); } catch (e) { return "refused"; } })()'), 'refused');
    assert.equal(await tab.eval('(() => { try { localStorage.length; return "storage"; } catch (e) { return "refused"; } })()'), 'refused');
  } finally { close(); }
});

test('the island takes the view’s top and bottom; its sheet moves nothing; it reaches nothing', { skip, timeout: 120000 }, async () => {
  const { origin } = await setup();
  const { tab, close } = await open();
  try {
    await tab.go(`${origin}${L}/03-list/`);
    await tab.eval('[...document.querySelectorAll("[data-track]")][0].click(), true');
    await tab.until('architect.playhead.playing', 'Joy playing');
    // Before the view opens, nothing of the island is loaded.
    assert.equal(await tab.eval('!!document.querySelector("iframe")'), false);
    await tab.eval('document.querySelector(".site-player-title").click(), true');
    await tab.until('document.querySelector(".site-now-body .demo-chart")', 'the chart');
    await tab.until('!document.querySelector(".site-now-island-top").hidden', 'the island drawn', 30000);
    // The sandbox frame: the node's document, sandboxed, no srcdoc.
    const frame = JSON.parse(await tab.eval('JSON.stringify((() => { const f = document.querySelector("iframe"); return { src: new URL(f.src).pathname, sandbox: f.getAttribute("sandbox"), srcdoc: f.hasAttribute("srcdoc") }; })())'));
    assert.match(frame.src, /^\/_ext\/sandbox\.[0-9a-f]+\.html$/);
    assert.deepEqual([frame.sandbox, frame.srcdoc], ['allow-scripts', false]);
    // Top, chart, bottom; the player's bar and controls gave way.
    const tops = JSON.parse(await tab.eval('JSON.stringify([".site-now-island-top", ".site-now-body", ".site-now-island-bottom"].map((s) => document.querySelector(s).getBoundingClientRect().top))'));
    assert.ok(tops[0] < tops[1] && tops[1] < tops[2], JSON.stringify(tops));
    assert.equal(await tab.eval('getComputedStyle(document.querySelector(".site-now-progress")).display'), 'none');
    // The controls' row stays for Click and Loop; its previous, play and
    // next give way to the island's transport, which sits between them.
    assert.equal(await tab.eval('getComputedStyle(document.querySelector(".site-now-controls .site-now-transport")).display'), 'none');
    assert.equal(await tab.eval('!!document.querySelector(".site-now-buttons > .site-now-island-bottom")'), true);
    // Its view, in a shadow root, styled by its sheet, which moved nothing.
    const host = 'document.querySelector(".site-now-island-top .architect-view-host")';
    await tab.until(`${host}.shadowRoot.querySelector(".wasi-count")`, 'its view');
    assert.equal(await tab.eval(`getComputedStyle(${host}.shadowRoot.querySelector(".wasi-count")).color`), 'rgb(1, 2, 3)');
    // Its sheet, after the motion guard the view puts first.
    const sheet = await tab.eval(`${host}.shadowRoot.querySelector("style:last-of-type").textContent`);
    assert.ok(sheet.includes('.architect-view{--island-mark:1;}'), sheet);
    // However it throws its content about, nothing of it shows outside its slots.
    assert.deepEqual(JSON.parse(await tab.eval(ESCAPES)), []);
    // It may ask nothing but the playhead intents: its own ask is refused.
    await tab.eval(`${host}.shadowRoot.querySelector(".wasi-ask").click(), true`);
    await tab.until(`/refused|not/i.test(${host}.shadowRoot.querySelector(".wasi-asked")?.textContent ?? "")`, 'its ask refused', 15000);
    // Closed: its views end, the player's come back.
    await tab.eval('document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })), true');
    await tab.until('document.querySelector(".site-now").hidden', 'closed');
    assert.equal(await tab.eval('document.querySelector(".site-now-island-top").hidden'), true);
    assert.equal(await tab.eval('document.querySelector(".site-now-progress").classList.contains("site-now-yielded")'), false);
  } finally { close(); }
});

test('the review’s payload sheet is refused whole: the island stays in its slots, unstyled', { skip, timeout: 120000 }, async () => {
  const { origin } = await setup(PAYLOAD);
  const { tab, close } = await open();
  try {
    await tab.go(`${origin}${L}/03-list/`);
    await tab.eval('[...document.querySelectorAll("[data-track]")][0].click(), true');
    await tab.until('architect.playhead.playing', 'Joy playing');
    await tab.eval('document.querySelector(".site-player-title").click(), true');
    await tab.until('!document.querySelector(".site-now-island-top").hidden', 'the island drawn', 30000);
    const host = 'document.querySelector(".site-now-island-top .architect-view-host")';
    await tab.until(`${host}.shadowRoot.querySelector(".wasi-count")`, 'its view');
    assert.equal(await tab.eval(`!!${host}.shadowRoot.querySelector("style")`), false, 'no sheet');
    assert.deepEqual(JSON.parse(await tab.eval(ESCAPES)), []);
    for (const [x, y] of [[10, 10], [200, 800]]) {
      const at = await tab.eval(`(() => { const e = document.elementFromPoint(${x}, ${y}); return !!e.closest(".site-now-island") && !e.closest(".site-now-island-top, .site-now-island-bottom")?.getBoundingClientRect; })()`);
      assert.equal(at, false, `${x},${y}`);
    }
  } finally { close(); }
});

test('whatever cleanSheet keeps, the browser reads no rule or value outside the allow-list, and requests nothing but the view’s own files', { skip, timeout: 120000 }, async () => {
  const { origin } = await setup();
  const ext = await islands(origin);
  const { tab, close } = await open();
  try {
    await tab.go(`${origin}${L}/03-list/`);
    const net = [];
    await tab.send('session.subscribe', { events: ['network.beforeRequestSent'] });
    tab.on((m) => { if (m.method === 'network.beforeRequestSent') net.push(m.params.request.url); });
    const report = await tab.eval(`(async () => {
      const { cleanSheet } = await import(${JSON.stringify(ext.runtime + 'sanitize.js')});
      const prefix = '/_ext/island/x.1/';
      // A seeded generator of sheets: hostile pieces, escaped every way, among good ones.
      let seed = 445;
      const rand = (n) => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) % n);
      const esc = (w) => [...w].map((c) => {
        const k = rand(6);
        const hex = c.codePointAt(0).toString(16);
        return k === 0 ? '\\\\' + hex + ' ' : k === 1 ? '\\\\' + hex.toUpperCase() + ' ' : k === 2 ? '\\\\' + hex.padStart(6, '0') : k === 3 && /[a-z]/.test(c) ? c.toUpperCase() : c;
      }).join('');
      // Mostly allowed but tricky (escaped good names, braces and url( in
      // strings, nesting), sometimes hostile, every way spelled.
      const pick = (good, bad) => (rand(5) === 0 ? bad[rand(bad.length)] : good[rand(good.length)]);
      const sel = () => pick(
        ['.' + esc('a'), 'div > .' + esc('b'), '.c:' + esc('hover'), '.d::' + esc('before'), ':is(.a, .' + esc('b') + ')', '.e[data-x="{"]', '.' + esc('html'), '.f:not(.' + esc('host') + ')'],
        [':' + esc('host'), ':' + esc('host') + '(.a)', '::' + esc('part') + '(x)', ':' + esc('root'), esc('html'), esc('body'), '.' + esc('architect-view'), ':is(.a, :' + esc('host') + ')'],
      );
      const val = () => pick(
        [esc('red'), 'calc(1px + 2px)', 'var(--x)', 'rgb(1 2 3)', '"{" "}" "url(x)"', 'url(' + prefix + 'a.png)', esc('absolute'), 'linear-gradient(' + esc('red') + ', blue)'],
        [esc('url') + '(https://e.vil/x)', esc('url') + '("https://e.vil/x")', esc('attr') + '(data-x)', esc('image-set') + '("x" 1x)', esc('fixed'), esc('element') + '(#x)',
          // What the URL parser reads as a parent folder or elsewhere (the second review).
          'url(' + prefix + '%2e%2e/%2e%2e/s/secret)', 'url("' + prefix + '.%2E/.%2e/s/secret")', 'url(' + prefix + '%2E./x)', 'url(' + prefix + '%252e%252e/x)',
          'url("' + prefix + '..\\\\x")', 'url(' + prefix + 'a.png?x)', 'url("' + prefix + 'a.png#x")', 'url(//e.vil' + prefix + 'a.png)', 'url("data:,x")', 'url(' + prefix + '../x)',
          'url("' + prefix + 'a\\9 b.png")', 'url/**/("https://e.vil/x")'],
      );
      const prop = () => pick([esc('color'), 'content', 'background', '--x', esc('position'), 'width'], [esc('position'), esc('z-index'), esc('behavior')]);
      const decl = () => prop() + ':' + val() + (rand(12) === 0 ? '!' + esc('important') : '');
      const rule = () => {
        const k = rand(10);
        if (k === 0 && rand(3) === 0) return '@' + esc('import') + ' url(https://e.vil/x.css);';
        if (k === 1) return '@media (min-width:1px){' + sel() + '{' + decl() + '}}';
        if (k === 2) return sel() + '{' + decl() + ';' + sel().replace(/^[^&]/, '&$&') + '{' + decl() + '}}';
        if (k === 3) return '@' + esc('layer') + ' t{' + sel() + '{' + decl() + '}}';
        return sel() + '{' + decl() + ';' + decl() + '}';
      };
      const BAD_SELECTOR = /:host|::part|::slotted|:root|:scope|(^|[\\s,>+~(])(html|body)(?![\\w-])|\\.architect-view(?![\\w-]|-host)/i;
      let kept = 0, refused = 0;
      const wrong = [];
      // Each kept sheet styles a real view, so the browser requests what it
      // names (judged from the network log outside).
      const stage = document.createElement('div');
      stage.style.cssText = 'position:absolute;left:0;top:0;width:300px;height:300px;overflow:hidden';
      document.body.append(stage);
      const shadow = stage.attachShadow({ mode: 'open' });
      shadow.innerHTML = '<div class="architect-view"><div class="a b c d e f"><div class="b"></div><div class="e" data-x="{"></div></div></div>';
      const own = new RegExp('^' + prefix.replace(/[.]/g, '\\\\.') + '[A-Za-z0-9._-]{1,128}$');
      for (let n = 0; n < 2000; n++) {
        const css = Array.from({ length: 1 + rand(2) }, rule).join('');
        const out = cleanSheet(css, prefix);
        if (out === null) { refused++; continue; }
        kept++;
        const sheet = new CSSStyleSheet();
        sheet.replaceSync(out);
        shadow.adoptedStyleSheets = [sheet];
        for (const e of shadow.querySelectorAll('*')) getComputedStyle(e).backgroundImage;
        const walk = (list) => {
          for (const r of list) {
            const type = r.constructor.name;
            if (!['CSSStyleRule', 'CSSMediaRule', 'CSSSupportsRule', 'CSSContainerRule', 'CSSKeyframesRule', 'CSSKeyframeRule', 'CSSPropertyRule', 'CSSNestedDeclarations'].includes(type)) wrong.push([type, css]);
            if (r.selectorText !== undefined) {
              const s = r.selectorText;
              const themeBox = s === '.architect-view' && [...r.style].every((p) => p.startsWith('--'));
              if (!themeBox && BAD_SELECTOR.test(s)) wrong.push(['selector ' + s, css]);
            }
            if (r.style) for (const p of r.style) {
              const v = r.style.getPropertyValue(p);
              // Every url() as the browser resolves it: the view's own file.
              for (const m of v.matchAll(/url\\(\\s*"((?:[^"\\\\]|\\\\.)*)"\\s*\\)|url\\(\\s*([^"')\\s]*)\\s*\\)/gi)) {
                const u = new URL((m[1] ?? m[2]).replace(/\\\\(.)/g, '$1'), document.baseURI);
                if (u.origin !== location.origin || !own.test(u.pathname) || u.search || u.hash) wrong.push(['url ' + p + ': ' + v, css]);
              }
              if (/url\\(/i.test(v) && !/url\\(\\s*["']?\\//i.test(v)) wrong.push(['url not a path ' + p + ': ' + v, css]);
              if (/attr\\(|image-set\\(|element\\(|image\\(/i.test(v)) wrong.push(['value ' + p + ': ' + v, css]);
              if (p === 'position' && /fixed|sticky/i.test(v)) wrong.push(['position ' + v, css]);
            }
            if (r.cssRules) walk(r.cssRules);
          }
        };
        walk(sheet.cssRules);
      }
      await new Promise((r) => setTimeout(r, 500));
      stage.remove();
      return JSON.stringify({ kept, refused, wrong: wrong.slice(0, 5) });
    })()`);
    const r = JSON.parse(report);
    assert.deepEqual(r.wrong, [], JSON.stringify(r));
    assert.ok(r.kept > 100 && r.refused > 100, JSON.stringify(r));
    // Nothing the kept sheets named was requested but the view's own files.
    const own = /^\/_ext\/island\/x\.1\/[A-Za-z0-9._-]{1,128}$/;
    const strays = net.filter((u) => u.startsWith(origin) ? /^\/_ext\/island\//.test(new URL(u).pathname) && !own.test(new URL(u).pathname) : !u.startsWith('data:'));
    assert.deepEqual(strays, [], 'requests outside the view’s own files');
    assert.ok(net.some((u) => own.test(new URL(u).pathname)), 'a kept url() was requested at all');
  } finally { close(); }
});

