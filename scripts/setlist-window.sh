#!/usr/bin/env bash
# The setlist window: one stock Architect, every pane a development copy
# of a runtime extension, in the layout `examples/layouts/setlist-window.json`
# (Ignition's room left, Studio's arrangement over its mixer in the centre,
# Keyflow's chart text and chart right; no sidebars, status or Now Playing
# bar). `just setlist-window` runs it.
#
#   scripts/setlist-window.sh [--no-build] [--play SECONDS] [-- <dev args>…]
#
# Where things are (defaults beside this checkout):
#   SETLIST_STUDIO    a Studio checkout with extensions/{daw,arrangement,mixer} (studio#519 at
#                     fffcf831 or later: Studio's own engine, arrangement and mixer)
#   SETLIST_KEYFLOW   a Keyflow checkout (features/extension)
#   SETLIST_IGNITION  an Ignition checkout (features/lighting/pane, Ignition#19 for the session clock)
#   SETLIST_RECITAL   the recital folder: charts/, show/…/Sr. Recital Showcase.als
#   SETLIST_SESSION   what Studio's engine opens (a song's .rpp, a .session, a setlist): FTS_DAW_OPEN
#   SETLIST_CHART     the chart, a vault path under Charts/ (default "02 Drive To LA.kf")
#   SETLIST_PORT      where Keyflow serves its session (default 4143)
#   PIPEWIRE_RUNTIME_DIR  passed through: a private audio server keeps the engine off real speakers
#
# The arrangement, the mixer and the room follow Studio's engine (the room
# by its `studio` clock, the engine's socket). The chart does not yet: it
# is on a session Keyflow hosts, its own clock, which `--play` starts at
# the same moment (keyflow#97, studio#522).
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
build=1 play="" dev_args=()
while [ $# -gt 0 ]; do
  case $1 in
    --no-build) build="" ;;
    --play) play=$2; shift ;;
    --) shift; dev_args=("$@"); break ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
  shift
done
studio=${SETLIST_STUDIO:-$here/../studio}
keyflow=${SETLIST_KEYFLOW:-$here/../keyflow}
ignition=${SETLIST_IGNITION:-$here/../Ignition}
recital=${SETLIST_RECITAL:-/run/media/Development/fts-dev/recital}
session=${SETLIST_SESSION:?SETLIST_SESSION: what the engine opens (a song .rpp, a .session, a setlist)}
chart="Charts/${SETLIST_CHART:-02 Drive To LA.kf}"
port=${SETLIST_PORT:-4143}
live_set=$(ls "$recital"/show/*/*.als | head -1)
work=${XDG_CACHE_HOME:-$HOME/.cache}/architect-setlist-window
pkg=$work/pkg
app=$here/target/debug/architect-desktop
host=$(rustc -vV | sed -n 's/^host: //p')
target_of() { (cd "$1" && direnv exec . cargo metadata --no-deps --format-version 1 2>/dev/null) |
  python3 -I -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])'; }

if [ -n "$build" ]; then
  (cd "$here" && direnv exec . cargo build -p architect-desktop --features process-views)
  (cd "$studio" && devenv shell -- cargo build --profile desktop-dev --target "$host" -p daw-extension -p studio-arrangement -p studio-mixer)
  (cd "$keyflow" && direnv exec . cargo build -p keyflow-extension)
  (cd "$ignition" && direnv exec . cargo build --profile desktop-dev --target "$host" -p ignition -p ignition-pane)
fi

# The packages: each manifest and its program (copied, so a rebuild of a
# checkout reloads nothing mid-show).
mkdir -p "$pkg"/{daw,arrangement,mixer,keyflow,ignition/bin} "$work/vault/Charts"
st=$(target_of "$studio")/$host/desktop-dev
for p in daw:daw-extension arrangement:studio-arrangement mixer:studio-mixer; do
  cp "$studio/extensions/${p%%:*}/extension.toml" "$pkg/${p%%:*}/"
  cp "$st/${p#*:}" "$pkg/${p%%:*}/"
done
cp "$keyflow/features/extension/extension.toml" "$pkg/keyflow/"
cp "$(target_of "$keyflow")/debug/keyflow-extension" "$pkg/keyflow/"
cp "$ignition/features/lighting/pane/extension.toml" "$pkg/ignition/"
it=$(target_of "$ignition")/$host/desktop-dev
cp "$it/ignition" "$it/ignition-pane" "$pkg/ignition/bin/"
cp "$recital"/charts/*.kf "$work/vault/Charts/"

export FTS_DAW_OPEN=$session IGNITION_ROOT=$ignition/features/lighting IGNITION_LIVE_SET=$live_set
cd "$here"
direnv exec . "$app" dev --name setlist-window --fresh --seed "$work/vault" --only playhead \
  --extension "$pkg/daw" --extension "$pkg/arrangement" --extension "$pkg/mixer" \
  --extension "$pkg/keyflow" --extension "$pkg/ignition" \
  --layout "$here/examples/layouts/setlist-window.json" --hide all \
  --run 'KEYFLOW_PALETTES' \
  --run 'IGNITION_CAMERA_PICK={"camera":"Wide"}' "${dev_args[@]}" > "$work/dev.json"
sock=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["socket"])' "$work/dev.json")
act() { "$app" actions --app "$sock" run "$1" --json "$2"; }

# Keyflow's session: the chart's text in a session it hosts and serves;
# the chart pane on it and the room following its transport.
open=$(python3 -I -c 'import json,sys; print(json.dumps({"path": sys.argv[1], "chart": open(sys.argv[2]).read()}))' \
  "$work/chart-$(date +%s).session" "$work/vault/$chart")
id=$(act KEYFLOW_SESSION_OPEN "$open" | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["id"])')
act KEYFLOW_SESSION_SERVE "{\"id\":\"$id\",\"bind\":\"127.0.0.1:$port\"}" >/dev/null
act WORKSPACE_NAVIGATE "{\"pane\":\"setlist-chart\",\"state\":\"session:$id\"}" >/dev/null
# The room follows Studio's engine itself (its socket in $XDG_RUNTIME_DIR/fts).
act IGNITION_CLOCK_PICK '{"clock":"studio"}' >/dev/null
echo "setlist window: socket $sock, Keyflow session $id on ws://127.0.0.1:$port/vox"

if [ -n "$play" ]; then
  # Studio's engine plays (its own SESSION_PLAY_FROM through DAW_RUN); the
  # chart's session transport is started at the same moment.
  act DAW_RUN "{\"id\":\"SESSION_PLAY_FROM\",\"args\":{\"at\":$play}}" >/dev/null
  act KEYFLOW_SESSION_PLAY "{\"playing\":true,\"position\":$play}" >/dev/null
  echo "playing from ${play}s (the engine; the chart's session transport started with it: its own clock)"
fi
