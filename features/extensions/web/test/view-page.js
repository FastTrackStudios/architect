// Applies frames a hostile view could send and records what the page
// ended up with (test/view.test.mjs reads `window.results`).
import { Pane } from '../view.js';

const SVG = 'http://www.w3.org/2000/svg';
const el = (tag, namespace = null) => ({ tag: 'create-element', val: { tag, namespace } });
const text = (t) => ({ tag: 'create-text', val: t });
const attr = (name, value, namespace = null) => ({ tag: 'set-attribute', val: { name, namespace, value: { tag: 'text', val: value } } });
const events = [];
const mount = document.getElementById('mount');
const pane = new Pane(mount, (e) => events.push(e));
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
  // The card again: one CSS property at a time.
  { tag: 'set-attribute', val: { name: 'position', namespace: 'style', value: { tag: 'text', val: 'absolute' } } },
  { tag: 'set-attribute', val: { name: 'width', namespace: 'style', value: { tag: 'text', val: '50%' } } },
  { tag: 'append-children', val: 1 },
  { tag: 'pop' },
]);
tryFrame('a handler', [{ tag: 'push-id', val: 1 }, attr('onclick', 'window.pwned=true'), { tag: 'pop' }]);
tryFrame('the box itself', [{ tag: 'push-id', val: 0 }, attr('style', 'position: fixed; inset: 0')]);
tryFrame('a node it has not got', [{ tag: 'push-id', val: 99 }]);
tryFrame('removing the box', [{ tag: 'push-id', val: 0 }, { tag: 'remove' }]);
tryFrame('a sibling of the box', [{ tag: 'push-id', val: 0 }, el('div'), { tag: 'insert-after', val: 1 }]);
document.querySelector('#mount button').click();

const card = mount.querySelector('.card');
Object.assign(results, {
  pwned: window.pwned,
  html: mount.innerHTML,
  banner: document.getElementById('page-banner').textContent,
  cardStyle: card?.getAttribute('style'),
  cardPosition: getComputedStyle(card).position,
  buttonType: mount.querySelector('button')?.getAttribute('type'),
  linkHref: mount.querySelector('a')?.getAttribute('href') ?? null,
  scripts: mount.querySelectorAll('script').length,
  iframes: mount.querySelectorAll('iframe').length,
  foreignObjects: mount.querySelectorAll('foreignObject').length,
  hiddenShown: [...mount.querySelectorAll('[hidden], [display="none"]')].length,
  rectFill: mount.querySelector('rect')?.getAttribute('fill') ?? null,
  img: mount.querySelector('img')?.getAttribute('src') ?? null,
  idDiv: mount.querySelector('[data-at]')?.outerHTML,
  events,
  mountChildren: mount.children.length,
});
window.results = results;
