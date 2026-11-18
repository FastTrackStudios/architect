#!/usr/bin/env bash
# `just frames` (#839): the window's frame-time report for the sandbox's
# Sketch timelines, linked in and as a process, on this machine's display
# at its real refresh rate.
#
# For each view (the 2,000-clip `timeline`, the 20,000-clip
# `timeline-20k`, the `arrangement` scene) and each scenario (playing:
# the playhead runs and nothing else; the wheel; a pan; a drag of the
# host-run box), it opens the view zoomed in an isolated instance of its
# own (`dev`, a scratch folder, never the person's app data), brings the
# window forward, records `APP_FRAMES` for SECONDS with the input driven
# by the window itself, and closes the instance. Linked: `architect-sandbox`
# (Sketch compiled in). Process: `architect-desktop dev --extension` with
# Sketch's own program, its batches paced to the display, and again with
# `ARCHITECT_FRAME_PACING=0` (each batch a frame at once, as before #840).
#
# Every report is a JSON file in OUT (default target/frames/<date>), its
# frames and its summary; a table of them is printed at the end and
# written to OUT/table.md.
#
# Environment: SECONDS_EACH (5), PROFILE (release; `dev` for the
# opt-level-1 build), VIEWS, SCENARIOS, WIDTH and HEIGHT (the window's
# size in points, set through System Events on macOS and xdotool on X11;
# unset, each instance opens at its default size: on an ultrawide, give
# WIDTH=5120 HEIGHT=1440 or the display's points).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
profile=${PROFILE:-release}
secs=${SECONDS_EACH:-5}
views=${VIEWS:-timeline timeline-20k arrangement}
scenarios=${SCENARIOS:-play wheel pan drag}
out=${OUT:-$root/target/frames/$(date +%Y%m%d-%H%M%S)}
mkdir -p "$out"
scratch=$(mktemp -d "${TMPDIR:-/tmp}/architect-frames.XXXXXX")
[ -d "$scratch" ] || exit 1
trap 'rm -rf "$scratch"' EXIT

dir=debug
flag=()
if [ "$profile" = release ]; then dir=release; flag=(--release); fi
echo "building ($profile)…" >&2
cargo build -q "${flag[@]}" -p architect-desktop --bin architect-desktop \
    -p architect-sandbox --bin architect-sandbox \
    -p architect-sandbox-sketch --bin architect-sandbox-sketch
bin="$root/target/$dir"

# Sketch as a development copy: its manifest and its program.
ext="$scratch/sketch"
mkdir -p "$ext"
cp apps/sandbox/sketch/extension.toml "$ext/"
ln -s "$bin/architect-sandbox-sketch" "$ext/architect-sandbox-sketch"

# Scratch folders for every instance, so nothing touches the person's.
export ARCHITECT_CONFIG_DIR="$scratch/config"
export ARCHITECT_SANDBOX_CONFIG_DIR="$scratch/sandbox-config"
export ARCHITECT_DEV_ROOT="$scratch/dev"
mkdir -p "$ARCHITECT_CONFIG_DIR" "$ARCHITECT_SANDBOX_CONFIG_DIR" "$ARCHITECT_DEV_ROOT"

front() { # bring pid $1's window forward, and size it
    local pid=$1
    if [ "$(uname)" = Darwin ]; then
        for _ in 1 2 3; do
            osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $pid) to true" >/dev/null 2>&1 || true
            sleep 1
        done
        if [ -n "${WIDTH:-}" ] && [ -n "${HEIGHT:-}" ]; then
            osascript -e "tell application \"System Events\" to tell (first process whose unix id is $pid) to set size of front window to {$WIDTH, $HEIGHT}" >/dev/null 2>&1 || true
        fi
    elif command -v xdotool >/dev/null; then
        local w
        w=$(xdotool search --sync --pid "$pid" | tail -1)
        xdotool windowactivate "$w" || true
        if [ -n "${WIDTH:-}" ] && [ -n "${HEIGHT:-}" ]; then xdotool windowsize "$w" "$WIDTH" "$HEIGHT" || true; fi
    fi
    sleep 2
}

drive() {
    case $1 in
        play) echo '' ;;
        wheel) echo ',"drive":{"Wheel":{"at":[700,450],"dy":1,"turn":20}}' ;;
        pan) echo ',"drive":{"Drag":{"from":[700,450],"reach":[300,120],"period":1.0}}' ;;
        # The timeline's box, a host-run drag (`BOX_AT` in the pane).
        drag) echo ',"drive":{"Drag":{"from":[155,85],"reach":[250,150],"period":1.0}}' ;;
    esac
}

run() { # name app view scenario [pacing]
    local name=$1 app=$2 view=$3 scenario=$4 pacing=${5:-1}
    local label="$view-$scenario-$name" args=(dev --name "frames-$name" --only lab.sketch)
    [ "$app" = architect-desktop ] && args+=(--extension "$ext")
    ARCHITECT_FRAME_PACING=$pacing "$bin/$app" "${args[@]}" >/dev/null
    local info socket pid
    info=$("$bin/$app" dev list | python3 -c "import json,sys; i=[i for i in json.load(sys.stdin) if i['name']=='frames-$name'][0]; print(i['socket'], i['pid'])")
    socket=${info% *}; pid=${info#* }
    act() { "$bin/$app" actions --app "$socket" run "$1" --json "$2" >/dev/null; }
    act LAB_GO '{"tab":"vault"}'
    sleep 1
    act EXT_VIEW_OPEN "{\"extension\":\"lab.sketch\",\"view\":\"$view\",\"placement\":\"zoomed\"}"
    front "$pid"
    "$bin/$app" actions --app "$socket" run APP_FRAMES \
        --json "{\"seconds\":$secs,\"label\":\"$label\",\"path\":\"$out/$label.json\"$(drive "$scenario")}" >/dev/null \
        || echo "$label: no report" >&2
    "$bin/$app" dev stop "frames-$name" >/dev/null
    echo "$label" >&2
}

for view in $views; do
    for scenario in $scenarios; do
        # The arrangement is a scene with no playhead or regions.
        if [ "$view" = arrangement ] && { [ "$scenario" = play ] || [ "$scenario" = drag ]; }; then continue; fi
        run linked architect-sandbox "$view" "$scenario"
        run process architect-desktop "$view" "$scenario" 1
        run unpaced architect-desktop "$view" "$scenario" 0
    done
done

python3 - "$out" <<'EOF' | tee "$out/table.md"
import json, pathlib, sys
out = pathlib.Path(sys.argv[1])
rows = []
for p in sorted(out.glob("*.json")):
    r = json.loads(p.read_text())["report"]
    f = lambda s: "%.2f / %.2f" % (s["p50"], s["p99"])
    rows.append("| %s | %dx%d | %.0f | %.1f | %d | %s | %s | %s | %s | %s |" % (
        r["label"], r["width"], r["height"], r["refresh_hz"], r["presented_fps"], r["dropped"],
        f(r["interval"]), f(r["work"]), f(r["draw"]), f(r["render"]), f(r["input_to_frame"])))
print("| run | px | Hz | fps | dropped | interval p50/p99 ms | work | draw | render | input→frame |")
print("|---|---|---|---|---|---|---|---|---|---|")
print("\n".join(rows))
EOF
echo "reports: $out" >&2
