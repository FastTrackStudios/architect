// A view's edits applied to the page's DOM: the remote-view wire's stack
// machine (features/extensions/src/wire.rs `Edit`, in the world's jco
// shape `{tag, val}`), a port of features/extensions/src/web.rs `Pane`,
// the applier Architect's own web app draws views with.
//
// What an edit may do is held to the share pages' rules (sanitize.js):
// a view draws inside a box of its own under the embedder's element and
// never reaches outside it; elements and attributes outside the
// allow-lists are left out; no `on…` handler and no `dangerous_inner_html`
// (the frame fails, as `Pane` fails it); no URL but a safe one; no CSS
// that loads, binds or positions itself over the page. Events on its
// elements go back to it as the world's `view-event`s.
import { attrRule, elementRule, styleProperty, SVG_NS, VIEW_BOX_STYLE } from './sanitize.js';

/// The attribute an element a view listens on carries its id in (as
/// Dioxus's own renderers, and `Pane`).
const DIOXUS_ID = 'data-dioxus-id';

/// Each node's rule: `{ svg, tag, sealed }`; `sealed` an element whose
/// attributes are all left out (unwrapped or dropped).
const meta = new WeakMap();

export class ViewError extends Error {}

const fail = (why) => {
  throw new ViewError(why);
};

/// One mounted view over `element`, its events to `onEvent`.
export class Pane {
  constructor(element, onEvent) {
    this.element = element;
    this.doc = element.ownerDocument;
    this.onEvent = onEvent;
    this.listening = new Map();
    this.reset();
  }

  /// Empties it, for a first render: a fresh box under its element.
  reset() {
    for (const [name, f] of this.listening) this.root?.removeEventListener(name, f);
    this.listening.clear();
    this.element.replaceChildren();
    this.root = this.doc.createElement('div');
    this.root.setAttribute('class', 'architect-view');
    setStyle(this.root, VIEW_BOX_STYLE);
    this.element.append(this.root);
    meta.set(this.root, { svg: false, tag: 'div', sealed: false });
    this.nodes = new Map([[0, this.root]]);
    this.stack = [];
  }

  /// Applies a frame's edits, in order; the first it cannot apply throws.
  apply(edits) {
    for (const e of edits) this.edit(e);
  }

  /// Shows why it stopped, in place of the view.
  failed(why) {
    this.reset();
    const p = this.doc.createElement('p');
    p.setAttribute('class', 'architect-view-stopped');
    p.textContent = `It stopped: ${why}`;
    this.root.append(p);
  }

  top() {
    if (!this.stack.length) fail('an edit on an empty stack');
    return this.stack[this.stack.length - 1];
  }

  take(m) {
    if (m > this.stack.length) fail('fewer nodes on the stack than an edit takes');
    return this.stack.splice(this.stack.length - m, m).map((s) => s.node);
  }

  /// Under the view's box, or made by it and not in the page yet.
  inside(node) {
    return this.root.contains(node) || !node.isConnected;
  }

  parentOf(node) {
    const parent = node.parentNode;
    if (!parent) fail('a node with no parent');
    if (!this.inside(parent) || parent === this.element) fail('an edit reached outside the view');
    return parent;
  }

  create(tag, namespace) {
    const svg = namespace === SVG_NS;
    const known = namespace == null || svg;
    const [rule, name] = known ? elementRule(tag, svg) : ['unwrap'];
    let el;
    if (rule === 'keep') el = svg ? this.doc.createElementNS(SVG_NS, name) : this.doc.createElement(name);
    else {
      el = svg ? this.doc.createElementNS(SVG_NS, 'g') : this.doc.createElement('span');
      if (rule === 'drop') el.setAttribute(svg ? 'display' : 'hidden', svg ? 'none' : '');
    }
    if (rule === 'keep' && name === 'button') el.setAttribute('type', 'button');
    meta.set(el, { svg, tag: rule === 'keep' ? name : String(tag), sealed: rule !== 'keep' });
    return el;
  }

