// The web view kind's SDK (#222): types over the runtime the host puts in
// the view (`window.architect`, `features/extensions/src/webview/runtime.js`).
// In a sandboxed frame no host can put a script, so importing this module
// installs the same runtime itself (`webview-runtime.js`); in a native web
// view the host's copy is there first and this one steps aside. See
// webview.d.ts.
import './webview-runtime.js';

/** The bridge; `undefined` outside an Architect web view. */
export const architect = typeof window !== 'undefined' ? window.architect : undefined;

/** The bridge, or an error saying the page is not in an Architect web view. */
export function requireArchitect() {
  if (!architect) {
    throw new Error('not in an Architect web view: window.architect is missing (open it as an extension\'s kind = "web" view)');
  }
  return architect;
}
