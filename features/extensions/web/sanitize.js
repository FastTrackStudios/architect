// What an extension's view may put into an embedder's page: the share
// pages' allow-lists (features/sites/src/sanitize.rs `element_rule`,
// `attr_rule`, `css_inert`), applied to the remote-view edit stream one
// element and one attribute at a time, instead of to markup.
//
// Stricter than share pages where a live view in someone else's page
// needs it:
// - **No network** but what the embedder allows: an image or media URL
//   only when it starts with one of `mountView`'s `urls` (none by
//   default), or is a `data:` raster image. Never a relative URL (it would
//   be a request to the embedder's own origin, with its cookies). A link
//   (`<a href>`) only to `https:`, `mailto:` or a fragment of the view's
//   own.
// - **Fields** a static share page drops (`input`, `textarea`, `select`,
//   `option`, `label`) are kept, since a mounted view is interactive, with
//   only what they show: no `name`, `form` or `autocomplete` (forced
//   off), input types from a short list (never `hidden`, `password` or
//   `file`). No `<form>` comes through and a `<button>` is always
//   `type="button"`; the view is in a shadow root, so its fields belong
//   to no form of the embedder's.
// - **Ids are the view's own**: each id, and every reference to one
//   (`for`, the ARIA id lists, `href="#…"`, `url(#…)`), gets the pane's
//   prefix, so nothing the view names reaches an element of the page's.
// The cases are test/sanitize.test.mjs, ported from sanitize.rs's tests.

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
/// A view's host's contract (`view.js`), inline and `!important`: shown,
/// clipped, contained, in place and at its size, drawing nothing outside
/// its box.
export const VIEW_HOST_STYLE = [
  'display:block', 'overflow:hidden', 'contain:layout paint style', 'position:relative', 'isolation:isolate',
  'transform:none', 'translate:none', 'scale:none', 'rotate:none', 'zoom:1', 'margin:0',
  'box-shadow:none', 'outline:none', 'filter:none',
].map((d) => `${d}!important`).join(';');

/// The sheet's motion turned off when the reader asked for less of it,
/// from a layer (whose `!important` beats an unlayered one).
export const VIEW_MOTION_GUARD =
  '@layer architect-guard{@media (prefers-reduced-motion: reduce){*,*::before,*::after{animation:none!important;transition:none!important}}}';

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

/// `value` as an absolute URL, or null: it must parse with no base, its
/// scheme followed by `//`, no userinfo, no backslash, no whitespace or
/// control, and the parser must not have rewritten it (no `..` or `%2e`
/// resolved, no case or IDN changed, no trailing dot added), so what is
/// checked is exactly what the browser will request. A browser reads
/// `https:host/x` RELATIVE to the page: that is never absolute here.
export function absolute(value, schemes) {
  const v = String(value);
  // eslint-disable-next-line no-control-regex
  if (/[\s\\\u0000-\u001f\u007f]/.test(v)) return null;
  const m = /^([a-z][a-z0-9+.-]*):\/\//.exec(v);
  if (!m || !schemes.includes(m[1])) return null;
  let u;
  try {
    u = new URL(v);
  } catch {
    return null;
  }
  if (u.username || u.password || u.href !== v || !u.hostname || u.hostname.endsWith('.')) return null;
  return u;
}

/// An allow-list entry as `{ origin, path }` (its path ending at a `/`).
function entry(a) {
  const u = absolute(a, ['https']);
  if (!u) return null;
  const path = u.pathname.endsWith('/') ? u.pathname : `${u.pathname}/`;
  return { origin: u.origin, path, exact: u.href };
}

/// The URL an image or media element may load from `value`, or null: a
/// `data:` raster image; or an absolute `https:` URL whose origin is an
/// allowed entry's exactly and whose path is under the entry's (on a `/`
/// boundary), with no query or fragment tricks needed; or a `blob:` URL
/// the embedder listed exactly. What comes back is the parsed URL, the
/// one to set.
export function loadUrl(value, used, allow = []) {
  const v = String(value);
  if (used === 'image' && urlOk(v, 'image') && squeeze(v).startsWith('data:')) return v;
  if (v.startsWith('blob:')) return allow.includes(v) ? v : null;
  const u = absolute(v, ['https']);
  if (!u) return null;
  for (const a of allow) {
    const e = entry(a);
    if (e && u.origin === e.origin && (u.href === e.exact || u.pathname.startsWith(e.path))) return u.href;
  }
  return null;
}

/// Whether `url` may be loaded as an image or media (`loadUrl`).
export function loadOk(url, used, allow = []) {
  return loadUrl(url, used, allow) !== null;
}

/// A link's href: an absolute `https:` URL (parsed, as above) or
/// `mailto:`; or null.
export function linkUrl(value) {
  const v = String(value);
  const u = absolute(v, ['https']);
  if (u) return u.href;
  // eslint-disable-next-line no-control-regex
  if (/^mailto:[^\s\\\u0000-\u001f\u007f]+$/.test(v)) return v;
  return null;
}

