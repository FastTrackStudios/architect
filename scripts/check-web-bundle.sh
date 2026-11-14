#!/usr/bin/env bash
# Checks a built web bundle (scripts/web-bundle.sh) the way a server serves
# it: behind apps/site/scripts/serve.ts, which applies exactly the rules in
# scripts/web-routes.md (the bundle's ROUTES.md), and nothing more.
#
#   bash scripts/check-web-bundle.sh BUNDLE [PORT]     # needs bun and curl
#
# - the web app's code under the assets/app-<sha12>/ start.js names
# - the files the server's package asserts exist (index.html, 404.html,
#   app/index.html, app/config.json, docs/index.html), and ROUTES.md
# - /, /download, /changelog, /extensions, /docs/, /app/ answer 200 with
#   their own page; an unknown path answers 404 with the site's 404 page
# - the landing page is the site's (its header carries the mark's path),
#   not the old apps/web/landing page (`hero-plan`)
# - /download links a release's Linux build (by convention: the browser
#   checks it against GitHub's API) and /changelog lists a release
# - what a share link's page loads from the bundle (#646: the node answers
#   /s/<code>/ with its copy of app/index.html, which loads /app/start.js
#   and /app/style.css from here), and /s/ is never the bundle's: a 404
#   here, so a proxy that forgot to send /s to the node shows
# Run by site.yml on a pull request and by release.yml's `web` job.
set -euo pipefail
bundle="$(cd "${1:?usage: check-web-bundle.sh BUNDLE [PORT]}" && pwd)"
port="${2:-4179}"
root="$(cd "$(dirname "$0")/.." && pwd)"
fail() { echo "check-web-bundle: $*" >&2; exit 1; }

for f in index.html 404.html app/index.html app/config.json docs/index.html ROUTES.md; do
    [ -s "$bundle/$f" ] || fail "$f is missing"
done

bun "$root/apps/site/scripts/serve.ts" "$bundle" "$port" >/dev/null 2>&1 &
pid=$!
trap 'kill $pid 2>/dev/null || true' EXIT
base="http://127.0.0.1:$port"
for _ in $(seq 1 50); do curl -s -o /dev/null "$base/" && break; sleep 0.1; done

tmp="$(mktemp -d)"
trap 'kill $pid 2>/dev/null || true; rm -rf "$tmp"' EXIT
# get PATH STATUS: fetches into $tmp/body, fails on another status.
get() {
    local code
    code="$(curl -s -o "$tmp/body" -w '%{http_code}' "$base$1")"
    [ "$code" = "$2" ] || fail "$1 answered $code, not $2"
}
has() { grep -aq -- "$2" "$tmp/body" || fail "$1: no $2"; }

# The mark's small cut (apps/site/src/components/ui/brand-mark.tsx MARK.small).
mark='M4 1h8v2H4ZM5.5 4h5L13 12H3Z'
get / 200
has / "$mark"
! grep -aq hero-plan "$tmp/body" || fail "/ is the old landing page (hero-plan)"
get /download 200
has /download '<title>Download'
has /download '/releases/download/v[^/"]*/Architect-linux-x86_64.AppImage'
get /changelog 200
has /changelog '<title>Changelog'
has /changelog 'releases/tag/v'
get /extensions 200
has /extensions '<title>Extensions'
get /docs/ 200
get /app/ 200
has /app/ '/app/start.js'
get /app/config.json 200
get /app/style.css 200
# A share link is the node's (scripts/web-routes.md): never a page here.
get /s/abcdefghjkmnpqrs/ 404
# The app's code is under the release's assets/app-<sha12>/ (cached for
# good), named by start.js (never cached).
get /app/start.js 200
app_assets="$(grep -ao "/assets/app-[0-9a-f]\{12\}/" "$tmp/body" | head -1)"
[ -n "$app_assets" ] || fail "/app/start.js names no /assets/app-<sha12>/"
get "${app_assets}architect_web_bg.wasm" 200
get "${app_assets}architect_web.js" 200
get "${app_assets}Inter-Variable.woff2" 200
get /no/such/page 404
has /no/such/page "$mark"
# A page's URL with a trailing slash is not the page (only folders take one).
get /download/ 404
echo "check-web-bundle: $bundle serves /, /download, /changelog, /extensions, /docs/, /app/, a share page's script and a 404"
