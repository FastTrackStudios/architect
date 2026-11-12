#!/usr/bin/env bash
# The web front end as one static site (#431, #534), into OUT:
#
#   OUT/            the site (apps/site, every page prerendered): the
#                   landing page (index.html), download.html,
#                   changelog.html, extensions.html (+ extensions/<id>.html),
#                   404.html, their hashed assets/ and fonts/
#   OUT/docs/       docs/content, built by `architect site build --base /docs/`
#   OUT/app/        the web app (apps/web): its page, its wasm and the JS
#                   wasm-bindgen writes beside it (architect_web.js, and
#                   snippets/: dioxus-web's interpreter), config.json, its
#                   face (Inter-Variable.ttf, OFL: Inter-OFL.txt)
#   OUT/ROUTES.md   how a server must serve OUT (scripts/web-routes.md)
#
# What `just web` serves, the release attaches as architect-web-<v>.tar.gz
# (the one deployment: the VPS's static file server serves it), and
# flake.nix's architect-web-image puts behind static-web-server. Nothing in
# it needs a Node or Nitro server at run time.
#
#   scripts/web-bundle.sh OUT            # builds what it needs (dev shell)
#
# Prebuilt pieces skip their build (flake.nix passes all of them):
#   ARCHITECT_WEB_WASM   architect_web.wasm (wasm32-unknown-unknown, profile `web`)
#   ARCHITECT_CLI        the `architect` CLI binary (apps/cli)
#   WASM_BINDGEN         a wasm-bindgen at Cargo.lock's version
#   ARCHITECT_SITE       apps/site's prerendered output (its .output/public)
# and two more for a check without Rust (site.yml: a release's bundle's):
#   ARCHITECT_WEB_BOUND  a folder with wasm-bindgen's output (architect_web.js,
#                        architect_web_bg.wasm, snippets/ when it has one),
#                        instead of the wasm and bindgen
#   ARCHITECT_DOCS       a built docs folder, instead of the CLI
# Otherwise the site is built here with bun (`bun install --frozen-lockfile`,
# `bun run build`), from a bun and node on the PATH or nixpkgs' (nix shell).
#
# ARCHITECT_RELEASE_TAG  the release this bundle is for (vX.Y.Z; release.yml
#                        sets it): the site's release data (/download,
#                        /changelog) is fetched for THAT tag and any failure
#                        fails the build (apps/site/scripts/data.ts, strict):
#                        the release and its notes, never its binaries
#                        (/download names them by convention and the browser
#                        checks them), so nothing waits for the other jobs.
#                        Unset, the site uses its committed data
#                        (apps/site/src/data), which must not be empty.
# ARCHITECT_WEB_URL      the site's public URL (default https://architect.fasttrackstudio.app)
# ARCHITECT_DISTRIBUTION which distribution it is (#491): what its binary's
#                        `distribution` printed, served as
#                        /app/distribution.json (its name and its compiled-in
#                        extensions, which the app shows); none, the Architect
#                        app with none listed. `cargo xtask dist web` sets it.
set -euo pipefail
out="${1:?usage: web-bundle.sh OUT}"
root="$(cd "$(dirname "$0")/.." && pwd)"
url="${ARCHITECT_WEB_URL:-https://architect.fasttrackstudio.app}"
url="${url%/}"
# The nixpkgs bun and node come from when the PATH has none (the dev shell
# has neither).
nixpkgs=github:NixOS/nixpkgs/e62746b0b472753ef019baceab6f1e2e52014980

# The site first: it is the cheapest piece and the likeliest to refuse.
site="${ARCHITECT_SITE:-}"
if [ -z "$site" ]; then
    build_site() {
        set -euo pipefail
        bun install --frozen-lockfile
        if [ -n "${ARCHITECT_RELEASE_TAG:-}" ]; then
            SITE_RELEASE_TAG="$ARCHITECT_RELEASE_TAG" bun run data
        fi
        bun run build
    }
    export -f build_site
    if command -v bun >/dev/null && command -v node >/dev/null; then
        (cd "$root/apps/site" && build_site)
    else
        (cd "$root/apps/site" && nix shell "$nixpkgs#bun" "$nixpkgs#nodejs_22" -c bash -c build_site)
    fi
    site="$root/apps/site/.output/public"
