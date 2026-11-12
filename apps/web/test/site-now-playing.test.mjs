// A now-playing view on a Site with no islands and no share link
// (studio#445, architect#601's review): the chart and the lyrics a playlist
// row's page holds still come into the view, cleaned by the site script's
// own allow-lists (`architect.rules`, `features/extensions/web/sanitize.js`
// carried in `_site/site.<hash>.js`), on
// - a Site the node serves (`ARCHITECT_SITES`), and
// - a static Site (`architect site serve`, the same site router).
// `just share-player-test` runs it (ARCHITECT_DESKTOP, ARCHITECT_CLI).
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
const cli = process.env.ARCHITECT_CLI;
const skip = desktop && cli && firefox() ? false : 'needs ARCHITECT_DESKTOP and ARCHITECT_CLI (just share-player-test) and a Firefox (FIREFOX=…)';
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


/// A vault of a playlist and a song: its recording, its LRC, its chart
/// drawn by the demo extension as its now-playing view. Rows name their
/// recording and page relatively, so the same vault works at any mount.
function vault(dir) {
  const v = join(dir, 'vault');
  put(v, 'Gig/01-joy.md', '---\ntitle: Joy\n---\n![[media/joy.wav]]\n\n![[media/joy.lrc]]\n\n```chart\n| C | G | Am | F |\n```\n');
  put(v, 'Gig/03-list.md', '---\ntitle: List\n---\n<div data-playlist><ul><li data-track data-media="../../Gig/media/joy.wav" data-page="../01-joy/" data-title="Joy">Joy</li></ul></div>\n');
  put(v, 'Gig/media/joy.lrc', '[00:00.00]Joy to the world\n[00:00.60]The Lord is come\n');
  put(v, 'Gig/media/joy.wav', wav(3.5));
  const ext = join(dir, 'ext', 'demo.chart');
  put(ext, 'extension.toml', 'id = "demo.chart"\nname = "Chart"\nversion = "0.1.0"\nkind = "data"\n\n[[embeds]]\nid = "chart"\npage = "pages/chart.tsx"\nclaims = { fence = ["chart"] }\n');
  put(ext, 'pages/chart.tsx', `import { h, useEmbed } from "architect";
export default function Chart() {
  const bars = (useEmbed()?.source ?? "").split("|").map((b) => b.trim()).filter((b) => b !== "");
  return (
    <div data-now-playing="">
      <svg class="demo-chart" viewBox={\`0 0 \${bars.length * 120} 60\`} width="100%" data-follow-playhead="">
        <style>{".demo-bar{fill:#3b82f6;fill-opacity:0}.demo-bar[aria-current=time]{fill-opacity:0.25}"}</style>
        {bars.map((b, i) => (<rect key={\`\${i}\`} class="demo-bar" data-at={\`\${i * 0.5}\`} x={\`\${i * 120}\`} y="0" width="120" height="60" />))}
      </svg>
    </div>
  );
}
`);
  return { vault: v, ext };
}

async function until(url, want) {
  for (let i = 0; i < 160; i++) {
    await sleep(250);
    const html = await fetch(url).then((r) => (r.ok ? r.text() : '')).catch(() => '');
    if (html.includes(want)) return;
  }
  throw new Error(`${url} never had ${want}`);
}

/// The now-playing view on `list`: Joy plays, its chart and lyrics come in.
async function nowPlaying(list) {
  const { tab, close } = await open();
  try {
    await tab.go(list);
    assert.equal(await tab.eval('!!document.querySelector("#site-islands")'), false, 'no islands here');
    assert.equal(await tab.eval('!!(window.architect && architect.rules && architect.rules.cleanSheet)'), true, "the site's own rules");
    await tab.eval('document.querySelector("[data-track]").click(), true');
    await tab.until('architect.playhead.playing', 'Joy playing');
    await tab.eval('document.querySelector(".site-player-title").click(), true');
    await tab.until('document.querySelector(".site-now-body .demo-chart")', 'the chart in the view', 20000);
    await tab.until('document.querySelector(".site-now .demo-bar[aria-current=time]")', 'it follows');
    assert.equal(await tab.eval('getComputedStyle(document.querySelector(".site-now .demo-bar:not([aria-current])")).fillOpacity'), '0', 'its sheet came');
    assert.equal(await tab.eval('document.querySelector(".site-now-lyrics").hidden'), false, 'its lyrics a tap away');
    await tab.eval('document.querySelector(".site-now-lyrics").click(), true');
    await tab.until('document.querySelector(".site-now-words [aria-current=time]")', 'its lyrics follow');
  } finally { close(); }
}

test('a Site the node serves, without islands, brings its chart and lyrics into the view', { skip, timeout: 120000 }, async () => {
  const dir = mkdtempSync(join(tmpdir(), 'architect-site-node-'));
  const { vault: v, ext } = vault(dir);
  const data = join(dir, 'data');
  mkdirSync(join(data, 'extensions'), { recursive: true });
  execFileSync('cp', ['-r', ext, join(data, 'extensions', 'demo.chart')]);
  const port = 19700 + Math.floor(Math.random() * 90);
  const config = join(dir, 'node.toml');
  writeFileSync(config, `name = "site-test"\nbind = "127.0.0.1:${port}"\ndata_dir = "${data}"\niroh = "local"\nsweep_secs = 3600\n\n[vaults]\nnotes = "${v}"\n\n[sites]\n"/site" = "notes"\n`);
  const proc = spawn(desktop, ['serve', '--config', config], { stdio: 'ignore' });
  try {
    const list = `http://127.0.0.1:${port}/site/gig/03-list/`;
    await until(list, 'data-playlist');
    await until(`http://127.0.0.1:${port}/site/gig/01-joy/`, 'demo-chart');
    await nowPlaying(list);
  } finally {
    proc.kill();
    rmSync(dir, { recursive: true, force: true });
  }
});

test('a static Site (architect site serve) brings its chart and lyrics into the view', { skip, timeout: 120000 }, async () => {
  const dir = mkdtempSync(join(tmpdir(), 'architect-site-static-'));
  const { vault: v, ext } = vault(dir);
  const port = 19600 + Math.floor(Math.random() * 90);
  const proc = spawn(cli, ['site', 'serve', '--vault', v, '--port', String(port), '--extension', ext], { stdio: 'ignore' });
  try {
    const list = `http://127.0.0.1:${port}/gig/03-list/`;
    await until(list, 'data-playlist');
    await nowPlaying(list);
  } finally {
    proc.kill();
    rmSync(dir, { recursive: true, force: true });
  }
});
