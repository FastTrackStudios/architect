// The web view kind's SDK (#222): types over the runtime the host puts in
// the view (`window.architect`, `features/extensions/src/webview/runtime.js`).
// See webview.d.ts.

/** The bridge; `undefined` outside an Architect web view. */
export const architect = typeof window !== "undefined" ? window.architect : undefined;

/** The bridge, or an error saying the page is not in an Architect web view. */
export function requireArchitect() {
  if (!architect) {
    throw new Error("not in an Architect web view: window.architect is missing (open it as an extension's kind = \"web\" view)");
  }
  return architect;
}
