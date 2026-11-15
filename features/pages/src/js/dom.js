// `import { document } from "architect/dom"`: a DOM in the page's own
// runtime, in the spirit of AMP's worker-dom, so a framework that renders
// to the DOM (Preact, Solid) runs in a page as it does in a browser. It
// is never the window's: the framework builds and mutates this tree, and
// the page runtime (features/pages/src/js/prelude.js) draws it as it
// draws a component's — each change marks the page dirty, and once its
// unit of work is over (the event, the promise jobs it made) the runtime
// asks for the tree, which becomes edits like any page's. An event on the
// pane comes back to the listener's element and goes through this tree
// as a browser's would: captured from the document down, then bubbled
// back up.
//
// A page draws it by exporting its body: `export default document.body`.
// Natively it is a module of the QuickJS loader (features/pages/src/js/
// mod.rs); in a browser, of the page's Web Worker (features/pages/src/
// web/page.js). There is no HTML parser: `innerHTML` only clears.
import { invalidate } from "architect";

const HTML = "http://www.w3.org/1999/xhtml";
const TEXT = "#text";

// ── nodes ────────────────────────────────────────────────────────────
let nextId = 1;

class Node {
  constructor(doc, type, name) {
    this.$id = nextId++;
    this.nodeType = type;
    this.nodeName = name;
    this.ownerDocument = doc;
    this.parentNode = null;
    this.firstChild = null;
    this.lastChild = null;
    this.previousSibling = null;
    this.nextSibling = null;
    this.$on = null;
  }
  get parentElement() {
    const p = this.parentNode;
    return p && p.nodeType === 1 ? p : null;
  }
  get childNodes() {
    const out = [];
    for (let c = this.firstChild; c; c = c.nextSibling) out.push(c);
    return out;
  }
  hasChildNodes() {
    return this.firstChild != null;
  }
  get isConnected() {
    let n = this;
    while (n.parentNode) n = n.parentNode;
    return n.nodeType === 9;
  }
  getRootNode() {
    let n = this;
    while (n.parentNode) n = n.parentNode;
    return n;
  }
  contains(other) {
    for (let n = other; n; n = n.parentNode) if (n === this) return true;
    return false;
  }
  get textContent() {
    let out = "";
    for (let c = this.firstChild; c; c = c.nextSibling) if (c.nodeType !== 8) out += c.textContent;
    return out;
  }
  set textContent(text) {
    while (this.firstChild) this.removeChild(this.firstChild);
    const t = text == null ? "" : String(text);
    if (t !== "") this.appendChild(this.ownerDocument.createTextNode(t));
  }
  get nodeValue() {
    return null;
  }

  insertBefore(node, before) {
    if (before != null && before.parentNode !== this) throw new Error("insertBefore: the reference node is not a child of this node");
    if (node.nodeType === 11) {
      for (const c of node.childNodes) this.insertBefore(c, before);
      return node;
    }
    for (let n = this; n; n = n.parentNode) if (n === node) throw new Error("insertBefore: a node cannot hold itself");
    if (node === before) return node;
    if (node.parentNode) node.parentNode.removeChild(node);
    node.parentNode = this;
    const prev = before ? before.previousSibling : this.lastChild;
    node.previousSibling = prev;
    node.nextSibling = before || null;
    if (prev) prev.nextSibling = node;
    else this.firstChild = node;
    if (before) before.previousSibling = node;
    else this.lastChild = node;
    invalidate();
    return node;
  }
  appendChild(node) {
    return this.insertBefore(node, null);
  }
  removeChild(node) {
    if (node.parentNode !== this) throw new Error("removeChild: not a child of this node");
    if (node.previousSibling) node.previousSibling.nextSibling = node.nextSibling;
    else this.firstChild = node.nextSibling;
    if (node.nextSibling) node.nextSibling.previousSibling = node.previousSibling;
    else this.lastChild = node.previousSibling;
    node.parentNode = node.previousSibling = node.nextSibling = null;
    invalidate();
    return node;
  }
  replaceChild(node, old) {
    this.insertBefore(node, old);
    return this.removeChild(old);
  }
  remove() {
    if (this.parentNode) this.parentNode.removeChild(this);
  }
  append(...nodes) {
    for (const n of nodes) this.appendChild(asNode(this, n));
  }
  prepend(...nodes) {
    const first = this.firstChild;
    for (const n of nodes) this.insertBefore(asNode(this, n), first);
  }
  replaceChildren(...nodes) {
    while (this.firstChild) this.removeChild(this.firstChild);
    this.append(...nodes);
  }
  before(...nodes) {
    const p = this.parentNode;
    if (p) for (const n of nodes) p.insertBefore(asNode(this, n), this);
  }
  after(...nodes) {
    const p = this.parentNode;
    if (!p) return;
    const next = this.nextSibling;
    for (const n of nodes) p.insertBefore(asNode(this, n), next);
  }
  replaceWith(...nodes) {
    const p = this.parentNode;
    if (!p) return;
    this.before(...nodes);
    p.removeChild(this);
  }
  cloneNode(deep) {
    const copy = this.$copy();
    if (deep) for (let c = this.firstChild; c; c = c.nextSibling) copy.appendChild(c.cloneNode(true));
    return copy;
  }

