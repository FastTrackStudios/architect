// sanitize.js: the share pages' rules (features/sites/src/sanitize.rs),
// its tests' cases as element and attribute rules, the form the edit
// stream needs them in.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { attrRule, cleanStyle, cssInert, elementRule, styleProperty, urlOk } from '../sanitize.js';

test('elements: a drawing and basic HTML kept, what runs or hides markup dropped, the rest unwrapped', () => {
  for (const t of ['svg', 'g', 'path', 'rect', 'text', 'linearGradient', 'use', 'a']) assert.equal(elementRule(t, true)[0], 'keep', t);
  assert.deepEqual(elementRule('lineargradient', true), ['keep', 'linearGradient'], 'its case as written out');
  for (const t of ['div', 'span', 'button', 'table', 'img', 'audio', 'details', 'svg']) assert.equal(elementRule(t, false)[0], 'keep', t);
  for (const t of ['script', 'SCRIPT', 'iframe', 'object', 'embed', 'base', 'meta', 'link', 'template', 'noscript', 'form', 'portal', 'frame'])
    assert.deepEqual(elementRule(t, false), ['drop'], t);
  for (const t of ['foreignObject', 'animate', 'set', 'animateTransform', 'script', 'style', 'filter', 'handler'])
    assert.deepEqual(elementRule(t, true), ['drop'], `svg ${t}`);
  assert.deepEqual(elementRule('custom-element', false), ['unwrap']);
  assert.deepEqual(elementRule('marquee', false), ['unwrap']);
  // A live view keeps its fields; a static rule set does not.
  for (const t of ['input', 'textarea', 'select', 'option', 'label']) {
    assert.equal(elementRule(t, false, true)[0], 'keep', t);
    assert.notEqual(elementRule(t, false, false)[0], 'keep', `static ${t}`);
  }
  assert.deepEqual(elementRule('input', true), ['drop'], 'no field inside a drawing');
});

test('attributes: never a handler, a frame or a form action; ids and data-* not the page’s own', () => {
  for (const name of ['onclick', 'ONLOAD', 'onmouseover', 'srcdoc', 'srcset', 'ping', 'action', 'formaction', 'form'])
    assert.equal(attrRule('div', name, 'x', false), null, name);
  assert.equal(attrRule('div', 'class', 'a b', false), 'a b');
  assert.equal(attrRule('div', 'id', 'mine', false), 'mine');
  for (const id of ['site-header', 'share-x', 'comments', 'architect', 'addEventListener']) assert.equal(attrRule('div', 'id', id, false), null, id);
  assert.equal(attrRule('div', 'data-at', '1.5', false), '1.5');
  for (const n of ['data-site-x', 'data-act', 'data-dioxus-id', 'data-']) assert.equal(attrRule('div', n, '1', false), null, n);
  assert.equal(attrRule('div', 'aria-label', 'Add', false), 'Add');
  assert.equal(attrRule('div', 'x"y', '1', false), null, 'not a name');
  assert.equal(attrRule('button', 'type', 'button', false), 'button');
  assert.equal(attrRule('button', 'type', 'submit', false), null, 'nothing submits');
  assert.equal(attrRule('input', 'type', 'checkbox', false), 'checkbox');
  assert.equal(attrRule('input', 'type', 'image', false), null, 'an image input loads a src');
  assert.equal(attrRule('input', 'value', 'typed', false), 'typed');
  assert.equal(attrRule('td', 'colspan', '2', false), '2');
  assert.equal(attrRule('div', 'colspan', '2', false), null, 'only where it belongs');
  assert.equal(attrRule('svg', 'xmlns', 'http://www.w3.org/2000/svg', true), 'http://www.w3.org/2000/svg');
  assert.equal(attrRule('svg', 'xmlns', 'http://evil', true), null);
  assert.equal(attrRule('div', 'title', 'a\u0000b', false), 'ab', 'controls taken out');
});

test('URLs: as a browser reads them; relative, #fragment, https and mailto for a link, no image from another site', () => {
  for (const u of ['/s/abc/joy.mp3', 'page.html', '#bar-2', 'https://example.com', 'mailto:a@b.c']) assert.ok(urlOk(u, 'link'), u);
  for (const u of [
    'javascript:alert(1)',
    'JaVaScRiPt:alert(1)',
    ' java\tscript:alert(1)',
    'java\u0000script:alert(1)',
    'vbscript:x',
    'data:text/html,<script>alert(1)</script>',
    'http://example.com',
    '//evil.com/x',
    '\\\\evil.com/x',
  ])
    assert.ok(!urlOk(u, 'link'), u);
  assert.ok(urlOk('data:image/png;base64,AA', 'image'));
  assert.ok(urlOk('/a.png', 'image'));
  for (const u of ['https://tracker.example/pixel.gif', 'data:image/svg+xml,<svg onload=alert(1)>', '#x', '//evil.com/a.png'])
    assert.ok(!urlOk(u, 'image'), u);
  assert.ok(urlOk('https://cdn.example/a.mp3', 'media'));
  assert.equal(attrRule('a', 'href', 'javascript:alert(1)', false), null);
  assert.equal(attrRule('use', 'href', '#g', true), '#g');
  assert.equal(attrRule('use', 'href', 'https://evil/x.svg#g', true), null, 'a <use> refers only to its own document');
  assert.equal(attrRule('img', 'src', 'https://tracker/p.gif', false), null);
  assert.equal(attrRule('div', 'href', '/x', false), null, 'no href but a link’s');
});

test('CSS: blunt; nothing that loads, binds, escapes or positions itself over the page', () => {
  assert.equal(cleanStyle('color: red; font-size: 12px'), 'color: red; font-size: 12px');
  assert.equal(cleanStyle('color:red; position:fixed; background:url(x.png)'), 'color:red');
  assert.equal(cleanStyle('position: relative'), 'position: relative');
  for (const s of ['position:absolute', 'position: fixed', 'position:sticky', 'background:url(x)', 'width:expression(alert(1))', '-moz-binding:x', 'color:var(--x)', '--x:1'])
    assert.equal(cleanStyle(s), null, s);
  for (const s of ['color:"red"', "content:'x'", 'color:r\\65 d', 'a{color:red}', 'color:red/* */']) assert.equal(cleanStyle(s), null, `dropped whole: ${s}`);
  assert.ok(cssInert('fill:red') && !cssInert('fill:url(#g)') && !cssInert('@import x'));
  // The edit stream's `style` namespace: one property at a time.
  assert.equal(styleProperty('width', '50%'), '50%');
  assert.equal(styleProperty('position', 'fixed'), null);
  assert.equal(styleProperty('position', 'relative'), 'relative');
  assert.equal(styleProperty('background', 'url(https://x/y.png)'), null);
  assert.equal(styleProperty('color', 'red; position: fixed'), null);
  assert.equal(styleProperty('--accent', 'red'), null);
  // A presentation attribute may point at its own drawing only.
  assert.equal(attrRule('rect', 'fill', 'url(#g)', true), 'url(#g)');
  assert.equal(attrRule('rect', 'fill', 'url(https://evil/x)', true), null);
  assert.equal(attrRule('rect', 'fill', 'url(#g) url(x)', true), null);
});
