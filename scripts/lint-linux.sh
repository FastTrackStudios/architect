#!/usr/bin/env bash
# What main's CI runs after a merge, run on Linux before the push that
# would otherwise find it there (`just lint-linux [REF]`).
#
# A PR's own checks (pr.yml) run only the fast checks: its Linux clippy
# waits for the `full-checks` label, and no PR runs any test (checks.yml's
# Tests and wasm jobs, and browser.yml, run on main after the merge). So a
# PR could merge green and turn main red on code this machine never
# builds: a cfg that is Linux's alone, a feature combination, another
# crate's test it broke. This runs, in a fresh clone of REF on a Linux
# host, in the repo's dev shell:
#
#   1. pr.yml's lint: `cargo clippy --workspace --all-targets`, warnings
#      denied, then each feature build scripts/pr-scope.py says REF reaches;
#   2. browser.yml's web bundle build (wasm, profile `web`);
#   3. checks.yml's tests for what REF changes since its merge base with
#      origin/main: `cargo rail plan` read by scripts/ci-rail-plan.py, the
#      same filter main's Tests job uses (every test when it can't narrow),
#      and the feature-tests.toml passes under that filter.
#
# REF must be pushed (default: HEAD, which must be on origin). The host is
# ARCHITECT_LINUX_HOST (default thebattleship), its scratch folder
# ARCHITECT_LINUX_SCRATCH; the clone is removed afterwards. The output is
# streamed back; the exit status is non-zero when any part failed.
set -euo pipefail

ref="${1:-HEAD}"
host="${ARCHITECT_LINUX_HOST:-thebattleship}"
scratch="${ARCHITECT_LINUX_SCRATCH:-/run/media/Development/scratch}"
repo="${ARCHITECT_LINUX_REPO:-https://github.com/FastTrackStudios/architect}"

sha="$(git rev-parse --verify "$ref^{commit}")"
git fetch -q origin
if [ -z "$(git branch -r --contains "$sha" 2>/dev/null)" ]; then
  echo "lint-linux: $sha is not on origin; push it first" >&2
  exit 2
fi

dir="$scratch/lint-linux-${sha:0:8}-$(date +%s)"
echo "lint-linux: $sha on $host in $dir"

# shellcheck disable=SC2087 # the variables are this machine's, on purpose
ssh -o BatchMode=yes "$host" bash -s <<EOF
set -uo pipefail
dir="$dir"
trap 'cd / && rm -rf "\$dir"' EXIT
git clone -q "$repo" "\$dir" && cd "\$dir" && git checkout -q "$sha" || exit 2
git fetch -q origin main
export CARGO_BUILD_WARNINGS=deny CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_TERM_COLOR=never
timeout 7200 devenv shell -- bash -c '
fail=0
step() { echo "== \$1"; }
step "clippy (the workspace)"
cargo clippy --workspace --all-targets || fail=1
step "feature builds"
python3 scripts/pr-scope.py "\$(git merge-base origin/main HEAD)" HEAD --cmds /tmp/lint-linux-builds.\$\$ >/dev/null
bash /tmp/lint-linux-builds.\$\$ || fail=1
rm -f /tmp/lint-linux-builds.\$\$
step "the web bundle (wasm)"
cargo build -q --profile web -p architect-web --lib --target wasm32-unknown-unknown || fail=1
step "tests (what the change reaches)"
plan=/tmp/lint-linux-plan.\$\$.json
if cargo rail plan --from "\$(git merge-base origin/main HEAD)" --to HEAD --json > "\$plan" 2>/dev/null \
  && gates="\$(python3 scripts/ci-rail-plan.py "\$plan")"; then :; else gates="full=true tests=true test_filter="; fi
rm -f "\$plan"
echo "\$gates"
tests="\$(echo "\$gates" | tr " " "\n" | sed -n "s/^tests=//p")"
filter="\$(echo "\$gates" | sed -n "s/^test_filter=//p")"
if [ "\$tests" != false ]; then
  (cd features/extensions/web && npm ci --ignore-scripts --no-audit --no-fund >/dev/null 2>&1) || true
  if [ -n "\$filter" ]; then
    cargo nextest run --workspace --profile ci --no-tests=pass -E "\$filter" || fail=1
    cargo run -q -p xtask -- feature-tests --run --filter "\$filter" -- --profile ci --no-tests=pass || fail=1
  else
    cargo nextest run --workspace --profile ci --no-tests=pass || fail=1
    cargo run -q -p xtask -- feature-tests --run -- --profile ci --no-tests=pass || fail=1
  fi
fi
echo "== lint-linux: \$([ \$fail = 0 ] && echo ok || echo FAILED)"
exit \$fail
'
EOF