  // ── listeners ──
  addEventListener(type, fn, options) {
    if (fn == null) return;
    const capture = typeof options === "boolean" ? options : !!(options && options.capture);
    const once = !!(options && typeof options === "object" && options.once);
    const on = this.$on || (this.$on = new Map());
    let list = on.get(type);
    if (!list) on.set(type, (list = []));
    if (list.some((l) => l.fn === fn && l.capture === capture)) return;
    list.push({ fn, capture, once });
    invalidate();
  }
  removeEventListener(type, fn, options) {
    const capture = typeof options === "boolean" ? options : !!(options && options.capture);
    const list = this.$on && this.$on.get(type);
    if (!list) return;
    const i = list.findIndex((l) => l.fn === fn && l.capture === capture);
    if (i < 0) return;
    list.splice(i, 1);
    if (!list.length) this.$on.delete(type);
    invalidate();
  }
  dispatchEvent(event) {
    dispatch(this, event);
    return !event.defaultPrevented;
  }
}

function asNode(at, n) {
  return n instanceof Node ? n : at.ownerDocument.createTextNode(String(n));
}

class CharacterData extends Node {
  constructor(doc, type, name, data) {
    super(doc, type, name);
    this.$data = String(data);
  }
  get data() {
    return this.$data;
  }
  set data(v) {
    const t = v == null ? "" : String(v);
    if (t === this.$data) return;
    this.$data = t;
    invalidate();
  }
  get nodeValue() {
    return this.$data;
  }
  set nodeValue(v) {
    this.data = v;
  }
  get textContent() {
    return this.$data;
  }
  set textContent(v) {
    this.data = v;
  }
  get length() {
    return this.$data.length;
  }
}

class Text extends CharacterData {
  constructor(doc, data) {
    super(doc, 3, TEXT, data);
  }
  get wholeText() {
    return this.$data;
  }
  $copy() {
    return new Text(this.ownerDocument, this.$data);
  }
}

class Comment extends CharacterData {
  constructor(doc, data) {
    super(doc, 8, "#comment", data);
  }
  $copy() {
    return new Comment(this.ownerDocument, this.$data);
  }
}

class DocumentFragment extends Node {
  constructor(doc) {
    super(doc, 11, "#document-fragment");
  }
  $copy() {
    return new DocumentFragment(this.ownerDocument);
  }
}

// ── style ────────────────────────────────────────────────────────────
const kebab = (n) => (n.startsWith("--") || n.includes("-") ? n : n.replace(/[A-Z]/g, (m) => "-" + m.toLowerCase()));

