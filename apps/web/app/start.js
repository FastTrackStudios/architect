// The web app's start (apps/web/app/index.html): its wasm, a file of this
// server, as the page's Content-Security-Policy allows.
import init from '/app/architect_web.js';
init({ module_or_path: '/app/architect_web_bg.wasm' });
