// The web view kind's browser host (#222): an extension's `kind = "web"`
// view in a sandboxed `<iframe>` (`sandbox="allow-scripts"`, never
// `allow-same-origin`: an opaque origin with no cookies, storage or DOM of
// the page's), speaking the bridge's messages
// (`features/extensions/src/webview.rs` `Inbound`/`Outbound`) over a
// MessagePort handed to it once. Every call goes to `call`: in Architect's
// web app that is the extension's `WebHost::call_json`, which checks it
// against the extension's capabilities (`hostcall::check`) before it does
// anything. The frame's files are served under `WEBVIEW_CSP`
// (`connect-src 'none'`: its network is the host's `fetch`).

/** The policy a web view's files are served under (as `webview::csp`). */
export const WEBVIEW_CSP =
  "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; " +
  "img-src 'self' data: blob:; font-src 'self' data:; media-src 'self' blob:; " +
  "worker-src 'self' blob:; connect-src 'none'; frame-src 'none'; child-src 'self' blob:; " +
  "object-src 'none'; manifest-src 'none'; base-uri 'none'; form-action 'none'";

/** The largest message a view may send (as `webview::MAX_MESSAGE`). */
export const MAX_MESSAGE = 4 * 1024 * 1024;

/** The theme as a style sheet: each token a custom property on `:root` (as `webview::theme_css`). */
export function themeCss(scheme, tokens) {
  const s = scheme === 'light' ? 'light' : 'dark';
  let css = `:root{color-scheme:${s};`;
  for (const [raw, value] of Object.entries(tokens ?? {})) {
    const name = raw.replace(/^--/, '').replaceAll('.', '-');
    const v = String(value);
    if (!/^[a-z0-9-]+$/.test(name) || !v || /[{};<>\\\n\r]/.test(v) || /url\(/i.test(v)) continue;
    css += `--${name}:${v.trim()};`;
  }
  return `${css}}`;
}

function covers(pattern, name) {
  return pattern === '*' || pattern === name || (pattern.endsWith('.*') && name.startsWith(pattern.slice(0, -1)));
}

/**
 * Mounts a web view in `container`.
 *
 * - `src`: its document's URL (its package's file, served under `WEBVIEW_CSP`);
 * - `call(request)`: answers one host-service request with `{ok}` or `{error}`
 *   (checked against what the extension may reach);
 * - `theme`: `{scheme, tokens}`;
 * - `onEscape()`: an Escape the view did not take.
 *
 * Returns `{frame, setTheme, setShown, emit, unmount}`; `emit(name, detail)`
 * passes an event on when the view listens for it.
 */
export function mountWebView(container, { src, call, theme, onEscape, title = 'Extension view' }) {
  const frame = document.createElement('iframe');
  frame.setAttribute('sandbox', 'allow-scripts');
  frame.setAttribute('referrerpolicy', 'no-referrer');
  frame.setAttribute('allow', '');
  frame.setAttribute('title', title);
  frame.style.cssText = 'border:0;width:100%;height:100%;display:block;background:transparent';
  frame.src = src;
  let port = null;
  let listening = [];
  let current = theme ?? { scheme: 'dark', tokens: {} };
  let shown = true;
  let inFlight = 0;
  const post = (m) => port?.postMessage(JSON.stringify(m));
  const sendTheme = () => post({ t: 'theme', scheme: current.scheme, css: themeCss(current.scheme, current.tokens) });

  async function receive(text) {
    if (typeof text !== 'string' || text.length > MAX_MESSAGE) return;
    let m;
    try { m = JSON.parse(text); } catch { return; }
    if (m.t === 'call' && Number.isSafeInteger(m.id)) {
      if (inFlight >= 64) return post({ t: 'answer', id: m.id, answer: { error: { kind: 'unsupported', what: 'more than 64 calls are waiting on the host' } } });
      inFlight += 1;
      let answer;
      try { answer = await call(m.req); } catch (e) { answer = { error: { kind: 'failed', message: String(e?.message ?? e) } }; }
      inFlight -= 1;
      post({ t: 'answer', id: m.id, answer });
    } else if (m.t === 'listen' && Array.isArray(m.events)) {
      listening = m.events.filter((e) => typeof e === 'string');
    } else if (m.t === 'escape') {
      onEscape?.();
    } else if (m.t === 'ready') {
      sendTheme();
      post({ t: 'shown', shown });
    }
  }

  // A fresh channel on every load: a document that navigated (its own
  // reload) gets a port of its own; the old one is closed.
  const onLoad = () => {
    port?.close();
    const channel = new MessageChannel();
    port = channel.port1;
    port.onmessage = (e) => receive(e.data);
    frame.contentWindow?.postMessage('architect:port', '*', [channel.port2]);
  };
  frame.addEventListener('load', onLoad);
  container.append(frame);
  return {
    frame,
    setTheme(next) { current = next; sendTheme(); },
    setShown(next) { shown = !!next; post({ t: 'shown', shown }); },
    emit(name, detail) {
      if (listening.some((p) => covers(p, name))) post({ t: 'event', name, detail });
    },
    unmount() {
      frame.removeEventListener('load', onLoad);
      port?.close();
      frame.remove();
    },
  };
}
