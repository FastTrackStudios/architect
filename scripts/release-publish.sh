#!/usr/bin/env bash
# release.yml's `sums`, as a script (and what the dry run runs on a
# throwaway draft, so the two cannot drift):
#
#   scripts/release-publish.sh run <tag> <EXPECTED> [--optional <asset>]...
#   scripts/release-publish.sh decide <is_draft> <tag> <highest stable tag>
#
# `run`:
#   1. an OPTIONAL asset (the Mac job's: it is continue-on-error) that is on
#      the Release but that no job reported is DELETED, and said so: a Mac
#      build that uploaded and then failed must not block the release (A1);
#   2. scripts/release-sums.sh plans SHA256SUMS over exactly the reported
#      digests, refusing an extra, missing or replaced asset;
#   3. it is signed with MINISIGN_KEY (when set) under the trusted comment
#      "architect <tag> SHA256SUMS", verified with ORG_RELEASE_KEY, uploaded;
#   4. the assets are checked again, then the Release is PUBLISHED, but only
#      if it is still a draft, and marked Latest only when <tag> is the
#      highest stable tag (`decide`): a hotfix on an older line, or a re-run,
#      never moves Latest (A2).
# EXPECTED: the build jobs' "<sha256>  <name>" lines. Needs gh (GH_TOKEN),
# jq, minisign, and the repo's tags (fetch-depth 0).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
die() { echo "release-publish: $*" >&2; exit 1; }

decide() { # <is_draft> <tag> <highest>
    if [ "$1" != true ]; then echo leave
    elif [ "$2" = "$3" ]; then echo publish-latest
    else echo publish; fi
}

case "${1:-}" in
    decide)
        [ "$#" -eq 4 ] || die "usage: release-publish.sh decide <is_draft> <tag> <highest>"
        decide "$2" "$3" "$4"; exit 0 ;;
    run) shift ;;
    *) die "usage: release-publish.sh run <tag> <EXPECTED> [--optional <asset>]... | decide …" ;;
esac
repo="${GITHUB_REPOSITORY:?}"
tag="${1:?tag}" expected="${2:?EXPECTED}"
shift 2
optional=()
while [ $# -gt 0 ]; do
    case "$1" in --optional) optional+=("$2"); shift 2 ;; *) die "unknown argument $1" ;; esac
done

id="$(gh api --paginate "repos/$repo/releases?per_page=100" --jq ".[] | select(.tag_name == \"$tag\") | .id" | sort -n | head -1)"
[ -n "$id" ] || die "no release for $tag"
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
assets() { gh api "repos/$repo/releases/$id/assets?per_page=100" --jq '[.[] | {name, digest}]' > "$dir/assets.json"; }
assets

# 1. Optional assets nobody reported: deleted (a Mac job that failed after uploading).
for name in "${optional[@]}"; do
    if jq -e --arg n "$name" 'map(.name) | index($n)' "$dir/assets.json" >/dev/null \
       && ! awk '{print $2}' "$expected" | grep -qxF "$name"; then
        echo "release-publish: $name is on the release but its job did not report it (it failed or was re-run): deleted"
        gh release delete-asset "$tag" "$name" --repo "$repo" --yes
    fi
done
assets

# 2. The sums, over exactly the reported digests.
bash "$here/release-sums.sh" plan "$expected" "$dir/assets.json" > "$dir/SHA256SUMS"
cat "$dir/SHA256SUMS"
upload=("$dir/SHA256SUMS")
# 3. Signed.
if [ -n "${MINISIGN_KEY:-}" ]; then
    key="$dir/release.key"
    ( umask 077; printf '%s\n' "$MINISIGN_KEY" > "$key" )
    minisign -S -s "$key" -m "$dir/SHA256SUMS" -x "$dir/SHA256SUMS.minisig" -t "architect $tag SHA256SUMS" </dev/null
    rm -f "$key"
    minisign -V -P "${ORG_RELEASE_KEY:?}" -m "$dir/SHA256SUMS" -x "$dir/SHA256SUMS.minisig"
    upload+=("$dir/SHA256SUMS.minisig")
else
    echo "::warning::no MINISIGN_KEY: SHA256SUMS goes out unsigned"
fi
gh release upload "$tag" "${upload[@]}" --clobber --repo "$repo"

# 4. Nothing moved while signing; then publish, the draft only.
assets
bash "$here/release-sums.sh" plan "$expected" "$dir/assets.json" > /dev/null
draft="$(gh api "repos/$repo/releases/$id" --jq .draft)"
highest="$(git tag --list 'v[0-9]*' | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | sort -V | tail -1 || true)"
case "$(decide "$draft" "$tag" "$highest")" in
    publish-latest) gh release edit "$tag" --repo "$repo" --draft=false --latest; echo "published $tag (Latest)" ;;
    publish) gh release edit "$tag" --repo "$repo" --draft=false --latest=false; echo "published $tag (not Latest: the highest is $highest)" ;;
    leave) echo "$tag is published already: nothing re-published, Latest left where it is" ;;
esac