  edit({ tag, val }) {
    switch (tag) {
      case 'push-id': {
        const node = this.nodes.get(Number(val));
        if (!node) fail(`no node ${val}`);
        this.stack.push({ node, id: Number(val) });
        break;
      }
      case 'set-id': {
        const top = this.top();
        const id = Number(val);
        if (id !== 0 && top.node === this.root) fail('an edit reached outside the view');
        this.nodes.set(id, top.node);
        top.id = id;
        break;
      }
      case 'child': {
        const top = this.top();
        const child = top.node.childNodes[Number(val)];
        if (!child) fail('a child that is not there');
        this.stack[this.stack.length - 1] = { node: child, id: null };
        break;
      }
      case 'pop':
        if (!this.stack.pop()) fail('an edit on an empty stack');
        break;
      case 'create-element':
        this.stack.push({ node: this.create(val.tag, val.namespace ?? null), id: null });
        break;
      case 'create-text':
        this.stack.push({ node: this.doc.createTextNode(String(val ?? '')), id: null });
        break;
      case 'clone': {
        const top = this.top();
        const copy = top.node.cloneNode(true);
        copyMeta(top.node, copy);
        this.stack[this.stack.length - 1] = { node: copy, id: null };
        break;
      }
      case 'append-children': {
        const kids = this.take(Number(val));
        const parent = this.top().node;
        if (!this.inside(parent)) fail('an edit reached outside the view');
        parent.append(...kids);
        break;
      }
      case 'replace-with': {
        const kids = this.take(Number(val));
        const anchor = this.stack.pop();
        if (!anchor) fail('an edit on an empty stack');
        if (anchor.node === this.root) fail('an edit reached outside the view');
        const parent = this.parentOf(anchor.node);
        for (const k of kids) parent.insertBefore(k, anchor.node);
        parent.removeChild(anchor.node);
        break;
      }
      case 'insert-after':
      case 'insert-before': {
        const kids = this.take(Number(val));
        const anchor = this.top().node;
        if (anchor === this.root) fail('an edit reached outside the view');
        const parent = this.parentOf(anchor);
        const next = tag === 'insert-after' ? anchor.nextSibling : anchor;
        for (const k of kids) parent.insertBefore(k, next);
        break;
      }
      case 'set-attribute':
        this.setAttribute(val.name, val.namespace ?? null, val.value);
        break;
      case 'set-text': {
        const node = this.top().node;
        if (node === this.root) fail('an edit reached outside the view');
        node.textContent = String(val ?? '');
        break;
      }
      case 'listen': {
        const top = this.top();
        if (top.id == null) fail('a handler on a node with no element id');
        if (top.node.nodeType === 1) top.node.setAttribute(DIOXUS_ID, String(top.id));
        this.listen(String(val));
        break;
      }
      case 'unlisten':
        break;
      case 'remove': {
        const top = this.stack.pop();
        if (!top) fail('an edit on an empty stack');
        if (top.node === this.root) fail('an edit reached outside the view');
        for (const [id, n] of this.nodes) if (top.node.contains(n)) this.nodes.delete(id);
        top.node.parentNode?.removeChild(top.node);
        break;
      }
      default:
        fail(`no edit ${tag}`);
    }
  }

