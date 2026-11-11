#!/usr/bin/env bash
# The landing page's binaries, copied from where the current page keeps them
# (apps/web/landing: media/, assets/fonts/, assets/mark.svg) into public/ at
# build time, so they are in git once. public/media, public/fonts and
# public/mark.svg are ignored (.gitignore). Run by `bun run dev` and
# `bun run build`.
#
# When apps/web/landing is retired (#534 step 7), point SRC at the orphan
# `media` branch's files instead (#133).
set -euo pipefail
site="$(cd "$(dirname "$0")/.." && pwd)"
src="${LANDING_DIR:-$site/../web/landing}"
pub="$site/public"
for need in "$src/media" "$src/assets/fonts" "$src/assets/mark.svg"; do
    [ -e "$need" ] || { echo "assets: $need is missing" >&2; exit 1; }
done
mkdir -p "$pub"
rm -rf "$pub/media" "$pub/fonts"
cp -r "$src/media" "$pub/media"
mkdir -p "$pub/fonts"
cp "$src"/assets/fonts/*.woff2 "$src"/assets/fonts/*.txt "$src"/assets/fonts/*.md "$pub/fonts/"
cp "$src/assets/mark.svg" "$pub/mark.svg"
echo "assets: $(find "$pub/media" "$pub/fonts" -type f | wc -l) files from ${src#"$site/"}"
