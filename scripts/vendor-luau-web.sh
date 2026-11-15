#!/usr/bin/env bash
# The browser's Luau (#812): luau-web's Asyncify build, vendored into
# apps/web/assets/luau-web/ from the npm tarball pinned below.
#
#   scripts/vendor-luau-web.sh          fetch, verify, write the files
#   scripts/vendor-luau-web.sh --check  verify the committed files are
#                                       exactly what the tarball gives
#
# Why these files and not a build of our own: luau-web
# (github.com/xNasuni/luau-web, MIT) publishes only its emscripten output
# (Luau's VM and compiler with its JavaScript glue, the wasm inlined as
# base64); the glue's C++ is not in its repository. So this is a pinned,
# integrity-checked fetch, not a build from source. Only the Asyncify
# build is kept: the JSPI one does not catch runtime errors in
# `pcall`/`xpcall` (Weblua's docs/luau-web-pcall-issue.md), and every
# browser runs Asyncify. Its `index.js` is the package's own, importing
# that build alone.
#
# The worker that runs it (features/pages/src/web/luau.js) takes away the
# network, storage and nested workers before it imports these files, and
# every host call it makes is the host service's to check: the files
# reach nothing an extension may not.
set -euo pipefail

VERSION=1.4.0
INTEGRITY='sha512-IZPMHMRBlM3bKYUnzwoptqIzSf7F5ZjuUeYpColLydq+2D6LucUKUEhViEMBD1r5gTEh/AV0bvuzTcis+KXfkQ=='
URL="https://registry.npmjs.org/luau-web/-/luau-web-$VERSION.tgz"

root="$(cd "$(dirname "$0")/.." && pwd)"
dest="$root/apps/web/assets/luau-web"
check=0
[ "${1:-}" = "--check" ] && check=1

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl -fsSL "$URL" -o "$work/pkg.tgz"
got="sha512-$(openssl dgst -sha512 -binary "$work/pkg.tgz" | base64 -w0)"
if [ "$got" != "$INTEGRITY" ]; then
    echo "luau-web $VERSION: integrity $got, expected $INTEGRITY" >&2
    exit 1
fi
mkdir "$work/x"
tar -xzf "$work/pkg.tgz" -C "$work/x"
pkg="$work/x/package"

mkdir -p "$work/out"
cp "$pkg/src/lib/Luau.Web.Asyncify.js" "$work/out/Luau.Web.Asyncify.js"
# Its index, with the Asyncify build alone (no JSPI pick).
python3 - "$pkg/src/index.js" "$work/out/index.js" <<'PY'
import sys
src = open(sys.argv[1]).read()
pick = src[src.index("if ('Suspending' in WebAssembly"):src.index("var Luau = {")]
kept = ("// luau-web's index (scripts/vendor-luau-web.sh): its Asyncify build alone.\n"
        "InternalLuauWasmModule = (await import('./Luau.Web.Asyncify.js')).default\n\n")
out = src.replace(pick, kept)
assert out != src
open(sys.argv[2], "w").write(out)
PY
cat > "$work/out/LICENSE" <<'EOF'
MIT License

Copyright (c) 2026 xNasuni

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
EOF
echo "luau-web $VERSION ($URL, $INTEGRITY)" > "$work/out/VERSION"

if [ "$check" = 1 ]; then
    diff -r "$work/out" "$dest" >/dev/null || {
        echo "apps/web/assets/luau-web differs from luau-web $VERSION: run scripts/vendor-luau-web.sh" >&2
        exit 1
    }
    echo "apps/web/assets/luau-web is luau-web $VERSION"
else
    rm -rf "$dest"
    mkdir -p "$dest"
    cp "$work/out/." "$dest/" -r
    echo "wrote $dest (luau-web $VERSION)"
fi
