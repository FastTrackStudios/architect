#!/usr/bin/env bash
# The showcase's screenshots (target/showcase/<platform>/*.png, from
# `just showcase-shots` and the web and iOS captures) onto the `media`
# branch under 710-showcase/<platform>/, where the matrix
# (docs/content/guide/showcase.md) links them from
# raw.githubusercontent.com/FastTrackStudios/architect/media/710-showcase/…
#
#   scripts/showcase-publish.sh [PUSH]      PUSH=1 pushes the branch
#
# With git's plumbing: this checkout, its branch and its index are never
# touched. A file of the same name is replaced; nothing else is removed.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$here"
push="${1:-0}"

git fetch -q origin media
index="$(mktemp)"
trap 'rm -f "$index"' EXIT
export GIT_INDEX_FILE="$index"
git read-tree origin/media
added=0
for platform in desktop web ios; do
    dir="target/showcase/$platform/"
    [ -d "$dir" ] || continue
    for f in "$dir"*.png; do
        [ -f "$f" ] || continue
        blob="$(git hash-object -w "$f")"
        git update-index --add --cacheinfo "100644,${blob},710-showcase/${platform}/$(basename "$f")"
        added=$((added + 1))
    done
done
tree="$(git write-tree)"
if [ "$tree" = "$(git rev-parse 'origin/media^{tree}')" ]; then
    echo "showcase-publish: nothing changed"
    exit 0
fi
rev="$(git rev-parse --short HEAD)"
commit="$(git commit-tree "$tree" -p origin/media -m "media: the showcase's screenshots from $rev (#710)")"
echo "showcase-publish: $added files, commit ${commit}"
if [ "$push" = 1 ]; then
    git push -q origin "${commit}:refs/heads/media"
    echo "showcase-publish: pushed to media"
fi
