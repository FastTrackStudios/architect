// A headless Firefox driven over WebDriver BiDi, and a static server for
// it to load (as apps/web/test/web.test.mjs drives one: Chromium headless
// hangs on this machine's http loads). FIREFOX=… names one, else
// `firefox` on the PATH. Every wait is bounded; `close` kills it.
import { spawn, execFileSync } from 'node:child_process';
import { createServer } from 'node:http';
import { mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { extname, join, normalize } from 'node:path';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export function firefox() {
  if (process.env.FIREFOX) return process.env.FIREFOX;
  try {
    return execFileSync('sh', ['-c', 'command -v firefox']).toString().trim() || null;
  } catch {
    return null;
  }
}

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json',
  '.wasm': 'application/wasm',
  '.svg': 'image/svg+xml',
};

/// Serves folder `root` on a free local port: `{ origin, close }`.
export async function serve(root) {
  const server = createServer((req, res) => {
    let path = decodeURIComponent(new URL(req.url, 'http://x').pathname);
    if (path.endsWith('/')) path += 'index.html';
    const file = normalize(join(root, path));
    if (!file.startsWith(normalize(root))) return res.writeHead(403).end();
    try {
      if (!statSync(file).isFile()) throw new Error('not a file');
      res.writeHead(200, { 'content-type': TYPES[extname(file)] ?? 'application/octet-stream', 'cache-control': 'no-store' });
      res.end(readFileSync(file));
    } catch {
      res.writeHead(404).end('not found');
    }
  });
  await new Promise((ok) => server.listen(0, '127.0.0.1', ok));
  return {
    origin: `http://127.0.0.1:${server.address().port}`,
    close: () => {
      server.closeAllConnections();
      server.close();
    },
  };
}

/// A headless Firefox and one tab of it: `{ tab, close }`.
export async function open() {
  const profile = mkdtempSync(join(tmpdir(), 'architect-npm-'));
  const port = 19000 + Math.floor(Math.random() * 900);
  // A dev shell's LD_LIBRARY_PATH (its libstdc++) stops Firefox loading.
  const env = { ...process.env };
  delete env.LD_LIBRARY_PATH;
  // Its own process group: Firefox is a wrapper script and its children.
  const proc = spawn(firefox(), ['--headless', '--no-remote', '--profile', profile, '--remote-debugging-port', String(port)], {
    stdio: 'ignore',
    env,
    detached: true,
  });
  proc.unref();
  const close = () => {
    try {
      ws?.close();
    } catch {}
    try {
      process.kill(-proc.pid, 'SIGKILL');
    } catch {}
    try {
      rmSync(profile, { recursive: true, force: true });
    } catch {}
  };
  let ws;
  for (let i = 0; i < 60 && !ws; i++) {
    await sleep(250);
    try {
      const s = new WebSocket(`ws://127.0.0.1:${port}/session`);
      await new Promise((ok, no) => {
        s.onopen = ok;
        s.onerror = no;
      });
      ws = s;
    } catch {}
  }
  if (!ws) {
    close();
    throw new Error('firefox did not start');
  }
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
  const send = (method, params = {}) =>
    new Promise((ok, no) => {
      const i = ++id;
      const timer = setTimeout(() => {
        waiting.delete(i);
        no(new Error(`${method}: no answer in 30 s`));
      }, 30000);
      waiting.set(i, { ok: (v) => (clearTimeout(timer), ok(v)), no: (e) => (clearTimeout(timer), no(e)) });
      ws.send(JSON.stringify({ id: i, method, params }));
    });
  await send('session.new', { capabilities: {} });
  const context = (await send('browsingContext.getTree', {})).contexts[0].context;
  const tab = {
    send,
    on: (f) => listeners.push(f),
    go: (url) => send('browsingContext.navigate', { context, url, wait: 'complete' }),
    eval: async (expression) => {
      const r = await send('script.evaluate', { expression, target: { context }, awaitPromise: true });
      if (r.type === 'exception') throw new Error(r.exceptionDetails.text);
      return r.result.value;
    },
    async until(expression, what, ms = 20000) {
      const started = Date.now();
      for (;;) {
        const v = await tab.eval(expression).catch(() => null);
        if (v) return v;
        if (Date.now() - started > ms) throw new Error(`waited ${ms} ms for ${what}`);
        await sleep(100);
      }
    },
  };
  return { tab, close };
}
