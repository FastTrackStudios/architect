#!/usr/bin/env bash
# shellcheck disable=SC2015 # echo cannot fail: A && ok || fail is if-then-else here
# release-sums.sh, hermetic (`just release-sums-test`, PR checks): it signs
# only what the build jobs reported, and refuses an extra, a missing or a
# replaced asset.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
t="$(mktemp -d)"
trap 'rm -rf "$t"' EXIT
a=$(printf 'a%.0s' {1..64}) b=$(printf 'b%.0s' {1..64}) c=$(printf 'c%.0s' {1..64}) f=$(printf 'f%.0s' {1..64})
printf '%s  node.tar.gz\n%s  node.tar.gz.sha256\n%s  release-facts.json\n' "$a" "$b" "$f" > "$t/expected"
assets() { jq -n --argjson extra "${1:-[]}" '[{"name":"node.tar.gz","digest":"sha256:'"$a"'"},{"name":"node.tar.gz.sha256","digest":"sha256:'"$b"'"},{"name":"release-facts.json","digest":"sha256:'"$f"'"},{"name":"SHA256SUMS","digest":"sha256:'"$c"'"}] + $extra'; }
fail=0
ok() { echo "ok   $1"; }
no() { echo "FAIL $1" >&2; fail=1; }

assets > "$t/a.json"
out="$(bash "$here/release-sums.sh" plan "$t/expected" "$t/a.json")" \
    && [ "$out" = "$(printf '%s  node.tar.gz\n%s  release-facts.json' "$a" "$f")" ] \
    && ok "signs exactly the reported assets (no sidecar, no SHA256SUMS)" || no "the good case: $out"
assets '[{"name":"evil.tar.gz","digest":"sha256:'"$c"'"}]' > "$t/a.json"
bash "$here/release-sums.sh" plan "$t/expected" "$t/a.json" 2>/dev/null && no "an extra asset was signed" || ok "refuses an asset no build job made"
assets | jq '(.[] | select(.name == "release-facts.json") | .digest) = "sha256:'"$c"'"' > "$t/a.json"
bash "$here/release-sums.sh" plan "$t/expected" "$t/a.json" 2>/dev/null && no "a replaced asset was signed" || ok "refuses a replaced asset (facts swapped after upload)"
assets | jq '(.[] | select(.name == "node.tar.gz.sha256") | .digest) = "sha256:'"$c"'"' > "$t/a.json"
bash "$here/release-sums.sh" plan "$t/expected" "$t/a.json" 2>/dev/null && no "a replaced sidecar was accepted" || ok "refuses a replaced .sha256 sidecar"
assets | jq 'map(select(.name != "node.tar.gz"))' > "$t/a.json"
bash "$here/release-sums.sh" plan "$t/expected" "$t/a.json" 2>/dev/null && no "a missing asset was accepted" || ok "refuses when a build job's asset is missing"
: > "$t/empty"; assets > "$t/a.json"
bash "$here/release-sums.sh" plan "$t/empty" "$t/a.json" 2>/dev/null && no "signed with no build outputs" || ok "refuses when no build job reported anything"
exit "$fail"
