// The app on the org's vaults, signed in, in a real browser (#628,
// `just web-org-test`): against a running local stack (`just dev`), whose
// web app is at ARCHITECT_ORG_WEB (default http://127.0.0.1:7413) and
// whose node keeps its vault in ARCHITECT_ORG_VAULT (the stack's
// `node/data/vaults/demo`). Signs in by the real redirect as the demo
// seed's ada@local.test. In a headless Firefox of its own over WebDriver
// BiDi (FIREFOX=… names one), with no session bus, display or audio.
//
// What it protects:
// - a note's text area is read-only until its session has attached, and
//   typing then does not add the note twice;
// - the navigator changes nothing on the org's vaults: a rename (F2), a
//   delete (the Delete key, and the palette's command) and a drag and drop
//   all say it is read-only, and the node's files stay as they were;
// - a new note never empties a note of that name made since the listing;
// - the lane is dialled again with a renewed token, and edits go on;
// - Sign out revokes the sign-in at the issuer: its refresh token is dead.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, readFileSync, readdirSync, writeFileSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const origin = process.env.ARCHITECT_ORG_WEB || 'http://127.0.0.1:7413';
const vault = process.env.ARCHITECT_ORG_VAULT;
const email = process.env.ARCHITECT_ORG_EMAIL || 'ada@local.test';
const password = process.env.ARCHITECT_ORG_PASSWORD || 'development-password';
function firefox() {
  if (process.env.FIREFOX) return process.env.FIREFOX;
  try { return execFileSync('sh', ['-c', 'command -v firefox']).toString().trim() || null; } catch { return null; }
}
const skip = vault && existsSync(vault) && firefox() ? false : 'needs a running local stack (ARCHITECT_ORG_VAULT, `just web-org-test`) and a Firefox';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const file = (p) => readFileSync(join(vault, p), 'utf8');
const listing = () => readdirSync(vault, { recursive: true }).map(String).sort().join('\n');
const TEXT = 'textarea[data-testid=note-text]';
const READ_ONLY = 'read-only in the browser';

/// A headless Firefox of its own (no session bus, display, audio or
/// PipeWire reached), over BiDi.
async function browser() {
  const profile = mkdtempSync(join(tmpdir(), 'architect-org-ff-'));
  const runtime = mkdtempSync(join(tmpdir(), 'architect-org-rt-'));
  writeFileSync(join(profile, 'user.js'), 'user_pref("media.cubeb.sandbox", true);\nuser_pref("media.autoplay.default", 5);\n');
  const env = { ...process.env, XDG_RUNTIME_DIR: runtime };
  for (const k of ['LD_LIBRARY_PATH', 'DBUS_SESSION_BUS_ADDRESS', 'PULSE_SERVER', 'PIPEWIRE_REMOTE', 'PIPEWIRE_RUNTIME_DIR', 'WAYLAND_DISPLAY', 'DISPLAY']) delete env[k];
  const port = 19500 + Math.floor(Math.random() * 400);
  const proc = spawn(firefox(), ['--headless', '--no-remote', '--profile', profile, '--remote-debugging-port', String(port)], { stdio: 'ignore', env });
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
  async function tab(width, height) {
    // A tab first, then its viewport.
    const context = (await send('browsingContext.create', { type: 'tab' })).context;
    await send('browsingContext.setViewport', { context, viewport: { width, height } });
    const element = async (expression) => {
      const r = await send('script.evaluate', { expression, target: { context }, awaitPromise: false });
      if (!r.result.sharedId) throw new Error(`no element: ${expression}`);
      return { sharedId: r.result.sharedId };
    };
    const t = {
      context,
      on: (f) => listeners.push(f),
      go: (url) => send('browsingContext.navigate', { context, url, wait: 'complete' }),
      eval: async (expression) => {
        const r = await send('script.evaluate', { expression, target: { context }, awaitPromise: true });
        if (r.type === 'exception') throw new Error(r.exceptionDetails.text);
        return r.result.value;
      },
      async until(expression, what, ms = 30000) {
        const started = Date.now();
        for (;;) {
          const v = await t.eval(expression).catch(() => null);
          if (v) return v;
          if (Date.now() - started > ms) throw new Error(`waited ${ms} ms for ${what}`);
          await sleep(100);
        }
      },
      async keys(...keys) {
        const actions = [];
        for (const k of keys) actions.push({ type: 'keyDown', value: k }, { type: 'keyUp', value: k });
        await send('input.performActions', { context, actions: [{ type: 'key', id: 'kb', actions }] });
      },
      async type(text) { await t.keys(...text); },
      async chord(mod, key) {
        await send('input.performActions', { context, actions: [{ type: 'key', id: 'kb', actions: [
          { type: 'keyDown', value: mod }, { type: 'keyDown', value: key }, { type: 'keyUp', value: key }, { type: 'keyUp', value: mod }] }] });
      },
      async click(expression, button = 0) {
        const el = await element(expression);
        await send('input.performActions', { context, actions: [{ type: 'pointer', id: 'm', actions: [
          { type: 'pointerMove', x: 0, y: 0, origin: { type: 'element', element: el } },
          { type: 'pointerDown', button }, { type: 'pointerUp', button }] }] });
      },
      async drag(from, to) {
        const a = await element(from);
        const b = await element(to);
        await send('input.performActions', { context, actions: [{ type: 'pointer', id: 'm', actions: [
          { type: 'pointerMove', x: 0, y: 0, origin: { type: 'element', element: a } },
          { type: 'pointerDown', button: 0 },
          { type: 'pointerMove', x: 10, y: 10, origin: { type: 'element', element: a }, duration: 150 },
          { type: 'pointerMove', x: 20, y: 20, origin: { type: 'element', element: a }, duration: 150 },
          { type: 'pointerMove', x: 0, y: 0, origin: { type: 'element', element: b }, duration: 400 },
          { type: 'pointerMove', x: 3, y: 1, origin: { type: 'element', element: b }, duration: 150 },
          { type: 'pause', duration: 200 },
          { type: 'pointerUp', button: 0 }] }] });
      },
    };
    return t;
  }
  return {
    send,
    tab,
    close: () => {
      try { ws.close(); } catch {}
      proc.kill();
      try { rmSync(profile, { recursive: true, force: true }); rmSync(runtime, { recursive: true, force: true }); } catch {}
    },
  };
}