/// Whether `url` may be used as a link, an image or media (`url_ok`).
export function urlOk(url, used) {
  // Set with `setAttribute`, a value is not HTML-decoded: what is checked
  // is what the browser reads.
  url = squeeze(url);
  if (url.startsWith('#')) return used === 'link';
  const end = url.search(/[:/?#]/);
  // No scheme: relative to the embedder's page, its origin: never.
  if (end < 0 || url[end] !== ':') return false;
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

/// The input types a view may make: none that a browser fills from what
/// it remembers, none hidden, none that reads a file.
const INPUT_TYPES = ['text', 'search', 'number', 'range', 'checkbox', 'radio', 'color', 'date', 'time'];

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
    return ['value', 'checked', 'placeholder', 'min', 'max', 'step', 'readonly', 'maxlength', 'minlength', 'size', 'spellcheck'].includes(lower);
  }
  if (t === 'textarea') return ['value', 'placeholder', 'rows', 'cols', 'readonly', 'maxlength', 'wrap', 'spellcheck'].includes(lower);
  if (t === 'select') return ['multiple', 'size', 'value'].includes(lower);
  if (t === 'option') return ['value', 'selected', 'label'].includes(lower);
  if (t === 'optgroup') return lower === 'label';
  if (t === 'progress' || t === 'meter') return ['value', 'max', 'min', 'low', 'high', 'optimum'].includes(lower);
  if (lower === 'tabindex') return /^-?\d+$/.test(value.trim());
  return false;
}

const ID_REFS = [
  'aria-labelledby', 'aria-describedby', 'aria-controls', 'aria-owns', 'aria-activedescendant', 'aria-details',
  'aria-errormessage', 'aria-flowto',
];

const ID_TOKEN = /^[A-Za-z][A-Za-z0-9_.:-]{0,127}$/;

/// `#id` with the pane's prefix, or null.
function fragment(value, prefix) {
  const v = String(value).trim();
  if (!v.startsWith('#') || !ID_TOKEN.test(v.slice(1))) return null;
  return `#${prefix}${v.slice(1)}`;
}

/// Attribute `name` = `value` on element `tag`: the value it keeps, or
/// `null` when it is left out (`attr_rule`). `view`: in a mounted view,
/// `{ prefix, urls }`: the pane's id prefix and the URLs it may load.
export function attrRule(tag, name, value, inSvg, live = true, view = { prefix: '', urls: [] }) {
  if (!isName(name)) return null;
  const lower = name.toLowerCase();
  const prefix = view.prefix ?? '';
  value = [...String(value)].filter((c) => !isControl(c) || c === '\t' || c === '\n' || c === '\r').join('');
  switch (lower) {
    case 'style':
      return cleanStyle(value);
    case 'id':
      return value && ID_TOKEN.test(value) && !reservedId(value) ? `${prefix}${value}` : null;
    case 'for':
      return String(tag).toLowerCase() === 'label' && ID_TOKEN.test(value) ? `${prefix}${value}` : null;
    case 'href':
    case 'xlink:href': {
      const t = String(tag);
      if (t === 'use') return fragment(value, prefix);
      if (t === 'a') return value.trim().startsWith('#') ? fragment(value, prefix) : linkUrl(value);
      if (t === 'image') return loadUrl(value, 'image', view.urls);
      return null;
    }
    case 'src': {
      const t = String(tag).toLowerCase();
      return t === 'img' ? loadUrl(value, 'image', view.urls) : ['audio', 'video', 'source'].includes(t) ? loadUrl(value, 'media', view.urls) : null;
    }
    case 'xmlns':
      return inSvg && value === SVG_NS ? value : null;
    case 'xmlns:xlink':
      return inSvg && value === XLINK_NS ? value : null;
  }
  if (ID_REFS.includes(lower)) {
    const ids = value.split(/\s+/).filter(Boolean);
    return ids.length && ids.every((i) => ID_TOKEN.test(i)) ? ids.map((i) => `${prefix}${i}`).join(' ') : null;
  }
  if (lower.startsWith('aria-') && lower.length > 5) return value;
  if (lower.startsWith('data-')) {
    const rest = lower.slice(5);
    // `data-dioxus-id` is the mount's own: which element an event is on.
    return rest && !rest.startsWith('site-') && rest !== 'act' && rest !== 'dioxus-id' ? value : null;
  }
  if (GLOBAL_ATTRS.includes(lower) || htmlAttr(tag, lower, value, live)) return value;
  if (inSvg && SVG_ATTRS.includes(lower)) {
    if (!presentationOk(value)) return null;
    // Its own drawing's ids, with the pane's prefix.
    return value.replace(/url\(#([A-Za-z0-9._-]+)\)/gi, (_, id) => `url(#${prefix}${id})`);
  }
  return null;
}

// ---- a view's stylesheet ---------------------------------------------------
//
// An island's sheet (studio#445) goes into its views' shadow roots, for any
// embedder of this package. It is never pattern-matched: it is tokenised
// as a browser tokenises CSS (CSS Syntax 3: comments, strings, escapes,
// url tokens, numbers, blocks), parsed into rules, held to an allow-list
// on the DECODED tokens, and written out again from those tokens, so what
// a browser parses is exactly what was checked. Anything the tokeniser or
// the parser does not understand, and anything outside the allow-list,
// refuses the whole sheet (`null`): the island then draws unstyled, or
// the embedder falls back.

/// What a sheet may not select (after unescaping), each a pseudo-class or
/// pseudo-element name: the shadow host and the page above it.
const SHEET_PSEUDO_REFUSED = ['host', 'host-context', 'root', 'part', 'slotted', 'scope', 'deep', 'shadow'];
/// The pseudo-classes taking arguments a selector may use.
const SHEET_PSEUDO_FUNCTIONS = ['not', 'is', 'where', 'has', 'nth-child', 'nth-last-child', 'nth-of-type', 'nth-last-of-type', 'lang', 'dir'];
/// What a selector may not name as an element, or as a class: the page's
/// root and body, the view's own host and box.
const SHEET_TYPES_REFUSED = ['html', 'body', ':root'];
const SHEET_CLASSES_REFUSED = ['architect-view', 'architect-view-host'];
/// The functions a value may call: none that loads, reads an attribute or
/// draws another element.
const SHEET_FUNCTIONS = new Set([
  'calc', 'min', 'max', 'clamp', 'var', 'env', 'round', 'mod', 'rem', 'abs', 'sign',
  'rgb', 'rgba', 'hsl', 'hsla', 'hwb', 'lab', 'lch', 'oklab', 'oklch', 'color', 'color-mix', 'light-dark',
  'linear-gradient', 'radial-gradient', 'conic-gradient', 'repeating-linear-gradient', 'repeating-radial-gradient', 'repeating-conic-gradient',
  'translate', 'translatex', 'translatey', 'translatez', 'translate3d', 'rotate', 'rotatex', 'rotatey', 'rotatez', 'rotate3d',
  'scale', 'scalex', 'scaley', 'scalez', 'scale3d', 'skew', 'skewx', 'skewy', 'matrix', 'matrix3d', 'perspective',
  'cubic-bezier', 'steps', 'linear',
  'blur', 'brightness', 'contrast', 'drop-shadow', 'grayscale', 'hue-rotate', 'invert', 'opacity', 'saturate', 'sepia',
  'repeat', 'minmax', 'fit-content', 'counter', 'counters', 'selector',
]);
/// The at-rules a sheet may hold. `@layer` blocks are flattened (their
/// rules kept in order, unlayered) and `@layer a, b;` dropped.
const SHEET_GROUPS = ['media', 'supports', 'container'];
/// What the view's box contract sets: never `!important` on them.
const SHEET_CONTRACT = /^(position|inset|top|right|bottom|left|width|height|min-width|min-height|max-width|max-height|contain|overflow|overflow-x|overflow-y|isolation|display|z-index|transform|translate|scale|rotate|margin|margin-top|margin-right|margin-bottom|margin-left|inset-inline|inset-block|inset-inline-start|inset-inline-end|inset-block-start|inset-block-end)$/;

/// A sheet's bounds: beyond any of them it is refused whole. An island's
/// own (Tailwind's theme and utilities for its components) is some 12 KB,
/// a few hundred rules, three blocks deep.
export const SHEET_LIMITS = Object.freeze({
  /// Characters of the sheet.
  bytes: 256 * 1024,
  /// Rules and at-rules, nested ones counted.
  rules: 4000,
  /// Blocks inside blocks (`@media{.a{&:hover{…}}}` is three).
  depth: 8,
  /// Brackets and functions inside each other, in a selector or a value
  /// (`:is(:not(.a))` is two).
  parens: 16,
  /// Tokens of one selector list.
  selector: 512,
});
/// Animations end, and soon: no `infinite`, at most this many runs, and
/// no time (a duration or a delay) beyond this many seconds.
const SHEET_ANIMATION_RUNS = 10;
const SHEET_ANIMATION_SECONDS = 30;

class SheetRefused extends Error {}
const refuse_ = (why) => {
  throw new SheetRefused(why);
};

const cssHex = (c) => /^[0-9a-fA-F]$/.test(c);
const cssNameStart = (c) => c !== undefined && (/^[A-Za-z_]$/.test(c) || c.codePointAt(0) >= 0x80);
const cssName = (c) => c !== undefined && (cssNameStart(c) || /^[0-9-]$/.test(c));
const cssDigit = (c) => c !== undefined && /^[0-9]$/.test(c);
const cssWs = (c) => c === ' ' || c === '\t' || c === '\n';

/// The tokens of `css` (CSS Syntax 3), or a throw on anything it does not
/// take: an unclosed comment, string or url, a newline in a string, an
/// escape at the end, `<!--`/`-->`, a NUL.
function tokenize(css) {
  if (String(css).length > SHEET_LIMITS.bytes) refuse_('a sheet too big');
  const s = [...String(css).replace(/\r\n?|\f/g, '\n')];
  if (s.includes('\0')) refuse_('a NUL');
  const out = [];
  let i = 0;
  // How deep in blocks, and in brackets and functions, the tokenizer is
  // (a stray closer is the parser's to refuse).
  let blocks = 0;
  let parens = 0;
  const at = (k) => s[i + k];
  const validEscape = (k = 0) => at(k) === '\\' && at(k + 1) !== undefined && at(k + 1) !== '\n';
  const startsIdent = (k = 0) => {
    const c = at(k);
    if (c === '-') return cssNameStart(at(k + 1)) || at(k + 1) === '-' || validEscape(k + 1);
    return cssNameStart(c) || validEscape(k);
  };
  const escape = () => {
    // At the backslash.
    i += 1;
    const c = s[i];
    if (c === undefined) refuse_('an escape at the end');
    if (cssHex(c)) {
      let hex = '';
      while (hex.length < 6 && cssHex(s[i])) hex += s[i++];
      if (cssWs(s[i])) i += 1;
      const n = parseInt(hex, 16);
      return n === 0 || (n >= 0xd800 && n <= 0xdfff) || n > 0x10ffff ? '�' : String.fromCodePoint(n);
    }
    i += 1;
    return c;
  };
  const name = () => {
    let v = '';
    for (;;) {
      if (cssName(s[i])) v += s[i++];
      else if (validEscape()) v += escape();
      else return v;
    }
  };
  const numberAhead = () => {
    let k = 0;
    if (at(k) === '+' || at(k) === '-') k += 1;
    if (cssDigit(at(k))) return true;
    return at(k) === '.' && cssDigit(at(k + 1));
  };
  const number = () => {
    let raw = '';
    if (s[i] === '+' || s[i] === '-') raw += s[i++];
    while (cssDigit(s[i])) raw += s[i++];
    if (s[i] === '.' && cssDigit(s[i + 1])) {
      raw += s[i++];
      while (cssDigit(s[i])) raw += s[i++];
    }
    if ((s[i] === 'e' || s[i] === 'E') && (cssDigit(s[i + 1]) || ((s[i + 1] === '+' || s[i + 1] === '-') && cssDigit(s[i + 2])))) {
      raw += s[i++];
      if (s[i] === '+' || s[i] === '-') raw += s[i++];
      while (cssDigit(s[i])) raw += s[i++];
    }
    return raw;
  };
  while (i < s.length) {
    const c = s[i];
    if (c === '/' && at(1) === '*') {
      const end = s.indexOf('*', i + 2);
      let j = i + 2;
      for (;;) {
        if (j >= s.length - 1) refuse_('an unclosed comment');
        if (s[j] === '*' && s[j + 1] === '/') break;
        j += 1;
      }
      void end;
      i = j + 2;
      continue;
    }
    if (cssWs(c)) {
      while (cssWs(s[i])) i += 1;
      out.push({ t: 'ws' });
      continue;
    }
    if (c === '"' || c === "'") {
      i += 1;
      let v = '';
      for (;;) {
        const d = s[i];
        if (d === undefined) refuse_('an unclosed string');
        if (d === c) {
          i += 1;
          break;
        }
        if (d === '\n') refuse_('a newline in a string');
        if (d === '\\') {
          if (s[i + 1] === undefined) refuse_('an escape at the end');
          if (s[i + 1] === '\n') {
            i += 2;
            continue;
          }
          v += escape();
          continue;
        }
        v += d;
        i += 1;
      }
      out.push({ t: 'string', v });
      continue;
    }
    if (c === '#') {
      if (cssName(at(1)) || validEscape(1)) {
        i += 1;
        out.push({ t: 'hash', v: name() });
        continue;
      }
      out.push({ t: 'delim', v: '#' });
      i += 1;
      continue;
    }
    if (numberAhead()) {
      const raw = number();
      if (startsIdent()) out.push({ t: 'dimension', raw, unit: name() });
      else if (s[i] === '%') {
        i += 1;
        out.push({ t: 'percentage', raw });
      } else out.push({ t: 'number', raw });
      continue;
    }
    if (c === '<' && at(1) === '!' && at(2) === '-' && at(3) === '-') refuse_('<!--');
    if (c === '-' && at(1) === '-' && at(2) === '>') refuse_('-->');
    if (startsIdent()) {
      const v = name();
      if (s[i] === '(') {
        i += 1;
        if (v.toLowerCase() === 'url') {
          let k = i;
          while (cssWs(s[k])) k += 1;
          if (s[k] === '"' || s[k] === "'") {
            if (++parens > SHEET_LIMITS.parens) refuse_('functions too deep');
            out.push({ t: 'function', v: 'url' });
            continue;
          }
          // An unquoted url token.
          i = k;
          let u = '';
          for (;;) {
            const d = s[i];
            if (d === undefined) refuse_('an unclosed url');
            if (d === ')') {
              i += 1;
              break;
            }
            if (cssWs(d)) {
              while (cssWs(s[i])) i += 1;
              if (s[i] !== ')') refuse_('a bad url');
              continue;
            }
            if (d === '"' || d === "'" || d === '(' || d.codePointAt(0) < 0x20 || d.codePointAt(0) === 0x7f) refuse_('a bad url');
            if (d === '\\') {
              if (!validEscape()) refuse_('a bad url');
              u += escape();
              continue;
            }
            u += d;
            i += 1;
          }
          out.push({ t: 'url', v: u });
          continue;
        }
        if (++parens > SHEET_LIMITS.parens) refuse_('functions too deep');
        out.push({ t: 'function', v });
        continue;
      }
      out.push({ t: 'ident', v });
      continue;
    }
    if (c === '@') {
      if (startsIdent(1)) {
        i += 1;
        out.push({ t: 'at', v: name() });
        continue;
      }
      out.push({ t: 'delim', v: '@' });
      i += 1;
      continue;
    }
    if (c === '\\') {
      if (!validEscape()) refuse_('a bad escape');
      // An ident starting with an escape is handled by startsIdent.
      refuse_('a bad escape');
    }
    const single = { ':': 'colon', ';': 'semicolon', ',': 'comma', '[': '[', ']': ']', '(': '(', ')': ')', '{': '{', '}': '}' };
    if (c === '{' && ++blocks > SHEET_LIMITS.depth) refuse_('blocks too deep');
    if (c === '}') blocks -= 1;
    if ((c === '(' || c === '[') && ++parens > SHEET_LIMITS.parens) refuse_('brackets too deep');
    if (c === ')' || c === ']') parens -= 1;
    if (single[c]) out.push({ t: single[c] });
    else out.push({ t: 'delim', v: c });
    i += 1;
  }
  return out;
}

/// `tokens` from `i` to its matching closer, for an opener at `i`; the
/// index past it.
function matching(tokens, i) {
  const pairs = { '{': '}', '[': ']', '(': ')', function: ')' };
  const stack = [pairs[tokens[i].t]];
  let j = i + 1;
  for (; j < tokens.length && stack.length; j++) {
    const t = tokens[j].t;
    if (pairs[t]) stack.push(pairs[t]);
    else if (t === '}' || t === ']' || t === ')') {
      if (stack.pop() !== t) refuse_('blocks that do not pair');
    }
  }
  if (stack.length) refuse_('an unclosed block');
  return j;
}

/// Rules of a list (`tokens`): `{at, prelude, block}` or `{prelude, block}`
/// (a block's tokens, without its braces; null for an at-rule ending in `;`).
function rules(tokens) {
  const out = [];
  let i = 0;
  while (i < tokens.length) {
    const t = tokens[i];
    if (t.t === 'ws' || t.t === 'semicolon') {
      i += 1;
      continue;
    }
    if (t.t === '}' || t.t === ']' || t.t === ')') refuse_('a stray closer');
    const at = t.t === 'at' ? t.v.toLowerCase() : null;
    const start = at ? i + 1 : i;
    let j = start;
    while (j < tokens.length && tokens[j].t !== '{' && !(at && tokens[j].t === 'semicolon')) {
      if (tokens[j].t === '}' || tokens[j].t === ']' || tokens[j].t === ')') refuse_('a stray closer');
      j = ['(', '[', 'function'].includes(tokens[j].t) ? matching(tokens, j) : j + 1;
    }
    const prelude = tokens.slice(start, j);
    if (j >= tokens.length) {
      if (at) {
        out.push({ at, prelude, block: null });
        break;
      }
      refuse_('a rule with no block');
    }
    if (tokens[j].t === 'semicolon') {
      out.push({ at, prelude, block: null });
      i = j + 1;
      continue;
    }
    const end = matching(tokens, j);
    out.push({ at, prelude, block: tokens.slice(j + 1, end - 1) });
    i = end;
  }
  return out;
}

/// A style rule's body: declarations `{name, value, important}` and nested
/// rules, in order.
function body(tokens) {
  const out = [];
  let i = 0;
  while (i < tokens.length) {
    const t = tokens[i];
    if (t.t === 'ws' || t.t === 'semicolon') {
      i += 1;
      continue;
    }
    if (t.t === 'at') {
      const [r] = rules(tokens.slice(i, endOfItem(tokens, i, true)));
      out.push(r);
      i = endOfItem(tokens, i, true);
      continue;
    }
    const end = endOfItem(tokens, i, false);
    const item = tokens.slice(i, end);
    const block = item.findIndex((x) => x.t === '{');
    if (customProperty(item)) {
      // `--x: {a:b}` is a custom property whose value holds a block, as
      // the browser reads it, never a nested rule: refused (a value holds
      // no block here).
      if (block >= 0) refuse_('a block in a custom property');
      out.push(declaration(item));
    } else if (block >= 0) {
      // A nested rule: its prelude, then its block.
      const close = matching(item, block);
      if (item.slice(close).some((x) => x.t !== 'ws')) refuse_('something after a nested rule');
      out.push({ at: null, prelude: item.slice(0, block), block: item.slice(block + 1, close - 1) });
    } else out.push(declaration(item));
    i = end;
  }
  return out;
}

/// Whether `item` starts as a custom property does (`--name:`).
function customProperty(item) {
  const toks = item.filter((x) => x.t !== 'ws');
  return toks[0]?.t === 'ident' && toks[0].v.startsWith('--') && toks[1]?.t === 'colon';
}

/// Where the item at `i` ends: past its `;`, or past its block.
function endOfItem(tokens, i, at) {
  let j = i;
  while (j < tokens.length && tokens[j].t !== 'semicolon') {
    if (tokens[j].t === '{') {
      const close = matching(tokens, j);
      return close;
    }
    j = ['(', '[', 'function'].includes(tokens[j].t) ? matching(tokens, j) : j + 1;
  }
  void at;
  return j;
}

function declaration(item) {
  const toks = item.filter((x, k) => !(x.t === 'ws' && (k === 0 || k === item.length - 1)));
  if (toks[0]?.t !== 'ident') refuse_('a declaration with no name');
  let k = 1;
  while (toks[k]?.t === 'ws') k += 1;
  if (toks[k]?.t !== 'colon') refuse_('a declaration with no colon');
  let value = toks.slice(k + 1);
  let important = false;
  const trimmed = value.filter((x) => x.t !== 'ws');
  const n = trimmed.length;
  if (n >= 2 && trimmed[n - 2].t === 'delim' && trimmed[n - 2].v === '!' && trimmed[n - 1].t === 'ident' && trimmed[n - 1].v.toLowerCase() === 'important') {
    important = true;
    const cut = value.lastIndexOf(trimmed[n - 2]);
    value = value.slice(0, cut);
  }
  if (value.some((x) => x.t === 'delim' && x.v === '!')) refuse_('a stray !');
  return { name: toks[0].v, value, important };
}

/// Value tokens: what loads, reads or draws another element refused.
function valueOk(value, urlPrefix) {
  for (let i = 0; i < value.length; i++) {
    const t = value[i];
    if (t.t === '{' || t.t === '}' || t.t === 'semicolon' || t.t === 'at') refuse_('a block in a value');
    if (t.t === 'url') {
      // Written back as the path it resolves to, not as the author wrote it.
      t.v = urlFor(t.v, urlPrefix) ?? refuse_('a url');
    } else if (t.t === 'function') {
      const f = t.v.toLowerCase();
      if (f === 'url') {
        const at = value.findIndex((x, k) => k > i && x.t !== 'ws');
        const arg = value[at];
        if (arg?.t !== 'string') refuse_('a url');
        const close = value.findIndex((x, k) => k > at && x.t !== 'ws');
        // `url("…")` and nothing more: no url modifiers.
        if (value[close]?.t !== ')') refuse_('a url');
        arg.v = urlFor(arg.v, urlPrefix) ?? refuse_('a url');
      } else if (!SHEET_FUNCTIONS.has(f)) refuse_(`${f}()`);
    } else if (t.t === 'delim' && t.v === '\\') refuse_('a backslash');
    else if (t.t === '(' && value[i - 1]?.t === 'ident') refuse_('a name and a bracket (a function split by a comment)');
  }
}

/// What an island's file may be called (`file_name_ok` in
/// features/sites/src/island.rs, one segment of it): what a sheet's
/// `url()` may name under its prefix.
const SHEET_FILE_NAME = /^[A-Za-z0-9._-]{1,128}$/;

/// Where relative URLs resolve: the document's base URL, as the browser
/// resolves the sheet's (a shadow root's `<style>` against its document);
/// a stand-in origin where there is no page (the unit tests).
function sheetBase() {
  const href = globalThis.document?.baseURI ?? globalThis.location?.href;
  return typeof href === 'string' && /^https?:/i.test(href) ? href : 'https://view.invalid/';
}

/// `u` as the browser will request it, if that is a file of the
/// extension's own (`prefix` and one file name, on the page's origin):
/// its normalised path, which is what is written back; otherwise null.
///
/// The URL is never judged as text: `%2e%2e`, `.%2E`, a backslash, `/./`,
/// `//host` all mean something to the URL parser that a prefix match does
/// not see. It is resolved as the browser resolves it, then held to the
/// same origin and a path of exactly `prefix` + one file name. A `%`, a
/// backslash, whitespace, a query, a fragment or credentials anywhere in it
/// refuse it outright: an island's files need none of them.
function urlFor(u, prefix) {
  if (!prefix || typeof u !== 'string' || u.length > 2048) return null;
  if (/[%\\?#@\s\u0000-\u001f\u007f]/.test(u)) return null;
  const base = sheetBase();
  let url;
  let want;
  try {
    url = new URL(u, base);
    want = new URL(prefix, base);
  } catch {
    return null;
  }
  if (!/^https?:$/.test(want.protocol) || url.protocol !== want.protocol || url.origin !== want.origin) return null;
  if (url.username || url.password || url.search || url.hash) return null;
  if (!want.pathname.endsWith('/') || !url.pathname.startsWith(want.pathname)) return null;
  const name = url.pathname.slice(want.pathname.length);
  if (!SHEET_FILE_NAME.test(name) || name === '.' || name === '..') return null;
  return want.pathname + name;
}

/// A selector's tokens: nothing of the host, the page's root or body,
/// the view's own box; pseudo-functions only from the list.
function selectorOk(prelude, nested) {
  if (!prelude.some((x) => x.t !== 'ws')) refuse_('an empty selector');
  if (prelude.length > SHEET_LIMITS.selector) refuse_('a selector too long');
  // `&` outside a style rule is the scoping root (the host, in a shadow
  // root): `:scope` by another name, refused as it is. And `--x:` is a
  // custom property, not a selector.
  if (!nested && prelude.some((x) => x.t === 'delim' && x.v === '&')) refuse_('& outside a rule');
  if (customProperty(prelude)) refuse_('a custom property where a rule goes');
  let depth = 0;
  for (let i = 0; i < prelude.length; i++) {
    const t = prelude[i];
    const prev = prelude[i - 1];
    if (t.t === '[') depth += 1;
    else if (t.t === ']') depth -= 1;
    if (depth > 0) {
      if (t.t === 'function' || t.t === 'url') refuse_('a function in an attribute selector');
      continue;
    }
    if (t.t === 'url' || t.t === '{' || t.t === 'at' || t.t === 'semicolon') refuse_('a selector');
    if (t.t === 'colon') continue;
    if ((t.t === 'ident' || t.t === 'function') && prev?.t === 'colon') {
      const n = t.v.toLowerCase();
      if (SHEET_PSEUDO_REFUSED.includes(n)) refuse_(`:${n}`);
      if (t.t === 'function' && !SHEET_PSEUDO_FUNCTIONS.includes(n)) refuse_(`:${n}()`);
      continue;
    }
    if (t.t === 'function') refuse_(`${t.v}() in a selector`);
    if (t.t === 'ident') {
      const n = t.v.toLowerCase();
      const cls = prev?.t === 'delim' && prev.v === '.';
      if (cls && SHEET_CLASSES_REFUSED.includes(n)) refuse_(`.${n}`);
      if (!cls && SHEET_TYPES_REFUSED.includes(n)) refuse_(n);
    }
  }
}

/// Whether a selector list is only `:root` and `:host` (no argument), or
/// `.architect-view` (where a theme is written).
function themeOnly(prelude) {
  const parts = [];
  let cur = [];
  for (const t of prelude) {
    if (t.t === 'comma') {
      parts.push(cur);
      cur = [];
    } else if (t.t !== 'ws') cur.push(t);
  }
  parts.push(cur);
  return parts.every(
    (p) =>
      p.length === 2 &&
      p[1].t === 'ident' &&
      ((p[0].t === 'colon' && ['root', 'host'].includes(p[1].v.toLowerCase())) ||
        // What this writes them as, cleaned again.
        (p[0].t === 'delim' && p[0].v === '.' && p[1].v === 'architect-view')),
  );
}

/// Group preludes (`@media`, `@supports`, `@container`): conditions only.
function conditionOk(prelude) {
  for (const t of prelude) {
    if (t.t === 'url' || t.t === '{' || t.t === 'at' || t.t === 'semicolon' || t.t === 'string') refuse_('a condition');
    if (t.t === 'function' && !SHEET_FUNCTIONS.has(t.v.toLowerCase())) refuse_(`${t.v}() in a condition`);
  }
}

// ---- writing it out again --------------------------------------------------

function ident(v) {
  let out = '';
  const chars = [...v];
  chars.forEach((c, k) => {
    const code = c.codePointAt(0);
    const first = k === 0 || (k === 1 && chars[0] === '-');
    if (code < 0x20 || code === 0x7f) out += `\\${code.toString(16)} `;
    else if (cssName(c) && !(first && cssDigit(c))) out += c;
    else if (c === '-' && k === 0 && chars.length > 1) out += c;
    else if (c === '-' && k > 0) out += c;
    else out += `\\${c}`;
  });
  return out;
}

function string(v) {
  return `"${[...v].map((c) => (c === '"' || c === '\\' ? `\\${c}` : c === '\n' ? '\\a ' : c.codePointAt(0) < 0x20 ? `\\${c.codePointAt(0).toString(16)} ` : c)).join('')}"`;
}

/// Whether `a` then `b`, written side by side, would read back as other
/// tokens (CSS Syntax 3, §9 "Serialization"): `url` and `(` as `url(`,
/// two idents as one, a number and an ident as a dimension. A comment
/// goes between them, as it did in the sheet a comment was dropped from.
function joins(a, b) {
  if (!a || !b) return false;
  const kind = (t) => (t.t === 'delim' ? t.v : t.t);
  const x = kind(a);
  const y = kind(b);
  const word = ['ident', 'function', 'url', '-', 'number', 'percentage', 'dimension'];
  if (x === 'ident') return word.includes(y) || y === '(';
  if (x === 'at' || x === 'hash' || x === 'dimension') return word.includes(y);
  if (x === '#' || x === '-') return word.includes(y);
  if (x === 'number') return ['ident', 'function', 'url', 'number', 'percentage', 'dimension', '%'].includes(y);
  if (x === '@') return ['ident', 'function', 'url', '-'].includes(y);
  if (x === '.' || x === '+') return ['number', 'percentage', 'dimension'].includes(y);
  if (x === '/') return y === '*';
  return false;
}

function write(tokens) {
  return tokens
    .map((t, k) => (joins(tokens[k - 1], t) ? '/**/' : '') + ((t) => {
      switch (t.t) {
        case 'ws': return ' ';
        case 'ident': return ident(t.v);
        case 'function': return `${ident(t.v)}(`;
        case 'at': return `@${ident(t.v)}`;
        case 'hash': return `#${[...t.v].map((c) => (cssName(c) ? c : `\\${c}`)).join('')}`;
        case 'string': return string(t.v);
        case 'url': return `url(${string(t.v)})`;
        case 'number': return t.raw;
        case 'percentage': return `${t.raw}%`;
        case 'dimension': return `${t.raw}${ident(t.unit)}`;
        case 'colon': return ':';
        case 'semicolon': return ';';
        case 'comma': return ',';
        // `<` only ever stands in markup (`</style>`) outside a string.
        case 'delim': return t.v === '\\' || t.v === '<' ? refuse_(t.v) : t.v;
        default: return t.t;
      }
    })(t))
    .join('')
    .replace(/\s+/g, ' ')
    .trim();
}

function sheetRules(list, urlPrefix, nested, count = { rules: 0 }) {
  let out = '';
  for (const r of list) {
    if (++count.rules > SHEET_LIMITS.rules) refuse_('too many rules');
    if (r.at) {
      if (r.at === 'layer') {
        // Flattened: its rules in place, unlayered; `@layer a, b;` dropped.
        if (r.block) out += nested ? ruleBody(body(r.block), urlPrefix, count) : sheetRules(rules(r.block), urlPrefix, false, count);
        continue;
      }
      if (SHEET_GROUPS.includes(r.at)) {
        if (!r.block) refuse_(`@${r.at} with no block`);
        conditionOk(r.prelude);
        out += `@${r.at} ${write(r.prelude)}{${nested ? ruleBody(body(r.block), urlPrefix, count) : sheetRules(rules(r.block), urlPrefix, false, count)}}`;
        continue;
      }
      if (r.at === 'keyframes' && !nested && r.block) {
        const n = r.prelude.filter((x) => x.t !== 'ws');
        if (n.length !== 1 || n[0].t !== 'ident') refuse_('@keyframes with no name');
        let frames = '';
        for (const f of rules(r.block)) {
          if (f.at || !f.block) refuse_('a keyframe');
          for (const t of f.prelude) if (!['ws', 'percentage', 'comma'].includes(t.t) && !(t.t === 'ident' && ['from', 'to'].includes(t.v.toLowerCase()))) refuse_('a keyframe selector');
          frames += `${write(f.prelude)}{${declarations(body(f.block), urlPrefix)}}`;
        }
        out += `@keyframes ${ident(n[0].v)}{${frames}}`;
        continue;
      }
      if (r.at === 'property' && !nested && r.block) {
        const n = r.prelude.filter((x) => x.t !== 'ws');
        if (n.length !== 1 || n[0].t !== 'ident' || !n[0].v.startsWith('--')) refuse_('@property with no custom name');
        const items = body(r.block);
        for (const d of items) if (d.at !== undefined || !['syntax', 'inherits', 'initial-value'].includes(d.name.toLowerCase())) refuse_('@property');
        out += `@property ${ident(n[0].v)}{${declarations(items, urlPrefix)}}`;
        continue;
      }
      refuse_(`@${r.at}`);
    }
    if (!nested && themeOnly(r.prelude)) {
      // Tailwind's theme (`:root, :host { --x: … }`): its custom
      // properties alone, on the view's own box, where they reach every
      // element of the view and nothing else. Anything but a custom
      // property there refuses the sheet.
      const items = body(r.block);
      if (items.some((d) => d.name === undefined || !d.name.startsWith('--'))) refuse_('more than custom properties for :root or :host');
      out += `.architect-view{${declarations(items, urlPrefix)}}`;
      continue;
    }
    selectorOk(r.prelude, nested);
    out += `${write(r.prelude)}{${ruleBody(body(r.block), urlPrefix, count)}}`;
  }
  return out;
}

function ruleBody(items, urlPrefix, count) {
  let out = '';
  const decls = [];
  const flush = () => {
    if (decls.length) out += declarations(decls.splice(0), urlPrefix);
  };
  for (const item of items) {
    if (item.name !== undefined) decls.push(item);
    else {
      flush();
      out += sheetRules([item], urlPrefix, true, count);
    }
  }
  flush();
  return out;
}

function declarations(items, urlPrefix) {
  return items
    .map((d) => {
      if (d.name === undefined) refuse_('a rule where a declaration goes');
      const n = d.name.toLowerCase();
      if (n === 'behavior' || n === 'behaviour' || n === '-moz-binding') refuse_(n);
      valueOk(d.value, urlPrefix);
      const v = write(d.value);
      if (n === 'position' && /^(fixed|sticky|-webkit-sticky)$/i.test(v)) refuse_(`position: ${v}`);
      if (d.important && SHEET_CONTRACT.test(n)) refuse_(`${n} !important`);
      if (/^(animation|animation-iteration-count|animation-duration|animation-delay|transition|transition-duration|transition-delay)$/.test(n)) animationOk(d.value);
      return `${ident(d.name)}:${v}${d.important ? '!important' : ''};`;
    })
    .join('');
}

/// An animation's or a transition's value: it ends, and soon (no
/// `infinite`, at most `SHEET_ANIMATION_RUNS` runs, no time past
/// `SHEET_ANIMATION_SECONDS`). A bare number in `animation` is its run
/// count. A view that wants motion has it; a view cannot keep the page's
/// compositor busy for as long as it is open.
function animationOk(value) {
  for (const t of value) {
    if (t.t === 'ident' && t.v.toLowerCase() === 'infinite') refuse_('an infinite animation');
    if (t.t === 'number' && Number(t.raw) > SHEET_ANIMATION_RUNS) refuse_('an animation run too often');
    if (t.t === 'dimension') {
      const unit = t.unit.toLowerCase();
      const seconds = unit === 's' ? Number(t.raw) : unit === 'ms' ? Number(t.raw) / 1000 : 0;
      if (Math.abs(seconds) > SHEET_ANIMATION_SECONDS) refuse_('an animation too long');
    }
    if (t.t === 'function' && t.v.toLowerCase() === 'var') refuse_('an animation from a variable');
  }
}

/// A view's stylesheet (an island's, studio#445; `mountView`'s `sheet`)
/// as its shadow root may hold it, or `null`, refused whole.
///
/// Tokenised and parsed as CSS (never pattern-matched), every name
/// unescaped before it is checked, written out again from the tokens:
/// - rules only, and `@media`, `@supports`, `@container`, `@keyframes`,
///   `@property` (`@layer` flattened); no `@import`, `@namespace`,
///   `@charset`, `@font-face` or any other at-rule;
/// - selectors with no `:host`, `:host()`, `:host-context()`, `::part`,
///   `::slotted`, `:root`, `:scope`, `html`, `body`, nor the view's own
///   host or box (`.architect-view`, `.architect-view-host`);
/// - values with no function but drawing and arithmetic (no `attr()`,
///   `image()`, `image-set()`, `element()`, `paint()`, `src()`), a `url()`
///   only under `urlPrefix` (the extension's own immutable path);
/// - no `position: fixed` or `sticky`, no `!important` on what the box
///   contract sets; no `&` outside a rule (the host, as `:scope` is);
/// - a `url()` resolved as the browser will and kept only as the path of
///   one of the extension's own files on the page's origin, written back
///   normalised (`urlFor`);
/// - animations and transitions that end (`animationOk`);
/// - within `SHEET_LIMITS` (size, rules, nesting, selector length).
/// The box contract holds over any sheet regardless: `mountView`'s host
/// carries it from outside (`VIEW_HOST_STYLE`), its box from inside, both
/// inline and `!important`, neither selectable by the sheet.
export function cleanSheet(css, urlPrefix = null) {
  try {
    return sheetRules(rules(tokenize(css)), urlPrefix, false);
  } catch {
    return null;
  }
}
