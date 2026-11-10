// A share link's player in a real browser (#511, `just share-player-test`):
// a headless node (`architect-desktop serve`, ARCHITECT_DESKTOP names the
// binary) on a vault of its own holding a link share of a two-song gig,
// each song a WAV with synced lyrics, in a headless Firefox driven over
// WebDriver BiDi (FIREFOX=… names one, else `firefox` on the PATH).
//
// - The highlighted lyric line advances while the first song plays.
// - A click on a line seeks there.
// - At the song's end the queue goes on to the second song's page, in
//   list order, and plays it there; its lines follow.
//
// Nothing leaves the machine: the node has no issuer and dials no relay.
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

/// A node serving a link share of `Gig/`, two songs in list order.
async function node(dir) {
  const vault = join(dir, 'vault');
  put(vault, 'Gig/01-joy.md', '---\ntitle: Joy\n---\n![[media/joy.wav]]\n\n![[media/joy.lrc]]\n');
  put(vault, 'Gig/02-noel.md', '---\ntitle: Noel\n---\n![[media/noel.wav]]\n\n```lrc\n[00:00.00]Noel\n[00:00.80]Noel\n[00:01.60]Born is the King\n```\n');
  put(vault, 'Gig/media/joy.lrc', '[00:00.00]Joy to the world\n[00:00.60]The Lord is come\n[00:01.20]Let earth\n[00:01.80]Receive her King\n[00:02.40]Let every heart\n');
  put(vault, 'Gig/media/joy.wav', wav(3.5));
  put(vault, 'Gig/media/noel.wav', wav(3));
  const share = {
    code: CODE, vault: 'notes', scope: { Folder: 'Gig' }, title: 'The gig', audience: 'Link', access: 'Read',
    expires_at: null, revoked_at: null, created_at: 1, created_by: 'test', domain: null,
  };
  put(vault, `_architect/shares/${CODE}.md`, `---\npublish: false\n---\n# Share ${CODE}\n\n\`\`\`json\n${JSON.stringify(share)}\n\`\`\`\n`);
  const port = 19900 + Math.floor(Math.random() * 90);
  const config = join(dir, 'node.toml');
  writeFileSync(config, `name = "player-test"\nbind = "127.0.0.1:${port}"\ndata_dir = "${join(dir, 'data')}"\niroh = "local"\nsweep_secs = 3600\n\n[vaults]\nnotes = "${vault}"\n`);
  const proc = spawn(desktop, ['serve', '--config', config], { stdio: 'ignore' });
  const origin = `http://127.0.0.1:${port}`;
  const page = `${origin}/s/${CODE}/01-joy/`;
  for (let i = 0; i < 120; i++) {
    await sleep(250);
    const html = await fetch(page).then((r) => (r.ok ? r.text() : '')).catch(() => '');
    if (html.includes('site-queue')) return { proc, origin };
  }
  proc.kill();
  throw new Error(`the node never served ${page}`);
}

/// A headless Firefox that may play sound unasked, and one tab of it.
async function open() {
  const profile = mkdtempSync(join(tmpdir(), 'architect-player-'));
  writeFileSync(join(profile, 'user.js'), 'user_pref("media.autoplay.default", 0);\nuser_pref("media.autoplay.blocking_policy", 0);\n');
  const port = 19000 + Math.floor(Math.random() * 900);
  const proc = spawn(firefox(), ['--headless', '--no-remote', '--profile', profile, '--remote-debugging-port', String(port)], { stdio: 'ignore' });
  let ws;
  for (let i = 0; i < 40 && !ws; i++) {
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
  const tab = {
    go: (url) => send('browsingContext.navigate', { context, url, wait: 'complete' }),
    eval: async (expression) => {
      const r = await send('script.evaluate', { expression, target: { context }, awaitPromise: true });
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

const current = 'document.querySelector(".site-lyrics [aria-current=time]")?.dataset.at';

test('the lyric line follows playback, a click seeks, and the queue goes on to the next song', { skip, timeout: 90000 }, async () => {
  const dir = mkdtempSync(join(tmpdir(), 'architect-gig-'));
  const { proc, origin } = await node(dir);
  const { tab, close } = await open();
  try {
    await tab.go(`${origin}/s/${CODE}/01-joy/`);
    assert.equal(await tab.eval('architect.playhead.snapshot().tracks'), 2);
    await tab.eval('architect.playhead.play()');
    // The first line, then a later one, as the song plays.
    await tab.until(`(${current}) === "0.000"`, 'the first line', 5000);
    const later = await tab.until(`Number(${current}) >= 1.2 && ${current}`, 'a later line', 6000);
    assert.ok(Number(later) >= 1.2, later);
    assert.ok(await tab.eval('architect.playhead.playing'));

    // A click on a line seeks there.
    await tab.eval('architect.playhead.pause()');
    await tab.eval('document.querySelector(\'.site-lyrics [data-at="0.600"] button\').click()');
    await tab.until('Math.abs(architect.playhead.time - 0.6) < 0.4', 'the seek to 0.6 s', 4000);
    await tab.until(`(${current}) === "0.600"`, 'the clicked line marked', 4000);

    // Played to its end, the queue goes to the next song's page and plays.
    await tab.eval('architect.playhead.play()');
    await tab.until('location.pathname.endsWith("/02-noel/")', 'the second song\'s page', 15000);
    await tab.until('architect.playhead.here && architect.playhead.playing', 'the second song playing', 8000);
    await tab.until(`Number(${current}) >= 0.8`, 'its lines following', 6000);
  } finally {
    close();
    proc.kill();
    rmSync(dir, { recursive: true, force: true });
  }
});
