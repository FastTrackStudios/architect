// view.js in a real DOM (headless Firefox): frames a hostile view could
// send, and what the page ends up with. The rules are sanitize.js's
// (test/sanitize.test.mjs); this checks the applier holds them.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { firefox, open, serve } from './firefox.mjs';

const skip = firefox() ? false : 'needs a Firefox (FIREFOX=…)';
const root = fileURLToPath(new URL('..', import.meta.url));

test('a view cannot run script, set handlers, load from elsewhere or reach outside its box', { skip, timeout: 60000 }, async () => {
  const server = await serve(root);
  const { tab, close } = await open();
  try {
    await tab.go(`${server.origin}/test/view.html`);
    const r = JSON.parse(await tab.until('window.results && JSON.stringify(window.results)', 'the frames'));
    assert.equal(r['a safe tree'], 'applied', r['a safe tree']);
    assert.equal(r['a handler'], 'failed: a view may not set onclick');
    assert.equal(r['the box itself'], 'failed: an edit reached outside the view');
    assert.equal(r['a node it has not got'], 'failed: no node 99');
    assert.equal(r['removing the box'], 'failed: an edit reached outside the view');
    assert.equal(r['a sibling of the box'], 'failed: an edit reached outside the view');
    assert.equal(r.pwned, false);
    assert.equal(r.banner, "the embedder's own");
    assert.equal(r.mountChildren, 1, 'one box under the mount');
    assert.equal(r.scripts, 0);
    assert.equal(r.iframes, 0);
    assert.equal(r.foreignObjects, 0);
    assert.equal(r.linkHref, null, 'no javascript: link');
    assert.equal(r.img, null, 'no image from another site');
    assert.equal(r.rectFill, 'url(#g)');
    assert.equal(r.buttonType, 'button', 'nothing submits');
    assert.equal(r.cardPosition, 'static', 'not over the page');
    assert.doesNotMatch(r.cardStyle ?? '', /fixed|url\(|absolute/);
    assert.match(r.cardStyle ?? '', /color: red/);
    assert.match(r.cardStyle ?? '', /width: 50%/);
    assert.doesNotMatch(r.idDiv, /site-header|data-act/);
    assert.match(r.idDiv, /data-at="1"/);
    assert.equal(r.events.length, 1);
    assert.equal(r.events[0].element, 1);
    assert.equal(r.events[0].name, 'click');
    assert.equal(r.events[0].data.tag, 'mouse');
  } finally {
    close();
    server.close();
  }
});
