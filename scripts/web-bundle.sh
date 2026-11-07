#!/usr/bin/env bash
# The web front end as one static site (#431), into OUT:
#
#   OUT/            the landing page: apps/web/landing copied as is (a
#                   self-contained static site), and the shared look and
#                   404 (apps/web/site)
#   OUT/docs/       docs/content, built by `architect site build --base /docs/`
#   OUT/app/        the web app (apps/web): its page, its wasm, config.json
#
# What `just web` serves, the release attaches as architect-web-<v>.tar.gz,
# and flake.nix's architect-web-image puts behind static-web-server.
#
#   scripts/web-bundle.sh OUT            # builds what it needs (dev shell)
#
# Prebuilt pieces skip their build (flake.nix passes all three):
#   ARCHITECT_WEB_WASM   architect_web.wasm (wasm32-unknown-unknown, release)
#   ARCHITECT_CLI        the `architect` CLI binary (apps/cli)
#   WASM_BINDGEN         a wasm-bindgen at Cargo.lock's version
# ARCHITECT_WEB_URL      the site's public URL (default https://architect.fasttrackstudio.app)
set -euo pipefail
out="${1:?usage: web-bundle.sh OUT}"
root="$(cd "$(dirname "$0")/.." && pwd)"
url="${ARCHITECT_WEB_URL:-https://architect.fasttrackstudio.app}"
url="${url%/}"

wasm="${ARCHITECT_WEB_WASM:-}"
if [ -z "$wasm" ]; then
    (cd "$root" && cargo build -q --release -p architect-web --target wasm32-unknown-unknown)
    target="$(cd "$root" && cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
    wasm="$target/wasm32-unknown-unknown/release/architect_web.wasm"
fi

bindgen="${WASM_BINDGEN:-}"
if [ -z "$bindgen" ]; then
    want=$(awk '/^name = "wasm-bindgen"$/{getline; gsub(/[^0-9.]/,""); print; exit}' "$root/Cargo.lock")
    bindgen="$root/target/tools/bin/wasm-bindgen"
    if ! "$bindgen" --version 2>/dev/null | grep -q "$want"; then
        cargo install -q wasm-bindgen-cli --version "$want" --locked --root "$root/target/tools"
    fi
fi

rm -rf "$out"
mkdir -p "$out/app"
out="$(cd "$out" && pwd)"
cp "$root"/apps/web/site/* "$out/"
# The landing page, as is.
cp -r "$root/apps/web/landing/." "$out/"
cp "$root"/apps/web/app/* "$out/app/"
"$bindgen" --target web --no-typescript --out-dir "$out/app" "$wasm"

cli="${ARCHITECT_CLI:-}"
if [ -n "$cli" ]; then
    run_cli() { "$cli" "$@"; }
else
    run_cli() { (cd "$root" && cargo run -q -p architect-cli -- "$@"); }
fi
run_cli site build --vault "$root/docs/content" --out "$out/docs" --base /docs/ \
    --extension "$root/apps/sandbox/board" --url "$url/docs/" >/dev/null

for f in index.html 404.html app/index.html app/config.json app/architect_web_bg.wasm docs/index.html; do
    [ -s "$out/$f" ] || { echo "web-bundle: $f is missing" >&2; exit 1; }
done
echo "web-bundle: $out ($(du -sh "$out" | cut -f1))"
