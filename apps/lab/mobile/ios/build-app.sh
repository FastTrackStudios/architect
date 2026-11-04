#!/usr/bin/env bash
# The iOS lab as an app bundle for the Simulator, without dx: cargo builds
# `architect-mobile` for aarch64-apple-ios-sim, and this folder's
# Info.plist, the binary and (with `--features wasi`) the WASI Counter's
# component go into `<target>/aarch64-apple-ios-sim/debug/ArchitectMobile.app`,
# signed ad hoc. It prints the bundle's path, for
# `scripts/mac-live-check.sh ios <it>`.
#
#   apps/lab/mobile/ios/build-app.sh [cargo build flags…]   (in `devenv shell`)
#   apps/lab/mobile/ios/build-app.sh --features wasi
#
# The dev shell's C toolchain is Nix's, for macOS: its xcrun knows no
# iphonesimulator SDK and its SDKROOT and flags are the Mac's. A crate's C
# code for the phone (objc2's exception helper, blake3, zlib) is built
# with Xcode's clang and xcrun instead; the host's build scripts keep the
# shell's. rquickjs has no bindings for iOS in its crate, so they are
# generated (bindgen) with Xcode's libclang against the Simulator's SDK.
# The shell's pkg-config answers with the Mac's libraries (zlib), so for
# the phone it answers nothing.
# With `wasi`, the component is the one built for wasm32-wasip2
# (`cargo build -p architect-lab-wasi-counter --target wasm32-wasip2
# --target-dir <target>/wasi-guest`, in `devenv --profile wasi shell`), or
# ARCHITECT_LAB_WASI_COUNTER.
set -euo pipefail

die() { echo "build-app: $*" >&2; exit 1; }

here="$(cd "$(dirname "$0")" && pwd)"
root="$(git -C "$here" rev-parse --show-toplevel)"
target_dir="${CARGO_TARGET_DIR:-$root/target}"
triple=aarch64-apple-ios-sim

xcode_bin="$(mktemp -d)"
trap 'rm -rf "$xcode_bin"' EXIT
ln -s /usr/bin/xcrun "$xcode_bin/xcrun"

# The shell's DEVELOPER_DIR and SDKROOT are its Nix SDK's (the Mac's);
# Xcode's tools follow them, so they go first.
unset DEVELOPER_DIR SDKROOT
toolchain="$(/usr/bin/xcode-select -p)/Toolchains/XcodeDefault.xctoolchain"
sdk="$(/usr/bin/xcrun --sdk iphonesimulator --show-sdk-path)"

env -u NIX_CFLAGS_COMPILE -u NIX_LDFLAGS -u MACOSX_DEPLOYMENT_TARGET \
    PATH="$xcode_bin:$PATH" \
    IPHONEOS_DEPLOYMENT_TARGET=16.0 \
    PKG_CONFIG_aarch64_apple_ios_sim=/usr/bin/false \
    LIBCLANG_PATH="$toolchain/usr/lib" \
    BINDGEN_EXTRA_CLANG_ARGS_aarch64_apple_ios_sim="-isysroot $sdk" \
    CC_aarch64_apple_ios_sim=/usr/bin/clang \
    CXX_aarch64_apple_ios_sim=/usr/bin/clang++ \
    AR_aarch64_apple_ios_sim=/usr/bin/ar \
    CARGO_TARGET_AARCH64_APPLE_IOS_SIM_LINKER=/usr/bin/clang \
    cargo build -p architect-mobile --target "$triple" "$@" >&2

app="$target_dir/$triple/debug/ArchitectMobile.app"
rm -rf "$app"
mkdir -p "$app"
cp "$here/Info.plist" "$app/Info.plist"
cp "$target_dir/$triple/debug/architect-mobile" "$app/architect-mobile"
if [[ " $* " == *" wasi "* || " $* " == *"--features=wasi"* || " $* " == *",wasi"* ]]; then
    wasm="${ARCHITECT_LAB_WASI_COUNTER:-$target_dir/wasi-guest/wasm32-wasip2/debug/architect_lab_wasi_counter.wasm}"
    [[ -f "$wasm" ]] || die "no WASI Counter component at $wasm (build it in devenv --profile wasi shell)"
    cp "$wasm" "$app/"
fi
/usr/bin/codesign --force --sign - --timestamp=none "$app" >&2
echo "$app"
