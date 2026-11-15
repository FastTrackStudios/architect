#!/usr/bin/env bash
# shellcheck disable=SC2015 # echo cannot fail: A && ok || fail is if-then-else here
# release-notes.sh's data-migration facts, in a throwaway git repo (no
# network): `just release-notes-test`, and the release workflow's lint job.
#   - a release whose change files declare `Data-Migration:` -> data_migration true,
#     the notes open with "### Data migration";
#   - the next release, without one -> data_migration false;
#   - quotes and backslashes in a declaration stay valid JSON.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
t="$(mktemp -d)"
trap 'rm -rf "$t"' EXIT
cd "$t"
git init -q -b main
git config user.name test; git config user.email test@example.invalid
mkdir -p scripts .changes
cp "$here/release-notes.sh" scripts/
change() { printf -- '---\n"architect" = "%s"\n---\n\n%s\n' "$2" "$3" >".changes/$1.md"; git add -A; git commit -qm "$1"; }
change base patch "Fixed a thing"
git tag v0.1.0
change notes-column minor $'Added a column to notes\n\nData-Migration: notes gain a "pinned" column (a \\ backslash), not undone by a rollback'
change other patch "Fixed another thing"
git tag v0.1.1
change later patch "Fixed a later thing"
git tag v0.1.2
fail=0
facts="$(bash scripts/release-notes.sh facts v0.1.1)"
python3 -c 'import json,sys; f=json.loads(sys.argv[1]); assert f["data_migration"] is True and f["tag"]=="v0.1.1" and "pinned" in f["migrations"][0], f' "$facts" \
    && echo "ok   v0.1.1 declares its migration: $facts" || { echo "FAIL v0.1.1: $facts" >&2; fail=1; }
notes="$(bash scripts/release-notes.sh notes v0.1.1)"
[ "$(sed -n 1p <<<"$notes")" = "### Data migration" ] \
    && echo "ok   v0.1.1's notes open with ### Data migration" || { echo "FAIL v0.1.1's notes" >&2; fail=1; }
facts="$(bash scripts/release-notes.sh facts v0.1.2)"
python3 -c 'import json,sys; f=json.loads(sys.argv[1]); assert f["data_migration"] is False and f["migrations"]==[], f' "$facts" \
    && echo "ok   v0.1.2 migrates nothing: $facts" || { echo "FAIL v0.1.2: $facts" >&2; fail=1; }
exit "$fail"
