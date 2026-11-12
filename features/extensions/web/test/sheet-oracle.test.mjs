// The judge of a view's stylesheet boundary, in a real browser (headless
// Firefox), after the second security review of architect#601: every
// payload of that review and the URL traversal family that escaped it,
// mounted as `cleanSheet` keeps it. Refused or kept, none may make the
// browser request anything but the view's own files (the server's log
// and the browser's network log both), hold a url() that resolves
// elsewhere, or draw or be hit outside its slot.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { firefox, open, serve } from './firefox.mjs';
import { decode } from './png.mjs';
import { P, PAYLOADS, TRAVERSAL } from './sheet-payloads.mjs';

const skip = firefox() ? false : 'needs a Firefox (FIREFOX=…)';
const root = fileURLToPath(new URL('..', import.meta.url));
const OWN = new RegExp(`^${P.replace(/[.]/g, '\\.')}[A-Za-z0-9._-]{1,128}$`);
/// The page's own files: the harness, not the view.
const harness = (path) => path.startsWith('/test/') || /^\/[a-z-]+\.js$/.test(path) || path === '/favicon.ico';

test('no stylesheet a view is given makes the browser request, draw or hit outside its own', { skip, timeout: 600000 }, async () => {
  const served = [];
  const server = await serve(root, (req) => served.push(req.url));
  const { tab, close } = await open();
  const wrong = [];
  let kept = 0;
  let refused = 0;
  try {
    const net = [];
    await tab.send('session.subscribe', { events: ['network.beforeRequestSent'] });
    tab.on((m) => { if (m.method === 'network.beforeRequestSent') net.push(m.params.request.url); });
    await tab.go(`${server.origin}/test/sheet-oracle.html`);
    const context = (await tab.send('browsingContext.getTree', {})).contexts[0].context;
    await tab.send('browsingContext.setViewport', { context, viewport: { width: 400, height: 400 } });
    await tab.until('window.ready', 'the page');
    for (const [name, css] of PAYLOADS) {
      net.length = 0;
      served.length = 0;
      const { out } = JSON.parse(await tab.eval(`mount(${JSON.stringify(css)})`));
      if (out === null) {
        refused += 1;
        continue;
      }
      kept += 1;
      if (TRAVERSAL.includes(name)) wrong.push([name, 'kept', out]);
      await new Promise((r) => setTimeout(r, 250));
      const m = JSON.parse(await tab.eval('measure()'));
      if (m.urls.length) wrong.push([name, 'url', m.urls]);
      if (m.escapes.length) wrong.push([name, 'hit outside', m.escapes]);
      const shot = decode(Buffer.from((await tab.send('browsingContext.captureScreenshot', { context })).data, 'base64'));
      const sc = shot.w / 400;
      const paint = [];
      for (let y = 0; y < shot.h && paint.length < 3; y += 4) for (let x = 0; x < shot.w && paint.length < 3; x += 4) {
        const cx = x / sc;
        const cy = y / sc;
        if (cx >= 99 && cx < 301 && cy >= 99 && cy < 221) continue;
        const [r, g, b] = shot.px(x, y);
        if (r < 250 || g < 250 || b < 250) paint.push([Math.round(cx), Math.round(cy)]);
      }
      if (paint.length) wrong.push([name, 'paint outside', paint]);
      // Every request after the mount: the browser's and the server's view.
      for (const u of net) {
        const url = new URL(u);
        if (url.origin === server.origin && harness(url.pathname)) continue;
        if (url.origin !== server.origin || !OWN.test(url.pathname) || url.search || url.hash) wrong.push([name, 'request', u]);
      }
      for (const u of served) {
        const path = new URL(u, server.origin).pathname;
        if (!harness(path) && !OWN.test(path)) wrong.push([name, 'served', u]);
      }
    }
  } finally {
    close();
    server.close();
  }
  assert.deepEqual(wrong, [], JSON.stringify(wrong.slice(0, 10), null, 1));
  // The judge judged both ways: some kept (and so mounted), most refused.
  assert.ok(kept > 50 && refused > 300, `${kept} kept, ${refused} refused`);
});