/// An element's `style`: its declarations in order, as a browser's
/// `CSSStyleDeclaration` gives them (`style.color = …`, `cssText`,
/// `setProperty`).
function styleOf(el) {
  const props = new Map();
  const changed = () => invalidate();
  const api = {
    get cssText() {
      let out = "";
      for (const [k, v] of props) out += (out ? " " : "") + k + ": " + v + ";";
      return out;
    },
    set cssText(text) {
      props.clear();
      for (const part of String(text == null ? "" : text).split(";")) {
        const at = part.indexOf(":");
        if (at < 0) continue;
        const k = part.slice(0, at).trim();
        const v = part.slice(at + 1).trim();
        if (k && v) props.set(k.startsWith("--") ? k : k.toLowerCase(), v);
      }
      changed();
    },
    get length() {
      return props.size;
    },
    item: (i) => [...props.keys()][i] ?? "",
    getPropertyValue: (k) => props.get(String(k)) ?? "",
    setProperty(k, v) {
      const name = String(k);
      const value = v == null ? "" : String(v);
      if (value === "") props.delete(name);
      else props.set(name, value);
      changed();
    },
    removeProperty(k) {
      const name = String(k);
      const was = props.get(name) ?? "";
      if (props.delete(name)) changed();
      return was;
    },
  };
  return new Proxy(api, {
    get(target, k) {
      if (typeof k !== "string" || k in target) return Reflect.get(target, k);
      return props.get(kebab(k)) ?? "";
    },
    set(target, k, v) {
      if (typeof k !== "string") return false;
      if (k in target) return Reflect.set(target, k, v);
      api.setProperty(kebab(k), v);
      return true;
    },
    has(target, k) {
      return typeof k === "string";
    },
  });
}

// ── elements ─────────────────────────────────────────────────────────
class Element extends Node {
  constructor(doc, name, ns) {
    const local = ns === HTML || ns == null ? String(name).toLowerCase() : String(name);
    super(doc, 1, local.toUpperCase());
    this.localName = local;
    this.tagName = this.nodeName;
    this.namespaceURI = ns == null ? HTML : ns;
    this.$attrs = new Map();
    this.$style = null;
  }
  get style() {
    return this.$style || (this.$style = styleOf(this));
  }
  set style(text) {
    this.style.cssText = text;
  }
  getAttribute(name) {
    const n = String(name).toLowerCase();
    if (n === "style") return this.$style && this.$style.length ? this.$style.cssText : this.$attrs.has("style") ? "" : null;
    return this.$attrs.has(n) ? this.$attrs.get(n) : null;
  }
  setAttribute(name, value) {
    const n = String(name).toLowerCase();
    const v = String(value);
    if (n === "style") {
      this.style.cssText = v;
      return;
    }
    if (this.$attrs.get(n) === v) return;
    this.$attrs.set(n, v);
    invalidate();
  }
  removeAttribute(name) {
    const n = String(name).toLowerCase();
    if (n === "style") {
      if (this.$style) this.$style.cssText = "";
      return;
    }
    if (this.$attrs.delete(n)) invalidate();
  }
  hasAttribute(name) {
    return this.getAttribute(name) != null;
  }
  toggleAttribute(name, force) {
    const on = force === undefined ? !this.hasAttribute(name) : !!force;
    if (on) this.setAttribute(name, "");
    else this.removeAttribute(name);
    return on;
  }
  getAttributeNS(_ns, name) {
    return this.getAttribute(name);
  }
  setAttributeNS(_ns, name, value) {
    this.setAttribute(String(name).replace(/^[a-z]+:/, ""), value);
  }
  removeAttributeNS(_ns, name) {
    this.removeAttribute(name);
  }
  getAttributeNames() {
    const out = [...this.$attrs.keys()];
    if (this.$style && this.$style.length) out.push("style");
    return out;
  }
  get attributes() {
    return this.getAttributeNames().map((name) => ({ name, value: this.getAttribute(name) }));
  }
  get classList() {
    const el = this;
    const list = () => (el.getAttribute("class") || "").split(/\s+/).filter(Boolean);
    const put = (l) => el.setAttribute("class", l.join(" "));
    return {
      contains: (c) => list().includes(c),
      add: (...cs) => put([...new Set([...list(), ...cs])]),
      remove: (...cs) => put(list().filter((c) => !cs.includes(c))),
      toggle(c, force) {
        const on = force === undefined ? !list().includes(c) : !!force;
        if (on) this.add(c);
        else this.remove(c);
        return on;
      },
      get length() {
        return list().length;
      },
      toString: () => list().join(" "),
    };
  }
  get dataset() {
    const el = this;
    const attr = (k) => "data-" + kebab(k);
    return new Proxy(
      {},
      {
        get: (_, k) => (typeof k === "string" ? el.getAttribute(attr(k)) ?? undefined : undefined),
        set: (_, k, v) => (el.setAttribute(attr(k), v), true),
        deleteProperty: (_, k) => (el.removeAttribute(attr(k)), true),
      },
    );
  }
  get children() {
    return this.childNodes.filter((c) => c.nodeType === 1);
  }
  get firstElementChild() {
    return this.children[0] ?? null;
  }
  get innerHTML() {
    let out = "";
    for (let c = this.firstChild; c; c = c.nextSibling) out += html(c);
    return out;
  }
  set innerHTML(text) {
    if (String(text == null ? "" : text) !== "") throw new Error("architect/dom has no HTML parser: build nodes (createElement) instead of innerHTML");
    this.textContent = "";
  }
  get outerHTML() {
    return html(this);
  }
  getElementsByTagName(tag) {
    const t = String(tag).toLowerCase();
    return all(this, (n) => t === "*" || n.localName === t);
  }
  getElementsByClassName(name) {
    return all(this, (n) => n.classList.contains(String(name)));
  }
  querySelectorAll(selector) {
    const match = matcher(selector);
    return all(this, match);
  }
  querySelector(selector) {
    return this.querySelectorAll(selector)[0] ?? null;
  }
  matches(selector) {
    return matcher(selector)(this);
  }
  closest(selector) {
    const match = matcher(selector);
    for (let n = this; n && n.nodeType === 1; n = n.parentNode) if (match(n)) return n;
    return null;
  }
  // Nothing is laid out here: the window lays out what it is drawn as.
  getBoundingClientRect() {
    return { x: 0, y: 0, left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0 };
  }
  focus() {}
  blur() {}
  click() {
    dispatch(this, new Event("click", { bubbles: true, cancelable: true }));
  }
  $copy() {
    const copy = new Element(this.ownerDocument, this.localName, this.namespaceURI);
    for (const [k, v] of this.$attrs) copy.$attrs.set(k, v);
    if (this.$style && this.$style.length) copy.style.cssText = this.$style.cssText;
    return copy;
  }
}

