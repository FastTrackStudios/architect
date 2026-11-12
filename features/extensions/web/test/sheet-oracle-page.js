// The page sheet-oracle.test.mjs drives (after the second security review
// of architect#601): a view mounted with a payload sheet as `cleanSheet`
// kept it, and what the browser then made of it.
import { cleanSheet } from '../sanitize.js';
import { Pane } from '../view.js';
import { P } from './sheet-payloads.mjs';

const el = (tag) => ({ tag: 'create-element', val: { tag, namespace: null } });
const attr = (name, value) => ({ tag: 'set-attribute', val: { name, namespace: null, value: { tag: 'text', val: value } } });
const text = (t) => ({ tag: 'create-text', val: t });
const slot = document.getElementById('slot');
const OWN = new RegExp(`^${P.replace(/[.]/g, '\\.')}[A-Za-z0-9._-]{1,128}$`);

window.mount = (css) => {
  const out = cleanSheet(css, P);
  slot.replaceChildren();
  if (out === null) return JSON.stringify({ out: null });
  // The page gives the view its slot: as tall as it may grow.
  const pane = new Pane(slot, () => {}, { sheet: out, limits: { viewHeight: 120 } });
  pane.apply([{ tag: 'push-id', val: 0 },
    el('div'), attr('class', 'card'), attr('data-secret', 'abc'), el('span'), attr('class', 'in'), text('hello'), { tag: 'append-children', val: 1 },
    el('p'), text('two'), { tag: 'append-children', val: 1 },
    { tag: 'append-children', val: 2 }, { tag: 'append-children', val: 1 }, { tag: 'pop' }]);
  return JSON.stringify({ out });
};

/// Every url() the browser holds for the view's rules, resolved, that is
/// not one of the view's own files; hits of the view outside its slot.
window.measure = () => {
  const host = slot.querySelector('.architect-view-host');
  if (!host) return JSON.stringify({ urls: [], escapes: [] });
  const urls = [];
  const look = (v) => {
    for (const m of String(v).matchAll(/url\(\s*(?:"((?:[^"\\]|\\.)*)"|'((?:[^'\\]|\\.)*)'|([^)\s]*))\s*\)/gi)) {
      const raw = (m[1] ?? m[2] ?? m[3] ?? '').replace(/\\(.)/g, '$1');
      const u = new URL(raw, document.baseURI);
      if (u.origin !== location.origin || !OWN.test(u.pathname) || u.search || u.hash) urls.push(raw);
    }
  };
  const walk = (list) => {
    for (const r of list) {
      if (r.style) for (const p of r.style) look(r.style.getPropertyValue(p));
      if (r.cssRules) walk(r.cssRules);
    }
  };
  for (const s of host.shadowRoot.querySelectorAll('style')) walk(s.sheet.cssRules);
  for (const e of host.shadowRoot.querySelectorAll('*')) for (const pseudo of [null, '::before', '::after', '::marker']) {
    const cs = getComputedStyle(e, pseudo);
    for (const p of ['background-image', 'list-style-image', 'mask-image', 'shape-outside', 'border-image-source', 'content', 'cursor']) look(cs.getPropertyValue(p));
  }
  const s = slot.getBoundingClientRect();
  const escapes = [];
  for (let x = 2; x < innerWidth && escapes.length < 5; x += 10) for (let y = 2; y < innerHeight && escapes.length < 5; y += 10) {
    if (x >= s.left && x < s.right && y >= s.top && y < s.bottom) continue;
    const e = document.elementFromPoint(x, y);
    if (e && (e === host || host.contains(e))) escapes.push([x, y]);
  }
  return JSON.stringify({ urls, escapes });
};
window.ready = true;
