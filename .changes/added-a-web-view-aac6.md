---
"architect-desktop" = "minor"
"architect-sites" = "minor"
---

Added a web view for share islands, so an island's views run on every browser with no JS Promise Integration (Safari before 27 and every iOS browser included): an island's web-* files (wasm-bindgen --target web glue exporting mount(view), its modules and its wasm) are found by SiteIsland::new or named with SiteIsland::web, each file's digest is in site-islands (web.files), and each view runs in a frame of the node's web view document (architect_sites::island::webview_document, webview_path, webview_headers, document_headers: sandboxed, its boot script by hash, blob: modules, wasm, inline style, nothing to fetch or connect to), handed the bytes, the page's theme and the playhead, asking only the five playhead intents. An island's transport now sits in the now-playing view's controls row between Click and Loop. Security: the web view document allows inline style, which the island sandbox does not
