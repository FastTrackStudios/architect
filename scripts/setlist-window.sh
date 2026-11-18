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
#   SETLIST_KEYFLOW   a Keyflow checkout (features/extension; main at 6687b931 or later, which
#                     declares [uses] "fts.session" for following the engine's session)
#   SETLIST_IGNITION  an Ignition checkout (features/lighting/pane, Ignition#19 for the session clock)
#   SETLIST_RECITAL   the recital folder: charts/, show/…/Sr. Recital Showcase.als
#   SETLIST_SESSION   what Studio's engine opens (a song's .rpp, a .session, a setlist): FTS_DAW_OPEN
#   SETLIST_CHART     the chart, a vault path under Charts/ (default "02 Drive To LA.kf")
#   PIPEWIRE_RUNTIME_DIR  passed through: a private audio server keeps the engine off real speakers
#
# One session, one transport (studio#522): Studio's engine (the `daw`
# extension) opens the song's session document and leads its transport;
# Keyflow's chart and Ignition's room and cue list follow that session.
# The handshake below is the one agreed on studio#522.
#
#   --play SECONDS   play from there      --seek SECONDS   then seek there
#   SETLIST_BAR_ONE  the song's bar 1 for the engine (FTS_DAW_BAR_ONE; Drive To LA: 5)
#   SETLIST_LANE, SETLIST_LOCATOR, SETLIST_SLUG, SETLIST_CUES: the room's song
#                    (defaults: Drive To LA's)
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
build=1 play="" seek="" dev_args=()
while [ $# -gt 0 ]; do
  case $1 in
    --no-build) build="" ;;
    --play) play=$2; shift ;;
    --seek) seek=$2; shift ;;
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
bar_one=${SETLIST_BAR_ONE:-5}
lane=${SETLIST_LANE:-dtla}
locator=${SETLIST_LOCATOR:-Drive To LA}
slug=${SETLIST_SLUG:-drive-to-la}
cues=${SETLIST_CUES:-$ignition/features/lighting/data/songs/jt-recital/$slug.json}
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

