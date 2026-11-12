#!/usr/bin/env bash
# The Architect app as an iOS app bundle, without dx: cargo builds
# `architect-mobile`, and this folder's Info.plist, its asset catalog (the
# app icon and the launch screen's colour, compiled by actool), the binary
# and (with `--features wasi`) the WASI Counter's component go into
# `<target>/<triple>/<profile>/ArchitectMobile.app`. It prints the
# bundle's path.
#
#   apps/mobile/ios/build-app.sh [--device] [cargo build flags…]   (in `devenv shell`)
#   apps/mobile/ios/build-app.sh --features wasi
#   apps/mobile/ios/build-app.sh --device --release
#
# By default the bundle is for the Simulator (aarch64-apple-ios-sim),
# signed ad hoc, for `scripts/mac-live-check.sh ios <it>`. `--device`
# builds for a phone (aarch64-apple-ios) and leaves the bundle unsigned:
# deploy-testflight.sh signs it for the App Store.
#
# The dev shell's C toolchain is Nix's, for macOS: its xcrun knows no
# iOS SDK and its SDKROOT and flags are the Mac's. A crate's C code for
# the phone (objc2's exception helper, blake3, zlib) is built with
# Xcode's clang and xcrun instead; the host's build scripts keep the
# shell's. rquickjs has no bindings for iOS in its crate, so they are
# generated (bindgen) with Xcode's libclang against the target's SDK.
# The shell's pkg-config answers with the Mac's libraries (zlib), so for
# the phone it answers nothing.
# With `wasi`, the component is the one built for wasm32-wasip2
# (`cargo build -p architect-sandbox-wasi-counter --target wasm32-wasip2
# --target-dir <target>/wasi-guest`, in `devenv --profile wasi shell`), or
# ARCHITECT_LAB_WASI_COUNTER.
#
# ACTOOL_DEVELOPER_DIR: an Xcode whose actool runs on this macOS, when
# the build's own cannot (Studio's deploy-testflight.sh says why).
set -euo pipefail

die() { echo "build-app: $*" >&2; exit 1; }

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
target_dir="${CARGO_TARGET_DIR:-$root/target}"
min_ios=16.0

triple=aarch64-apple-ios-sim
sdk_name=iphonesimulator
cargo_args=()
for a in "$@"; do
    if [[ "$a" == "--device" ]]; then
        triple=aarch64-apple-ios
        sdk_name=iphoneos
    else
        cargo_args+=("$a")
    fi
done
profile=debug
if [[ " ${cargo_args[*]-} " == *" --release "* ]]; then profile=release; fi
t="${triple//-/_}"
T="$(echo "$t" | tr '[:lower:]' '[:upper:]')"

xcode_bin="$(mktemp -d "${TMPDIR:-/tmp}/architect-ios.XXXXXX")"
trap 'rm -rf "$xcode_bin"' EXIT
ln -s /usr/bin/xcrun "$xcode_bin/xcrun"

# The shell's DEVELOPER_DIR and SDKROOT are its Nix SDK's (the Mac's);
# Xcode's tools follow them, so they go first. XCODE_DIR picks an Xcode
# other than xcode-select's.
unset SDKROOT
if [[ -n "${XCODE_DIR:-}" ]]; then export DEVELOPER_DIR="$XCODE_DIR"; else unset DEVELOPER_DIR; fi
dev="${XCODE_DIR:-$(/usr/bin/xcode-select -p)}"
toolchain="$dev/Toolchains/XcodeDefault.xctoolchain"
sdk="$(/usr/bin/xcrun --sdk "$sdk_name" --show-sdk-path)"

env -u NIX_CFLAGS_COMPILE -u NIX_LDFLAGS -u MACOSX_DEPLOYMENT_TARGET \
    PATH="$xcode_bin:$PATH" \
    IPHONEOS_DEPLOYMENT_TARGET="$min_ios" \
    "PKG_CONFIG_$t=/usr/bin/false" \
    LIBCLANG_PATH="$toolchain/usr/lib" \
    "BINDGEN_EXTRA_CLANG_ARGS_$t=-isysroot $sdk" \
    "CC_$t=/usr/bin/clang" \
    "CXX_$t=/usr/bin/clang++" \
    "AR_$t=/usr/bin/ar" \
    "CARGO_TARGET_${T}_LINKER=/usr/bin/clang" \
    cargo build -p architect-mobile --target "$triple" ${cargo_args[@]+"${cargo_args[@]}"} >&2

app="$target_dir/$triple/$profile/ArchitectMobile.app"
rm -rf "$app"
mkdir -p "$app"
cp "$here/Info.plist" "$app/Info.plist"
cp "$target_dir/$triple/$profile/architect-mobile" "$app/architect-mobile"
if [[ " ${cargo_args[*]-} " == *" wasi "* || " ${cargo_args[*]-} " == *"--features=wasi"* || " ${cargo_args[*]-} " == *",wasi"* ]]; then
    wasm="${ARCHITECT_LAB_WASI_COUNTER:-$target_dir/wasi-guest/wasm32-wasip2/debug/architect_sandbox_wasi_counter.wasm}"
    [[ -f "$wasm" ]] || die "no WASI Counter component at $wasm (build it in devenv --profile wasi shell)"
    cp "$wasm" "$app/"
fi

# The app icon and the launch screen's colour (Info.plist's UILaunchScreen
# names it) into Assets.car; actool's partial plist adds CFBundleIcons.
icon_dev="${ACTOOL_DEVELOPER_DIR:-$dev}"
partial="$xcode_bin/icon.plist"
DEVELOPER_DIR="$icon_dev" /usr/bin/xcrun actool "$here/Assets.xcassets" --compile "$app" \
    --platform "$sdk_name" --minimum-deployment-target "$min_ios" \
    --target-device iphone --target-device ipad --app-icon AppIcon \
    --output-partial-info-plist "$partial" >"$xcode_bin/actool.log" 2>&1 \
    || { tail -20 "$xcode_bin/actool.log" >&2; die "actool failed"; }
[[ -f "$app/Assets.car" ]] || { tail -20 "$xcode_bin/actool.log" >&2; die "actool wrote no Assets.car (set ACTOOL_DEVELOPER_DIR)"; }
/usr/libexec/PlistBuddy -c "Merge $partial" "$app/Info.plist" >/dev/null 2>&1 || true

if [[ "$triple" == aarch64-apple-ios-sim ]]; then
    /usr/bin/codesign --force --sign - --timestamp=none "$app" >&2
fi
echo "$app"