// Properties a framework sets as properties (Preact sets what `in` finds),
// each its attribute.
const reflect = (prop, attr) =>
  Object.defineProperty(Element.prototype, prop, {
    get() {
      return this.getAttribute(attr) ?? "";
    },
    set(v) {
      if (v == null) this.removeAttribute(attr);
      else this.setAttribute(attr, v);
    },
    configurable: true,
  });
const flag = (prop, attr) =>
  Object.defineProperty(Element.prototype, prop, {
    get() {
      return this.hasAttribute(attr);
    },
    set(v) {
      this.toggleAttribute(attr, !!v);
    },
    configurable: true,
  });
for (const p of ["id", "title", "placeholder", "type", "name", "alt", "src", "lang", "dir", "min", "max", "step"]) reflect(p, p);
reflect("className", "class");
reflect("htmlFor", "for");
reflect("value", "value");
reflect("defaultValue", "value");
for (const p of ["checked", "disabled", "hidden", "selected", "multiple", "required", "autofocus"]) flag(p, p);
flag("readOnly", "readonly");
flag("defaultChecked", "checked");

function all(root, ok) {
  const out = [];
  const walk = (n) => {
    for (let c = n.firstChild; c; c = c.nextSibling) {
      if (c.nodeType !== 1) continue;
      if (ok(c)) out.push(c);
      walk(c);
    }
  };
  walk(root);
  return out;
}

