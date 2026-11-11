#!/usr/bin/env bash
# The launcher's version choice (choose.sh), with shell scripts standing in
# for AppImages. Run by the flake's checks.<system>.architect-desktop-choose.
#
#   nix/desktop/test-choose.sh [path/to/choose.sh]
set -euo pipefail

choose="${1:-$(cd "$(dirname "$0")" && pwd)/choose.sh}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

store="$tmp/store/Architect-linux-x86_64.AppImage"
mkdir -p "$(dirname "$store")"
printf '#!/bin/sh\necho store\n' >"$store"
chmod +x "$store"

fail=0
# case <name> <store-version> <download-version|-> <expect: store|download> [no-exec]
case_() {
    local name="$1" sv="$2" dv="$3" expect="$4" mode="${5:-}"
    local dir="$tmp/$name"
    mkdir -p "$dir"
    if [ "$dv" != "-" ]; then
        printf '#!/bin/sh\necho download\n' >"$dir/Architect-linux-x86_64.AppImage"
        [ "$mode" = no-exec ] || chmod +x "$dir/Architect-linux-x86_64.AppImage"
        [ "$mode" = no-version ] || printf '%s' "$dv" >"$dir/version"
    fi
    [ "$mode" = no-dir ] && dir="$tmp/does-not-exist"
    local got want
    got="$(bash "$choose" "$sv" "$store" "$dir")"
    if [ "$expect" = store ]; then want="$store"; else want="$dir/Architect-linux-x86_64.AppImage"; fi
    if [ "$got" = "$want" ]; then
        echo "ok   $name"
    else
        echo "FAIL $name: got $got, want $want"
        fail=1
    fi
}

case_ no-update-dir      0.12.0 -                     store no-dir
case_ empty-update-dir   0.12.0 -                     store
case_ older-download     0.12.0 0.11.9                store
case_ newer-download     0.12.0 0.12.1                download
case_ newer-minor        0.12.0 0.13.0                download
case_ numeric-not-text   0.9.0  0.10.0                download
case_ newer-with-newline 0.12.0 $'  0.12.1\n'         download
case_ equal              0.12.0 0.12.0                store
case_ garbage-version    0.12.0 'not a version'       store
case_ empty-version      0.12.0 ''                    store
case_ partial-version    0.12.0 0.13                  store
case_ missing-version    0.12.0 0.13.0                store no-version
case_ not-executable     0.12.0 0.13.0                store no-exec
case_ release-beats-nightly-same 0.12.0 0.12.0-nightly.20261111 store
case_ release-over-store-nightly 0.12.0-nightly.20261111 0.12.0 download
case_ newer-nightly      0.12.0 0.12.1-nightly.20261111 download
case_ older-nightly      0.12.1 0.12.0-nightly.20261111 store
case_ nightly-by-date    0.12.1-nightly.20261110 0.12.1-nightly.20261111 download
case_ nightly-same-date  0.12.1-nightly.20261111 0.12.1-nightly.20261111 store
case_ bad-nightly        0.12.0 0.12.1-nightly.2026   store

# The chosen file is the one that runs.
out="$("$(bash "$choose" 0.12.0 "$store" "$tmp/newer-download")")"
[ "$out" = download ] && echo "ok   runs-chosen" || { echo "FAIL runs-chosen: $out"; fail=1; }

exit "$fail"
