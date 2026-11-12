// The web front end in a real browser (`just web-test`): the bundle
// (scripts/web-bundle.sh) behind static-web-server with the image's config
// (apps/web/sws.toml) at ARCHITECT_WEB_ORIGIN, in a headless Firefox of its
// own driven over WebDriver BiDi (FIREFOX=… names one, else `firefox` on
// the PATH). Nothing leaves the machine: the request to the issuer is
// caught and failed.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const origin = process.env.ARCHITECT_WEB_ORIGIN;
function firefox() {
  if (process.env.FIREFOX) return process.env.FIREFOX;
  try { return execFileSync('sh', ['-c', 'command -v firefox']).toString().trim() || null; } catch { return null; }
}
const skip = origin && firefox() ? false : 'needs ARCHITECT_WEB_ORIGIN (just web-test) and a Firefox (FIREFOX=…)';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/// A headless Firefox and one tab of it, over BiDi.
async function open() {
  const profile = mkdtempSync(join(tmpdir(), 'architect-web-'));
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
  const tab = {
    send,
    on: (f) => listeners.push(f),
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
        await sleep(100);
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

const problem = 'document.querySelector("#app .problem")?.textContent';

test('the site, the docs and the app share one host', { skip }, async () => {
  const { tab, close } = await open();
  try {
    // The site (apps/site, prerendered): the landing page with the mark in
    // its header, and its pages at their clean URLs (scripts/web-routes.md).
    await tab.go(`${origin}/`);
    assert.ok(await tab.eval('!!document.querySelector("h1")'));
    assert.ok(await tab.eval('!!document.querySelector("header .brand-mark path")'));
    assert.equal(await tab.eval('!!document.querySelector(".hero-plan")'), false);
    for (const [path, title] of [['/download', /^Download/], ['/changelog', /^Changelog/], ['/extensions', /^Extensions/]]) {
      await tab.go(`${origin}${path}`);
      assert.match(await tab.eval('document.title'), title, path);
    }
    await tab.go(`${origin}/docs/`);
    assert.match(await tab.eval('document.title'), /architect/);

    // Signed out, the app offers sign-in; its button goes to the issuer
    // as client `architect`, with PKCE, back to /auth/callback.
    await tab.go(`${origin}/app/`);
    await tab.until('!!document.querySelector("[data-action=sign-in]")', 'the sign-in button');
    const issuer = new URL(await tab.eval('fetch("/app/config.json").then((r) => r.json()).then((c) => c.issuer)'));
    await tab.send('network.addIntercept', { phases: ['beforeRequestSent'], urlPatterns: [{ type: 'pattern', hostname: issuer.hostname }] });
    const caught = new Promise((ok) => tab.on((m) => {
      if (m.method === 'network.beforeRequestSent' && m.params.isBlocked) {
        tab.send('network.failRequest', { request: m.params.request.request });
        ok(m.params.request.url);
      }
    }));
    await tab.send('session.subscribe', { events: ['network.beforeRequestSent'] });
    await tab.eval('document.querySelector("[data-action=sign-in]").click()');
    const go = new URL(await caught);
    assert.equal(go.pathname, '/oauth2/authorize');
    assert.equal(go.searchParams.get('client_id'), 'architect');
    assert.equal(go.searchParams.get('code_challenge_method'), 'S256');
    assert.equal(go.searchParams.get('redirect_uri'), `${origin}/auth/callback`);
    assert.match(go.searchParams.get('scope'), /offline_access/);
    assert.equal(await tab.eval('String(!!sessionStorage.getItem("architect.pkce.verifier"))'), 'true');

    // Back with a state that is not this tab's: refused, shown, and the
    // URL is the app's again.
    await tab.go(`${origin}/auth/callback?code=c&state=not-mine`);
    assert.match(await tab.until(problem, 'the refusal'), /Sign in again|state/);
    assert.equal(await tab.eval('location.pathname'), '/app/');

    // The issuer said no.
    await tab.go(`${origin}/auth/callback?error=access_denied&error_description=No%20thanks`);
    assert.equal(await tab.until(problem, 'the refusal'), 'No thanks');
  } finally {
    close();
  }
});

test('a path the node owns is a 404 here, never the app', { skip }, async () => {
  for (const path of ['/vox', '/org/x/vox', '/s/abcdefghjkmnpqrs/', '/node/status', '/mcp', '/health']) {
    const res = await fetch(`${origin}${path}`);
    assert.equal(res.status, 404, path);
  }
});

// A distribution's bundle (`cargo xtask dist web`, #491) names its app
// and lists the extensions compiled into it; a bundle without
// distribution.json is the Architect app's, with none listed.
test('the app is the distribution its bundle was built for', { skip }, async () => {
  const res = await fetch(`${origin}/app/distribution.json`);
  const dist = res.ok ? await res.json() : { name: 'Architect', extensions: [] };
  const { tab, close } = await open();
  try {
    await tab.go(`${origin}/app/`);
    await tab.until('!!document.querySelector("[data-action=sign-in]")', 'the sign-in button');
    assert.match(await tab.eval('document.querySelector("#app h2").textContent'), new RegExp(`Sign in to ${dist.name}`));
    // Signed out, the demo is a press away.
    assert.equal(await tab.eval('document.querySelector(\'#app a[href="/app/?demo"]\')?.textContent'), 'Try the demo');
    if (res.ok) {
      assert.equal(await tab.eval('document.title'), dist.name);
      assert.equal(await tab.eval('document.querySelector("header .brand").textContent'), dist.name);
    }
    const listed = await tab.eval('JSON.stringify([...document.querySelectorAll("#app [data-extension]")].map((e) => e.dataset.extension))');
    assert.deepEqual(JSON.parse(listed), dist.extensions.map((e) => e.id));
  } finally {
    close();
  }
});

// The landing page's Demo button (`/app/?demo`): the app itself (the
// shell, the workspace, the navigator, ⌘K) on the demo vault, in the
// tab's memory: no sign-in, and no request leaves the app's own files.
test('the demo is the app on the demo vault, and sends nothing', { skip }, async () => {
  const { tab, close } = await open();
  try {
    const seen = [];
    tab.on((m) => { if (m.method === 'network.beforeRequestSent') seen.push(m.params.request.url); });
    await tab.send('session.subscribe', { events: ['network.beforeRequestSent'] });
    await tab.go(`${origin}/app/?demo`);
    await tab.until('!!document.querySelector(\'[data-note="Welcome.md"] [data-testid=note-text]\')', 'the Welcome note', 30000);
    assert.ok(await tab.eval('!!document.querySelector("[data-testid=demo-badge]")'), 'the demo says so, and signs in');
    const leaf = (text) => `[...document.querySelectorAll("#app *")].some((e) => e.childElementCount === 0 && e.textContent.trim().startsWith(${JSON.stringify(text)}))`;
    assert.ok(await tab.eval(leaf('Projects')), 'the navigator lists the vault');

    // An edit is the vault's at once: the navigator's preview follows it.
    await tab.eval(`(() => {
      const t = document.querySelector('[data-note="Welcome.md"] [data-testid=note-text]');
      t.focus();
      t.value = 'Edited in the test. ' + t.value;
      t.dispatchEvent(new Event('input', { bubbles: true }));
    })()`);
    await tab.until(leaf('Edited in the test.'), 'the edit in the navigator');

    // ⌘K: the Command Palette, its field focused.
    await tab.send('input.performActions', {
      context: (await tab.send('browsingContext.getTree', {})).contexts[0].context,
      actions: [{ type: 'key', id: 'keys', actions: [
        { type: 'keyDown', value: '' }, { type: 'keyDown', value: 'k' },
        { type: 'keyUp', value: 'k' }, { type: 'keyUp', value: '' },
      ] }],
    });
    await tab.until('document.activeElement && document.activeElement.tagName === "INPUT"', 'the palette');

    // Only the app's own files (and the site's icon) were asked for.
    const off = seen.filter((u) => { const p = new URL(u); return p.origin !== origin || !(p.pathname.startsWith('/app/') || p.pathname === '/favicon.svg'); });
    assert.deepEqual(off, [], 'requests outside /app/');
  } finally {
    close();
  }
});
