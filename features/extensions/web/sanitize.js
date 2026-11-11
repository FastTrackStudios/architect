// What an extension's view may put into an embedder's page: the share
// pages' allow-lists (features/sites/src/sanitize.rs `element_rule`,
// `attr_rule`, `css_inert`), applied to the remote-view edit stream one
// element and one attribute at a time, instead of to markup.
//
// The same rules, with one difference a live view needs: a mounted view
// is interactive (its events go back to the extension), so the form
// fields a static share page drops (`input`, `textarea`, `select`,
// `option`, `label`) are kept, with only the attributes they need. No
// `<form>` comes through, so nothing submits, and a `<button>` is always
// `type="button"`. The cases are test/sanitize.test.mjs, ported from
// sanitize.rs's tests.

const SVG_ELEMENTS = [
  'svg', 'g', 'path', 'rect', 'circle', 'ellipse', 'line', 'polyline', 'polygon', 'text', 'tspan', 'textPath',
  'defs', 'use', 'symbol', 'clipPath', 'mask', 'pattern', 'linearGradient', 'radialGradient', 'stop', 'marker',
  'title', 'desc', 'image', 'a',
  // An SVG `<style>` sheet is kept by share pages only inside a block's
  // box, scoped to it; a mounted view has none (dropped below).
];

const HTML_ELEMENTS = [
  'svg', 'div', 'span', 'p', 'br', 'hr', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'ul', 'ol', 'li', 'dl', 'dt', 'dd',
  'table', 'caption', 'colgroup', 'col', 'thead', 'tbody', 'tfoot', 'tr', 'td', 'th', 'a', 'strong', 'em', 'b', 'i',
  'u', 's', 'code', 'pre', 'blockquote', 'figure', 'figcaption', 'img', 'audio', 'video', 'source', 'details',
  'summary', 'small', 'sub', 'sup', 'time', 'button',
];

/// Kept in a live view only: its fields (share pages drop them).
const LIVE_ELEMENTS = ['input', 'textarea', 'select', 'option', 'optgroup', 'label', 'progress', 'meter'];

const DROPPED = [
  'script', 'style', 'title', 'noscript', 'template', 'textarea', 'xmp', 'plaintext', 'listing', 'noembed',
  'noframes', 'iframe', 'frame', 'frameset', 'object', 'embed', 'applet', 'portal', 'fencedframe', 'base', 'meta',
  'link', 'head', 'foreignobject', 'math', 'select', 'option', 'optgroup', 'datalist', 'input', 'canvas', 'animate',
  'set', 'animatetransform', 'animatemotion', 'animatecolor', 'discard', 'mpath', 'handler', 'listener', 'filter',
  'form',
];

const SVG_ATTRS = [
  'viewbox', 'preserveaspectratio', 'version', 'x', 'y', 'x1', 'y1', 'x2', 'y2', 'cx', 'cy', 'r', 'rx', 'ry', 'fx',
  'fy', 'fr', 'dx', 'dy', 'width', 'height', 'd', 'points', 'pathlength', 'transform', 'fill', 'fill-opacity',
  'fill-rule', 'stroke', 'stroke-width', 'stroke-opacity', 'stroke-linecap', 'stroke-linejoin', 'stroke-dasharray',
  'stroke-dashoffset', 'stroke-miterlimit', 'opacity', 'color', 'visibility', 'display', 'overflow', 'vector-effect',
  'shape-rendering', 'text-rendering', 'paint-order', 'clip-path', 'clip-rule', 'mask', 'marker-start',
  'marker-mid', 'marker-end', 'font-family', 'font-size', 'font-weight', 'font-style', 'font-variant',
  'text-anchor', 'text-decoration', 'dominant-baseline', 'alignment-baseline', 'baseline-shift', 'letter-spacing',
  'word-spacing', 'writing-mode', 'rotate', 'textlength', 'lengthadjust', 'startoffset', 'method', 'spacing',
  'side', 'offset', 'stop-color', 'stop-opacity', 'gradientunits', 'gradienttransform', 'spreadmethod',
  'patternunits', 'patterncontentunits', 'patterntransform', 'clippathunits', 'maskunits', 'maskcontentunits',
  'markerwidth', 'markerheight', 'markerunits', 'refx', 'refy', 'orient',
];

const GLOBAL_ATTRS = ['class', 'id', 'title', 'lang', 'dir', 'role', 'style'];
const CSS_BLUNT = ['"', "'", '/*', '\\', 'var(', '--', '@', '<', '&'];
const CSS_REFUSED = [
  'url(', 'image-set(', 'image(', 'src(', 'cross-fade(', 'element(', 'paint(', 'expression(', '-moz-binding',
  'behavior', 'behaviour', 'javascript:',
];
const POSITIONS = ['static', 'relative'];
const RESERVED_IDS = [
  'architect', 'addeventlistener', 'removeeventlistener', 'dispatchevent', 'eventsource', 'matchmedia',
  'getcomputedstyle', 'comments',
];
export const SVG_NS = 'http://www.w3.org/2000/svg';
const XLINK_NS = 'http://www.w3.org/1999/xlink';

