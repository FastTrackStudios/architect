// The web view kind's browser host (webview-host.js) and its runtime
// (webview-runtime.js, the same file as features/extensions/src/webview/runtime.js)
// in headless Firefox: a web view app in its sandboxed frame reaches the
// page only through the bridge; its calls are the host's to answer; it has
// no network, cookies, storage or parent; the theme and events reach it; an
// Escape gives the focus back.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { firefox, open, serve } from './firefox.mjs';
import { WEBVIEW_CSP, themeCss } from '../webview-host.js';

const skip = firefox() ? false : 'needs a Firefox (FIREFOX=…)';
const root = fileURLToPath(new URL('..', import.meta.url));

test('the theme sheet is custom properties and nothing else', () => {
  assert.equal(
    themeCss('light', { brand: '#6c5ce7', 'studio.accent': 'red', 'bad name': 'x', evil: 'red;}body{}', img: 'url(x)' }),
    ':root{color-scheme:light;--brand:#6c5ce7;--studio-accent:red;}',
  );
});

test('a web view app reaches the page only through its bridge', { skip, timeout: 60000 }, async () => {
  const requests = [];
  // A document's policy; every file readable from the frame's opaque
  // origin (its module scripts are fetched with CORS, `Origin: null`).
  const server = await serve(root, (req) => requests.push(req.url), (path) => ({
    'access-control-allow-origin': '*',
    ...(path.startsWith('/test/webview-app/') && path.endsWith('.html') ? { 'content-security-policy': WEBVIEW_CSP } : {}),
  }));
  const { tab, close } = await open();
  try {
    await tab.go(`${server.origin}/test/webview.html`);
    const r = JSON.parse(await tab.until('window.report && JSON.stringify(window.report)', 'the app’s report').catch(async (e) => {
      throw new Error(`${e.message}; calls ${await tab.eval('JSON.stringify(window.calls)')}; requests ${requests.join(' ')}`);
    }));
    assert.deepEqual(r['own action'], { ok: 'ran' });
    assert.deepEqual(r['outside action'], { error: 'refused' });
    assert.deepEqual(r['host fetch'], { ok: 200 });
    assert.ok(r['direct fetch'].error, `its own fetch is blocked: ${JSON.stringify(r['direct fetch'])}`);
    assert.ok(r.cookie.error, `no cookies in an opaque origin: ${JSON.stringify(r.cookie)}`);
    assert.ok(r.storage.error, `no storage in an opaque origin: ${JSON.stringify(r.storage)}`);
    assert.ok(r['parent dom'].error, 'the page’s document is not its');
    assert.ok(r['top navigation'].error, `no navigating the page: ${JSON.stringify(r['top navigation'])}`);
    assert.equal(r.webrtc, 'undefined', 'no WebRTC past the policy');
    assert.equal(r.brand, '#6c5ce7');
    assert.equal(r.scheme, 'light');
    assert.equal(r.components, 'solid', 'the component sheet is installed');
    assert.deepEqual(r.heard, { name: 'doc.saved', detail: { path: 'a.md' } });
    const page = JSON.parse(await tab.eval('JSON.stringify({ calls: window.calls, escapes: window.escapes, title: document.title })'));
    assert.equal(page.title, 'webview host', 'the page is as it was');
    assert.ok(page.escapes >= 1, 'an Escape handed the focus back');
    assert.deepEqual([...new Set(page.calls)].sort(), ['fetch', 'log', 'run']);
    assert.ok(!requests.includes('/package.json'), 'its own fetch never reached the server');
  } finally {
    await close();
    server.close();
  }
});

test('a frame that navigates away is stopped and never handed the bridge again', { skip, timeout: 60000 }, async () => {
  const requests = [];
  const server = await serve(root, (req) => requests.push(req.url), (path) => ({
    'access-control-allow-origin': '*',
    ...(path === '/test/webview-nav/index.html' ? { 'content-security-policy': WEBVIEW_CSP } : {}),
  }));
  const { tab, close } = await open();
  try {
    await tab.go(`${server.origin}/test/webview-nav.html`);
    await tab.until('window.view && window.view.stopped', 'the host to stop the frame');
    // Its navigation happened: the document it reached loaded…
    for (let i = 0; i < 40 && !requests.includes('/outside-loaded'); i++) await new Promise((r) => setTimeout(r, 100));
    await new Promise((r) => setTimeout(r, 500));
    // …and no port was posted to it.
    assert.ok(!requests.includes('/got-port'), `the outside document got the bridge: ${requests.join(' ')}`);
    const page = JSON.parse(await tab.eval(`JSON.stringify({
      logs: window.logs,
      frames: document.querySelectorAll('iframe').length,
      note: document.querySelector('[data-webview-stopped]')?.textContent ?? null,
    })`));
    assert.deepEqual(page.logs, ['about to leave'], 'it had the bridge before it left');
    assert.equal(page.frames, 0, 'the frame is gone');
    assert.match(page.note ?? '', /tried to leave its package/);
  } finally {
    await close();
    server.close();
  }
});
