// TypeScript actions in a real browser (#536, #710 R7): the showcase's
// Recipe scaler (`?page=recipes`, apps/sandbox/web/src/recipes.rs), its
// actions each run in a Web Worker on the browser's engine
// (features/pages/src/web/actions.rs) and its page in another — no
// QuickJS, no JSPI. Driven in a headless Firefox over WebDriver BiDi
// (FIREFOX=… names one, else `firefox` on the PATH): the page lists the
// recipes its action read, scales them as + is pressed, and its Write
// button runs the action that makes the shopping list.
//
// `just sandbox-web 0` first. Skipped when the lab is not built or there is
// no Firefox. SHOTS=<dir> writes its screenshots there (390 px and 1280
// px wide).
import { after, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { built, serve } from './browser.mjs';

function firefox() {
  if (process.env.FIREFOX) return process.env.FIREFOX;
  try {
    return execFileSync('sh', ['-c', 'command -v firefox']).toString().trim() || null;
  } catch {
    return null;
  }
}
const ready = existsSync(join(built, 'architect_sandbox_web_bg.wasm')) && firefox();
const skip = ready ? false : 'needs `just sandbox-web 0` and a Firefox (FIREFOX=…)';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/// A headless Firefox and its one tab, over BiDi.
async function open() {
  const profile = mkdtempSync(join(tmpdir(), 'architect-actions-'));
  const port = 19000 + Math.floor(Math.random() * 900);
  const proc = spawn(firefox(), ['--headless', '--no-remote', '--profile', profile, '--remote-debugging-port', String(port)], { stdio: 'ignore' });
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
  const send = (method, params = {}) =>
    new Promise((ok, no) => {
      const i = ++id;
      waiting.set(i, { ok, no });
      ws.send(JSON.stringify({ id: i, method, params }));
    });
  await send('session.new', { capabilities: {} });
  const context = (await send('browsingContext.getTree', {})).contexts[0].context;
  const tab = {
    send,
    context,
    go: (url) => send('browsingContext.navigate', { context, url, wait: 'complete' }),
    eval: async (expression) => {
      const r = await send('script.evaluate', { expression, target: { context }, awaitPromise: true });
      if (r.type === 'exception') throw new Error(r.exceptionDetails.text);
      return r.result.value;
    },
    async until(expression, what, ms = 15000) {
      const started = Date.now();
      for (;;) {
        const v = await tab.eval(expression).catch(() => null);
        if (v) return v;
        if (Date.now() - started > ms) throw new Error(`waited ${ms} ms for ${what}`);
        await sleep(100);
      }
    },
    text: (selector) => tab.eval(`document.querySelector(${JSON.stringify(selector)})?.textContent ?? null`),
    // A real click at the element's middle (the pane's handlers are the
    // applier's, on the main thread).
    async click(selector, n = 0) {
      const at = await tab.eval(`(() => { const e = document.querySelectorAll(${JSON.stringify(selector)})[${n}]; if (!e) return null; e.scrollIntoView({ block: 'center' }); const r = e.getBoundingClientRect(); return JSON.stringify([Math.round(r.x + r.width / 2), Math.round(r.y + r.height / 2)]); })()`).then((v) => (v ? JSON.parse(v) : null));
      if (!at) throw new Error(`no ${selector} [${n}]`);
      await send('input.performActions', {
        context,
        actions: [{ type: 'pointer', id: 'mouse', parameters: { pointerType: 'mouse' }, actions: [
          { type: 'pointerMove', x: at[0], y: at[1] },
          { type: 'pointerDown', button: 0 },
          { type: 'pointerUp', button: 0 },
        ] }],
      });
    },
    async shot(path, width) {
      await send('browsingContext.setViewport', { context, viewport: { width, height: 860 } });
      await sleep(400);
      const { data } = await send('browsingContext.captureScreenshot', { context });
      writeFileSync(path, Buffer.from(data, 'base64'));
    },
  };
  return {
    tab,
    close: () => {
      ws.close();
      proc.kill();
      try {
        rmSync(profile, { recursive: true, force: true });
      } catch {}
    },
  };
}

let server;
let fx;
before(async () => {
  if (skip) return;
  server = await serve();
  fx = await open();
});
after(() => {
  fx?.close();
  server?.close();
});

test('the Recipe scaler: TypeScript actions in Web Workers, read, scaled and written', { skip }, async () => {
  const { tab } = fx;
  await tab.go(`http://127.0.0.1:${server.port}/?page=recipes`);
  // Its list action ran in a worker: the recipes, Chili first.
  await tab.until(`document.querySelectorAll('.recipe').length === 2`, 'the recipes its action listed').catch(async (e) => {
    const state = await tab.eval(`import('./architect_sandbox_web.js').then((m) => m.lab_status())`).catch(String);
    const page = await tab.eval(`document.querySelector('#pane')?.textContent ?? ''`).catch(String);
    const log = await tab.eval(`document.querySelector('#log')?.textContent ?? ''`).catch(String);
    const status = await tab.eval(`document.querySelector('#status')?.textContent ?? ''`).catch(String);
    throw new Error(`${e.message}\nstatus: ${status}\npane: ${page}\nlab: ${state}\nlog: ${log}`);
  });
  assert.equal(await tab.text('#servings'), '12');
  assert.match(await tab.text('#items'), /1000 g minced beef/);
  // Pancakes, then + twice: 8 → 10.
  await tab.click('.recipe', 1);
  await tab.until(`document.querySelector('#servings')?.textContent === '8'`, 'Pancakes for 8');
  await tab.click('#scaler button', 3);
  await tab.until(`document.querySelector('#servings')?.textContent === '9'`, '9');
  await tab.click('#scaler button', 3);
  await tab.until(`document.querySelector('#servings')?.textContent === '10'`, '10');
  assert.match(await tab.text('#items'), /3 3\/4 cup flour/);
  // Write: SHOWCASE_RECIPES_SCALE in a worker of its own makes the note.
  await tab.click('#scaler button', 4);
  const made = await tab.until(`document.querySelector('#made')?.textContent`, 'the shopping list', 20000);
  assert.equal(made, 'Wrote Shopping/Pancakes for 10.md');
  const note = JSON.parse(await tab.eval(`import('./architect_sandbox_web.js').then((m) => m.lab_note('Shopping/Pancakes for 10.md'))`));
  assert.equal(note.properties.servings, 10);
  assert.equal(note.properties.recipe, '[[Pancakes]]');
  assert.match(note.body, /- \[ \] 3 3\/4 cup flour/);
  // Its toast went through the page's host, as the extension.
  await tab.until(`document.querySelector('#status')?.textContent.includes('Shopping list: Shopping/Pancakes for 10.md')`, 'its toast');
  if (process.env.SHOTS) {
    mkdirSync(process.env.SHOTS, { recursive: true });
    for (const width of [390, 1280]) await tab.shot(join(process.env.SHOTS, `recipe-scaler-web-${width}-dark.png`), width);
  }
});
