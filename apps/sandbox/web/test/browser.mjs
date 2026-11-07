// What the web app's browser tests share: the built lab (`just sandbox-web 0`,
// target/sandbox-web) served on 127.0.0.1, and a headless Chrome of its own
// (its own profile, gone after) driven over the DevTools protocol with
// node's own WebSocket — no packages.
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, extname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
export const root = resolve(here, '../../../..');
export const built = process.env.ARCHITECT_LAB_WEB ?? join(root, 'target/sandbox-web');

/// Chrome: `CHROME`, else the Mac's or a Linux one on the PATH; null when
/// there is none.
export function chrome() {
  const found = [
    process.env.CHROME,
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    '/usr/bin/google-chrome',
    '/usr/bin/chromium',
    '/run/current-system/sw/bin/chromium',
  ].find((p) => p && existsSync(p));
  return found ?? null;
}

const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.wasm': 'application/wasm', '.txt': 'text/plain', '.json': 'application/json' };

/// The built lab, served: its port.
export function serve() {
  const server = createServer((req, res) => {
    const path = join(built, decodeURIComponent(new URL(req.url, 'http://x').pathname));
    const file = existsSync(path) && statSync(path).isDirectory() ? join(path, 'index.html') : path;
    if (!file.startsWith(built) || !existsSync(file)) {
      res.writeHead(404).end();
      return;
    }
    res.writeHead(200, { 'content-type': TYPES[extname(file)] ?? 'application/octet-stream' });
    res.end(readFileSync(file));
  });
  return new Promise((ok) => server.listen(0, '127.0.0.1', () => ok({ port: server.address().port, close: () => server.close() })));
}

/// A headless Chrome, and a tab in it.
export async function browser() {
  const profile = mkdtempSync(join(tmpdir(), 'architect-web-'));
  const proc = spawn(chrome(), [
    '--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`,
    '--no-first-run', '--no-default-browser-check', '--window-size=1280,860', 'about:blank',
  ], { stdio: ['ignore', 'ignore', 'pipe'] });
  const ws = await new Promise((ok, no) => {
    let err = '';
    proc.stderr.on('data', (d) => {
      err += d;
      const m = err.match(/DevTools listening on (ws:\/\/\S+)/);
      if (m) ok(m[1]);
    });
    proc.on('exit', (c) => no(new Error(`chrome exited ${c}: ${err}`)));
  });
  const http = ws.replace('ws://', 'http://').replace(/\/devtools\/.*/, '');
  const tabs = await (await fetch(`${http}/json/list`)).json();
  const tab = new Tab(tabs.find((t) => t.type === 'page').webSocketDebuggerUrl);
  await tab.open();
  return {
    tab,
    close: () => {
      tab.socket.close();
      proc.kill();
      try { rmSync(profile, { recursive: true, force: true }); } catch {}
    },
  };
}

export class Tab {
  constructor(url) {
    this.url = url;
    this.next = 1;
    this.waiting = new Map();
    this.console = [];
  }

  open() {
    this.socket = new WebSocket(this.url);
    this.socket.onmessage = (e) => {
      const m = JSON.parse(e.data);
      if (m.id && this.waiting.has(m.id)) {
        const { ok, no } = this.waiting.get(m.id);
        this.waiting.delete(m.id);
        if (m.error) no(new Error(m.error.message));
        else ok(m.result);
      } else if (m.method === 'Runtime.consoleAPICalled') {
        this.console.push(m.params.args.map((a) => a.value ?? a.description).join(' '));
      } else if (m.method === 'Runtime.exceptionThrown') {
        this.console.push('exception: ' + m.params.exceptionDetails.exception?.description);
      }
    };
    return new Promise((ok) => (this.socket.onopen = async () => {
      await this.send('Runtime.enable');
      await this.send('Page.enable');
      ok();
    }));
  }

  send(method, params = {}) {
    const id = this.next++;
    this.socket.send(JSON.stringify({ id, method, params }));
    return new Promise((ok, no) => this.waiting.set(id, { ok, no }));
  }

  /// `expression`'s value (awaited).
  async eval(expression) {
    const r = await this.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
    return r.result.value;
  }

  async go(url) {
    await this.send('Page.navigate', { url });
    await this.until(() => this.eval('document.readyState === "complete"'), 'the page to load');
  }

  /// Waits for `f()` to be truthy: its value.
  async until(f, what, ms = 10000) {
    const started = Date.now();
    let last;
    for (;;) {
      try {
        last = await f();
        if (last) return last;
      } catch (e) {
        last = e;
      }
      if (Date.now() - started > ms) throw new Error(`waited ${ms} ms for ${what}; last: ${last}`);
      await new Promise((r) => setTimeout(r, 50));
    }
  }

  text(selector) {
    return this.eval(`document.querySelector(${JSON.stringify(selector)})?.textContent ?? null`);
  }

  /// A real click (the mouse pressed and let go) at `selector`'s middle.
  async click(selector) {
    const at = await this.until(
      () => this.eval(`(() => { const r = document.querySelector(${JSON.stringify(selector)})?.getBoundingClientRect(); return r && r.width ? { x: r.x + r.width / 2, y: r.y + r.height / 2 } : null; })()`),
      selector,
    );
    for (const type of ['mousePressed', 'mouseReleased'])
      await this.send('Input.dispatchMouseEvent', { type, x: at.x, y: at.y, button: 'left', clickCount: 1 });
  }

  /// Types `text` into `selector`, as keys would.
  async type(selector, text) {
    await this.click(selector);
    await this.send('Input.insertText', { text });
  }

  async shot(path) {
    const { data } = await this.send('Page.captureScreenshot', { format: 'png' });
    const { writeFileSync, mkdirSync } = await import('node:fs');
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, Buffer.from(data, 'base64'));
  }

  /// The lab's open page, as `lab_status` says it.
  status() {
    return this.eval(`import('./architect_sandbox_web.js').then((m) => JSON.parse(m.lab_status()))`);
  }
}
