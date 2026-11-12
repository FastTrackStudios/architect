#!/usr/bin/env bash
# GitHub rulesets the release chain assumes, for the OWNER to apply (they
# need admin on the repo; agents never run --apply). Prints the JSON
# without --apply.
#
#   scripts/github-rulesets.sh tag architect|keyflow [--apply]
#       v* tags: only repository admins and the fts-forge App may create,
#       move or delete one. Without it any token with contents:write can push
#       a release tag, and a tag is what makes a release workflow sign.
#       fts-forge stays a bypass actor because agents cut releases as
#       fts-forge (`just release`); YOUR own tagging as the bot needs it too.
#       Studio has this ruleset already (25679805).
#
#   scripts/github-rulesets.sh immutable architect|keyflow|studio [--apply]
#       GitHub's immutable releases: once a release is PUBLISHED its assets
#       and its tag can no longer be changed or deleted (notes still can).
#       Architect publishes its release only after `sums` has signed every
#       asset (release.yml), so it can turn this on now. Studio cannot yet:
#       several workflows attach to its one Release after it is published
#       (desktop, studio-architect), which immutability would refuse; it
#       needs one publishing job that waits for all of them first.
#
#   scripts/github-rulesets.sh codeowners architect|keyflow|studio [--apply]
#       main: a PR that touches what .github/CODEOWNERS lists (the
#       workflows, the release scripts) needs the code owner's approval.
#       Admins may bypass (merge their own). fts-forge may NOT: that is the
#       point (it can push workflow files). The cost: fts-forge can no longer
#       push merge commits straight to main (the gh-stack merge workaround);
#       such stacks merge through the PR UI or by an admin.
#       Not available on infra (private, Free plan).
set -euo pipefail

FTS_FORGE_APP_ID=5391924
kind="${1:?usage: github-rulesets.sh tag|codeowners|immutable <repo> [--apply]}"
repo="${2:?repo}"
apply="${3:-}"

case "$kind" in
    immutable)
        case "$repo" in architect | keyflow | studio) ;; *) echo "immutable: architect, keyflow or studio" >&2; exit 1 ;; esac
        if [ "$apply" = --apply ]; then
            gh api -X PUT "repos/FastTrackStudios/$repo/immutable-releases" --silent && echo "immutable releases on for $repo"
        else
            echo "would run: gh api -X PUT repos/FastTrackStudios/$repo/immutable-releases (now: $(gh api "repos/FastTrackStudios/$repo/immutable-releases" --jq .enabled 2>/dev/null || echo unknown))"
        fi
        exit 0 ;;
    tag)
        case "$repo" in architect | keyflow) ;; *) echo "tag ruleset: architect or keyflow (studio has one)" >&2; exit 1 ;; esac
        json="$(cat <<JSON
{
  "name": "v* release tags (fts-forge and admins only)",
  "target": "tag",
  "enforcement": "active",
  "conditions": { "ref_name": { "include": ["refs/tags/v*"], "exclude": [] } },
  "rules": [ { "type": "creation" }, { "type": "update" }, { "type": "deletion" } ],
  "bypass_actors": [
    { "actor_id": 5, "actor_type": "RepositoryRole", "bypass_mode": "always" },
    { "actor_id": $FTS_FORGE_APP_ID, "actor_type": "Integration", "bypass_mode": "always" }
  ]
}
JSON
)" ;;
    codeowners)
        case "$repo" in architect | keyflow | studio) ;; *) echo "codeowners ruleset: architect, keyflow or studio" >&2; exit 1 ;; esac
        json="$(cat <<'JSON'
{
  "name": "main: code owners approve workflow and release changes",
  "target": "branch",
  "enforcement": "active",
  "conditions": { "ref_name": { "include": ["~DEFAULT_BRANCH"], "exclude": [] } },
  "rules": [
    { "type": "pull_request", "parameters": {
        "required_approving_review_count": 0,
        "require_code_owner_review": true,
        "dismiss_stale_reviews_on_push": true,
        "require_last_push_approval": false,
        "required_review_thread_resolution": false } }
  ],
  "bypass_actors": [
    { "actor_id": 5, "actor_type": "RepositoryRole", "bypass_mode": "pull_request" }
  ]
}
JSON
)" ;;
    *) echo "usage: github-rulesets.sh tag|codeowners|immutable <repo> [--apply]" >&2; exit 1 ;;
esac

if [ "$apply" = --apply ]; then
    gh api -X POST "repos/FastTrackStudios/$repo/rulesets" --input - <<<"$json" --jq '"created ruleset \(.id): \(.name)"'
else
    echo "$json"
fi
