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
    assert.equal(r['too many edits'], 'failed: a frame of 10001 edits: past its limit of 10000');
    assert.equal(r.pwned, false);
    assert.equal(r.banner, "the embedder's own");
    assert.ok(r.inShadow, 'drawn in a shadow root');
    assert.equal(r.lightChildren, 1, 'only its host in the page’s tree');
    assert.equal(r.scripts, 0);
    assert.equal(r.iframes, 0);
    assert.equal(r.foreignObjects, 0);
    assert.equal(r.linkHref, null, 'no javascript: link');
    assert.equal(r.buttonType, 'button', 'nothing submits');
    assert.equal(r.cardPosition, 'static', 'not over the page');
    assert.doesNotMatch(r.cardStyle ?? '', /fixed|url\(|absolute/);
    assert.match(r.cardStyle ?? '', /color: red/);
    assert.match(r.cardStyle ?? '', /width: 50%/);
    assert.doesNotMatch(r.idDiv, /site-header|data-act/);
    assert.match(r.idDiv, /data-at="1"/);
    // P2: no request the embedder did not allow.
    assert.equal(r['P2 network'], 'applied');
    assert.equal(r.video, null);
    assert.deepEqual(r.img, [null, null]);
    assert.equal(r.svgImage, null);
    assert.deepEqual(r.loads, [], 'no request to evil, the tracker or the page’s own csrf-get');
    // P3: the embedder's form is as it was; no field to autofill.
    assert.equal(r['P3 fields'], 'applied');
    assert.deepEqual(r.formAfter, r.formBefore);
    for (const f of r.fields) {
      assert.ok(!['hidden', 'password'].includes(f.type), JSON.stringify(f));
      assert.equal(f.name, null);
      assert.equal(f.form, null, 'in no form of the page’s');
      assert.equal(f.autocomplete, 'off');
    }
    // P4: the page's ids and window are its own.
    assert.equal(r['P4 ids'], 'applied');
    assert.equal(r.byIdPw, 'login', 'getElementById("pw") is still the page’s field');
    assert.equal(r.byIdPwType, 'password');
    assert.equal(r.windowConfig, 'undefined', 'no named property on window');
    assert.match(r.labelFor, /^av\d+-pw$/);
    assert.match(r.useHref, /^#av\d+-logo$/);
    assert.equal(r.useRefersToPage, false, 'its <use> draws nothing of the page’s');
    assert.match(r.rectFill, /^url\(#av\d+-g\)$/);
    // The box is capped.
    assert.equal(r.tallBox, 'applied');
    assert.ok(r.hostHeight <= 600, `${r.hostHeight} px tall`);
    assert.equal(r.events.length, 1);
    assert.equal(r.events[0].element, 1);
    assert.equal(r.events[0].name, 'click');
    assert.equal(r.events[0].data.val.screenX, 0, 'not where the screen is');
  } finally {
    close();
    server.close();
  }
});
