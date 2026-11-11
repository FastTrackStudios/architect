// The example page in a headless Firefox (`npm test`: it builds first):
// the extension loads through the package's public API under the page's
// strict CSP (pinned by its lock, in its opaque-origin sandbox), its typed action and an app action it was granted answer,
// its view draws and answers a click, its embed view draws its input.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { firefox, open, serve } from '../../../features/extensions/web/test/firefox.mjs';

const dist = fileURLToPath(new URL('../dist/', import.meta.url));
const skip = !firefox()
  ? 'needs a Firefox (FIREFOX=…)'
  : !existsSync(`${dist}counter/architect.json`)
    ? 'not built (npm run build, after the counter component is built)'
    : false;

test('the counter runs in the page: actions, a granted app action, its views, under a strict CSP', { skip, timeout: 120000 }, async () => {
  const server = await serve(dist);
  const { tab, close } = await open();
  try {
    const errors = [];
    await tab.send('session.subscribe', { events: ['log.entryAdded'] });
    tab.on((m) => {
      if (m.method === 'log.entryAdded' && m.params.level === 'error') errors.push(m.params.text);
    });
    await tab.go(`${server.origin}/`);
    const ready = await tab.until('document.body.dataset.ready', 'the extension to load', 60000);
    const status = await tab.eval('document.getElementById("status").textContent');
    assert.equal(ready, 'true', status);
    assert.match(status, /^Loaded WASI Counter 0\.1\.0 \(architect:extension@0\.1\.0\): 4 actions, 2 views\.$/);

    // The typed action, run by the page.
    await tab.eval('document.getElementById("add").click()');
    await tab.until('document.getElementById("answer").textContent === "5"', 'Add 5 to answer 5');
    // Its view followed, and a click in the view reaches the component.
    await tab.until('document.querySelector("#counter .architect-view-host")?.shadowRoot?.querySelector(".wasi-count")?.textContent === "5"', 'the view to show 5');
    await tab.eval('document.querySelector("#counter .architect-view-host").shadowRoot.querySelector(".wasi-add").click()');
    await tab.until('document.querySelector("#counter .architect-view-host")?.shadowRoot?.querySelector(".wasi-count")?.textContent === "6"', 'a click in the view to make it 6');
    // An app action it asked for and the page granted, through the host.
    await tab.eval('document.getElementById("ask").click()');
    await tab.until('document.getElementById("answer").textContent === \'["light","dark","sepia"]\'', 'the granted THEME_LIST');
    // The embed view drew the input it was sent.
    const bars = JSON.parse(
      await tab.until(
        '(() => { const t = [...(document.querySelector("#bars .architect-view-host")?.shadowRoot?.querySelectorAll("svg.wasi-bars text") ?? [])].map((e) => e.textContent); return t.length ? JSON.stringify(t) : null; })()',
        'the bars to draw',
      ),
    );
    for (const b of ['C', 'G', 'Am', 'F']) assert.ok(bars.includes(b), `${b} in ${bars}`);
    // Drawn inside its own box, with nothing that runs.
    assert.equal(await tab.eval('document.querySelectorAll("#counter > .architect-view-host").length'), 1, 'in a shadow root of its own');
    assert.equal(await tab.eval('[...document.querySelector("#counter .architect-view-host").shadowRoot.querySelectorAll("*")].filter((e) => [...e.attributes].some((a) => /^on/i.test(a.name))).length'), 0);
    // Its instance ran in the sandbox frame, an opaque origin; its files were pinned.
    assert.equal(await tab.eval('document.querySelectorAll("iframe[sandbox=allow-scripts]").length'), 1);
    assert.deepEqual(errors.filter((e) => /Content-Security-Policy|CSP/i.test(e)), [], 'no CSP violation');
  } finally {
    close();
    server.close();
  }
});

test('under Trusted Types, with the package’s policy allowed, it still runs', { skip, timeout: 120000 }, async () => {
  const { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } = await import('node:fs');
  const { tmpdir } = await import('node:os');
  const { join } = await import('node:path');
  const dir = mkdtempSync(join(tmpdir(), 'architect-tt-'));
  cpSync(dist, dir, { recursive: true });
  const html = readFileSync(join(dir, 'index.html'), 'utf8');
  writeFileSync(join(dir, 'index.html'), html.replace("object-src 'none'", "object-src 'none'; require-trusted-types-for 'script'; trusted-types fasttrackstudio-architect"));
  const server = await serve(dir);
  const { tab, close } = await open();
  try {
    await tab.go(`${server.origin}/`);
    assert.equal(await tab.until('document.body.dataset.ready', 'the extension to load', 60000), 'true');
    assert.equal(await tab.eval('"trustedTypes" in window'), true, 'this Firefox enforces them');
    await tab.eval('document.getElementById("add").click()');
    await tab.until('document.getElementById("answer").textContent === "5"', 'Add 5 to answer 5');
  } finally {
    close();
    server.close();
    rmSync(dir, { recursive: true, force: true });
  }
});
