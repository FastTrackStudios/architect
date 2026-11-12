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
   refuses is renewed once, then the person signs in again.

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

The app is its shell: sign-in, who is signed in, the node's status over
same-origin HTTP (`POST /node/status`,
`apps/desktop/src/node/service.rs` `Node`), and links to the docs and the
desktop download. The app's own views (the vault, projects, files) are
Blitz-native and not built for the browser yet. They come to `/app/` as
they are ported.

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
