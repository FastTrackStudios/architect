(() => {
  "use strict";
  // The web view's frame (`architect_sites::island::webview_document`): a
  // document of its own with its own policy (an opaque origin; no fetch,
  // no connection, no image or font from anywhere but `blob:`/`data:`).
  // Its parent, the share page's player, hands it one port; through it
  // come the bundle's files (fetched and checked by the page), the page's
  // theme and where the song is; out go only the view's asks, its height,
  // that it is ready, and its answer to the page's heartbeat.
  //
  // What makes traffic past `connect-src` is taken away here, before any
  // of the bundle runs: WebRTC (`RTCPeerConnection` and its kin; also
  // `webrtc 'block'` in the policy, which not every engine honours), and
  // the Presentation API. A frame whose extension declares
  // `capabilities.webrtc = ["peer-connections"]` and whose operator grants
  // it is served a variant of this document whose policy allows WebRTC
  // and which leaves it (the node rewrites the line below; nothing a page,
  // a note or a message says can).
  var RTC = false;
  if (!RTC) {
    ["RTCPeerConnection", "webkitRTCPeerConnection", "RTCDataChannel", "RTCSessionDescription", "RTCIceCandidate", "RTCRtpSender", "RTCRtpReceiver", "RTCRtpTransceiver", "RTCCertificate"].forEach(function (name) {
      try { delete globalThis[name]; } catch (e) {}
      try { Object.defineProperty(globalThis, name, { value: undefined, writable: false, configurable: false, enumerable: false }); } catch (e) {}
    });
  }
  try { Object.defineProperty(navigator, "presentation", { value: undefined, writable: false, configurable: false }); } catch (e) {}
  try { Object.defineProperty(navigator, "sendBeacon", { value: function () { return false; }, writable: false, configurable: false }); } catch (e) {}

  var port = null;
  var props = null;
  var listeners = [];
  var page = {
    get props() { return props; },
    subscribe: function (fn) {
      listeners.push(fn);
      if (props) { try { fn(props); } catch (e) { console.error(e); } }
      return function () {
        var i = listeners.indexOf(fn);
        if (i >= 0) listeners.splice(i, 1);
      };
    },
    ask: function (action, args) {
      if (!port) return;
      var data = null;
      try { data = args === undefined ? null : JSON.parse(JSON.stringify(args)); } catch (e) { data = null; }
      port.postMessage({ t: "ask", action: String(action), args: data });
    },
  };
  Object.defineProperty(globalThis, "architectPage", { value: Object.freeze(page) });
  // Its height: at most once a frame, and only when it changed.
  var shown = -1, sizing = 0;
  function size() {
    if (sizing) return;
    sizing = requestAnimationFrame(function () {
      sizing = 0;
      var h = Math.ceil(document.documentElement.getBoundingClientRect().height);
      if (port && h !== shown) {
        shown = h;
        port.postMessage({ t: "size", height: h });
      }
    });
  }
  function decode(bytes) { return new TextDecoder().decode(bytes); }
  // The bundle, once: its modules import each other as `./<name>.js` (one
  // folder, no other path), each made a blob: module and the imports
  // pointed at those; its sheet as `<style>`; its one wasm module given to
  // the glue as bytes; then its view mounted.
  var booted = false;
  function boot(d) {
    if (booted) return Promise.resolve();
    booted = true;
    var files = d.files || {};
    var names = Object.keys(files);
    var urls = {};
    var entry = String(d.entry || "");
    var mods = names.filter(function (n) { return /\.js$/.test(n) && n !== entry; }).concat([entry]);
    mods.forEach(function (n) {
      var src = decode(files[n]).replace(/(\bfrom\s*|\bimport\s*\(\s*|\bimport\s+)(["'])\.\/([A-Za-z0-9._-]+\.js)\2/g, function (all, kw, q, name) {
        return urls[name] ? kw + q + urls[name] + q : all;
      });
      urls[n] = URL.createObjectURL(new Blob([src], { type: "text/javascript" }));
    });
    // Its sheet (the island's, pinned by the page).
    if (typeof d.sheet === "string" && d.sheet) {
      var s = document.createElement("style");
      s.textContent = d.sheet;
      document.head.appendChild(s);
    }
    var wasm = names.filter(function (n) { return /\.wasm$/.test(n); })[0];
    return import(urls[entry]).then(function (m) {
      return Promise.resolve(m.default({ module_or_path: files[wasm] })).then(function () {
        if (typeof m.mount !== "function") throw new Error("the bundle has no mount(view)");
        m.mount(String(d.view || ""));
        if (window.ResizeObserver) new ResizeObserver(size).observe(document.documentElement);
        size();
        port.postMessage({ t: "ready" });
      });
    });
  }
  function theme(vars, scheme) {
    var root = document.documentElement.style;
    Object.keys(vars || {}).forEach(function (k) {
      if (/^--[A-Za-z0-9_-]{1,64}$/.test(k)) root.setProperty(k, String(vars[k]));
    });
    if (scheme === "dark" || scheme === "light") root.setProperty("color-scheme", scheme);
  }
  addEventListener("message", function once(e) {
    if (e.source !== parent || e.data !== "architect-web" || !e.ports || !e.ports[0]) return;
    removeEventListener("message", once);
    port = e.ports[0];
    port.onmessage = function (m) {
      var d = m.data || {};
      if (d.t === "props") {
        props = d.props;
        listeners.slice().forEach(function (fn) { try { fn(props); } catch (err) { console.error(err); } });
      } else if (d.t === "ping") port.postMessage({ t: "pong", n: d.n });
      else if (d.t === "theme") theme(d.vars, d.scheme);
      else if (d.t === "boot") {
        boot(d).catch(function (err) { port.postMessage({ t: "error", message: String((err && err.message) || err) }); });
      }
    };
    port.postMessage({ t: "hello" });
  });
})();