/// What box a view is drawn in: it cannot style its way out of it
/// (sanitize.rs `BLOCK_BOX_STYLE`).
export const VIEW_BOX_STYLE =
  'contain:layout paint style!important;overflow:auto!important;position:relative!important;isolation:isolate!important';

const isControl = (c) => {
  const n = c.codePointAt(0);
  return n < 0x20 || (n >= 0x7f && n < 0xa0);
};

/// Whether `name` may be an attribute's or element's name (`is_name`).
export function isName(name) {
  return /^[A-Za-z][A-Za-z0-9_:.-]*$/.test(name);
}

/// What becomes of element `tag`: `['keep', name]`, `['unwrap']` (kept as
/// a plain `span`/`g`, its attributes left out) or `['drop']` (kept out of
/// sight with everything in it). `live`: in a mounted view.
export function elementRule(tag, inSvg, live = true) {
  const list = inSvg ? SVG_ELEMENTS : HTML_ELEMENTS;
  const lower = String(tag).toLowerCase();
  const kept = list.find((n) => n.toLowerCase() === lower);
  if (kept) return ['keep', kept];
  if (live && !inSvg) {
    const field = LIVE_ELEMENTS.find((n) => n === lower);
    if (field) return ['keep', field];
  }
  if (DROPPED.includes(lower)) return ['drop'];
  return ['unwrap'];
}

function squeeze(url) {
  return [...url]
    .filter((c) => !/\s/.test(c) && !isControl(c))
    .map((c) => (c === '\\' ? '/' : c))
    .join('')
    .toLowerCase();
}

const isFragment = (url) => {
  const s = squeeze(url);
  return s.startsWith('#') && s.length > 1;
};

