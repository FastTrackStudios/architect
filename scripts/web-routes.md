# How to serve this bundle

The Architect web bundle (`scripts/web-bundle.sh` in the architect repo,
released as `architect-web-<X.Y.Z>.tar.gz`) is plain static files. Nothing
in it needs a Node, Nitro or any other server at run time. This file is
copied into every bundle as `ROUTES.md`.

```
index.html        /             the landing page (apps/site)
download.html     /download
changelog.html    /changelog
extensions.html   /extensions
extensions/<id>.html  /extensions/<id>
404.html          any path with no file, with status 404
assets/           the site's scripts and styles, content-hashed names
fonts/            the site's font (Archivo) and its licence
favicon.svg, logo.svg
docs/             the docs, every page a folder with index.html (/docs/…/)
app/              the web app's page: index.html, start.js, style.css,
                  config.json (the server's own copy replaces it)
assets/app-<sha12>/  the web app's code and face, named by their content:
                  the wasm and its JS (architect_web.js, snippets/), its
                  font (Inter-Variable.woff2); start.js imports from here
ROUTES.md         this file
```

## The rules

For a request path P (percent-decoded, never resolving above the root),
serve the first that is a regular file:

1. `P`
2. `P.html`, only when P does not end in `/` (`/download` → `download.html`)
3. `P/index.html` (`/` → `index.html`, `/docs/` → `docs/index.html`, `/app/` → `app/index.html`)
4. otherwise `404.html` with status **404** (never 200: an unknown path is a real 404)

The query string plays no part. No other redirect or rewrite is part of
the bundle. In Caddy that is:

```
root * /srv/architect-web
try_files {path} {path}.html {path}/index.html
file_server
handle_errors 404 {
    rewrite * /404.html
    file_server
}
```

(Caddy's `try_files` matches only files unless a candidate ends in `/`,
so `/extensions` reaches `extensions.html` though `extensions/` is a
folder.) `apps/site/scripts/serve.ts` implements exactly these rules
(`bun apps/site/scripts/serve.ts <bundle> [port]`); `apps/web/sws.toml`
does the same for static-web-server (`just web`, the `architect-web`
image).

What the server adds around them, as before this bundle carried the site:

- the node's paths (`/s/`, `/_ext`, `/mcp`, the node's other prefixes) go
  to the node, before the static files;
- `/auth/callback` (and `/auth/callback/`) is served `app/index.html`, the
  sign-in's way back;
- `app/config.json` is the server's own.

The web app's page (`/app/`, `/app/index.html` and `/auth/callback`)
carries a Content-Security-Policy `<meta>`, and a server sends this one
as a header too (`frame-ancestors` holds only as a header), with its own
issuer's origin (`app/config.json`'s `issuer`):

```
Content-Security-Policy: default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'self' https://<issuer>; base-uri 'none'; form-action 'self'; object-src 'none'; frame-ancestors 'none'
```

Never `'unsafe-eval'`: the app's bundle still links wasm-bindgen's
`new Function` import and dioxus-web's `eval`, and this policy is what
stops them (the app answers its own evaluations without them,
`apps/web/src/app/page.rs`). `apps/web/sws.toml` sends it for
production's issuer.

The rest of the bundle sets no Content-Security-Policy. A server that adds one must
let the site's pages call GitHub's API (`connect-src 'self'
https://api.github.com`): `/download` and the landing page ask it which
of the release's files are up (`apps/site/src/lib/release-check.ts`);
blocked, they keep their links and lose only the check.

`/app/…` and `/s/…` are same-origin links in the site's pages: the site
and the node share one domain, nothing redirects them elsewhere.

## Not inferable from the files

The site was first built for Vercel. Of what its Vercel config did, this
is what a static server does not get from the files alone:

- clean URLs: `<name>.html` is served at `/<name>` (rule 2). Vercel served
  it there only; here `/download.html` answers too (same page, harmless).
- the 404 page with a 404 status (rule 4).
- a trailing slash after a page (`/download/`) is a 404, as it was on
  Vercel; only folders (`/`, `/docs/…/`, `/app/`) take one.
- `/app` and `/s/*` were 307 redirects to another host. Gone: same origin.

## Caching

`assets/*` names carry a content hash (`index-DnYs32fa.css`, the web
app's whole folder `assets/app-<sha12>/`): they may be cached for good
(`Cache-Control: public, max-age=31536000, immutable`). Nothing else may:
the pages, `fonts/`, `docs/` and `app/` (`index.html`, `start.js` and
`config.json` keep their names across releases; `start.js` names the
release's `assets/app-…/`) keep a short or no cache (`no-cache` or
`no-store`).
