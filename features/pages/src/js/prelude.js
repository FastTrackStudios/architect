// The page runtime: what `import … from "architect"` gives a page
// (features/pages/src/js/mod.rs runs it first in each page's runtime).
// Its types are architect.d.ts (features/pages/src/codegen.rs).
//
// `__host` is Rust's: call(kind, a, b) -> id starts a host call whose
// answer comes back through `answer`; sync(kind, a, b) answers at once;
// log(level, text); now() (ms); spawn(file, args) -> id; post(id, json);
// terminate(id). What Rust calls here is `__architect`'s: render, event,
// answer, shared, worker, setTheme, tick, dirty.
(() => {
  "use strict";
  const host = globalThis.__host;
  delete globalThis.__host;
  const TEXT = "#text";

  // ── style ─────────────────────────────────────────────────────────
  const UNITLESS = new Set([
    "opacity", "flex", "flexGrow", "flexShrink", "fontWeight", "zIndex",
    "lineHeight", "order", "zoom", "flex-grow", "flex-shrink", "font-weight",
    "z-index", "line-height",
  ]);
  const kebab = (n) => (n.startsWith("--") ? n : n.replace(/[A-Z]/g, (m) => "-" + m.toLowerCase()));
  const tokens = (v) => v.replace(/\$([A-Za-z0-9_][A-Za-z0-9_.-]*)/g, (_, t) => "var(--" + t.replace(/\./g, "-") + ")");
  const cssValue = (name, v) =>
    typeof v === "number" ? (UNITLESS.has(name) || v === 0 ? String(v) : v + "px") : tokens(String(v));
  function styleText(s) {
    if (s == null || s === false) return undefined;
    if (typeof s === "string") return tokens(s);
    const out = [];
    for (const k of Object.keys(s)) {
      const v = s[k];
      if (v == null || v === false) continue;
      out.push(kebab(k) + ": " + cssValue(k, v));
    }
    return out.join("; ");
  }

  // ── elements ──────────────────────────────────────────────────────
  const EVENTS = {
    onClick: "click", onDblClick: "dblclick", onContextMenu: "contextmenu",
    onMouseDown: "mousedown", onMouseUp: "mouseup", onMouseEnter: "mouseenter",
    onMouseLeave: "mouseleave", onInput: "input", onChange: "input",
    onKeyDown: "keydown", onKeyUp: "keyup", onFocus: "focus", onBlur: "blur",
  };
  const eventName = (k) =>
    EVENTS[k] || (k.length > 2 && k.startsWith("on") && k[2] === k[2].toUpperCase() ? k.slice(2).toLowerCase() : null);

  function Fragment(props) {
    return props ? props.children : null;
  }
  function h(tag, props, ...children) {
    const p = props ? Object.assign({}, props) : {};
    if (children.length) p.children = children.length === 1 ? children[0] : children;
    return { $$node: true, tag, props: p, key: p.key };
  }

  // ── rendering: descriptions to the element tree, hooks by place ────
  let handlers = [];
  const instances = new Map();
  let seen = new Set();
  let current = null;
  let effects = [];
  let dirty = true;
  let rendering = false;

  function invalidate() {
    dirty = true;
  }

  function expand(node, path, out) {
    if (node == null || node === false || node === true) return;
    if (Array.isArray(node)) {
      for (let i = 0; i < node.length; i++) {
        const c = node[i];
        const at = c && c.$$node && c.key != null ? "k" + c.key : String(i);
        expand(c, path + "." + at, out);
      }
      return;
    }
    if (typeof node !== "object" || !node.$$node) {
      out.push({ tag: TEXT, text: String(node) });
      return;
    }
    const { tag, props } = node;
    if (typeof tag === "function") {
      const at = path + ":" + (tag.name || "component");
      seen.add(at);
      let inst = instances.get(at);
      if (!inst) {
        inst = { hooks: [] };
        instances.set(at, inst);
      }
      const outer = current;
      current = { inst, i: 0 };
      let made;
      try {
        made = tag(props);
      } finally {
        current = outer;
      }
      const before = out.length;
      expand(made, at, out);
      // A component's key keys what it drew.
      if (node.key != null && out.length === before + 1 && out[before].key == null) out[before].key = String(node.key);
      return;
    }
    const el = { tag: String(tag), attrs: [], on: [], children: [] };
    if (node.key != null) el.key = String(node.key);
    for (const k of Object.keys(props)) {
      if (k === "children" || k === "key" || k === "ref") continue;
      const v = props[k];
      if (typeof v === "function") {
        const ev = eventName(k);
        if (ev) el.on.push({ event: ev, handler: handlers.push(v) - 1 });
        continue;
      }
      if (v == null || v === false) continue;
      if (k === "style") {
        const s = styleText(v);
        if (s) el.attrs.push({ name: "style", value: s });
        continue;
      }
      const name = k === "className" ? "class" : k === "htmlFor" ? "for" : k;
      el.attrs.push({ name, value: v === true ? "" : String(v) });
    }
    expand(props.children, path + "/" + el.tag, el.children);
    out.push(el);
  }

  let root = null;
  function mount(component) {
    if (typeof component !== "function") throw new TypeError("a page's default export is its component: export default function Page() { … }");
    root = component;
    dirty = true;
  }

  function render() {
    if (!root) throw new Error("the page exported no component (export default function Page() { … })");
    dirty = false;
    handlers = [];
    seen = new Set();
    effects = [];
    rendering = true;
    const out = [];
    try {
      expand(h(root, null), "", out);
    } finally {
      rendering = false;
    }
    // What is no longer drawn unmounts: its effects clean up.
    for (const [at, inst] of instances) {
      if (seen.has(at)) continue;
      instances.delete(at);
      for (const hk of inst.hooks) if (hk && typeof hk.cleanup === "function") try { hk.cleanup(); } catch (e) { report(e); }
    }
    const due = effects;
    effects = [];
    for (const [hk, fn] of due) {
      if (typeof hk.cleanup === "function") try { hk.cleanup(); } catch (e) { report(e); }
      try {
        const c = fn();
        hk.cleanup = typeof c === "function" ? c : undefined;
      } catch (e) {
        report(e);
      }
    }
    return JSON.stringify(out);
  }

  function hook(init) {
    if (!current) throw new Error("a hook runs only while a component renders");
    const { inst } = current;
    const i = current.i++;
    if (inst.hooks.length <= i) inst.hooks.push(init());
    return inst.hooks[i];
  }
  function useState(initial) {
    const s = hook(() => ({ v: typeof initial === "function" ? initial() : initial }));
    if (!s.set)
      s.set = (next) => {
        const v = typeof next === "function" ? next(s.v) : next;
        if (!Object.is(v, s.v)) {
          s.v = v;
          dirty = true;
        }
      };
    return [s.v, s.set];
  }
  function useRef(initial) {
    return hook(() => ({ current: initial }));
  }
  function useEffect(fn, deps) {
    const s = hook(() => ({ deps: undefined, cleanup: undefined, ran: false }));
    const changed = !deps || !s.ran || deps.length !== s.deps.length || deps.some((d, i) => !Object.is(d, s.deps[i]));
    if (changed) {
      s.deps = deps;
      s.ran = true;
      effects.push([s, fn]);
    }
  }
  function useData(id, args, options) {
    const s = hook(() => ({ value: undefined, error: undefined, loading: true, seq: 0 }));
    s.id = id;
    s.args = args;
    if (!s.refresh)
      s.refresh = () => {
        const seq = ++s.seq;
        actions.run(s.id, s.args).then(
          (v) => {
            if (seq !== s.seq) return;
            s.value = v;
            s.error = undefined;
            s.loading = false;
            dirty = true;
          },
          (e) => {
            if (seq !== s.seq) return;
            s.error = String((e && e.message) || e);
            s.loading = false;
            dirty = true;
          },
        );
      };
    const every = options && options.every;
    useEffect(() => {
      s.refresh();
      if (every > 0) {
        const t = setInterval(() => s.refresh(), every);
        return () => clearInterval(t);
      }
    }, [id, JSON.stringify(args === undefined ? null : args), every]);
    return { value: s.value, error: s.error, loading: s.loading, refresh: s.refresh };
  }
  function useShared(key, initial) {
    const got = state.get(key);
    useEffect(() => state.watch(key, () => (dirty = true)), [key]);
    return [got === undefined ? initial : got, (v) => state.set(key, v)];
  }
  // An inline embed's input ({claim, source, props}), or undefined in a
  // pane: the page renders again when the host's block changes.
  function useEmbed() {
    useEffect(() => state.watch("$embed", () => (dirty = true)), []);
    const v = host.sync("embed", "", "");
    const out = v == null ? null : JSON.parse(v);
    return out === null ? undefined : out;
  }
  function useTheme() {
    return theme;
  }

  // ── host calls ────────────────────────────────────────────────────
  const pending = new Map();
  function call(kind, a, b) {
    return new Promise((resolve, reject) => {
      const id = host.call(kind, a == null ? "" : String(a), b == null ? "" : String(b));
      pending.set(id, { resolve, reject });
    });
  }
  function answer(id, ok, json) {
    const p = pending.get(id);
    if (!p) return;
    pending.delete(id);
    if (ok) p.resolve(json === "" ? undefined : JSON.parse(json));
    else p.reject(new Error(json));
    dirty = true;
  }
  const json = (v) => JSON.stringify(v === undefined ? null : v);
  const actions = Object.freeze({
    run: (id, args) => call("run", id, json(args)),
    list: (filter) => call("actions", filter || ""),
  });
  // Another extension's service (one its manifest [requires]): its
  // provider's action answers, checked as the host service checks it.
  const services = Object.freeze({
    call: (service, method, args) =>
      call("run", "EXT_SERVICE_CALL", json({ service, method, args: args === undefined ? null : args })),
  });
  // The app's interface (capabilities.ui), as the page's extension: each
  // a host-service call; pick, prompt and menu wait for the person (a
  // cancel is null).
  const ask = (op, fields) => call("host", JSON.stringify(Object.assign({}, fields || {}, { op })));
  const ui = Object.freeze({
    notify: (text, kind, title) => ask("notify", { text: String(text), kind: kind || "info", title: title || null }),
    pick: (options) => ask("pick", options),
    prompt: (options) => ask("prompt", options),
    input: (title, options) => ask("prompt", Object.assign({}, options || {}, { kind: "text", title: String(title) })),
    confirm: (title, message) => ask("prompt", { kind: "confirm", title: String(title), message: message == null ? null : String(message) }),
    pickOne: (title, choices) => ask("prompt", { kind: "pick_one", title: String(title), choices }),
    menu: (items, at) => ask("menu", Object.assign({ items }, at || {})),
    decorate: (target, decoration) => ask("decorate", { target, decoration: decoration === undefined ? null : decoration }),
    call: (request) => call("host", JSON.stringify(request)),
  });
  const clipboard = Object.freeze({
    read: () => ask("clipboard_read"),
    write: (text) => ask("clipboard_write", { text: String(text) }),
  });
  // Hooks made at run time, as the page's extension: one of its actions on
  // an app event or a schedule (`{ on }`, `{ every }`, `{ at }`).
  const events = Object.freeze({
    subscribe: (hook) => ask("subscribe", hook),
    unsubscribe: (key) => ask("unsubscribe", { key: String(key) }),
    list: () => ask("events"),
  });
  // The app's documents, as the page's extension (capabilities.reads /
  // .writes "documents"): an edit is one transaction, one undo step.
  const documents = Object.freeze({
    list: (under) => ask("list_documents", { under: under == null ? null : String(under) }),
    read: (path) => ask("read_document", { path: String(path) }),
    create: (path, text) => ask("create_document", { path: String(path), text: text == null ? "" : String(text) }),
    edit: (path, edits, name) => ask("edit_document", { path: String(path), edits, name: name == null ? null : String(name) }),
    rename: (path, to) => ask("rename_document", { path: String(path), to: String(to) }),
    move: (path, folder) => ask("move_document", { path: String(path), folder: String(folder) }),
    delete: (path) => ask("delete_document", { path: String(path) }),
    watch: (hook) => ask("watch_documents", hook),
  });
  // The network, as the page's extension (capabilities.hosts): the host
  // fetches, every redirect checked. A header's value may name one of its
  // secrets ("Bearer {{secret:api_key}}"): the host puts it in.
  const net = Object.freeze({
    fetch: (url, options) => ask("fetch", Object.assign({}, options || {}, { url: String(url) })),
    read: (stream, max) => ask("read_body", { stream: String(stream), max: max == null ? null : Number(max) }),
    close: (stream) => ask("close_body", { stream: String(stream) }),
    secret: (name) => ask("secret", { name: String(name) }),
    askSecret: (name) => ask("ask_secret", { name: String(name) }),
  });
  const settings = Object.freeze({
    get: (key) => call("setting", key),
    set: (key, value) => call("set_setting", key, json(value)),
  });
  const watchers = new Map();
  const state = Object.freeze({
    get(key) {
      const v = host.sync("shared", key, "");
      const out = v == null ? null : JSON.parse(v);
      return out === null ? undefined : out;
    },
    set(key, value) {
      host.sync("set_shared", key, json(value));
    },
    watch(key, fn) {
      let set = watchers.get(key);
      if (!set) watchers.set(key, (set = new Set()));
      set.add(fn);
      return () => set.delete(fn);
    },
  });
  function shared(key, value) {
    const set = watchers.get(key);
    if (!set) return;
    const v = JSON.parse(value);
    for (const fn of [...set]) try { fn(v === null ? undefined : v); } catch (e) { report(e); }
  }
  function notify(text, kind) {
    host.sync("notify", String(text), kind || "info");
  }
  // The raw service, as vox's TypeScript client answers.
  const wrap = (p) => p.then((value) => ({ ok: true, value: value === undefined ? undefined : JSON.stringify(value) }), (e) => ({ ok: false, error: { tag: "Failed", value: String(e.message || e) } }));
  const raw = Object.freeze({
    run: (id, args) => wrap(call("run", id, args)),
    actions: (filter) => call("actions", filter).then((value) => ({ ok: true, value }), (e) => ({ ok: false, error: { tag: "Failed", value: String(e.message) } })),
    setting: (key) => wrap(call("setting", key)),
    setSetting: (key, value) => wrap(call("set_setting", key, value)),
    theme: () => Promise.resolve({ ok: true, value: snapshot }),
    shared: (key) => Promise.resolve({ ok: true, value: host.sync("shared", key, "") }),
    setShared: (key, value) => Promise.resolve((host.sync("set_shared", key, value), { ok: true, value: undefined })),
    notify: (n) => Promise.resolve((notify(n.text, n.kind), { ok: true, value: undefined })),
    hostCall: (request) => wrap(call("host", request)),
  });

  // ── theme ─────────────────────────────────────────────────────────
  let snapshot = { id: "", scheme: "dark", revision: 0, tokens: [] };
  let byName = {};
  const theme = {
    get id() { return snapshot.id; },
    get scheme() { return snapshot.scheme; },
    get revision() { return snapshot.revision; },
    get tokens() { return byName; },
    value: (name) => byName[name],
    css: (name) => "var(--" + String(name).replace(/\./g, "-") + ")",
  };
  Object.freeze(theme);
  function setTheme(text) {
    snapshot = JSON.parse(text);
    const m = {};
    for (const t of snapshot.tokens) m[t.name] = t.value;
    byName = Object.freeze(m);
    dirty = true;
  }

  // ── timers ────────────────────────────────────────────────────────
  const timers = new Map();
  let nextTimer = 1;
  function addTimer(fn, ms, every) {
    const id = nextTimer++;
    const wait = Math.max(0, Number(ms) || 0);
    timers.set(id, { fn, at: host.now() + wait, every: every ? Math.max(wait, 1) : 0 });
    return id;
  }
  function tick() {
    const now = host.now();
    for (const [id, t] of [...timers]) {
      if (t.at > now || !timers.has(id)) continue;
      if (t.every) t.at = now + t.every;
      else timers.delete(id);
      try { t.fn(); } catch (e) { report(e); }
    }
    let soonest = -1;
    for (const t of timers.values()) if (soonest < 0 || t.at < soonest) soonest = t.at;
    return soonest < 0 ? -1 : Math.max(0, soonest - host.now());
  }
  globalThis.setTimeout = (fn, ms) => addTimer(fn, ms, false);
  globalThis.setInterval = (fn, ms) => addTimer(fn, ms, true);
  globalThis.clearTimeout = globalThis.clearInterval = (id) => void timers.delete(id);

  // ── workers ───────────────────────────────────────────────────────
  const workers = new Map();
  const worker = Object.freeze({
    spawn(file, args) {
      const id = host.spawn(String(file), json(args));
      const w = { messages: [], done: [] };
      workers.set(id, w);
      return Object.freeze({
        id,
        post: (m) => host.post(id, json(m)),
        onMessage: (fn) => void w.messages.push(fn),
        onDone: (fn) => void w.done.push(fn),
        cancel: () => host.terminate(id),
        terminate: () => host.terminate(id),
      });
    },
  });
  function workerNews(id, kind, value) {
    const w = workers.get(id);
    if (!w) return;
    const v = value === "" ? undefined : JSON.parse(value);
    if (kind === "message") for (const fn of w.messages) try { fn(v); } catch (e) { report(e); }
    else {
      workers.delete(id);
      for (const fn of w.done) try { kind === "done" ? fn(v, undefined) : fn(undefined, String(v)); } catch (e) { report(e); }
    }
    dirty = true;
  }

  // ── events, logs ──────────────────────────────────────────────────
  function event(text) {
    const e = JSON.parse(text);
    const fn = handlers[e.handler];
    if (typeof fn === "function") fn(e);
  }
  const show = (v) => {
    if (typeof v === "string") return v;
    if (v instanceof Error) return v.message + (v.stack ? "\n" + v.stack : "");
    try { return JSON.stringify(v); } catch { return String(v); }
  };
  function report(e) {
    host.log("error", show(e));
  }
  globalThis.console = Object.freeze({
    log: (...a) => host.log("info", a.map(show).join(" ")),
    info: (...a) => host.log("info", a.map(show).join(" ")),
    warn: (...a) => host.log("warn", a.map(show).join(" ")),
    error: (...a) => host.log("error", a.map(show).join(" ")),
  });

  // ── overlays ──────────────────────────────────────────────────────
  function Overlay({ kind, title, onClose, children }) {
    const dialog = (kind || "dialog") === "dialog";
    return h(
      "div",
      {
        class: "page-overlay",
        style: dialog
          ? "position: fixed; left: 0; top: 0; right: 0; bottom: 0; display: flex; align-items: center; justify-content: center; background: rgba(0,0,0,0.45); z-index: 10;"
          : "position: fixed; right: 12px; bottom: 12px; z-index: 10;",
      },
      h(
        "div",
        { style: "min-width: 260px; max-width: 80%; padding: 14px 16px; border-radius: 10px; border: 1px solid var(--border); background: var(--popover, var(--card)); color: var(--popover-foreground, var(--foreground)); display: flex; flex-direction: column; gap: 10px;" },
        title || onClose
          ? h(
              "div",
              { style: "display: flex; align-items: center; gap: 8px; font-weight: 600;" },
              h("span", { style: "flex: 1;" }, title || ""),
              onClose ? h("button", { class: "page-overlay-close", onClick: () => onClose() }, "✕") : null,
            )
          : null,
        children,
      ),
    );
  }

  const api = Object.freeze({
    h, Fragment, useState, useRef, useEffect, useData, useShared, useEmbed, useTheme, invalidate,
    theme, actions, services, settings, state, notify, ui, clipboard, events, documents, net, worker, Overlay, host: raw,
  });
  globalThis.__architect = Object.freeze({
    api,
    mount,
    render,
    event,
    answer,
    shared,
    worker: workerNews,
    setTheme,
    tick,
    dirty: () => dirty,
  });
})();
