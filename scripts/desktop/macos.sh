#!/usr/bin/env bash
# Architect's desktop app for macOS (#431): a Developer-ID signed,
# notarized, stapled .dmg, attached to the release as Architect-macos-arm64.dmg
# beside latest-macos.json (the updater's manifest).
#
#   scripts/desktop/macos.sh     # -> $OUT_DIR/Architect-macos-arm64.dmg
#
# Runs on airlock (Apple silicon), where the Developer ID identity lives in
# the build keychain and the App Store Connect API key is in
# ~/.appstoreconnect/config.env, which notarytool reads. Nothing comes from
# GitHub secrets. Called by release.yml's `macos` job; a human can run it.
#
# Env (all optional):
#   VERSION     X.Y.Z (default: the workspace version), into Info.plist
#   BUILD_NO    CFBundleVersion (default: unix time)
#   OUT_DIR     default target/desktop
#   PROFILE     cargo profile (default release)
#   KEYCHAIN / KEYCHAIN_PW   the build keychain (default fts-build.keychain)
#   NOTARIZE    1 (default) notarize + staple; 0 signs only (not distributable)
#   SKIP_BUILD  1 reuses target/$PROFILE/architect-desktop
#
# arm64 only (a universal build is a lipo away: x86_64-apple-darwin is in
# the toolchain).
set -euo pipefail
crate=architect-desktop; bin=architect-desktop; title=Architect
icon_svg=apps/desktop/icons/architect.svg
category=public.app-category.productivity
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
SCRIPT_DIR="$ROOT/scripts/desktop"
VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || { echo "version '$VERSION' is not X.Y.Z" >&2; exit 2; }
BUILD_NO="${BUILD_NO:-$(date +%s)}"
OUT_DIR="${OUT_DIR:-$ROOT/target/desktop}"
PROFILE="${PROFILE:-release}"
NOTARIZE="${NOTARIZE:-1}"
BUNDLE_ID="app.fasttrackstudio.architect.mac"
mkdir -p "$OUT_DIR"
# ── Tools ───────────────────────────────────────────────────────────────────
NIX="${NIX:-}"
if [ -z "$NIX" ]; then
    for c in /nix/var/nix/profiles/default/bin/nix /run/current-system/sw/bin/nix "$(command -v nix 2>/dev/null || true)"; do
        [ -n "$c" ] && [ -x "$c" ] && { NIX="$c"; break; }
    done
fi
[ -n "$NIX" ] || { echo "no nix on this host" >&2; exit 1; }

# ── The Developer ID identity ───────────────────────────────────────────────
KEYCHAIN="${KEYCHAIN:-fts-build.keychain}"
if [ -n "${KEYCHAIN_PW:-}" ]; then security unlock-keychain -p "$KEYCHAIN_PW" "$KEYCHAIN"; fi
SIGN_ID="${SIGN_ID:-$(security find-identity -v -p codesigning "$KEYCHAIN" \
    | awk -F'"' '/Developer ID Application/ {print $2; exit}')}"
if [ -z "$SIGN_ID" ]; then
    echo "ERROR: no \"Developer ID Application\" identity in $KEYCHAIN." >&2
    echo "       The App Store identities cannot sign a directly distributed .dmg;" >&2
    echo "       the Captain has to issue and import one (developer.apple.com)." >&2
    exit 1
fi
echo "=== $title $VERSION (build $BUILD_NO): signing identity: $SIGN_ID ==="

if [ "$NOTARIZE" = 1 ]; then
    # shellcheck disable=SC1090,SC1091
    [ -f "$HOME/.appstoreconnect/config.env" ] && source "$HOME/.appstoreconnect/config.env"
    : "${ASC_KEY_ID:?ASC_KEY_ID missing (~/.appstoreconnect/config.env): needed to notarize}"
    : "${ASC_ISSUER_ID:?ASC_ISSUER_ID missing: needed to notarize}"
    : "${ASC_KEY_PATH:?ASC_KEY_PATH missing: needed to notarize}"
fi

# ── Build ───────────────────────────────────────────────────────────────────
BIN="$ROOT/target/$PROFILE/$bin"
if [ "${SKIP_BUILD:-}" = 1 ]; then
    echo "=== SKIP_BUILD=1: reusing $BIN ==="
else
    echo "=== cargo build --profile $PROFILE -p $crate ==="
    devenv shell -- cargo build --profile "$PROFILE" -p "$crate" --bin "$bin"
fi
[ -x "$BIN" ] || { echo "ERROR: the build produced no $BIN" >&2; exit 1; }
file -b "$BIN" | grep -q 'Mach-O 64-bit executable arm64' \
    || { echo "ERROR: $BIN is not an arm64 Mach-O: $(file -b "$BIN")" >&2; exit 1; }

# ── The bundle ──────────────────────────────────────────────────────────────
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
APP="$STAGE/$title.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/$bin"
chmod +x "$APP/Contents/MacOS/$bin"

