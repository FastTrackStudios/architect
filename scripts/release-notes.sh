#!/usr/bin/env bash
# architect's release notes and next version, from the cargo-rail change
# files (.changes/*.md, one per PR; .changes/README), never from commits.
#
#   scripts/release-notes.sh notes [TAG|REV]   the notes of a tag (since the
#                                              tag before it), or the draft
#                                              for a revision (default
#                                              origin/main) since its newest tag
#   scripts/release-notes.sh next [REV]        the next semver version for REV
#                                              (default origin/main)
#
# A release's notes are the change files ADDED between the previous tag and
# the tagged commit, read at that commit, in the order their PRs merged,
# grouped: major -> "Breaking / Migration", minor -> "Added", patch ->
# "Fixed and improved", none -> no entry. Nothing is deleted or committed.
# release.yml puts them in the GitHub Release; `just release-notes` shows
# them. Adapted from studio's scripts/release.sh (render_notes); architect
# keeps semver tags, so the highest bump picks the next version:
#
#   0.Y.Z: major -> 0.(Y+1).0, anything else -> 0.Y.(Z+1)
#   X.Y.Z: major -> (X+1).0.0, minor -> X.(Y+1).0, anything else -> X.Y.(Z+1)
#
# Needs only git and awk (no cargo, no cargo-rail).
set -euo pipefail

CHANGES_DIR=".changes"

die() { echo "release-notes: $*" >&2; exit 1; }

cd "$(git rev-parse --show-toplevel)"

# Release tags, newest first; a `-` suffix (none used) never counts.
release_tags() { # [git tag args...]
    git -c versionsort.suffix=- tag --list 'v[0-9]*' --sort=-v:refname "$@" \
        | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' || true
}

# The newest release tag reachable from a revision: where its notes start.
last_tag() { release_tags --merged "$1" | head -1; }

# The change files a release of <sha> lists, oldest first (the order their
# PRs merged): those ADDED between <prev> (empty: the whole history) and <sha>.
release_change_files() { # <prev> <sha>
    local prev="$1" sha="$2" added
    if [ -n "$prev" ]; then
        added="$(git diff --no-renames --diff-filter=A --name-only "$prev" "$sha" -- "$CHANGES_DIR/" | grep -E '\.md$' || true)"
    else
        added="$(git ls-tree -r --name-only "$sha" -- "$CHANGES_DIR/" | grep -E '\.md$' || true)"
    fi
    [ -n "$added" ] || return 0
    { if [ -n "$prev" ]; then git log --first-parent --diff-filter=A --reverse --format= --name-only "$prev..$sha" -- "$CHANGES_DIR" 2>/dev/null || true
      else git log --first-parent --diff-filter=A --reverse --format= --name-only "$sha" -- "$CHANGES_DIR" 2>/dev/null || true; fi
      printf '%s\n' "$added"; } \
        | awk 'NF && !seen[$0]++' | grep -Fx -f <(printf '%s\n' "$added") || true
}

# major | minor | patch | none, or "bad" for a file that is not one.
change_bump() { # <rev> <file>
    git show "$1:$2" | awk '
        NR == 1 { if ($0 != "---") { bad = 1; exit } next }
        $0 == "---" { closed = 1; exit }
        /^[[:space:]]*$/ { next }
        {
            v = $0; sub(/^[^=]*=[[:space:]]*"/, "", v); sub(/".*$/, "", v)
            r = (v == "major") ? 3 : (v == "minor") ? 2 : (v == "patch") ? 1 : (v == "none") ? 0 : -1
            if (r < 0) bad = 1
            if (r > max) max = r
        }
        END {
            if (bad || !closed) { print "bad"; exit }
            split("none patch minor major", name, " "); print name[max + 1]
        }'
}

# The note: everything after the front matter, as one list item (later
# lines and paragraphs indented under it, so a Migration: paragraph stays
# with its entry).
change_entry() { # <rev> <file>
    git show "$1:$2" \
        | awk 'c >= 2 { print; next } $0 == "---" { c++ }' \
        | sed -e '/./,$!d' \
        | awk '{ l[NR] = $0 } END { n = NR; while (n > 0 && l[n] ~ /^[[:space:]]*$/) n--
                 for (i = 1; i <= n; i++) {
                   if (i == 1) print "- " l[i]; else if (l[i] == "") print ""; else print "  " l[i] } }'
}

# Every listed file must parse; a broken one fails the release, not silently.
checked_files() { # <prev> <sha>
    local files f
    files="$(release_change_files "$1" "$2")"
    [ -n "$files" ] || return 0
    while read -r f; do
        [ "$(change_bump "$2" "$f")" != bad ] \
            || die "$f is not a change file (front matter: \"crate\" = \"none|patch|minor|major\"): fix it on main (cargo rail change add)"
    done <<<"$files"
    printf '%s\n' "$files"
}

render_notes() { # <prev> <sha>
    local prev="$1" sha="$2" files f group title out any=0
    files="$(checked_files "$prev" "$sha")"
    for group in major minor patch; do
        [ -n "$files" ] || break
        case "$group" in
            major) title="Breaking / Migration" ;;
            minor) title="Added" ;;
            patch) title="Fixed and improved" ;;
        esac
        out=""
        while read -r f; do
            [ "$(change_bump "$sha" "$f")" = "$group" ] || continue
            out+="$(change_entry "$sha" "$f")"$'\n'
        done <<<"$files"
        [ -n "$out" ] || continue
        any=1
        echo "### $title"
        echo
        printf '%s\n' "$out"
    done
    if [ "$any" = 0 ]; then
        echo "No user-visible changes (internal changes only)."
        echo
    fi
    if [ -n "$prev" ]; then
        echo "Since $prev."
    fi
}

resolve() { # <rev> -> sha
    git rev-parse -q --verify "$1^{commit}" || die "'$1' is not a tag or commit"
}

cmd_notes() { # [TAG|REV]
    local r="${1:-origin/main}" sha
    sha="$(resolve "$r")"
    if [[ "$r" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] && git rev-parse -q --verify "refs/tags/$r" >/dev/null; then
        render_notes "$(last_tag "$sha^")" "$sha"
    else
        render_notes "$(last_tag "$sha")" "$sha"
    fi
}

cmd_next() { # [REV]
    local sha prev files f b max=none x y z
    sha="$(resolve "${1:-origin/main}")"
    prev="$(last_tag "$sha")"
    [ -n "$prev" ] || { echo "v0.1.0"; return; }
    files="$(checked_files "$prev" "$sha")"
    if [ -n "$files" ]; then
        while read -r f; do
            b="$(change_bump "$sha" "$f")"
            case "$max:$b" in
                *:major | none:minor | patch:minor | none:patch) max="$b" ;;
            esac
        done <<<"$files"
    fi
    IFS=. read -r x y z <<<"${prev#v}"
    if [ "$x" = 0 ]; then
        case "$max" in
            major) echo "v0.$((y + 1)).0" ;;
            *) echo "v0.$y.$((z + 1))" ;;
        esac
    else
        case "$max" in
            major) echo "v$((x + 1)).0.0" ;;
            minor) echo "v$x.$((y + 1)).0" ;;
            *) echo "v$x.$y.$((z + 1))" ;;
        esac
    fi
}

case "${1:-}" in
    notes) shift; cmd_notes "$@" ;;
    next) shift; cmd_next "$@" ;;
    *) die "usage: release-notes.sh notes [TAG|REV] | next [REV]" ;;
esac
