#!/usr/bin/env bash
# Architect's desktop app for Linux as an AppImage (#431): what the
# release attaches as Architect-linux-x86_64.AppImage, beside
# latest-linux.json (the updater's manifest).
#
#   scripts/desktop/linux.sh        # -> $OUT_DIR/Architect-linux-x86_64.AppImage
#
# Studio's scripts/desktop/linux.sh, for architect-desktop (Blitz + wgpu,
# like Studio: no WebKit). A binary built in a Nix shell points into
# /nix/store, which no other machine has, so this
#   - copies every shared library the binary links into the AppDir, except
#     the graphics stack (GL / EGL / Vulkan loader / DRM / GBM, matched to
#     the user's driver) and glibc (the user's);
#   - sets the interpreter to /lib64/ld-linux-x86-64.so.2 and RUNPATH to
#     $ORIGIN, so nothing points at /nix/store;
#   - refuses a glibc floor over MAX_GLIBC.
# Libraries winit opens at run time (libwayland-client, libxkbcommon,
# libX11 and friends) are not linked, so not bundled: they come from the
# user's desktop, which has them.
#
# Env (all optional):
#   VERSION     X.Y.Z, written into the desktop entry (default: the
#               workspace version)
#   OUT_DIR     default target/desktop
#   PROFILE     cargo profile (default release)
#   MAX_GLIBC   default 2.42 (the Nix toolchain's today; a ratchet)
#   SKIP_BUILD  1 reuses $CARGO_TARGET_DIR (or target)/$PROFILE/architect-desktop
#
# Another distribution of Architect (#491; `cargo xtask dist desktop`
# sets these from `<bin> distribution`), all defaulting to the Architect app:
#   APP_BIN_PATH  its built binary (implies SKIP_BUILD=1)
#   APP_BIN       the binary's name in the bundle (default architect-desktop)
#   APP_TITLE     its name (default Architect)
#   APP_FILE      the AppImage's file name before -linux (default APP_TITLE)
#   APP_ID        its bundle id (default app.fasttrackstudio.architect)
#   APP_ICON      its icon's SVG (default apps/desktop/icons/architect.svg)
#   APP_COMMENT   the desktop entry's comment
set -euo pipefail

bin="${APP_BIN:-architect-desktop}"; title="${APP_TITLE:-Architect}"; slug="${APP_ID:-app.fasttrackstudio.architect}"
file="${APP_FILE:-$title}"
icon_svg="${APP_ICON:-apps/desktop/icons/architect.svg}"
comment="${APP_COMMENT:-Notes, projects and files on every device, local-first and in sync}"
categories="Office;Utility;"

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || { echo "version '$VERSION' is not X.Y.Z" >&2; exit 2; }
OUT_DIR="${OUT_DIR:-$ROOT/target/desktop}"
PROFILE="${PROFILE:-release}"
MAX_GLIBC="${MAX_GLIBC:-2.42}"
mkdir -p "$OUT_DIR"
OUT="$OUT_DIR/$file-linux-x86_64.AppImage"

# In the repo's dev shell (devenv), then with the packaging tools from the
# nixpkgs the flake pins.
if [ -z "${DESKTOP_IN_SHELL:-}" ] && [ -z "${DEVENV_ROOT:-}" ]; then
    exec devenv shell -- env DESKTOP_IN_SHELL=1 bash "$0" "$@"
fi
if [ -z "${DESKTOP_IN_TOOLS:-}" ]; then
    pk="$(nix eval --raw --impure --expr "(builtins.getFlake \"$ROOT\").inputs.nixpkgs.outPath" 2>/dev/null || echo nixpkgs)"
    [ "$pk" = nixpkgs ] || pk="path:$pk"
    exec nix shell "$pk#patchelf" "$pk#squashfsTools" "$pk#binutils" "$pk#curl" "$pk#coreutils" "$pk#librsvg" \
        --command env DESKTOP_IN_TOOLS=1 bash "$0" "$@"
fi

