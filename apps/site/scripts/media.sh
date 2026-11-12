#!/usr/bin/env bash
# The screenshots this site adds to the landing page's own (apps/web/landing/media),
# derived from the scenarios' output and committed in media/ as the last good
# copy, the way src/data is: the build never needs the network or an encoder.
#
# Every file is a real capture, resized or cropped to its window, never drawn:
#   MEDIA_DIR   a checkout of this repo's orphan `media` branch (#133)
#               git fetch origin media && git worktree add /tmp/media FETCH_HEAD
#   STUDIO_DIR  FastTrackStudios/studio's share-page captures (studio#406),
#               pr-398/*.png at d91a6781356a9116dd1aaf5ab3f529ed1dd99af6
#
#   MEDIA_DIR=/tmp/media STUDIO_DIR=/tmp/studio bash scripts/media.sh
#
# Needs ImageMagick (`nix shell nixpkgs#imagemagick`). The media branch was at
# d8fb6a40 when these were made.
set -euo pipefail
site="$(cd "$(dirname "$0")/.." && pwd)"
out="$site/media"
m="${MEDIA_DIR:?MEDIA_DIR: a checkout of the media branch}"
s="${STUDIO_DIR:-}"
mkdir -p "$out"

# name  source  [crop WxH+X+Y]
shot() {
    local name="$1" src="$2" crop="${3:-}"
    local args=()
    [ -n "$crop" ] && args+=(-crop "$crop" +repage)
    magick "$src" "${args[@]}" -strip -quality 82 -define webp:method=6 "$out/$name.webp"
}
# A second, narrower copy for phones.
small() {
    magick "$out/$1.webp" -resize "$2" -quality 80 -define webp:method=6 "$out/$1-$2.webp"
}

shot hero-notes     "$m/442-editor/garden-tasks-callouts.png"
shot hero-tasks     "$m/445-capture/desk-composer-live-chips.png"
shot hero-projects  "$m/468-decks/final-album.png"
# The window alone: these scenarios run it on a desktop.
shot hero-files     "$m/347-registry/3-after-pin.png"            681x757+12+39
shot calendar       "$m/444-dayplans/3-week-with-plans.png"
shot setlist        "$m/468-decks/final-song.png"
for n in hero-notes hero-tasks hero-projects calendar setlist; do small "$n" 840; done

if [ -n "$s" ]; then
    # The song's page alone, without the site's sidebar.
    shot studio-follow "$s/song-desktop.png" 730x890+300+60
fi
ls -la "$out"