elif [ -n "${ARCHITECT_RELEASE_TAG:-}" ]; then
    echo "web-bundle: ARCHITECT_RELEASE_TAG needs the site built here, not ARCHITECT_SITE" >&2
    exit 1
fi
# The data the site was built from (fetched for the tag, or committed):
# a bundle that names no release offers nothing to download.
data="$root/apps/site/src/data"
grep -q '"tag": "v' "$data/release.json" \
    || { echo "web-bundle: apps/site/src/data/release.json names no release" >&2; exit 1; }
grep -q '"tag": "v' "$data/changelog.json" \
    || { echo "web-bundle: apps/site/src/data/changelog.json lists no release" >&2; exit 1; }
if [ -n "${ARCHITECT_RELEASE_TAG:-}" ]; then
    grep -q "\"tag\": \"$ARCHITECT_RELEASE_TAG\"" "$data/release.json" \
        || { echo "web-bundle: release.json is not $ARCHITECT_RELEASE_TAG's" >&2; exit 1; }
fi

bound="${ARCHITECT_WEB_BOUND:-}"
wasm="${ARCHITECT_WEB_WASM:-}"
if [ -z "$bound" ] && [ -z "$wasm" ]; then
    # The `web` profile (Cargo.toml): size first, what the browser fetches.
    (cd "$root" && cargo build -q --profile web -p architect-web --lib --target wasm32-unknown-unknown)
    target="$(cd "$root" && cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
    wasm="$target/wasm32-unknown-unknown/web/architect_web.wasm"
fi

bindgen="${WASM_BINDGEN:-}"
if [ -z "$bound" ] && [ -z "$bindgen" ]; then
    want=$(awk '/^name = "wasm-bindgen"$/{getline; gsub(/[^0-9.]/,""); print; exit}' "$root/Cargo.lock")
    bindgen="$root/target/tools/bin/wasm-bindgen"
    if ! "$bindgen" --version 2>/dev/null | grep -q "$want"; then
        cargo install -q wasm-bindgen-cli --version "$want" --locked --root "$root/target/tools"
    fi
fi

rm -rf "$out"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
# The site's pages and assets at the root (no docs/ or app/ of its own).
cp -r "$site/." "$out/"
chmod -R u+w "$out"
rm -rf "$out/docs" "$out/app"
cp "$root/scripts/web-routes.md" "$out/ROUTES.md"
mkdir -p "$out/app"
cp "$root"/apps/web/app/* "$out/app/"
# The app's face, as the desktop app's (apps/ui/assets/fonts).
cp "$root/apps/ui/assets/fonts/Inter-Variable.ttf" "$out/app/"
cp "$root/apps/ui/assets/fonts/OFL.txt" "$out/app/Inter-OFL.txt"
if [ -n "${ARCHITECT_DISTRIBUTION:-}" ]; then
    cp "$ARCHITECT_DISTRIBUTION" "$out/app/distribution.json"
fi
if [ -n "$bound" ]; then
    cp "$bound/architect_web.js" "$bound/architect_web_bg.wasm" "$out/app/"
    if [ -d "$bound/snippets" ]; then
        cp -r "$bound/snippets" "$out/app/"
    fi
else
    "$bindgen" --target web --no-typescript --out-dir "$out/app" "$wasm"
fi

if [ -n "${ARCHITECT_DOCS:-}" ]; then
    cp -r "$ARCHITECT_DOCS" "$out/docs"
else
    cli="${ARCHITECT_CLI:-}"
    if [ -n "$cli" ]; then
        run_cli() { "$cli" "$@"; }
    else
        run_cli() { (cd "$root" && cargo run -q -p architect-cli -- "$@"); }
    fi
    run_cli site build --vault "$root/docs/content" --out "$out/docs" --base /docs/ \
        --extension "$root/apps/sandbox/board" --url "$url/docs/" >/dev/null
fi

# What the server's package asserts, and the pages the site links.
for f in index.html 404.html app/index.html app/config.json app/architect_web_bg.wasm docs/index.html \
    download.html changelog.html extensions.html; do
    [ -s "$out/$f" ] || { echo "web-bundle: $f is missing" >&2; exit 1; }
done
echo "web-bundle: $out ($(du -sh "$out" | cut -f1))"
