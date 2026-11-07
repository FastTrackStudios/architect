// The web app in a real browser (`just sandbox-web-test`): TSX pages in Web
// Workers (features/pages/src/web/mod.rs) — the Board's own page file, its
// Preact page (architect/dom), a runaway, a page's reach — driven by real
// clicks and keys in a headless Chrome. Skipped when the lab is not built
// or there is no Chrome.
import { after, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { browser, built, chrome, root, serve } from './browser.mjs';

const ready = existsSync(join(built, 'architect_sandbox_web_bg.wasm')) && chrome();
const skip = ready ? false : 'needs `just sandbox-web 0` and a Chrome (CHROME=…)';
const SHOTS = process.env.ARCHITECT_LAB_WEB_SHOTS ?? join(root, 'target/web-shots');

let server;
let chromium;
let tab;
before(async () => {
  if (skip) return;
  server = await serve();
  chromium = await browser();
  tab = chromium.tab;
});
after(() => {
  chromium?.close();
  server?.close();
});

const url = (page) => `http://127.0.0.1:${server.port}/?page=${page}`;

/// The longest gap between animation frames from now until `stop()`: the
/// main thread's, so a page that waits on it would show.
async function gaps() {
  await tab.eval(`(() => { window.__gaps = { worst: 0, last: performance.now(), on: true };
    const f = (t) => { const g = window.__gaps; if (!g.on) return; g.worst = Math.max(g.worst, t - g.last); g.last = t; requestAnimationFrame(f); };
    requestAnimationFrame(f); })()`);
  return { stop: () => tab.eval('(window.__gaps.on = false, window.__gaps.worst)') };
}

test('the Board page renders from its own file and its handlers run', { skip }, async () => {
  await tab.go(url('board'));
  assert.equal(await tab.until(() => tab.text('#board-title'), 'the Board page'), 'Board');
  assert.match(await tab.text('#board-page'), /drawn in the dark theme lab-web-zinc/);
  // Typed in and pinned: the page ran LAB_BOARD_PIN, then drew the board.
  await tab.type('#pin-title', 'From the browser');
  await tab.until(async () => (await tab.eval(`document.querySelector('#pin-title').value`)) === 'From the browser', 'the field to hold what was typed');
  await tab.click('#pin');
  await tab.until(async () => (await tab.text('.pin'))?.includes('From the browser'), 'the pin');
  assert.equal(await tab.eval(`document.querySelector('#pin-title').value`), '', 'the field is emptied after a pin');
  // Its worker counts primes in a Web Worker of its own, and says how far.
  const frames = await gaps();
  await tab.click('#primes');
  await tab.until(() => tab.text('#primes-at'), 'the worker to say how far it is', 20000);
  const done = await tab.until(async () => {
    const t = await tab.text('#primes-state');
    return t?.includes('primes below') ? t : null;
  }, 'the worker to finish', 60000);
  assert.equal(done, '78498 primes below 1,000,000.');
  const worst = await frames.stop();
  assert.ok(worst < 250, `the main thread kept drawing while the worker counted: worst gap ${worst} ms`);
  await tab.click('#pin');
  await tab.until(async () => (await tab.eval(`document.querySelectorAll('.pin').length`)) === 2, 'a second pin');
  await tab.shot(join(SHOTS, 'board.png'));
  const status = await tab.status();
  assert.equal(status.error, null, JSON.stringify(status));
  assert.equal(status.running, true);
});

test('a Preact page renders to the DOM in its worker and answers clicks', { skip }, async () => {
  await tab.go(url('preact'));
  assert.equal(await tab.until(() => tab.text('#preact-title'), 'the Preact page'), 'Preact');
  assert.equal(await tab.text('#preact-count'), '0');
  await tab.click('#preact-inc');
  await tab.until(async () => (await tab.text('#preact-count')) === '1', 'a click to count');
  await tab.click('#preact-inc');
  await tab.until(async () => (await tab.text('#preact-count')) === '2', 'a second click');
  // Typed in: the input event goes to Preact's listener, its state back.
  await tab.type('#preact-draft', 'From Preact');
  await tab.until(async () => (await tab.eval(`document.querySelector('#preact-draft').value`)) === 'From Preact', 'the field to hold what was typed');
  await tab.click('#preact-add');
  await tab.until(async () => (await tab.text('.preact-item')) === 'From Preact', 'the item');
  assert.equal(await tab.eval(`document.querySelector('#preact-draft').value`), '', 'the field is emptied');
  await tab.click('#preact-reset');
  await tab.until(async () => (await tab.text('#preact-count')) === '0', 'the reset');
  await tab.shot(join(SHOTS, 'preact.png'));
  const status = await tab.status();
  assert.equal(status.error, null, JSON.stringify(status));
});

test('a runaway render is stopped and the rest of the app keeps working', { skip }, async () => {
  await tab.go(url('runaway'));
  await tab.click('#inc');
  await tab.until(async () => (await tab.text('#count')) === '1', 'a click to count');
  const frames = await gaps();
  const started = Date.now();
  await tab.click('#spin');
  const banner = await tab.until(() => tab.text('.page-error'), 'the banner', 5000);
  const took = Date.now() - started;
  assert.match(banner, /runaway\.tsx: ran past its time of 500 ms: hand heavy work to a worker \(worker\.spawn\)/);
  assert.match(banner, /it was stopped and started again/);
  // The rest of the app never waited: its frames, its other pane.
  const worst = await frames.stop();
  assert.ok(worst < 250, `the main thread kept drawing: worst gap ${worst} ms over ${took} ms`);
  await tab.until(async () => ((await tab.text('#counter')) ?? '').length > 0, 'the WASI counter, still drawn');
  // Started again from its file: fresh, and it answers.
  await tab.until(async () => (await tab.text('#count')) === '0', 'the page, started again');
  await tab.click('#inc');
  await tab.until(async () => (await tab.text('#count')) === '1', 'the restarted page to count');
  // A handler that runs away is stopped too.
  await tab.click('#spin-handler');
  await tab.until(async () => (await tab.status()).log.filter((l) => l.includes('ran past its time')).length >= 2, 'the second stop');
  await tab.until(async () => (await tab.text('#count')) === '0', 'the page, started again again');
  await tab.shot(join(SHOTS, 'runaway.png'));
});

test('a host call outside the page\'s capabilities is refused', { skip }, async () => {
  await tab.go(url('net'));
  const out = await tab.until(async () => {
    const t = await tab.text('#out');
    return t ? t : null;
  }, 'the fetches');
  const [body, refused] = out.split('|');
  assert.equal(body, 'hello from the server', out);
  assert.match(refused, /refused \(hosts\)/, out);
  assert.match(refused, /localhost/, out);
  const run = await tab.until(() => tab.text('#run'), 'the run');
  assert.match(run, /LAB_BOARD_STATE: refused \(actions\)/, run);
  const ambient = await tab.text('#ambient');
  for (const k of ['fetch', 'XMLHttpRequest', 'WebSocket', 'importScripts', 'indexedDB', 'Worker', 'postMessage'])
    assert.match(ambient, new RegExp(`${k}:undefined`), ambient);
  const loads = await tab.until(() => tab.text('#loads'), 'the import');
  assert.match(loads, /^import:TypeError eval:EvalError$/, loads);
  await tab.shot(join(SHOTS, 'net.png'));
});

test('a broken save keeps the last good page and says where', { skip }, async () => {
  await tab.go(url('board'));
  await tab.until(() => tab.text('#board-title'), 'the Board page');
  const reload = (text) => tab.eval(`import('./architect_sandbox_web.js').then((m) => m.lab_reload(${JSON.stringify(text)}))`);
  // It does not parse: the last good page stays, the error at its line.
  await reload('import { h } from "architect";\n\nexport default function P() {\n  return <div style={{ fontSize: 26 }>x</div>;\n}\n');
  const parse = await tab.until(async () => (await tab.status()).error, 'the error');
  assert.match(parse, /^board\.tsx:4:\d+: /, parse);
  assert.equal(await tab.text('#board-title'), 'Board');
  assert.match(await tab.text('.page-error'), /the last good page is showing/);
  // It throws as it renders: said at its source's line, not the module's.
  await reload('import { h } from "architect";\ntype T = { a: number };\ninterface U { b: string }\n\nexport default function P() {\n  const n: number = 1;\n  throw new Error("boom " + n);\n}\n');
  const thrown = await tab.until(async () => {
    const e = (await tab.status()).error;
    return e?.includes('boom') ? e : null;
  }, 'the throw');
  assert.match(thrown, /board\.tsx:7:\d+/, thrown);
  assert.equal(await tab.text('#board-title'), 'Board');
  // A good save: swapped in, the banner gone.
  await reload('import { h, useState } from "architect";\nexport default function P() {\n  const [n, setN] = useState(0);\n  return <button id="again" onClick={() => setN(n + 1)}>Saved {n}</button>;\n}\n');
  await tab.until(async () => (await tab.text('#again')) === 'Saved 0', 'the new page');
  await tab.click('#again');
  await tab.until(async () => (await tab.text('#again')) === 'Saved 1', 'its handler');
  assert.equal((await tab.status()).error, null);
  assert.equal(await tab.eval(`document.querySelector('.page-error')`), null);
});