/// Signed in by the real redirect: the app, then the issuer's form.
async function signIn(t) {
  await t.go(`${origin}/app/`);
  await t.until('!!document.querySelector("[data-action=sign-in]")', 'the sign-in button');
  await t.eval('document.querySelector("[data-action=sign-in]").click()');
  await t.until('!!document.querySelector("input[name=email]")', "the issuer's form");
  await t.eval(`(() => { const f = document.querySelector("input[name=email]").form; f.email.value = ${JSON.stringify(email)}; f.password.value = ${JSON.stringify(password)}; f.submit(); return true })()`);
  await t.until(`location.origin === ${JSON.stringify(origin)} && document.body.innerText.includes("on the node")`, 'the app on the org', 60000);
}

/// A leaf element whose text is `text`, on screen.
const byText = (text) => `[...document.querySelectorAll("body *")].find((e) => e.children.length === 0 && e.textContent.trim() === ${JSON.stringify(text)} && e.getClientRects().length)`;
const said = (text) => `document.body.innerText.includes(${JSON.stringify(text)})`;

/// The vault's folder row in the navigator's tree.
const DEMO = '[...document.querySelectorAll("[data-drop-zone=folder]")].find((e) => /^demo/.test(e.textContent.trim()) && e.getClientRects().length)';

async function openWelcome(t) {
  await t.until(`!!(${DEMO})`, 'the vault in the navigator');
  const row = 'document.querySelector(\'[data-file="demo/Welcome.md"]\')';
  // One click on the folder the moment it shows, as a person would: it
  // must take (the sidebar is in place as the app shows, never sliding
  // open under the click).
  await t.click(DEMO);
  await t.until(`!!${row}`, 'Welcome in the list after one click on the folder', 15000);
  const open = `!!document.querySelector(${JSON.stringify(TEXT)})`;
  for (let i = 0; !(await t.eval(open)); i++) {
    if (i > 20) throw new Error('the note did not open');
    await t.click(row).catch(() => {});
    await t.until(open, 'the note', 3000).catch(() => {});
  }
}