# ── Build ───────────────────────────────────────────────────────────────────
BIN="${APP_BIN_PATH:-${CARGO_TARGET_DIR:-$ROOT/target}/$PROFILE/$bin}"
if [ -n "${APP_BIN_PATH:-}" ] || [ "${SKIP_BUILD:-}" = 1 ]; then
    echo "=== SKIP_BUILD=1: reusing $BIN ==="
else
    echo "=== cargo build --profile $PROFILE -p architect-desktop ==="
    cargo build --profile "$PROFILE" -p architect-desktop --bin "$bin"
fi
[ -x "$BIN" ] || { echo "ERROR: the build produced no $BIN" >&2; exit 1; }

# ── The AppDir ──────────────────────────────────────────────────────────────
WORK="$(mktemp -d -p "$OUT_DIR" .appdir.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT
AD="$WORK/$file.AppDir"
mkdir -p "$AD/usr/bin" "$AD/usr/lib" "$AD/usr/share/applications" "$AD/usr/share/icons/hicolor/256x256/apps"
cp "$BIN" "$AD/usr/bin/$bin"
chmod u+w "$AD/usr/bin/$bin"

excluded() {
    case "$1" in
        libGL.so*|libGLX*|libGLdispatch*|libOpenGL*|libEGL*|libGLESv*|libvulkan*|libdrm*|libgbm*) return 0 ;;
        libc.so*|libm.so*|libdl.so*|libpthread.so*|librt.so*|libresolv.so*|libutil.so*|libmvec.so*|libanl.so*|libBrokenLocale.so*|libthread_db.so*|libnsl.so*|libnss_*|ld-linux*) return 0 ;;
        *) return 1 ;;
    esac
}

declare -A seen=()
: > "$WORK/needed-excluded.txt"
while read -r name arrow path _; do
    if [ "$arrow" != "=>" ]; then
        case "$name" in /*) path="$name" ;; *) continue ;; esac
    fi
    name="$(basename "$name")"
    [ -f "${path:-}" ] || continue
    if excluded "$name"; then echo "$name" >> "$WORK/needed-excluded.txt"; continue; fi
    [ -z "${seen[$name]:-}" ] || continue
    seen[$name]=1
    cp -L "$path" "$AD/usr/lib/$name"
    chmod u+w "$AD/usr/lib/$name"
done < <(ldd "$BIN")
echo "=== bundled ${#seen[@]} libraries; from the host: $(sort -u "$WORK/needed-excluded.txt" | tr '\n' ' ')==="

# shellcheck disable=SC2016 # $ORIGIN is the loader's
patchelf --set-interpreter /lib64/ld-linux-x86-64.so.2 --set-rpath '$ORIGIN/../lib' "$AD/usr/bin/$bin"
for f in "$AD"/usr/lib/*; do
    # shellcheck disable=SC2016
    patchelf --set-rpath '$ORIGIN' "$f" 2>/dev/null || true
done
for f in "$AD"/usr/bin/* "$AD"/usr/lib/*; do
    readelf -d "$f" 2>/dev/null | sed -n 's/.*Shared library: \[\(\/[^]]*\)\].*/\1/p' | while read -r abs; do
        [ -e "$AD/usr/lib/$(basename "$abs")" ] || { echo "ERROR: $f needs $abs, which is not in the bundle" >&2; exit 1; }
        patchelf --replace-needed "$abs" "$(basename "$abs")" "$f"
    done
