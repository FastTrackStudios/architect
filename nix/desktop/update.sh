#!/usr/bin/env bash
# Point the flake's architect-desktop at a release: rewrite source.json from
# the release's latest-linux.json, which carries the AppImage's sha256, so
# nothing is downloaded but that small manifest.
#
#   nix/desktop/update.sh            the newest release
#   nix/desktop/update.sh v0.12.0    a given tag
#
# release.yml (job nix-bump) runs it after a release and opens the bump PR.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="${ARCHITECT_REPO:-FastTrackStudios/architect}"

if [ "$#" -gt 0 ]; then
    url="https://github.com/$repo/releases/download/$1/latest-linux.json"
else
    url="https://github.com/$repo/releases/latest/download/latest-linux.json"
fi

manifest="$(curl -fsSL "$url")"
version="$(jq -r .version <<<"$manifest")"
hex="$(jq -r .sha256 <<<"$manifest")"
[ -n "$version" ] && [ "$version" != null ] || { echo "no version in $url" >&2; exit 1; }
[[ "$hex" =~ ^[0-9a-f]{64}$ ]] || { echo "no sha256 in $url" >&2; exit 1; }
# SRI is base64 of the raw digest (coreutils only, no nix needed).
hash="sha256-$(printf "$(sed 's/../\\x&/g' <<<"$hex")" | base64 -w0)"

jq -n --arg version "$version" --arg hash "$hash" '{version: $version, hash: $hash}' \
    >"$here/source.json"
echo "architect-desktop: $version ($hash)"