/// A compound selector (`button#pin.big[type=submit]`), or a list of them
/// (`a, b`); no combinators.
function matcher(selector) {
  const one = (s) => {
    const parts = s.trim().match(/^([a-zA-Z][\w-]*|\*)?((?:[#.][\w-]+|\[[^\]]+\])*)$/);
    if (!parts) throw new Error(`architect/dom: an unsupported selector: ${s}`);
    const tag = parts[1] && parts[1] !== "*" ? parts[1].toLowerCase() : null;
    const tests = [];
    for (const m of (parts[2] || "").matchAll(/#([\w-]+)|\.([\w-]+)|\[\s*([\w-]+)\s*(?:=\s*"?([^"\]]*)"?)?\s*\]/g)) {
      if (m[1]) tests.push((n) => n.getAttribute("id") === m[1]);
      else if (m[2]) tests.push((n) => n.classList.contains(m[2]));
      else tests.push((n) => (m[4] === undefined ? n.hasAttribute(m[3]) : n.getAttribute(m[3]) === m[4]));
    }
    return (n) => n.nodeType === 1 && (!tag || n.localName === tag) && tests.every((t) => t(n));
  };
  const each = String(selector).split(",").map(one);
  return (n) => each.some((m) => m(n));
}

const VOID = new Set(["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"]);
const escape = (s, attr) => s.replace(attr ? /[&"]/g : /[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
function html(n) {
  if (n.nodeType === 3) return escape(n.data, false);
  if (n.nodeType === 8) return "<!--" + n.data + "-->";
  if (n.nodeType !== 1) return n.childNodes.map(html).join("");
  let out = "<" + n.localName;
  for (const { name, value } of n.attributes) out += " " + name + '="' + escape(value, true) + '"';
  out += ">";
  if (VOID.has(n.localName)) return out;
  return out + n.innerHTML + "</" + n.localName + ">";
}

// ── the document ─────────────────────────────────────────────────────
class Document extends Node {
  constructor() {
    super(null, 9, "#document");
    this.ownerDocument = null;
    this.documentElement = new Element(this, "html", HTML);
    this.head = new Element(this, "head", HTML);
    this.body = new Element(this, "body", HTML);
    this.documentElement.appendChild(this.head);
    this.documentElement.appendChild(this.body);
    this.appendChild(this.documentElement);
    // The page draws its body: `export default document.body`.
    Object.defineProperty(this.body, "$$tree", { value: (handle) => tree(this.body, handle) });
  }
  get defaultView() {
    return globalThis;
  }
  createElement(name) {
    return new Element(this, name, HTML);
  }
  createElementNS(ns, name) {
    return new Element(this, name, ns || HTML);
  }
  createTextNode(text) {
    return new Text(this, text);
  }
  createComment(text) {
    return new Comment(this, text);
  }
  createDocumentFragment() {
    return new DocumentFragment(this);
  }
  createEvent() {
    return new Event("");
  }
  importNode(node, deep) {
    return node.cloneNode(deep);
  }
  getElementById(id) {
    return all(this, (n) => n.getAttribute("id") === String(id))[0] ?? null;
  }
  querySelectorAll(selector) {
    return all(this, matcher(selector));
  }
  querySelector(selector) {
    return this.querySelectorAll(selector)[0] ?? null;
  }
  get activeElement() {
    return this.body;
  }
  $copy() {
    throw new Error("architect/dom: a document is not cloned");
  }
}

// ── events ───────────────────────────────────────────────────────────
class Event {
  constructor(type, init) {
    this.type = String(type);
    this.bubbles = !!(init && init.bubbles);
    this.cancelable = !!(init && init.cancelable);
    this.composed = false;
    this.defaultPrevented = false;
    this.eventPhase = 0;
    this.target = null;
    this.currentTarget = null;
    this.timeStamp = Date.now();
    this.isTrusted = false;
    this.$stop = false;
    this.$stopNow = false;
    this.$path = [];
  }
  preventDefault() {
    if (this.cancelable) this.defaultPrevented = true;
  }
  stopPropagation() {
    this.$stop = true;
  }
  stopImmediatePropagation() {
    this.$stop = this.$stopNow = true;
  }
  composedPath() {
    return this.$path.slice();
  }
  get srcElement() {
    return this.target;
  }
}

class CustomEvent extends Event {
  constructor(type, init) {
    super(type, init);
    this.detail = init && init.detail !== undefined ? init.detail : null;
  }
}

function call(node, event, capture) {
  const list = node.$on && node.$on.get(event.type);
  if (!list) return;
  event.currentTarget = node;
  for (const l of list.slice()) {
    if (l.capture !== capture && event.eventPhase !== 2) continue;
    if (l.once) node.removeEventListener(event.type, l.fn, l.capture);
    try {
      if (typeof l.fn === "function") l.fn.call(node, event);
      else if (l.fn && typeof l.fn.handleEvent === "function") l.fn.handleEvent(event);
    } catch (e) {
      // As a browser does: the others still run; the page's log says it.
      console.error(e);
    }
    if (event.$stopNow) return;
  }
}

/// `event` at `target`: captured from the top down, at the target, then
/// (if it bubbles) back up.
function dispatch(target, event) {
  const path = [];
  for (let n = target; n; n = n.parentNode) path.push(n);
  event.target = target;
  event.$path = path;
  for (let i = path.length - 1; i > 0 && !event.$stop; i--) {
    event.eventPhase = 1;
    call(path[i], event, true);
  }
  if (!event.$stop) {
    event.eventPhase = 2;
    call(target, event, false);
  }
  for (let i = 1; event.bubbles && i < path.length && !event.$stop; i++) {
    event.eventPhase = 3;
    call(path[i], event, false);
  }
  event.eventPhase = 0;
  event.currentTarget = null;
}

// What the window sends a listener on `type` as: a form's `change` is its
// `input`.
const SENT = { change: "input", focusin: "focus", focusout: "blur", dblclick: "dblclick" };
const sentAs = (type) => SENT[type] || type;
const BUBBLES = new Set(["click", "dblclick", "contextmenu", "mousedown", "mouseup", "input", "change", "keydown", "keyup", "focusin", "focusout", "pointerdown", "pointerup", "pointermove", "pointercancel", "touchstart", "touchmove", "touchend", "wheel", "compositionstart", "compositionupdate", "compositionend"]);

/// The page event the window sent for `node` (the nearest element with a
/// listener for it), as the DOM events its listeners expect.
function deliver(node, pe) {
  const make = (type) => {
    const e = new Event(type, { bubbles: BUBBLES.has(type), cancelable: true });
    e.isTrusted = true;
    e.altKey = !!pe.alt;
    e.ctrlKey = !!pe.ctrl;
    e.metaKey = !!pe.meta;
    e.shiftKey = !!pe.shift;
    e.offsetX = e.clientX = e.pageX = e.x = pe.x || 0;
    e.offsetY = e.clientY = e.pageY = e.y = pe.y || 0;
    e.button = 0;
    e.buttons = pe.buttons || 0;
    e.pointerId = pe.pointer_id || 0;
    e.pointerType = pe.pointer_type || "";
    e.deltaX = pe.dx || 0;
    e.deltaY = pe.dy || 0;
    e.deltaMode = 0;
    if (type.startsWith("composition")) e.data = pe.value || "";
    e.detail = type === "dblclick" ? 2 : 1;
    if (pe.key != null) {
      e.key = pe.key;
      e.code = pe.key;
    }
    return e;
  };
  switch (pe.event) {
    case "input":
      if (pe.value != null) node.value = pe.value;
      dispatch(node, make("input"));
      dispatch(node, make("change"));
      break;
    case "focus":
      dispatch(node, make("focus"));
      dispatch(node, make("focusin"));
      break;
    case "blur":
      dispatch(node, make("blur"));
      dispatch(node, make("focusout"));
      break;
    case "scroll":
      node.scrollTop = pe.scroll_top || 0;
      node.scrollLeft = pe.scroll_left || 0;
      dispatch(node, make("scroll"));
      break;
    default:
      dispatch(node, make(pe.event));
  }
}

// ── as the page's tree ───────────────────────────────────────────────
/// `root`'s children as the page runtime's element tree: each node keyed
/// by its own id (so a node the framework moves stays itself), each
/// listener a handler (`handle(fn)` → its number).
function tree(root, handle) {
  const out = [];
  for (let c = root.firstChild; c; c = c.nextSibling) {
    const n = node(c, handle);
    if (n) out.push(n);
  }
  return out;
}

function node(n, handle) {
  if (n.nodeType === 3) return { tag: TEXT, key: "n" + n.$id, text: n.$data };
  if (n.nodeType !== 1) return null;
  const attrs = [];
  for (const [name, value] of n.$attrs) attrs.push({ name, value });
  if (n.$style && n.$style.length) attrs.push({ name: "style", value: n.$style.cssText });
  const on = [];
  const types = new Set();
  if (n.$on) for (const type of n.$on.keys()) types.add(sentAs(type));
  // Solid delegates its events: `$$click` on the element, one listener on
  // the document.
  for (const k of Object.keys(n)) if (k.startsWith("$$") && typeof n[k] === "function") types.add(sentAs(k.slice(2)));
  if (types.size) {
    const fire = (pe) => deliver(n, pe);
    const handler = handle(fire);
    for (const event of types) on.push({ event, handler });
  }
  return { tag: n.localName, key: "n" + n.$id, attrs, on, children: tree(n, handle) };
}

export const document = new Document();
if (globalThis.document === undefined) globalThis.document = document;
export { Node, Text, Comment, Element, DocumentFragment, Document, Event, CustomEvent };
export default document;
