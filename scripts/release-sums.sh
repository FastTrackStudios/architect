#!/usr/bin/env bash
# What release.yml's `sums` signs: ONLY the digests the release's own build
# jobs reported (their job outputs, "<sha256>  <name>" lines), never
# whatever happens to be on the (draft) Release when it runs.
#
#   scripts/release-sums.sh plan EXPECTED ASSETS.json   prints SHA256SUMS, or refuses
#
# EXPECTED: the build jobs' lines. ASSETS.json: the Release's assets as
# GitHub reports them, [{"name", "digest": "sha256:<hex>"}].
# It refuses (exit 1, the reasons on stderr) when an asset is on the Release
# that no build job made (anyone with write, or a step with contents: write,
# could have put it there), when a build job's asset is missing, or when an
# asset's digest is not the one its job reported (replaced after upload).
# SHA256SUMS and SHA256SUMS.minisig themselves are ignored (a re-run finds
# them). The `.sha256` sidecars must match too, but are not listed in the
# sums (consumers use SHA256SUMS). `scripts/release-sums-test.sh` tests it.
set -euo pipefail

die() { echo "release-sums: $*" >&2; exit 1; }
[ "${1:-}" = plan ] && [ "$#" -eq 3 ] || die "usage: release-sums.sh plan EXPECTED ASSETS.json"
expected="$2" assets="$3"

declare -A want=() have=()
while read -r hex name; do
    [ -n "${name:-}" ] || continue
    [[ "$hex" =~ ^[0-9a-f]{64}$ ]] || die "a build job reported '$hex' for $name: not a sha256"
    [ -z "${want[$name]:-}" ] || [ "${want[$name]}" = "$hex" ] || die "two build jobs reported different digests for $name"
    want[$name]="$hex"
done <"$expected"
[ "${#want[@]}" -gt 0 ] || die "no build job reported an asset"

while IFS=$'\t' read -r name digest; do
    case "$name" in SHA256SUMS | SHA256SUMS.minisig) continue ;; esac
    have[$name]="${digest#sha256:}"
done < <(jq -r '.[] | [.name, (.digest // "")] | @tsv' "$assets")

bad=0
for name in "${!have[@]}"; do
    if [ -z "${want[$name]:-}" ]; then
        echo "release-sums: $name is on the Release but no build job made it" >&2; bad=1
    elif [ "${have[$name]}" != "${want[$name]}" ]; then
        echo "release-sums: $name is ${have[$name]:-without a digest} on the Release, its build job made ${want[$name]}" >&2; bad=1
    fi
done
for name in "${!want[@]}"; do
    [ -n "${have[$name]+x}" ] || { echo "release-sums: $name (reported by a build job) is not on the Release" >&2; bad=1; }
done
[ "$bad" = 0 ] || die "refusing to sign"

for name in "${!want[@]}"; do
    case "$name" in *.sha256) continue ;; esac
    printf '%s  %s\n' "${want[$name]}" "$name"
done | sort -k2
