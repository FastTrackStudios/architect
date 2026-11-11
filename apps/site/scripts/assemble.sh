#!/usr/bin/env bash
# The deploy, as Vercel's prebuilt output (Build Output API v3), from two
# builds made before it:
#
#   apps/site/.output/public   this app, prerendered (`bun run build`)
#   $DOCS_DIR                  docs/content, built by
#                              `architect site build --base /docs/ --url <site>/docs/`
#
# into OUT (default apps/site/.vercel/output):
#
#   OUT/config.json            version 3, the routes (scripts/vercel-config.ts):
#                              clean URLs, the /app and /s/* redirects to the node,
#                              the 404 page
#   OUT/static/                the app's pages and assets
#   OUT/static/docs/           the docs
#
# then `vercel deploy --prebuilt` uploads OUT as it is (site.yml). Nothing
# here needs a network, a server or the cluster.
#
#   DOCS_DIR=target/site bash apps/site/scripts/assemble.sh [OUT]
set -euo pipefail
site="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-$site/.vercel/output}"
app="$site/.output/public"
docs="${DOCS_DIR:?DOCS_DIR: the docs build (architect site build --base /docs/)}"

[ -s "$app/index.html" ] || { echo "assemble: $app/index.html is missing: run bun run build" >&2; exit 1; }
[ -s "$docs/index.html" ] || { echo "assemble: $docs/index.html is missing: build the docs first" >&2; exit 1; }

rm -rf "$out"
mkdir -p "$out/static"
cp -r "$app/." "$out/static/"
# The Nitro server build (.output/server) is never deployed: every page
# was prerendered.
rm -rf "$out/static/docs"
cp -r "$docs/." "$out/static/docs/"
(cd "$site" && bun scripts/vercel-config.ts "$out")

for f in index.html download.html changelog.html extensions.html 404.html docs/index.html config.json; do
    p="$out/static/$f"; [ "$f" = config.json ] && p="$out/$f"
    [ -s "$p" ] || { echo "assemble: $f is missing" >&2; exit 1; }
done
echo "assemble: $out ($(du -sh "$out" | cut -f1), $(find "$out/static" -type f | wc -l) files)"