test('a note is read-only until its session attaches, and is never added twice', { skip }, async () => {
  const b = await browser();
  try {
    // Each note text area's state the moment it appears, and any input it
    // takes while it is still connecting.
    await b.send('script.addPreloadScript', { functionDeclaration: `() => {
      window.__first = null; window.__earlyInputs = 0;
      new MutationObserver(() => document.querySelectorAll(${JSON.stringify(TEXT)}).forEach((t) => {
        if (t.__seen) return; t.__seen = true;
        window.__first = window.__first || { readOnly: t.readOnly, live: t.dataset.live };
        t.addEventListener('input', () => { if (t.dataset.live !== 'ready') window.__earlyInputs++; });
      })).observe(document, { subtree: true, childList: true });
    }` });
    const t = await b.tab(1280, 900);
    await signIn(t);
    const before = file('Welcome.md');
    await openWelcome(t);
    // Typed the moment it shows.
    await t.eval(`document.querySelector(${JSON.stringify(TEXT)}).focus()`);
    await t.type('EARLY');
    await t.until(`document.querySelector(${JSON.stringify(TEXT)}).dataset.live === "ready"`, 'the note live');
    const first = JSON.parse(await t.eval('JSON.stringify(window.__first)'));
    assert.deepEqual(first, { readOnly: true, live: 'connecting' });
    assert.equal(await t.eval('window.__earlyInputs'), 0, 'typing was taken while connecting');
    await sleep(1500);
    assert.equal(file('Welcome.md'), before, 'the note changed without an edit');
    assert.ok(!(await t.eval(`document.querySelector(${JSON.stringify(TEXT)}).value.includes("EARLY")`)));

    // Live now: an edit reaches the node's file, once.
    const mark = `Typed in the browser ${Date.now()}.`;
    await t.eval(`(() => { const a = document.querySelector(${JSON.stringify(TEXT)}); a.focus(); a.setSelectionRange(a.value.length, a.value.length); return true })()`);
    await t.type(`\n${mark}`);
    for (let i = 0; !file('Welcome.md').includes(mark); i++) {
      if (i > 150) throw new Error('the edit did not reach the file');
      await sleep(200);
    }
    assert.equal(file('Welcome.md').split(mark).length - 1, 1);
    const firstLine = before.split('\n').find((l) => l.trim().length > 20);
    assert.equal(file('Welcome.md').split(firstLine).length - 1, 1, 'the note was added twice');
  } finally {
    b.close();
  }
});

test("the navigator changes nothing on the org's vaults: rename, delete, palette, drag and drop", { skip }, async () => {
  const b = await browser();
  try {
    const t = await b.tab(1280, 900);
    await signIn(t);
    // Open a note: the navigator stays beside it, its row selected.
    await openWelcome(t);
    assert.ok(await t.eval('document.querySelectorAll("[data-drop-zone=folder]").length > 0'), 'the navigator went when a note opened');
    await t.until('!!document.querySelector("[data-file][aria-selected=true]")', 'the row selected');
    const before = listing();
    const notice = async (what) => {
      await t.until(said(READ_ONLY), `the read-only notice after ${what}`, 10000);
      assert.equal(listing(), before, `${what} changed the node's files`);
      // Gone again before the next one.
      await t.keys('\uE00C');
      await sleep(400);
    };

    const list = 'document.querySelector("[data-scroller=files]")';
    // F2 on the selected note: a rename, typed and confirmed.
    await t.eval(`${list}.focus(); true`);
    await t.keys('\uE032'); // F2
    const field = 'document.querySelector("[data-file] input")';
    await t.until(`!!${field}`, 'the rename field');
    await t.eval(`(() => { const i = ${field}; i.focus(); i.select(); return true })()`);
    await t.type('Renamed by the browser');
    await t.keys('\uE007'); // Enter
    await notice('a rename (F2)');

    // The Delete key on the selected note, confirmed.
    await t.eval(`${list}.focus(); true`);
    await t.keys('\uE017'); // Delete
    await sleep(400);
    const confirm = '[...document.querySelectorAll("button")].find((b) => b.textContent.trim() === "Delete" && b.getClientRects().length)';
    if (await t.eval(`!!(${confirm})`)) await t.click(confirm);
    await notice('a delete (the Delete key)');

    // The palette's "Delete the selected notes".
    await t.eval(`${list}.focus(); true`);
    await t.chord('\uE009', 'k');
    await t.until('!!document.querySelector("input:focus")', 'the palette');
    await t.type('Delete the selected notes');
    await sleep(300);
    await t.keys('\uE007');
    await sleep(400);
    if (await t.eval(`!!(${confirm})`)) await t.click(confirm);
    await notice("the palette's delete");

    // A note dragged onto a tag (it would tag it), and onto the org's
    // root folder (it would move it out of its vault).
    const tag = '[...document.querySelectorAll("[data-drop-zone=tag]")].find((e) => e.dataset.tag && !e.dataset.tag.startsWith("__") && e.getClientRects().length)';
    await t.drag('document.querySelector("[data-file]")', tag);
    await notice('a drop onto a tag');
    const root = '[...document.querySelectorAll("[data-drop-zone=folder]")].find((e) => /^acme/.test(e.textContent.trim()) && e.getClientRects().length)';
    await t.drag('document.querySelector("[data-file]")', root);
    await notice("a drop onto the org's root");
  } finally {
    b.close();
  }
});