done
for f in "$AD"/usr/bin/* "$AD"/usr/lib/*; do
    if readelf -d "$f" 2>/dev/null | grep -E 'RPATH|RUNPATH|NEEDED' | grep -q '/nix/store'; then
        echo "ERROR: $f still names /nix/store in its dynamic section" >&2; exit 1
    fi
    if readelf -l "$f" 2>/dev/null | grep -q 'interpreter: /nix/store'; then
        echo "ERROR: $f still has a /nix/store interpreter" >&2; exit 1
    fi
done

glibc_refs() { objdump -p "$1" 2>/dev/null | sed -n '/Version References/,$p' | grep -o 'GLIBC_[0-9][0-9.]*' || true; }
floor="$(for f in "$AD"/usr/bin/* "$AD"/usr/lib/*; do glibc_refs "$f"; done | sed 's/GLIBC_//' | sort -uV | tail -1)"
echo "=== glibc floor: ${floor:-none} (limit $MAX_GLIBC) ==="
if [ -n "$floor" ] && [ "$(printf '%s\n%s\n' "$floor" "$MAX_GLIBC" | sort -V | tail -1)" != "$MAX_GLIBC" ]; then
    echo "ERROR: needs glibc $floor, over the limit $MAX_GLIBC" >&2; exit 1
fi
echo "$floor" > "$OUT_DIR/$file-linux-x86_64.AppImage-glibc-floor.txt"

# The bundle runs without /nix/store: through the system's loader (or, on
# NixOS, the toolchain's) with only the bundle's libraries.
ld=/lib64/ld-linux-x86-64.so.2
[ -e "$ld" ] || ld="$(ldd "$BIN" | sed -n 's/^\s*\(\/nix\/store\/[^ ]*ld-linux[^ ]*\).*/\1/p' | head -1)"
"$ld" --library-path "$AD/usr/lib" "$AD/usr/bin/$bin" serve --help > /dev/null \
    || { echo "ERROR: the bundled binary does not start" >&2; exit 1; }

# ── Desktop entry, icon, AppRun ─────────────────────────────────────────────
cat > "$AD/$slug.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=$title
Comment=$comment
Exec=$bin
Icon=$slug
Categories=$categories
Terminal=false
StartupWMClass=$bin
X-AppImage-Version=$VERSION
DESKTOP
cp "$AD/$slug.desktop" "$AD/usr/share/applications/$slug.desktop"
rsvg-convert -w 256 -h 256 "$icon_svg" -o "$AD/$slug.png"
cp "$AD/$slug.png" "$AD/usr/share/icons/hicolor/256x256/apps/$slug.png"
cp "$AD/$slug.png" "$AD/.DirIcon"

cat > "$AD/AppRun" <<APPRUN
#!/bin/sh
# The Vulkan loader finds a driver through XDG_DATA_DIRS' vulkan/icd.d.
HERE="\$(dirname "\$(readlink -f "\$0")")"
export XDG_DATA_DIRS="\${XDG_DATA_DIRS:+\$XDG_DATA_DIRS:}/usr/local/share:/usr/share"
exec "\$HERE/usr/bin/$bin" "\$@"
APPRUN
chmod +x "$AD/AppRun"

# ── Pack ────────────────────────────────────────────────────────────────────
# The type2 runtime with the AppDir's squashfs appended (what appimagetool
# does; nixpkgs dropped it). The runtime is pinned by sha256.
RUNTIME_URL="https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64"
RUNTIME_SHA256="2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d"
RT_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/fts-appimage"
RT="$RT_DIR/runtime-x86_64-20251108"
mkdir -p "$RT_DIR"
if ! { [ -f "$RT" ] && echo "$RUNTIME_SHA256  $RT" | sha256sum -c --status; }; then
    curl -fsSL --retry 3 -o "$RT.part" "$RUNTIME_URL"
    echo "$RUNTIME_SHA256  $RT.part" | sha256sum -c --status \
        || { echo "ERROR: the AppImage runtime does not match its pinned sha256" >&2; rm -f "$RT.part"; exit 1; }
    mv "$RT.part" "$RT"
fi
mksquashfs "$AD" "$WORK/app.squashfs" -root-owned -noappend -comp zstd -Xcompression-level 15 -no-xattrs -quiet
rm -f "$OUT"
cat "$RT" "$WORK/app.squashfs" > "$OUT"
chmod +x "$OUT"
echo "=== done: $OUT ($(du -h "$OUT" | cut -f1)) ==="
echo "appimage=$OUT"