# Only what every Mac has: the dev shell links its own libiconv, and a
# /nix/store path exists on no other Mac (the hardened runtime also refuses a
# library of another team). libiconv and libz are part of macOS, so those are
# repointed at /usr/lib; anything else from the store fails here, not at launch.
for lib in $(otool -L "$APP/Contents/MacOS/$bin" | awk 'NR>1 && $1 ~ /^\/nix\/store\// {print $1}'); do
    case "$lib" in
        */libiconv.2.dylib) install_name_tool -change "$lib" /usr/lib/libiconv.2.dylib "$APP/Contents/MacOS/$bin" ;;
        */libz.1.dylib) install_name_tool -change "$lib" /usr/lib/libz.1.dylib "$APP/Contents/MacOS/$bin" ;;
        *) ;;
    esac
done
if otool -L "$APP/Contents/MacOS/$bin" | awk 'NR>1' | grep -q '/nix/store/'; then
    echo "ERROR: $bin links libraries from /nix/store:" >&2
    otool -L "$APP/Contents/MacOS/$bin" | grep '/nix/store/' >&2
    exit 1
fi

icon="$STAGE/icon-1024.png"
"$NIX" shell nixpkgs#librsvg -c rsvg-convert -w 1024 -h 1024 "$icon_svg" -o "$icon"
if [ -f "$icon" ]; then
    iconset="$STAGE/$title.iconset"
    mkdir -p "$iconset"
    for s in 16 32 128 256 512; do
        sips -z "$s" "$s" "$icon" --out "$iconset/icon_${s}x${s}.png" >/dev/null
        sips -z $((s * 2)) $((s * 2)) "$icon" --out "$iconset/icon_${s}x${s}@2x.png" >/dev/null
    done
    iconutil -c icns "$iconset" -o "$APP/Contents/Resources/$title.icns"
fi

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$title</string>
    <key>CFBundleDisplayName</key><string>$title</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundleExecutable</key><string>$bin</string>
    <key>CFBundleIconFile</key><string>$title</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$BUILD_NO</string>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>LSApplicationCategoryType</key><string>$category</string>
</dict>
</plist>
PLIST
plutil -lint "$APP/Contents/Info.plist" >/dev/null

# ── Sign (hardened runtime: notarization requires it) ───────────────────────
ENT="$SCRIPT_DIR/macos.entitlements"
plutil -lint "$ENT" >/dev/null
echo "=== signing (Developer ID + hardened runtime) ==="
find "$APP" \( -name '*.dylib' -o -name '*.so' -o -name '*.framework' \) -print0 \
    | while IFS= read -r -d '' f; do
        codesign --force --keychain "$KEYCHAIN" --timestamp --options runtime --sign "$SIGN_ID" "$f"
      done
codesign --force --keychain "$KEYCHAIN" --timestamp --options runtime \
    --entitlements "$ENT" --sign "$SIGN_ID" "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"

# ── The disk image ──────────────────────────────────────────────────────────
DMG="$OUT_DIR/$title-macos-arm64.dmg"
DMGROOT="$STAGE/dmg"
mkdir -p "$DMGROOT"
cp -R "$APP" "$DMGROOT/"
ln -s /Applications "$DMGROOT/Applications"
rm -f "$DMG"
hdiutil create -volname "$title $VERSION" -srcfolder "$DMGROOT" -ov -format UDZO "$DMG" >/dev/null
# Gatekeeper checks the .dmg a person downloaded, not only the .app in it.
codesign --force --keychain "$KEYCHAIN" --timestamp --sign "$SIGN_ID" "$DMG"

if [ "$NOTARIZE" != 1 ]; then
    echo "=== NOTARIZE=0: signed only, NOT distributable: $DMG ==="
    exit 0
fi

# ── Notarize + staple ───────────────────────────────────────────────────────
echo "=== notarizing $DMG (waits for Apple) ==="
xcrun notarytool submit "$DMG" \
    --key "$ASC_KEY_PATH" --key-id "$ASC_KEY_ID" --issuer "$ASC_ISSUER_ID" --wait \
    | tee "$STAGE/notary.log"
grep -q 'status: Accepted' "$STAGE/notary.log" \
    || { echo "ERROR: notarization was not Accepted (see the log above)" >&2; exit 1; }
xcrun stapler staple "$DMG"
xcrun stapler validate "$DMG"

# What Gatekeeper itself says about the image and about the app a person drags out.
spctl -a -vv -t open --context context:primary-signature "$DMG"
MOUNT="$STAGE/mnt"
mkdir -p "$MOUNT"
hdiutil attach "$DMG" -nobrowse -readonly -mountpoint "$MOUNT" >/dev/null
trap 'hdiutil detach "$MOUNT" >/dev/null 2>&1 || true; rm -rf "$STAGE"' EXIT
spctl -a -vv -t exec "$MOUNT/$title.app" \
    || { echo "ERROR: Gatekeeper refused the app inside the image" >&2; exit 1; }
codesign -dv --verbose=2 "$MOUNT/$title.app" 2>&1 | grep -E 'Identifier|Authority|TeamIdentifier|flags|Timestamp' || true

echo "=== done: $DMG ($(du -h "$DMG" | cut -f1)) ==="
echo "dmg=$DMG"
