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

/// What CSS a view's stylesheet may never hold, looked for in lower case:
/// what loads, imports, binds or runs. An escape (`\`) is refused
/// everywhere but in a rule's selector, where Tailwind writes them
/// (`.text-\[10px\]`) and nothing can load: in a declaration or an
/// at-rule it could spell any of these. The sheet is set as text, never
/// parsed as markup; `</style` is refused all the same.
const SHEET_REFUSED = /@import|@font-face|@namespace|@document|expression\(|javascript:|-moz-binding|behaviou?r\s*:|image-set\(|image\(|cross-fade\(|element\(|paint\(|src\(|<\/style/i;
const SHEET_BLOCKS = ['media', 'supports', 'layer', 'container'];
const SHEET_VERBATIM = ['keyframes', '-webkit-keyframes', 'property'];

/// A view's stylesheet (an island's, studio#445) as its shadow root holds
/// it, or null when it is refused whole: no comment (taken out), no
/// `@import`, font, namespace or anything that loads, no escape but in a
/// selector; a `url()` only under `urlPrefix` (the extension's own node
/// path), quoted again; at-rules only `@media`, `@supports`, `@layer`,
/// `@container`, `@keyframes`, `@property`; nested rules (`&:hover{…}`)
/// held to the same; a top-level `:host` rule keeps only its custom
/// properties (`--x`), so nothing can change the box's size or place.
/// The box's own contract (`VIEW_BOX_STYLE`, inline and `!important`)
/// holds over any sheet: `contain: layout paint style` keeps even a
/// `position: fixed` inside it.
export function cleanSheet(css, urlPrefix = null) {
  let text = String(css).replace(/\/\*[\s\S]*?\*\//g, '');
  if (text.includes('/*') || SHEET_REFUSED.test(text)) return null;
  const urls = (text.match(/url\(/gi) ?? []).length;
  let seen = 0;
  text = text.replace(/url\(\s*(["']?)([^"')\s]*)\1\s*\)/gi, (_, _q, u) => {
    seen += 1;
    const ok = !!urlPrefix && u.startsWith(urlPrefix) && !u.includes('..') && /^[A-Za-z0-9._~\/%-]+$/.test(u);
    return ok ? `url("${u}")` : 'none';
  });
  if (seen !== urls) return null;
  return sheetBlock(text, false);
}

/// Declarations (in a rule's body): no escape, no at-rule.
const declsOk = (d) => !d.includes('\\') && !d.includes('@');

/// `text` as rules (`inRule` false: a sheet or an at-rule's body) or as a
/// rule's body (declarations and nested rules); null when anything in it
/// is refused.
function sheetBlock(text, inRule) {
  let out = '';
  let i = 0;
  for (;;) {
    const open = text.indexOf('{', i);
    const seg = text.slice(i, open < 0 ? text.length : open);
    if (seg.includes('}')) return null;
    const semi = seg.lastIndexOf(';');
    const lead = semi >= 0 ? seg.slice(0, semi) : '';
    const head = (semi >= 0 ? seg.slice(semi + 1) : seg).trim();
    if (lead.trim()) {
      if (inRule) {
        if (!declsOk(lead)) return null;
        out += `${lead.trim()};`;
      } else {
        // Between rules, only `@layer a, b;`.
        for (const stmt of lead.split(';').map((x) => x.trim()).filter(Boolean)) {
          if (!/^@layer\s+[\w\s,.-]+$/i.test(stmt)) return null;
          out += `${stmt};`;
        }
      }
    }
    if (open < 0) {
      if (!head) return out;
      // A rule's last declaration, without its `;`.
      if (inRule && declsOk(head)) return out + head;
      return null;
    }
    let depth = 1;
    let j = open + 1;
    for (; j < text.length && depth; j++) {
      if (text[j] === '{') depth += 1;
      else if (text[j] === '}') depth -= 1;
    }
    if (depth || !head) return null;
    const body = text.slice(open + 1, j - 1);
    if (head.startsWith('@')) {
      if (head.includes('\\')) return null;
      const name = (/^@([\w-]+)/.exec(head)?.[1] ?? '').toLowerCase();
      if (SHEET_BLOCKS.includes(name)) {
        const inner = sheetBlock(body, inRule);
        if (inner === null) return null;
        out += `${head}{${inner}}`;
      } else if (SHEET_VERBATIM.includes(name) && !inRule) {
        if (body.includes('@') || body.includes('\\')) return null;
        out += `${head}{${body}}`;
      } else return null;
    } else if (!inRule && /:host/i.test(head)) {
      const keep = body.split(';').map((d) => d.trim()).filter((d) => d.startsWith('--') && !d.includes('{') && !d.includes('\\'));
      if (keep.length) out += `${head}{${keep.join(';')}}`;
    } else {
      const inner = sheetBlock(body, true);
      if (inner === null) return null;
      out += `${head}{${inner}}`;
    }
    i = j;
  }
}
