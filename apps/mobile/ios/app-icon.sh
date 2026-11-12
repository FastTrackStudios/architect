#!/usr/bin/env bash
# Regenerate the phone's app icon (Assets.xcassets/AppIcon.appiconset) from
# the app's SVG, the one the desktop app's icon comes from
# (apps/desktop/icons/architect.svg). Only needed when the SVG changes: the
# PNG is checked in so a build needs no converter.
#
# The catalog is Studio's shape: one 1024 `universal` image, from which
# actool makes every size the phone and the iPad want
# (deploy-testflight.sh compiles it into Assets.car). iOS masks the corners
# itself and the App Store rejects an icon with an alpha channel, so the
# SVG is drawn full bleed on its own background colour and saved opaque.
#
#   apps/mobile/ios/app-icon.sh [icon.svg]
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
svg="${1:-$root/apps/desktop/icons/architect.svg}"
bg="${ICON_BG:-#18181b}"
out="$here/Assets.xcassets/AppIcon.appiconset"
NIX="${NIX:-$(command -v nix || echo /nix/var/nix/profiles/default/bin/nix)}"

mkdir -p "$out"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
"$NIX" shell nixpkgs#librsvg -c rsvg-convert -w 1024 -h 1024 \
    --background-color "$bg" "$svg" -o "$tmp/icon.png"
# Through JPEG and back: the only way sips drops the alpha channel.
sips -s format jpeg -s formatOptions 100 "$tmp/icon.png" --out "$tmp/icon.jpg" >/dev/null
sips -s format png "$tmp/icon.jpg" --out "$out/icon-1024.png" >/dev/null
cat > "$out/Contents.json" <<'JSON'
{
  "images" : [
    { "filename" : "icon-1024.png", "idiom" : "universal", "platform" : "ios", "size" : "1024x1024" }
  ],
  "info" : { "author" : "xcode", "version" : 1 }
}
JSON
echo '{ "info" : { "author" : "xcode", "version" : 1 } }' > "$here/Assets.xcassets/Contents.json"
sips -g pixelWidth -g hasAlpha "$out/icon-1024.png" | tail -2
echo "wrote $out"
