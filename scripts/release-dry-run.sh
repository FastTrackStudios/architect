#!/usr/bin/env bash
# shellcheck disable=SC2015,SC2329 # ok() never fails; cleanup runs from the EXIT trap
# The release MECHANICS against a throwaway draft, never a real release
# (release-dry-run.yml: every PR that touches them, and by hand). It runs
# scripts/release-publish.sh, exactly what release.yml's `sums` runs:
#   1. an asset no job made (injected): refused, nothing published;
#   2. the Mac job uploaded its .dmg and then failed (it reported nothing):
#      the .dmg is deleted, the rest is signed and the draft PUBLISHED, not
#      Latest (the name is not a stable tag), Latest untouched;
#   3. a re-run on the published release: signed again, nothing re-published.
# The release is `dry-run-<run id>-<attempt>` (never a v* tag), signed with
# an ephemeral key, and deleted with its tag at the end whatever happens.
# Needs gh (GH_TOKEN with contents: write), jq, minisign, git tags.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="${GITHUB_REPOSITORY:?}"
name="dry-run-${GITHUB_RUN_ID:-local$$}-${GITHUB_RUN_ATTEMPT:-1}"
t="$(mktemp -d)"
fail=0
ok() { echo "ok   $*"; }
no() { echo "FAIL $*" >&2; fail=1; }
cleanup() {
    gh release delete "$name" --repo "$repo" --yes --cleanup-tag >/dev/null 2>&1 || true
    rm -rf "$t"
}
trap cleanup EXIT

minisign -G -W -p "$t/key.pub" -s "$t/key.sec" >/dev/null
export MINISIGN_KEY ORG_RELEASE_KEY
MINISIGN_KEY="$(cat "$t/key.sec")"
ORG_RELEASE_KEY="$(sed -n 2p "$t/key.pub")"

gh release create "$name" --repo "$repo" --draft --title "$name (dry run, deleted at the end)" \
    --notes "Dry run of the release mechanics (scripts/release-dry-run.sh). Not a release." \
    --target "${GITHUB_SHA:-$(git rev-parse HEAD)}" >/dev/null
id="$(gh api --paginate "repos/$repo/releases?per_page=100" --jq ".[] | select(.tag_name == \"$name\") | .id" | head -1)"
names() { gh api "repos/$repo/releases/$id/assets?per_page=100" --jq '.[].name' | sort | tr '\n' ' '; }
draft() { gh api "repos/$repo/releases/$id" --jq .draft; }
latest() { gh api "repos/$repo/releases/latest" --jq .tag_name 2>/dev/null || echo none; }
publish() { bash "$here/release-publish.sh" run "$name" "$t/expected" --optional Architect-macos-arm64.dmg --optional latest-macos.json; }

# What the build jobs made and reported.
mkdir -p "$t/b"
echo node > "$t/b/architect-node.tar.gz"; echo web > "$t/b/architect-web.tar.gz"; echo '{}' > "$t/b/release-facts.json"
gh release upload "$name" --repo "$repo" "$t/b/architect-node.tar.gz" "$t/b/architect-web.tar.gz" "$t/b/release-facts.json" >/dev/null
(cd "$t/b" && sha256sum architect-node.tar.gz architect-web.tar.gz release-facts.json) > "$t/expected"
latest_before="$(latest)"

# 1. An injected asset.
echo evil > "$t/evil.bin"; gh release upload "$name" --repo "$repo" "$t/evil.bin" >/dev/null
if publish > "$t/log1" 2>&1; then no "an injected asset was signed and published"
elif [ "$(draft)" = true ]; then ok "an asset no job made is refused; the release stays a draft"
else no "refused, but the release is no longer a draft"; fi
gh release delete-asset "$name" evil.bin --repo "$repo" --yes >/dev/null

# 2. The Mac job uploaded and failed.
echo dmg > "$t/Architect-macos-arm64.dmg"; gh release upload "$name" --repo "$repo" "$t/Architect-macos-arm64.dmg" >/dev/null
if publish > "$t/log2" 2>&1; then
    grep -q "Architect-macos-arm64.dmg is on the release but its job did not report it" "$t/log2" \
        && ok "the unreported Mac asset is deleted, and said so" || no "no deletion message"
    case "$(names)" in *Architect-macos*) no "the .dmg is still there" ;; *) ok "the .dmg is gone: $(names)" ;; esac
    [ "$(draft)" = false ] && ok "the draft is published" || no "still a draft"
    [ "$(latest)" = "$latest_before" ] && ok "Latest is still $latest_before (not the dry run)" || no "Latest moved to $(latest)"
    gh release download "$name" --repo "$repo" --dir "$t/dl" -p SHA256SUMS -p SHA256SUMS.minisig >/dev/null
    [ "$(sed -n 3p "$t/dl/SHA256SUMS.minisig")" = "trusted comment: architect $name SHA256SUMS" ] \
        && minisign -V -q -P "$ORG_RELEASE_KEY" -m "$t/dl/SHA256SUMS" -x "$t/dl/SHA256SUMS.minisig" >/dev/null \
        && [ "$(wc -l < "$t/dl/SHA256SUMS")" = 3 ] \
        && ok "SHA256SUMS: the 3 reported assets, signed, its trusted comment naming the release" || no "SHA256SUMS"
else no "the optional-asset case did not publish: $(cat "$t/log2")"; fi

# 3. A re-run.
if publish > "$t/log3" 2>&1 && grep -q "published already: nothing re-published" "$t/log3"; then
    ok "a re-run signs again and re-publishes nothing"
else no "the re-run: $(cat "$t/log3")"; fi
[ "$(latest)" = "$latest_before" ] && ok "Latest still $latest_before after the re-run" || no "Latest moved on the re-run"

for f in "$t"/log*; do echo "--- $(basename "$f")"; sed 's/^/    /' "$f"; done
exit "$fail"
