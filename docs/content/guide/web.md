+++
title = "The web front end"
description = "architect.fasttrackstudio.app: the site, the docs and the web app as one static bundle the server serves, signed in with the issuer's redirect flow and PKCE."
weight = 76
+++

Architect's web front end is a static site, apart from the headless node
([deploy](@/guide/deploy.md)). Each side has its own image, Deployment and
replica count, and one host serves both: a reverse proxy sends the node's
paths to the node and everything else to the front end.

| Path | What | From |
| --- | --- | --- |
| `/`, `/download`, `/changelog`, `/extensions` | the site | `apps/site`, prerendered ([the site](@/guide/shipping.md)) |
| `/docs/` | these docs | `docs/content`, built by `architect site build --base /docs/` |
| `/app/` | the web app | `apps/web` (wasm) and `apps/web/app` |
| `/auth/callback` | where the issuer sends the browser back (the app's page) | `apps/web/sws.toml` |

**Where the app lives.** The app is always at `/app/`, signed in or not,
and `/` is always the landing page, which links to it. One fixed place
keeps the routing static: no server decides by cookie, and a bookmark of
`/app/` always opens the app.

**Not a single-page fallback.** An unknown path is a real `404`
(the site's `404.html`), never the app's page answering `200`. A node
path the proxy forgot to route then fails visibly instead of returning
HTML (the trap Task's routing records).

## How it is served

The bundle is plain files, served by one rule: the path, then
`<path>.html`, then `<path>/index.html`, else `404.html` with a 404.
`scripts/web-routes.md` (in every bundle as `ROUTES.md`) is the whole
contract a server must keep, with a Caddy example: the node's paths and
`/auth/callback` ahead of the files, and a long cache only for the
site's content-hashed `assets/`. `apps/web/sws.toml` does the same for
static-web-server (`just web`, the `architect-web` image), which needs
each page's clean URL listed as a rewrite.

## Build and run it

```sh
just web            # builds target/web-bundle, serves it on http://localhost:8080
just web-test       # the same bundle in a headless Firefox (apps/web/test/web.test.mjs)
```

To work on the app, not the bundle: `just dev plain` (or `devenv up
web`) serves it under `dx serve` at `http://127.0.0.1:7413/app/`,
rebuilt on a save, signed in at the local issuer
([the local stack](@/guide/local-stack.md)).

`scripts/web-bundle.sh` builds the bundle: the site (`apps/site`, with
bun), the app's wasm (`wasm32-unknown-unknown`, release) through
`wasm-bindgen` at `Cargo.lock`'s version, and the docs. `just web` serves
it with static-web-server and the image's own config (`apps/web/sws.toml`),
so local routing is the deployed routing. Port 8080 is a redirect the
issuer knows (`http://localhost:8080/auth/callback`), so signing in works
locally against the real issuer.

## Signing in

The app signs in with the issuer's redirect flow and PKCE, as the public
client `architect` (`apps/web/src/lib.rs` `Config`, `route`;
`features/auth/auth-client/src/oidc.rs` `Pkce`, `authorize_url`). The
desktop app uses the same client id, over the device grant
(`apps/ui/src/accounts.rs`).

1. **Sign in** sends the browser to `<issuer>/oauth2/authorize` with an
   S256 challenge and asks for `openid email profile offline_access`. The
   verifier and the state wait in `sessionStorage`: one attempt, one tab.
2. The issuer sends the browser back to `/auth/callback`. The app checks
   the state, redeems the code at `<issuer>/oauth2/token`, keeps the
   session in `localStorage`, and moves to `/app/`.
3. Before each start the session is renewed with its refresh token when it
   is about to run out (`oidc::refresh_if_needed`). A token the issuer
   refuses is renewed once, then the person signs in again. The refresh
   token is single use (the client `architect` is registered with
   `rotate_refresh_tokens`): each renewal answers a new one, and the old
   one presented again more than 10 seconds later revokes the whole
   sign-in (a copy was taken). Within those seconds (two tabs renewing at
   once, a response lost on the way) it answers the same new one, so the
   sign-in never splits in two.
4. **Sign out** revokes the sign-in at the issuer (`/oauth2/revoke`,
   RFC 7009; `apps/web/src/browser.rs` `revoke`), then forgets it here. Its
   refresh token mints nothing more and its access tokens are refused
   where the issuer takes them; the issuer session the person signed in
   with stays, so other apps signed in from it are not signed out.

The issuer must know the client. For architect.fasttrackstudio.app that is
fts-auth's `appDomains` entry `architect.fasttrackstudio.app`, which
registers the public client `architect`, its redirect
`https://architect.fasttrackstudio.app/auth/callback` and CORS for the
origin. A self-hosted issuer needs the same client.

## Its settings

`/app/config.json` holds what differs between deployments, read when the
app starts, so one bundle serves any host:

```json
{ "issuer": "https://auth.fasttrackstudio.app", "client_id": "architect" }
```

The chart (`deploy/chart/architect`, `web.config`) replaces it with a
ConfigMap. Outside Kubernetes, edit
the file in the bundle.

## What the app does today

**The demo** (`/app/?demo`, the landing page's Demo button) is the app
itself, the same components the desktop and phone apps draw with, on
dioxus-web (`apps/web/src/app/mod.rs`): the shell, the Vault's workspace
(tabs, splits, docks; a phone's bar, drawer and tab switcher), the
navigator over the demo vault (`apps/ui/demo-vault`, built in), ⌘K and ⌘O,
and the settings page. It runs in the tab's memory: nothing is signed in,
nothing is sent, a reload starts again. Notes are edited as their markdown
in a text area: the editor's web view runs JavaScript through `eval`,
which the page's Content-Security-Policy refuses (`style-src` allows
inline styles, the components' own; `script-src` is `'self'
'wasm-unsafe-eval'` only). Settings are kept in this browser's
`localStorage`. Pop-out windows, folders on the device, the files agent
and extensions (#495) are not in the browser.

**Signed in**, when this host's node hosts the account's org (its status
names the org), the app is the same app on the org's vaults: it dials the
node's org lane, `/org/{slug}/vox` on this origin, with its session as the
WebSocket's `vox.bearer.<token>` subprotocol (never in a URL), lists the
vaults' notes (`vault_proto::notes` `Notes`, each vault a folder), reads
them in the background so the navigator fills its previews, and keeps an
open note live with the node (`crdt::sync::SyncedDoc` on
`vault_proto::collab_doc_id`): typing reaches the node's document and
file, and another's typing shows as it comes.

- **The bearer** is the issuer's short-lived access token: the web app is
  the public client `architect`, which gets no issuer session. The node
  hands it to the issuer's `whoami` and org list, which take a
  first-party client's access token (one registered `first_party`, with
  `openid`, the default audience; `skip_consent` alone is not enough) as
  well as a session; the node
  verifies no JWT itself. Before the token runs out the app renews it and
  dials the lane again (`apps/web/src/browser.rs` `keep_lane_signed_in`,
  `apps/web/src/app/org.rs` `OrgVaults::redial`); every open note's
  session attaches on the new lane.
- **The node trusts a bearer for 30 seconds** (`lane_recheck_secs`,
  `ARCHITECT_LANE_RECHECK_SECS`; `apps/desktop/src/node/org.rs`
  `IssuerSessions`), then asks the issuer again. An open note's live
  session checks its bearer as often and ends once it is refused: signed
  out, revoked, expired or no longer a member, it ends within about a
  minute, not when the connection happens to drop (`VaultSync`). The
  bearer checked is the one the session's call was admitted with, sent on
  the connection or with the call. When the issuer cannot be reached an
  open note is kept for 5 minutes after its last good check
  (`lane_issuer_grace_secs`, `ARCHITECT_LANE_ISSUER_GRACE_SECS`), then
  ends; a new call is refused meanwhile.
- **Only the app's own origin opens the lane from a browser.** A WebSocket
  upgrade with any other `Origin` is refused (403); one without `Origin`
  (another node, the desktop app) goes on to the bearer check. A web app
  served from another origin is listed in `ARCHITECT_LANE_ORIGINS`
  (`apps/desktop/src/node/org.rs` `origin_allowed`).
- **A note may be typed in once its session has attached**, when the
  node's copy is in the document; until then the text area is read-only
  and says it is connecting. The text area is not a controlled input:
  another's edit is written into it with the caret kept, never during an
  IME composition. Typing is read back as a splice of the text the text
  area last showed and handed to Loro at the document version it showed,
  so Loro merges it with whatever arrived since, as it merges any two
  peers (`apps/web/src/live_text.rs` `LiveText`, `merge_at`). An edit that
  does not apply is said in the note.
- **A new note** goes in a vault (the open note's, else the first) and is
  made with `Notes::create` before it opens. That refuses a path a note
  holds already, so a note someone else made since the listing is never
  emptied: the next `Untitled N` is tried. Renaming, moving and deleting
  notes, and their tags and properties, are read-only in the browser for
  now (`apps/web/src/read_only.rs` `NodeLayout`): the navigator says so
  when they are tried, from its menu, its keys, the palette or a drag.
- **What fails is shown**: a note that cannot be read says why, and a
  failed background read, new note, lost session or ended sign-in is in
  the status line.
- **The proxy's access log must not record the bearer.** A WebSocket has
  no other header to carry it in, and a reverse proxy logs
  `Sec-WebSocket-Protocol` unless told not to (Caddy redacts
  `Authorization` and cookies by default, not this header). Delete or
  redact that request header in the proxy's log format. The node itself
  never logs it, not even at `RUST_LOG=trace`: every host's log filter
  holds the WebSocket library, which dumps frames, at INFO
  (`crates/architect/src/host.rs` `log_filter`;
  `crates/architect/tests/host_mounts.rs`
  `the_host_never_logs_a_browser_bearer`, `apps/desktop/tests/node.rs`
  `the_node_never_logs_a_browser_bearer_even_at_trace`).

Nothing is kept in the browser between visits: a reload lists and reads
again (no IndexedDB or OPFS copy yet), and with the node out of reach the
app says why. A node that names no org, or a lane that refuses the
session (not a member), leaves the status page: who is signed in, the
node's status over same-origin HTTP (`POST /node/status`,
`apps/desktop/src/node/service.rs` `Node`), why the vaults did not open,
and links to the demo, the docs and the desktop download.

`just web-org-test` drives all of this in a headless Firefox against the
running local stack (`apps/web/test/org-vaults.test.mjs`).

The sandbox's browser demo, which used to be `apps/web`, is
`apps/sandbox/web` (`just sandbox-web`).

## The image

`flake.nix` builds `architect-web-image` the way it builds the node's,
with `dockerTools.streamLayeredImage` and no Docker. The pure build
compiles the wasm and the `architect` CLI in the Nix sandbox and runs the
same `scripts/web-bundle.sh`. `architect-web-image-prebuilt` wraps a
bundle built in the dev shell (`ARCHITECT_WEB_BUNDLE`, `--impure`). The
image runs static-web-server on `8080/tcp` with the site in `/srv/www` as
real files (so a ConfigMap can replace `/srv/www/app/config.json`) and
`/srv/www/version.json` naming the build.

```sh
nix build .#architect-web-image
./result | nix run nixpkgs#skopeo -- copy docker-archive:/dev/stdin docker-daemon:architect-web:dev
```

Nothing is cached (`Cache-Control: no-store`): every file in the image has
the store's 1970 mtime, so revalidating by `Last-Modified` would keep an
old release's files.