# The packages: each extension folder's own files (manifest, icons, images,
# data; not its sources) and its program, copied so a rebuild of a checkout
# reloads nothing mid-show. A toolbar's `image` or a view's `program` the
# manifest names and the package lacks fails here, loudly: missing, the
# host shows the item's text label instead of its icon.
pack() { # pack <extension folder> <package>
  rm -rf "$2" && mkdir -p "$2"
  for f in "$1"/* "$1"/.architect; do
    [ -e "$f" ] || continue
    case $(basename "$f") in src | tests | target | Cargo.toml | Cargo.lock | build.rs | README.md) ;; *) cp -r "$f" "$2/" ;; esac
  done
}
check_files() { # check_files <package>: every file its manifest names is there
  python3 -I - "$1" <<'PY'
import os, re, sys
pkg = sys.argv[1]
text = open(os.path.join(pkg, "extension.toml")).read()
named = set(re.findall(r'^\s*(?:image|program)\s*=\s*"([^"]+)"', text, re.M))
missing = [n for n in sorted(named) if "/" in n or n.endswith((".svg", ".png"))
           if not os.path.exists(os.path.join(pkg, n))]
if missing:
    sys.exit(f"setlist-window: {pkg} lacks files its manifest names: {', '.join(missing)}")
PY
}
st=$(target_of "$studio")/$host/desktop-dev
for p in daw:daw-extension arrangement:studio-arrangement mixer:studio-mixer; do
  pack "$studio/extensions/${p%%:*}" "$pkg/${p%%:*}"
  cp "$st/${p#*:}" "$pkg/${p%%:*}/"
done
pack "$keyflow/features/extension" "$pkg/keyflow"
cp "$(target_of "$keyflow")/debug/keyflow-extension" "$pkg/keyflow/"
pack "$ignition/features/lighting/pane" "$pkg/ignition"
it=$(target_of "$ignition")/$host/desktop-dev
mkdir -p "$pkg/ignition/bin" && cp "$it/ignition" "$it/ignition-pane" "$pkg/ignition/bin/"
for p in daw arrangement mixer keyflow ignition; do check_files "$pkg/$p"; done
mkdir -p "$work/vault/Charts"
cp "$recital"/charts/*.kf "$work/vault/Charts/"

export FTS_DAW_OPEN=$session FTS_DAW_BAR_ONE=$bar_one IGNITION_ROOT=$ignition/features/lighting IGNITION_LIVE_SET=$live_set
cd "$here"
direnv exec . "$app" dev --name setlist-window --fresh --seed "$work/vault" --only playhead \
  --extension "$pkg/daw" --extension "$pkg/arrangement" --extension "$pkg/mixer" \
  --extension "$pkg/keyflow" --extension "$pkg/ignition" \
  --layout "$here/examples/layouts/setlist-window.json" --hide all \
  --run 'KEYFLOW_SET_CHART_THEME={"follow":true}' \
  --run 'IGNITION_CAMERA_PICK={"camera":"Wide"}' "${dev_args[@]}" > "$work/dev.json"
sock=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["socket"])' "$work/dev.json")
act() { "$app" actions --app "$sock" run "$1" --json "$2"; }
json() { python3 -I -c 'import json,sys; print(json.dumps(dict(zip(sys.argv[1::2], sys.argv[2::2]))))' "$@"; }

# The engine's session, once it is open and led: {path, id, url}.
url="" id=""
for _ in $(seq 120); do
  s=$("$app" actions --app "$sock" run DAW_STATE 2>/dev/null |
    python3 -I -c 'import json,sys; s=json.load(sys.stdin).get("session") or {}; print(s.get("url") or "", s.get("id") or "")' || true)
  url=${s% *} id=${s#* }
  [ -n "$url" ] && [ -n "$id" ] && break
  sleep 1
done
[ -n "$url" ] || { echo "setlist-window: the engine's session never came up (DAW_STATE)" >&2; exit 1; }

# The handshake (studio#522), in its order.
act EXT_SERVICE_USE '{"service":"fts.session","provider":"daw"}' >/dev/null
act KEYFLOW_SESSION_FOLLOW "$(json url "$url" id "$id" chart_path "$work/vault/$chart")" >/dev/null
act WORKSPACE_NAVIGATE "{\"pane\":\"setlist-chart\",\"state\":\"session:$id\"}" >/dev/null
act IGNITION_SESSION_SONG "$(python3 -I -c 'import json,sys; print(json.dumps({"doc":sys.argv[1],"lane":sys.argv[2],"live_set":sys.argv[3],"locator":sys.argv[4],"cues":sys.argv[5],"tempo":False}))' \
  "$id" "$lane" "$live_set" "$locator" "$cues")" >/dev/null
act IGNITION_SESSION_ROOM "$(json url "$url" doc "$id" song "$slug" lane "$lane")" >/dev/null
echo "setlist window: socket $sock; the engine's session $id, led at $url"

# The chart comes up with its icons and its palettes, or this says so.
state=$(act KEYFLOW_STATE '{}' 2>/dev/null || true)
if ! python3 -I -c 'import json,sys; s=json.loads(sys.argv[1] or "{}"); sys.exit(0 if s.get("palettes", True) else 1)' "$state"; then
  echo "setlist-window: the chart's palettes sidebar is closed" >&2; exit 1
fi

if [ -n "$play" ]; then
  act DAW_RUN "{\"id\":\"SESSION_PLAY_FROM\",\"args\":{\"at\":$play}}" >/dev/null
  echo "playing from ${play}s: one transport, the engine's"
fi
if [ -n "$seek" ]; then
  act DAW_RUN "{\"id\":\"SESSION_SEEK\",\"args\":{\"at\":$seek}}" >/dev/null
  echo "seeked to ${seek}s"
fi
