#!/usr/bin/env bash
# The architect-desktop launcher (#523), run inside the package's FHS env.
# @choose@, @storeVersion@, @storeImage@ and @storeContents@ are filled in
# by package.nix.
#
# Runs the newer of the store's AppImage and the one the app's updater
# downloaded into $ARCHITECT_UPDATE_DIR (choose.sh), with APPIMAGE set to
# the file it runs and every argument passed through. The store's image is
# extracted at build time; a downloaded one is extracted once into
# $XDG_CACHE_HOME/architect/appimage, then both run the same way
# (appimage-exec.sh -w, as appimageTools.wrapType2 does). Nothing is
# printed on stdout: `architect-desktop mcp` speaks on it.
set -euo pipefail

export ARCHITECT_UPDATE_DIR="${ARCHITECT_UPDATE_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/architect/updates}"

image="$(bash @choose@ @storeVersion@ @storeImage@ "$ARCHITECT_UPDATE_DIR")"
export APPIMAGE="$image"

if [ "$image" = @storeImage@ ]; then
    appdir=@storeContents@
else
    cache="${XDG_CACHE_HOME:-$HOME/.cache}/architect/appimage"
    # The updater writes a new file and renames it into place, so size,
    # mtime and inode name one download.
    appdir="$cache/$(stat -c '%s-%Y-%i' "$image")"
    if [ ! -x "$appdir/AppRun" ]; then
        mkdir -p "$cache"
        tmp="$cache/.extract-$$"
        rm -rf "$tmp"
        appimage-exec.sh -x "$tmp" "$image" >&2
        mv -T "$tmp" "$appdir" 2>/dev/null || rm -rf "$tmp"
        # Keep this extraction and the one before it (an instance may
        # still be running from it); older ones are not run again.
        (cd "$cache" && ls -1t | tail -n +3 | xargs -r rm -rf --) || true
    fi
fi

exec appimage-exec.sh -w "$appdir" -- "$@"
