# Extensions in the web app: handoff notes (draft, #495)

Nothing of the host is built yet. This file is the starting point for whoever builds it; delete it once the host lands.

## The owner, 2026-11-12

- "The web version of Architect, we need to get extensions running on there ASAP, that's important."
- "The 'share pages' shouldn't be a thing, it's supposed to be on the Architect app itself": a share link opens Architect (the installed app, or the hosted web app as a guest of the link), not a node-rendered page (#629).
- "Since it's just wasm anyway, the Studio extension should be able to have the exact same performance compiled in or not, because it's just compiled as a separate wasm."

## Routes to compare (decide with measurements, write it on #495)

- **A. The frame route**, generalising #608's web view from "a share page's island" to "an extension's views in the app". Each packaged extension is a wasm-bindgen bundle (`web-*.js|wasm`, pinned by digest) in its own sandboxed frame. The app gives it a box, theme tokens and props; its host calls go over a `MessagePort` to the app, which answers them from the node session. It runs on every browser the app does (no JSPI), it's the same package in an iOS WKWebView later, and it keeps third-party code out of the app's origin.
- **B. Linked into the app's wasm** (#495 as written): `linked::Extension` without `host`, with a browser host answering the host service. One wasm and one Dioxus tree, but every core extension change rebuilds the web app, and each extension has to build without Luau and tokio.
- **C. The WASI runner** (`features/extensions/web`) with an Asyncify build for browsers without JSPI.

Expectation: A for packaged and third-party extensions (it unblocks Studio's existing bundles at once), B only where a view can't live in a frame.

## What exists and works today (merged, v0.12.6)

- The frame (#608):
  - `features/sites/assets/webview.js`: the frame's one script. It removes WebRTC by script, and through `architectPage` exposes `subscribe` (props) and `ask` (intents out).
  - `features/sites/src/island.rs` `webview_csp` and `webview_headers`: `sandbox allow-scripts`, `connect-src 'none'`, `frame-src 'none'`, `frame-ancestors 'self'`.
- The page side: `features/sites/assets/player.js` `webPane`.
  - The protocol: `hello` → `theme` + `boot` (the files, each checked against its SHA-256 by the page) → `ready`, then `props`, `ping`/`pong` liveness (2 s beat, 6 s late, 15 s to ready), `size`, `ask`, `error`.
  - A frame that loads a second document dies, and the page's own control comes back.
- The node serves islands under `/_ext/` (`apps/desktop/src/node/shares.rs` `islands`, `architect_sites::island::serve`).
  - A share page gets the pinned listing embedded (`architect_sites::island::page_json`).
  - The app has no listing to fetch yet. The first host PR should add `GET /_ext/islands.json` (`page_json` with `no-cache`); `/_ext` is already routed to the node beside `/app/`.
- The attack suite: `apps/web/test/share-webview.test.mjs` (Firefox BiDi; `DRIVER=safari`), to adapt for the app host.

## Measured so far

- Studio's island web bundle, built from Studio main `8b06bc26` with `just island-web` (`apps/architect/island`, Dioxus web, `mount(view)` for `progress`, `transport` or `lyrics`):

  | File | Raw | gzip -9 |
  |---|---|---|
  | `web-studio-island_bg.wasm` | 833,732 B | 335,331 B |
  | `web-studio-island.js` | 78,177 B | 11,724 B |
  | 3 snippets | 19,296 B | 4,370 B |
  | **Whole bundle** | | **352 KB** |

- **Not measured yet:** mount time in a frame, the port round trip per call, memory per frame.

## Security, carried over from the #608 reviews, plus what is new in the app

- Opaque-origin frame (header CSP `sandbox` and the iframe `sandbox`); no network from the frame (`connect-src 'none'`; WebRTC removed by script, since no engine enforces a `webrtc` directive); files pinned by digest; once-guards; the death and liveness fallback; the frame can't navigate the app or itself off the node; rate-limited calls.
- A guest of a share gets exactly the share's scope.
- New in the app:
  - The frame never receives the session token or any credential.
  - Host calls are authorised in the app and the node, never trusted from the frame.
  - A member's vault writes need the extension's capability and are attributed.
  - The app's CSP gains `frame-src 'self'` (see `scripts/web-routes.md` for the policy).

## Build against

- #629: the guest session is a read-only host service, `ShareGuest` (`POST /s/<code>/_guest/<method>`, `ShareGuestHttpClient`, `Principal::Guest { link_id }`), being built by another thread (PR #632). The view a note gets is chosen by claims (`Embeds::resolve`), never by a list of types.
- #631: core extensions are a curated list by reference (`core-extensions.toml`); extensions stay in their own repos; each has a packaged or linked form per platform.

## Layers proposed

1. The host: `/_ext/islands.json`; mount and unmount; sizing; theme; focus and keyboard; liveness; grants.
2. The host service over the port: the minimal read set first, checked against the extension's declared capabilities and the session's rights (guest or member).
3. An extension's view for a note type in a workspace tab, and its actions in the palette.
4. The ask to Studio: one bundle per view, or one bundle with several mount points (as its island does).