  setAttribute(name, namespace, value) {
    const el = this.top().node;
    if (el.nodeType !== 1) return;
    if (name === 'dangerous_inner_html' || /^on/i.test(name)) fail(`a view may not set ${name}`);
    if (el === this.root) fail('an edit reached outside the view');
    const m = meta.get(el) ?? { svg: false, tag: el.localName, sealed: true };
    if (m.sealed) return;
    let text;
    switch (value?.tag) {
      case 'text':
        text = String(value.val);
        break;
      case 'float':
      case 'int':
        text = String(Number(value.val));
        break;
      case 'bool':
        text = value.val === false && name === 'checked' ? null : String(value.val);
        break;
      default:
        text = null;
    }
    if (namespace === 'style') {
      if (text === null) el.style.removeProperty(name);
      else {
        const kept = styleProperty(name, text);
        if (kept !== null) el.style.setProperty(name, kept);
      }
      return;
    }
    if (namespace != null) return;
    if (text === null) {
      el.removeAttribute(name);
      if (name === 'checked' && 'checked' in el) el.checked = false;
      return;
    }
    const kept = attrRule(m.tag, name, text, m.svg, true);
    if (kept === null) return;
    // A field's value and a box's tick are what it shows, not only its
    // attribute (which stops counting once it is typed in).
    if (name === 'value' && 'value' in el) el.value = kept;
    if (name === 'checked' && 'checked' in el) el.checked = kept !== 'false';
    if (m.tag === 'button' && name === 'type') return;
    if (name.toLowerCase() === 'style') {
      // Through the CSSOM, not the attribute: a page whose CSP has no
      // 'unsafe-inline' for styles still lets these through.
      el.removeAttribute('style');
      setStyle(el, kept);
      return;
    }
    el.setAttribute(name, kept);
  }

  /// Events `name` under its box, to the component: from the nearest
  /// element with an id.
  listen(name) {
    if (this.listening.has(name)) return;
    if (!/^[a-z][a-z0-9-]*$/.test(name)) fail(`no event ${name}`);
    const f = (e) => {
      let at = e.target?.nodeType === 1 ? e.target : e.target?.parentElement;
      while (at) {
        const id = at.getAttribute?.(DIOXUS_ID);
        if (id != null && /^\d+$/.test(id)) {
          this.onEvent(viewEvent(Number(id), e));
          return;
        }
        if (at === this.root) return;
        at = at.parentElement;
      }
    };
    this.root.addEventListener(name, f);
    this.listening.set(name, f);
  }
}

/// `declarations` (already checked) onto `el`'s style, one property at a
/// time.
function setStyle(el, declarations) {
  for (const d of declarations.split(';')) {
    const colon = d.indexOf(':');
    if (colon < 0) continue;
    const name = d.slice(0, colon).trim();
    let value = d.slice(colon + 1).trim();
    const important = /!\s*important\s*$/i.test(value);
    if (important) value = value.replace(/!\s*important\s*$/i, '').trim();
    el.style.setProperty(name, value, important ? 'important' : '');
  }
}

function copyMeta(from, to) {
  const m = meta.get(from);
  if (m) meta.set(to, m);
  for (let i = 0; i < from.childNodes.length; i++) copyMeta(from.childNodes[i], to.childNodes[i]);
}

const mods = (e) => ({ alt: !!e.altKey, ctrl: !!e.ctrlKey, meta: !!e.metaKey, shift: !!e.shiftKey });

/// A DOM event as the world's `view-event`.
export function viewEvent(element, e) {
  let data;
  if (typeof e.clientX === 'number') {
    data = {
      tag: 'mouse',
      val: {
        clientX: e.clientX, clientY: e.clientY, pageX: e.pageX ?? 0, pageY: e.pageY ?? 0,
        screenX: e.screenX ?? 0, screenY: e.screenY ?? 0, elementX: e.offsetX ?? 0, elementY: e.offsetY ?? 0,
        button: e.button ?? 0, buttons: e.buttons ?? 0, mods: mods(e),
      },
    };
  } else if (typeof e.key === 'string') {
    data = {
      tag: 'key',
      val: { key: e.key, code: e.code ?? '', location: e.location ?? 0, repeat: !!e.repeat, composing: !!e.isComposing, mods: mods(e) },
    };
  } else if (e.type === 'input' || e.type === 'change') {
    data = { tag: 'form', val: String(e.target?.value ?? '') };
  } else data = { tag: 'focus' };
  return { element, name: e.type, bubbles: !!e.bubbles, data };
}
