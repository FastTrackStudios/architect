#!/usr/bin/env bash
# The web front end as one static site (#431, #534), into OUT:
#
#   OUT/            the site (apps/site, every page prerendered): the
#                   landing page (index.html), download.html,
#                   changelog.html, extensions.html (+ extensions/<id>.html),
#                   404.html, their hashed assets/ and fonts/
#   OUT/docs/       docs/content, built by `architect site build --base /docs/`
#   OUT/app/        the web app (apps/web): its page (index.html, style.css),
#                   start.js (written here), config.json; never cached
#   OUT/assets/app-<sha12>/  its code and face, named by their content and
#                   so cached for good under the site's /assets/ rule: the
#                   wasm and the JS wasm-bindgen writes beside it
#                   (architect_web.js, snippets/: dioxus-web's
#                   interpreter), its face (Inter-Variable.woff2, OFL:
#                   Inter-OFL.txt). start.js imports from there and puts
#                   the folder on <html data-assets> for the app
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
#                        instead of the wasm and bindgen: a release bundle's
#                        app/ (an older one holds them there, a newer one in
#                        ../assets/app-*/; either is found)
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
if [ -n "${ARCHITECT_DISTRIBUTION:-}" ]; then
    cp "$ARCHITECT_DISTRIBUTION" "$out/app/distribution.json"
fi
# The app's code and face, staged, then put under a folder named by their
# content: a new build is a new folder, an old one stays cached as it was.
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
if [ -n "$bound" ]; then
    from="$bound"
    if [ ! -f "$from/architect_web.js" ]; then
        from="$(find "$bound/../assets" -maxdepth 1 -type d -name 'app-*' 2>/dev/null | head -1)"
    fi
    [ -n "$from" ] && [ -f "$from/architect_web.js" ] \
        || { echo "web-bundle: no architect_web.js in $bound" >&2; exit 1; }
    cp "$from/architect_web.js" "$from/architect_web_bg.wasm" "$stage/"
    if [ -d "$from/snippets" ]; then
        cp -r "$from/snippets" "$stage/"
    fi
else
    "$bindgen" --target web --no-typescript --out-dir "$stage" "$wasm"
fi
cp "$root/apps/web/assets/Inter-Variable.woff2" "$root/apps/web/assets/Inter-OFL.txt" "$stage/"
if command -v sha256sum >/dev/null; then sum() { sha256sum; }; else sum() { shasum -a 256; }; fi
hash="$( (cd "$stage" && find . -type f | LC_ALL=C sort | while read -r f; do printf '%s ' "$f"; sum < "$f"; done) | sum | cut -c1-12)"
assets="/assets/app-$hash/"
mkdir -p "$out$assets"
cp -r "$stage/." "$out$assets"
cat > "$out/app/start.js" <<EOF
// The web app's start (apps/web/app/index.html), written by
// scripts/web-bundle.sh: its code and face are under $assets, named by
// their content (cached for good); this file and the page never are.
import init from '${assets}architect_web.js';
document.documentElement.dataset.assets = '$assets';
init({ module_or_path: '${assets}architect_web_bg.wasm' });
EOF

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
for f in index.html 404.html app/index.html app/config.json app/start.js "${assets#/}architect_web_bg.wasm" docs/index.html \
    download.html changelog.html extensions.html; do
    [ -s "$out/$f" ] || { echo "web-bundle: $f is missing" >&2; exit 1; }
done
echo "web-bundle: $out ($(du -sh "$out" | cut -f1))"