test('a new note never empties one of that name made since the listing', { skip }, async (t0) => {
  const made = join(vault, 'Untitled.md');
  if (existsSync(made)) return t0.skip('the vault has an Untitled.md already');
  const theirs = `Someone else's note ${Date.now()}.\n`;
  const existing = new Set(readdirSync(vault));
  const b = await browser();
  try {
    const t = await b.tab(1280, 900);
    await signIn(t);
    await openWelcome(t);
    // Made on the node after this tab listed the vault.
    writeFileSync(made, theirs);
    const others = readdirSync(vault).filter((n) => /^Untitled( \d+)?\.md$/.test(n)).length;
    await t.chord('\uE009', 'k');
    await t.until('!!document.querySelector("input:focus")', 'the palette');
    await t.type('New note');
    await sleep(300);
    await t.keys('\uE007');
    for (let i = 0; readdirSync(vault).filter((n) => /^Untitled( \d+)?\.md$/.test(n)).length === others; i++) {
      if (i > 100) throw new Error('no new note was made');
      await sleep(200);
    }
    assert.equal(readFileSync(made, 'utf8'), theirs, "someone else's note was emptied");
  } finally {
    b.close();
    // What this test made, and nothing else.
    for (const n of readdirSync(vault)) {
      if (!existing.has(n) && /^Untitled( \d+)?\.md$/.test(n)) rmSync(join(vault, n));
    }
    try { rmSync(made); } catch {}
  }
});

test('the lane is dialled again with a renewed token, and Sign out revokes the sign-in', { skip }, async () => {
  const b = await browser();
  try {
    const t = await b.tab(1280, 900);
    await signIn(t);
    // The stored session runs out in 75 s (the token is never read out here).
    await t.eval('(() => { const s = JSON.parse(localStorage.getItem("architect.session")); s.expires_at_unix = Math.floor(Date.now() / 1000) + 75; localStorage.setItem("architect.session", JSON.stringify(s)); return true })()');
    const dials = [];
    await b.send('session.subscribe', { events: ['network.beforeRequestSent'], contexts: [t.context] });
    t.on((m) => { if (m.method === 'network.beforeRequestSent' && /\/org\/[^/]+\/vox/.test(m.params.request.url)) dials.push(Date.now()); });
    await t.go(`${origin}/app/`);
    await t.until(said('on the node'), 'the app on the org', 60000);
    await openWelcome(t);
    await t.until(`document.querySelector(${JSON.stringify(TEXT)}).dataset.live === "ready"`, 'the note live');
    const firstDials = dials.length;
    await t.eval('window.__token = JSON.parse(localStorage.getItem("architect.session")).token; true');
    const renewed = 'JSON.parse(localStorage.getItem("architect.session")).token !== window.__token';
    await t.until(renewed, 'the token renewed', 90000);
    for (let i = 0; dials.length === firstDials; i++) {
      if (i > 100) throw new Error('the lane was not dialled again');
      await sleep(200);
    }
    await sleep(1500);
    const mark = `After the re-dial ${Date.now()}.`;
    await t.eval(`(() => { const a = document.querySelector(${JSON.stringify(TEXT)}); a.focus(); a.setSelectionRange(a.value.length, a.value.length); return true })()`);
    await t.type(`\n${mark}`);
    for (let i = 0; !file('Welcome.md').includes(mark); i++) {
      if (i > 150) throw new Error('an edit after the re-dial did not reach the file');
      await sleep(200);
    }
    assert.ok(!(await t.eval(said('did not reconnect'))));

    // Sign out (the account menu): the refresh token it held is refused at
    // the issuer afterwards. Held in this process, never printed.
    const held = JSON.parse(await t.eval('JSON.stringify({ refresh: JSON.parse(localStorage.getItem("architect.session")).refresh_token })'));
    assert.ok(held.refresh, 'the sign-in kept no refresh token');
    const config = JSON.parse(await t.eval('fetch("/app/config.json").then((r) => r.text())'));
    const account = '[...document.querySelectorAll("button,[role=button]")].find((e) => /Account/.test(e.getAttribute("aria-label") || "") && e.getClientRects().length)';
    await t.click(account);
    await t.until(`!!(${byText('Sign out')})`, 'the Sign out item');
    await t.click(byText('Sign out'));
    await t.until('!!document.querySelector("[data-action=sign-in]")', 'signed out', 30000);
    const refreshed = await fetch(`${config.issuer}/oauth2/token`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({ grant_type: 'refresh_token', refresh_token: held.refresh, client_id: config.client_id }),
    });
    assert.ok(refreshed.status >= 400, `the refresh token still works after Sign out (${refreshed.status})`);
  } finally {
    b.close();
  }
});
