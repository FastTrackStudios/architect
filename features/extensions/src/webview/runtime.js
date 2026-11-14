// The web view kind's runtime (#222; `features/extensions/src/webview.rs`):
// `window.architect`, put in a web view before its own scripts. It speaks
// the bridge's messages (`Inbound`, `Outbound`) over whichever transport
// the host gave it: wry's `window.ipc`, WKWebView's message handler, or a
// MessagePort a sandboxed frame's parent hands it. Every call is a
// host-service request; the host checks it against what the extension was
// granted, so nothing here is a boundary.
(function () {
  "use strict";
  if (window.architect) return;
  var next = 1;
  var waiting = new Map();
  var listeners = new Map();
  var themeFns = [];
  var shownFns = [];
  var queue = [];
  var port = null;
  var state = { scheme: "dark", shown: true };

  function transport() {
    if (window.ipc && typeof window.ipc.postMessage === "function") {
      return function (text) { window.ipc.postMessage(text); };
    }
    var wk = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.architect;
    if (wk) return function (text) { wk.postMessage(text); };
    if (port) return function (text) { port.postMessage(text); };
    return null;
  }

  function send(message) {
    var text = JSON.stringify(message);
    var t = transport();
    if (t) t(text); else queue.push(text);
  }

  function flush() {
    var t = transport();
    if (!t) return;
    var q = queue; queue = [];
    q.forEach(function (text) { t(text); });
  }

  function styleSheet(id, css) {
    var el = document.getElementById(id);
    if (!el) {
      el = document.createElement("style");
      el.id = id;
      (document.head || document.documentElement).prepend(el);
    }
    el.textContent = css;
  }

  function receive(text) {
    var m;
    try { m = typeof text === "string" ? JSON.parse(text) : text; } catch (_) { return; }
    if (!m || typeof m.t !== "string") return;
    if (m.t === "answer") {
      var w = waiting.get(m.id);
      if (!w) return;
      waiting.delete(m.id);
      var a = m.answer || {};
      if (Object.prototype.hasOwnProperty.call(a, "ok")) w.resolve(a.ok);
      else {
        var e = a.error || { kind: "failed", message: "no answer" };
        var err = new Error(e.why || e.message || e.what || e.kind);
        err.kind = e.kind;
        err.detail = e;
        w.reject(err);
      }
    } else if (m.t === "event") {
      listeners.forEach(function (fns, pattern) {
        if (covers(pattern, m.name)) fns.forEach(function (f) { f(m.detail, m.name); });
      });
    } else if (m.t === "theme") {
      state.scheme = m.scheme;
      styleSheet("architect-theme", m.css);
      document.documentElement.dataset.scheme = m.scheme;
      themeFns.forEach(function (f) { f(m.scheme); });
    } else if (m.t === "shown") {
      state.shown = !!m.shown;
      shownFns.forEach(function (f) { f(state.shown); });
    }
  }

  function covers(pattern, name) {
    return pattern === "*" || pattern === name ||
      (pattern.slice(-2) === ".*" && name.indexOf(pattern.slice(0, -2) + ".") === 0);
  }

  function call(req) {
    return new Promise(function (resolve, reject) {
      var id = next++;
      waiting.set(id, { resolve: resolve, reject: reject });
      send({ t: "call", id: id, req: req });
    });
  }

  function listen() {
    send({ t: "listen", events: Array.from(listeners.keys()) });
  }

  // A sandboxed frame's parent hands over its port once, first thing.
  window.addEventListener("message", function (ev) {
    if (port || ev.source !== window.parent || ev.data !== "architect:port" || !ev.ports || !ev.ports[0]) return;
    port = ev.ports[0];
    port.onmessage = function (e) { receive(e.data); };
    flush();
    send({ t: "ready" });
  });

  // Typed keys stay here; an Escape nothing took gives the app the focus.
  window.addEventListener("keydown", function (ev) {
    if (ev.key === "Escape" && !ev.defaultPrevented && !ev.altKey && !ev.ctrlKey && !ev.metaKey && !ev.shiftKey) {
      send({ t: "escape" });
    }
  });

  if (window.__architectComponents) {
    var install = function () { styleSheet("architect-components", window.__architectComponents); };
    if (document.head) install(); else document.addEventListener("DOMContentLoaded", install);
  }

  var architect = {
    /** Any host-service request (`{op, …}`): its answer, or an Error with `.kind`. */
    call: call,
    run: function (action, args) { return call({ op: "run", action: action, args: args === undefined ? null : args }); },
    setting: function (key) { return call({ op: "setting", key: key }); },
    setSetting: function (key, value) { return call({ op: "set_setting", key: key, value: value }); },
    data: function (key) { return call({ op: "data", key: key }); },
    setData: function (key, value) { return call({ op: "set_data", key: key, value: value === undefined ? null : value }); },
    documents: {
      list: function (under) { return call({ op: "list_documents", under: under === undefined ? null : under }); },
      read: function (path) { return call({ op: "read_document", path: path }); },
      create: function (path, text) { return call({ op: "create_document", path: path, text: text || "" }); },
      edit: function (path, edits, name) { return call({ op: "edit_document", path: path, edits: edits, name: name === undefined ? null : name }); },
      rename: function (path, to) { return call({ op: "rename_document", path: path, to: to }); },
      move: function (path, folder) { return call({ op: "move_document", path: path, folder: folder }); },
      remove: function (path) { return call({ op: "delete_document", path: path }); },
      open: function (path) { return call({ op: "open", path: path }); }
    },
    /** An HTTP request through the host, to the hosts in its manifest only. */
    fetch: function (url, options) {
      var req = Object.assign({ op: "fetch", url: url }, options || {});
      return call(req);
    },
    secrets: {
      isSet: function (name) { return call({ op: "secret", name: name }); },
      ask: function (name) { return call({ op: "ask_secret", name: name }); },
      /** Insert-only: the host keeps it; nothing reads it back. */
      store: function (name, value) { return call({ op: "store_secret", name: name, value: value }); }
    },
    notify: function (text, options) { return call(Object.assign({ op: "notify", text: text }, options || {})); },
    prompt: function (options) { return call(Object.assign({ op: "prompt" }, options)); },
    pick: function (options) { return call(Object.assign({ op: "pick" }, options)); },
    menu: function (items, at) { return call(Object.assign({ op: "menu", items: items }, at || {})); },
    clipboard: {
      read: function () { return call({ op: "clipboard_read" }); },
      write: function (text) { return call({ op: "clipboard_write", text: text }); }
    },
    log: function (text, level) { return call({ op: "log", text: String(text), level: level || null }); },
    /** Hears event `name` (`doc.saved`, `doc.*`, `app:<name>`) while it may: a function that stops it. */
    on: function (name, fn) {
      var fns = listeners.get(name);
      if (!fns) { fns = new Set(); listeners.set(name, fns); listen(); }
      fns.add(fn);
      return function () {
        fns.delete(fn);
        if (fns.size === 0) { listeners.delete(name); listen(); }
      };
    },
    theme: {
      get scheme() { return state.scheme; },
      onChange: function (fn) { themeFns.push(fn); return function () { themeFns = themeFns.filter(function (f) { return f !== fn; }); }; }
    },
    get shown() { return state.shown; },
    onShown: function (fn) { shownFns.push(fn); return function () { shownFns = shownFns.filter(function (f) { return f !== fn; }); }; }
  };

  Object.defineProperty(window, "architect", { value: Object.freeze(architect), enumerable: true });
  // The native hosts' way in (wry's and WKWebView's `evaluate_script`).
  Object.defineProperty(window, "__architectReceive", { value: receive });
  if (transport()) send({ t: "ready" });
})();