/// Whether `url` may be used as a link, an image or media (`url_ok`).
export function urlOk(url, used) {
  // Set with `setAttribute`, a value is not HTML-decoded: what is checked
  // is what the browser reads.
  url = squeeze(url);
  if (url.startsWith('#')) return used === 'link';
  const end = url.search(/[:/?#]/);
  if (end < 0 || url[end] !== ':') return !url.startsWith('//');
  const scheme = url.slice(0, end);
  const rest = url.slice(end);
  switch (scheme) {
    case 'https':
      return used !== 'image';
    case 'mailto':
      return used === 'link';
    case 'data':
      return (
        used === 'image' &&
        ['png', 'jpeg', 'gif', 'webp'].some((k) => {
          const p = `:image/${k}`;
          return rest.startsWith(p) && /^[;,]/.test(rest.slice(p.length));
        })
      );
    default:
      return false;
  }
}

/// Whether `css` loads, binds or escapes nothing (`css_inert`).
export function cssInert(css) {
  const lower = css.toLowerCase();
  if (CSS_BLUNT.some((b) => lower.includes(b))) return false;
  const squeezed = [...lower].filter((c) => !/\s/.test(c) && !isControl(c)).join('');
  return !CSS_REFUSED.some((b) => squeezed.includes(b)) && positionsOk(squeezed);
}

function positionsOk(squeezed) {
  let at = squeezed.indexOf('position:');
  while (at >= 0) {
    const before = squeezed.slice(0, at);
    const starts = before === '' || before.endsWith(';') || before.endsWith('{');
    const value = squeezed.slice(at + 'position:'.length).split(/[;}!]/)[0];
    if (starts && !POSITIONS.includes(value)) return false;
    at = squeezed.indexOf('position:', at + 1);
  }
  return true;
}

function declarationOk(decl) {
  const colon = decl.indexOf(':');
  if (colon < 0) return false;
  const name = decl.slice(0, colon).trim();
  const value = decl.slice(colon + 1);
  return (
    name !== '' &&
    /^[A-Za-z-]+$/.test(name) &&
    !name.startsWith('--') &&
    value.trim() !== '' &&
    !/[{};]/.test(value) &&
    cssInert(decl)
  );
}

/// A `style` attribute with only its declarations that are inert, or
/// `null` (`clean_style`).
export function cleanStyle(value) {
  if (/["'\\{}]/.test(value) || value.includes('/*')) return null;
  const kept = value
    .split(';')
    .map((d) => d.trim())
    .filter((d) => d && declarationOk(d));
  const out = kept.join('; ');
  return out && cssInert(out) ? out : null;
}

/// One CSS property (the edit stream's `style` namespace), or `null`.
export function styleProperty(name, value) {
  const decl = `${name}: ${value}`;
  return /^[A-Za-z-]+$/.test(name) && !/[;{}]/.test(value) && declarationOk(decl) ? value : null;
}

function presentationOk(value) {
  let left = '';
  let rest = value;
  for (;;) {
    const at = rest.toLowerCase().indexOf('url(#');
    if (at < 0) break;
    const after = rest.slice(at + 5);
    const id = /^[A-Za-z0-9._-]*/.exec(after)[0];
    if (!id || after[id.length] !== ')') return false;
    left += `${rest.slice(0, at)}#`;
    rest = after.slice(id.length + 1);
  }
  return cssInert(left + rest);
}

function reservedId(id) {
  const lower = id.trim().toLowerCase();
  return lower.startsWith('site-') || lower.startsWith('share-') || RESERVED_IDS.includes(lower);
}

const INPUT_TYPES = [
  'text', 'search', 'number', 'range', 'checkbox', 'radio', 'color', 'date', 'time', 'datetime-local', 'month',
  'week', 'email', 'tel', 'url', 'password', 'hidden',
];

function htmlAttr(tag, lower, value, live) {
  const t = String(tag).toLowerCase();
  const pair = `${t} ${lower}`;
  if (
    [
      'img width', 'img height', 'video width', 'video height', 'img alt', 'audio controls', 'audio preload',
      'video controls', 'video preload', 'source type', 'time datetime', 'td colspan', 'td rowspan', 'th colspan',
      'th rowspan', 'col span', 'colgroup span', 'details open',
    ].includes(pair)
  )
    return true;
  if (pair === 'button type') return value.toLowerCase() === 'button';
  if (!live) return false;
  // A live view's fields: what they show and how they are used, never
  // where they send anything.
  if (lower === 'disabled' && ['button', 'input', 'textarea', 'select', 'option', 'optgroup'].includes(t)) return true;
  if (t === 'input') {
    if (lower === 'type') return INPUT_TYPES.includes(value.toLowerCase());
    return ['value', 'checked', 'placeholder', 'min', 'max', 'step', 'readonly', 'maxlength', 'minlength', 'size', 'name', 'autocomplete', 'spellcheck', 'inputmode'].includes(lower);
  }
  if (t === 'textarea') return ['value', 'placeholder', 'rows', 'cols', 'readonly', 'maxlength', 'wrap', 'spellcheck', 'name'].includes(lower);
  if (t === 'select') return ['multiple', 'size', 'name', 'value'].includes(lower);
  if (t === 'option') return ['value', 'selected', 'label'].includes(lower);
  if (t === 'optgroup') return lower === 'label';
  if (t === 'label') return lower === 'for';
  if (t === 'progress' || t === 'meter') return ['value', 'max', 'min', 'low', 'high', 'optimum'].includes(lower);
  if (lower === 'tabindex') return /^-?\d+$/.test(value.trim());
  return false;
}

/// Attribute `name` = `value` on element `tag`: the value it keeps, or
/// `null` when it is left out (`attr_rule`).
export function attrRule(tag, name, value, inSvg, live = true) {
  if (!isName(name)) return null;
  const lower = name.toLowerCase();
  value = [...String(value)].filter((c) => !isControl(c) || c === '\t' || c === '\n' || c === '\r').join('');
  switch (lower) {
    case 'style':
      return cleanStyle(value);
    case 'id':
      return value && !reservedId(value) ? value : null;
    case 'href':
    case 'xlink:href': {
      const t = String(tag);
      const ok = t === 'use' ? isFragment(value) : t === 'a' ? urlOk(value, 'link') : t === 'image' ? urlOk(value, 'image') : false;
      return ok ? value : null;
    }
    case 'src': {
      const t = String(tag).toLowerCase();
      const ok = t === 'img' ? urlOk(value, 'image') : ['audio', 'video', 'source'].includes(t) ? urlOk(value, 'media') : false;
      return ok ? value : null;
    }
    case 'xmlns':
      return inSvg && value === SVG_NS ? value : null;
    case 'xmlns:xlink':
      return inSvg && value === XLINK_NS ? value : null;
  }
  if (lower.startsWith('aria-') && lower.length > 5) return value;
  if (lower.startsWith('data-')) {
    const rest = lower.slice(5);
    // `data-dioxus-id` is the mount's own: which element an event is on.
    return rest && !rest.startsWith('site-') && rest !== 'act' && rest !== 'dioxus-id' ? value : null;
  }
  if (GLOBAL_ATTRS.includes(lower) || htmlAttr(tag, lower, value, live)) return value;
  if (inSvg && SVG_ATTRS.includes(lower)) return presentationOk(value) ? value : null;
  return null;
}
