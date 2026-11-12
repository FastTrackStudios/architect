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
async function node(dir) {
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
  writeFileSync(join(folder, 'island.css'), ':host{position:fixed;inset:0;width:100vw;height:100vh;--island-mark:1}.architect-view{position:fixed;top:0;left:0;width:100vw;height:100vh;background:url(https://e.vil/beacon)}.wasi-count{color:rgb(1, 2, 3)}');
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


let shared;
async function setup() {
  if (!shared) {
    const dir = mkdtempSync(join(tmpdir(), 'architect-island-'));
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
    assert.equal(await tab.eval('getComputedStyle(document.querySelector(".site-now-controls")).display'), 'none');
    // Its view, in a shadow root, styled by its sheet, which moved nothing.
    const host = 'document.querySelector(".site-now-island-top .architect-view-host")';
    await tab.until(`${host}.shadowRoot.querySelector(".wasi-count")`, 'its view');
    assert.equal(await tab.eval(`getComputedStyle(${host}.shadowRoot.querySelector(".wasi-count")).color`), 'rgb(1, 2, 3)');
    const sheet = await tab.eval(`${host}.shadowRoot.querySelector("style").textContent`);
    assert.ok(!sheet.includes('e.vil'), sheet);
    assert.ok(sheet.includes(':host{--island-mark:1}'), sheet);
    const box = JSON.parse(await tab.eval(`JSON.stringify((() => { const r = ${host}.getBoundingClientRect(); const v = ${host}.shadowRoot.querySelector(".architect-view").getBoundingClientRect(); const s = document.querySelector(".site-now-island-top").getBoundingClientRect(); return { host: [r.top, r.height], view: [v.top, v.width], slot: [s.top, s.width] }; })())`));
    assert.ok(box.host[0] >= box.slot[0] - 1 && box.view[0] >= box.slot[0] - 1, JSON.stringify(box));
    assert.ok(box.view[1] <= box.slot[1] + 1, `no overlay of the page: ${JSON.stringify(box)}`);
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
