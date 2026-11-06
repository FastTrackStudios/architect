#!/usr/bin/env bash
# A live check of a fleet app on a Mac: the counterpart of the Linux
# agents' lab-session.sh (a headless test desktop on THEBATTLESHIP). It
# launches the app you built, drives it through its Action List, looks at it,
# gives it real input in its own window only, runs an iOS build on a
# Simulator of its own, and quits all of it.
#
#   scripts/mac-live-check.sh launch <binary> [args…]   start it: FTS_CONTROL=1,
#                                       a config folder and a TMPDIR of its own
#                                       (so its control socket and one-instance
#                                       claim are this session's), and wait for
#                                       its socket and window
#   scripts/mac-live-check.sh act <ACTION> [--arg k=v…]  `<binary> actions run` against it
#   scripts/mac-live-check.sh cli <args…>                `<binary> actions <args…>` (list, status, …)
#   scripts/mac-live-check.sh shot <out.png> [--os]      APP_SHOT (the app paints its own
#                                       window); its window as macOS shows it (cua-driver's
#                                       capture, else `screencapture -l`) when that
#                                       fails, or with --os (the title bar, what
#                                       macOS draws)
#   scripts/mac-live-check.sh input <what> …             cua-driver, scoped to its window:
#       click <x> <y> [right] [count]   window-local pixels of `input state`'s screenshot
#       element <token> [action]        an AX element from `input state` (s0000002a:11)
#       type <text>                     into its focused element
#       key <key> [mod…]                one key (return, escape, up, a…), with modifiers
#       hotkey <mod…> <key>             a combination (cmd shift n)
#       menu <label…>                   its menu bar: menu Edit Undo
#       scroll <up|down|left|right> [amount] [x y]
#       drag <x1> <y1> <x2> <y2>
#       state [query] [--png out.png]   its AX tree (narrowed by query) and a screenshot
#       call <tool> [json]              any cua-driver tool; pid and window_id forced to its
#   scripts/mac-live-check.sh ios <App.app | bundle-id | scheme> [args…]
#                                       boot a Simulator of its own, install and launch
#                                       (FTS_CONTROL=1), wait, screenshot; a scheme is
#                                       built with xcodebuild first
#   scripts/mac-live-check.sh ios-act <ACTION> [--arg k=v…]
#                                       its Action List, over the control socket it serves
#                                       in a folder on the Mac (FTS_CONTROL_DIR; MLC_CLI:
#                                       an architect app's `actions` CLI)
#   scripts/mac-live-check.sh ios-tap <x> <y>            a tap, in points (idb)
#   scripts/mac-live-check.sh ios-shot <out.png>         the Simulator's screen again
#   scripts/mac-live-check.sh status                     what this session runs
#   scripts/mac-live-check.sh stop                       quit the app, shut down and delete
#                                       the Simulator, remove the session's folders
#
# Sessions: MLC_SESSION (default `default`) names one; several can run at
# once, each with its own app and Simulator. State is in
# ${MLC_STATE_DIR:-/tmp/mac-live-check-$USER}/<session>.
# Other knobs: MLC_TIMEOUT (seconds to wait for a socket or window, 90),
# MLC_CONFIG_ENV (the app's config-folder variable, ARCHITECT_CONFIG_DIR;
# ARCHITECT_SANDBOX_CONFIG_DIR for architect-sandbox),
# MLC_IOS_DEVICE (the Simulator's device type, the newest iPhone),
# MLC_DELIVERY (cua-driver's delivery for clicks and keys: foreground,
# as a winit window hears no input in the background; or background),
# MLC_SHOT_SETTLE (seconds before a shot, 1), MLC_IOS_SETTLE (seconds
# between launch and screenshot, 8), MLC_KEEP=1
# (stop keeps the config folder and log).
#
# It never acts on another window: every input names the launched pid and
# its window id, and `call` overwrites both. Docs:
# docs/content/guide/live-check-mac.md.
set -euo pipefail

SESSION="${MLC_SESSION:-default}"
STATE="${MLC_STATE_DIR:-/tmp/mac-live-check-${USER:-$(id -un)}}/$SESSION"
TIMEOUT="${MLC_TIMEOUT:-90}"
CONFIG_ENV="${MLC_CONFIG_ENV:-ARCHITECT_CONFIG_DIR}"

die() { echo "mac-live-check: $*" >&2; exit 1; }
say() { echo "mac-live-check: $*" >&2; }

get() { [[ -f "$STATE/$1" ]] && cat "$STATE/$1" || true; }
put() { mkdir -p "$STATE"; printf '%s' "$2" >"$STATE/$1"; }
alive() { local pid; pid="$(get pid)"; [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; }
need_app() { alive || die "no app running in session '$SESSION' (launch first)"; }
abspath() { (cd "$(dirname "$1")" && printf '%s/%s' "$(pwd -P)" "$(basename "$1")"); }

# The socket the app serves in its own TMPDIR (`<tmp>/fts/<app>.sock`).
socket() {
    local s
    s="$(get socket)"
    if [[ -z "$s" || ! -S "$s" ]]; then
        s="$(ls "$(get tmp)"/fts/*.sock 2>/dev/null | head -1 || true)"
        [[ -n "$s" ]] && put socket "$s"
    fi
    printf '%s' "$s"
}

need_cua() {
    command -v cua-driver >/dev/null || die "cua-driver is not installed (see the cua-driver skill)"
}

# The app's main window: its largest layer-0 window.
find_window() {
    cua-driver call list_windows "{\"pid\":$(get pid)}" 2>/dev/null |
        jq -r '[.windows[]? | select(.layer == 0)]
               | max_by((.bounds.width // 0) * (.bounds.height // 0)) | .window_id // empty' 2>/dev/null ||
        true
}

window() {
    local w
    w="$(get window)"
    if [[ -z "$w" ]] || ! cua-driver call list_windows "{\"pid\":$(get pid)}" 2>/dev/null |
        jq -e --argjson w "$w" '.windows[]? | select(.window_id == $w)' >/dev/null; then
        w="$(find_window)"
        [[ -n "$w" ]] || die "the app (pid $(get pid)) has no window"
        put window "$w"
    fi
    printf '%s' "$w"
}

has_socket() { [[ -n "$(socket)" ]]; }
has_window() { [[ -n "$(find_window)" ]]; }

# Wait until `$@` succeeds, up to MLC_TIMEOUT seconds.
wait_for() {
    local what="$1" i=0
    shift
    until "$@"; do
        alive || die "the app exited before $what (log: $STATE/app.log)"$'\n'"$(tail -20 "$STATE/app.log" 2>/dev/null)"
        ((i++ < TIMEOUT * 4)) || die "no $what after ${TIMEOUT}s (log: $STATE/app.log)"
        sleep 0.25
    done
}

cmd_launch() {
    [[ $# -ge 1 ]] || die "launch <binary> [args…]"
    alive && die "session '$SESSION' already runs pid $(get pid) (stop it, or set MLC_SESSION)"
    local bin
    bin="$(abspath "$1")"
    shift
    [[ -x "$bin" ]] || die "$bin is not an executable"
    rm -rf "$STATE"
    mkdir -p "$STATE"
    # Short paths: a Unix socket's path is at most 104 bytes on macOS.
    local tmp cfg
    tmp="$(mktemp -d /tmp/mlc.XXXXXX)"
    cfg="$(mktemp -d /tmp/mlc-config.XXXXXX)"
    put bin "$bin"
    put tmp "$tmp"
    put config "$cfg"
    env FTS_CONTROL=1 TMPDIR="$tmp/" "$CONFIG_ENV=$cfg" \
        nohup "$bin" "$@" >"$STATE/app.log" 2>&1 </dev/null &
    put pid "$!"
    say "pid $(get pid), config $cfg, log $STATE/app.log"
    wait_for "control socket" has_socket
    say "socket $(socket)"
    if command -v cua-driver >/dev/null; then
        wait_for "window" has_window
        put window "$(find_window)"
        say "window $(get window)"
    fi
}

app_cli() { need_app; "$(get bin)" actions --app "$(socket)" "$@"; }

cmd_act() {
    [[ $# -ge 1 ]] || die "act <ACTION> [--arg k=v…]"
    app_cli run "$@"
}

cmd_shot() {
    [[ $# -ge 1 ]] || die "shot <out.png> [--os]"
    need_app
    local out
    out="$(abspath "$1")"
    # A frame for what the last action changed.
    sleep "${MLC_SHOT_SETTLE:-1}"
    if [[ "${2:-}" != "--os" ]] && app_cli shot "$out" >/dev/null 2>"$STATE/shot.err" && [[ -s "$out" ]]; then
        say "APP_SHOT → $out"
        return
    fi
    [[ "${2:-}" == "--os" ]] || say "APP_SHOT failed ($(tail -1 "$STATE/shot.err")); the window as macOS shows it"
    os_shot "$out"
}

# The window as macOS composites it: cua-driver's capture (its Screen
# Recording grant), else `screencapture -l` (the caller's own grant).
os_shot() {
    local out="$1"
    need_cua
    rm -f "$out"
    if cua get_window_state '{"include_accessibility_tree":false}' "$out" >/dev/null 2>&1 && [[ -s "$out" ]]; then
        say "cua-driver capture of window $(window) → $out"
    elif screencapture -x -o -l "$(window)" "$out" 2>/dev/null && [[ -s "$out" ]]; then
        say "screencapture -l $(window) → $out"
    else
        die "could not capture window $(window): grant Screen Recording to cua-driver (cua-driver permissions grant)"
    fi
}

# The session's cua-driver session, started or revived (cua-driver ends
# one left idle, and refuses its label until it is started again), once
# per run of this script.
CUA_STARTED=0
cua_session() {
    [[ "$CUA_STARTED" == 1 ]] && return
    cua-driver call start_session "{\"session\":\"mlc-$SESSION\"}" >/dev/null 2>&1 || true
    CUA_STARTED=1
}

# One cua-driver call with the app's pid and window forced into `json`.
cua() {
    local tool="$1" json="${2:-}" png="${3:-}"
    [[ -n "$json" ]] || json='{}'
    need_app
    need_cua
    cua_session
    # One cua-driver session per live-check session: a pixel click is
    # refused unless the same session read the window first.
    json="$(jq -c --argjson pid "$(get pid)" --argjson w "$(window)" --arg s "mlc-$SESSION" \
        '. + {pid: $pid, window_id: $w, session: $s}' <<<"$json")" ||
        die "not a JSON object: $2"
    # winit takes no input posted to a window that is not in front: a
    # Blitz app hears a click or a key only when cua-driver fronts its
    # window for it (and gives the front back after).
    case "$tool" in
    click | double_click | right_click | type_text | press_key | hotkey | scroll | drag)
        json="$(jq -c --arg d "${MLC_DELIVERY:-foreground}" '{delivery_mode: $d} + .' <<<"$json")"
        ;;
    esac
    if [[ -n "$png" ]]; then
        cua-driver call "$tool" "$json" --screenshot-out-file "$png"
    else
        cua-driver call "$tool" "$json"
    fi
}

# A read of the window, so pixel input has the screenshot it is relative to.
read_window() { cua get_window_state '{"include_accessibility_tree":false}' >/dev/null; }

# `a b c` as a JSON array of strings.
strings() { jq -cn '$ARGS.positional' --args "$@"; }

cmd_input() {
    [[ $# -ge 1 ]] || die "input <click|element|type|key|hotkey|menu|scroll|drag|state|call> …"
    local what="$1"
    shift
    case "$what" in
    click)
        [[ $# -ge 2 ]] || die "input click <x> <y> [right] [count]"
        read_window
        cua click "$(jq -cn --argjson x "$1" --argjson y "$2" --arg b "${3:-left}" --argjson n "${4:-1}" \
            '{x: $x, y: $y, button: $b, count: $n}')"
        ;;
    element)
        [[ $# -ge 1 ]] || die "input element <token> [action]"
        cua click "$(jq -cn --arg t "$1" --arg a "${2:-press}" '{element_token: $t, action: $a}')"
        ;;
    type)
        [[ $# -ge 1 ]] || die "input type <text>"
        cua type_text "$(jq -cn --arg t "$*" '{text: $t}')"
        ;;
    key)
        [[ $# -ge 1 ]] || die "input key <key> [mod…]"
        local key="$1"
        shift
        cua press_key "$(jq -cn --arg k "$key" --argjson m "$(strings "$@")" '{key: $k, modifiers: $m}')"
        ;;
    hotkey)
        [[ $# -ge 2 ]] || die "input hotkey <mod…> <key>"
        cua hotkey "$(jq -cn --argjson k "$(strings "$@")" '{keys: $k}')"
        ;;
    menu)
        [[ $# -ge 1 ]] || die "input menu <label…>"
        cua invoke_menu "$(jq -cn --argjson p "$(strings "$@")" '{path: $p}')"
        ;;
    scroll)
        [[ $# -ge 1 ]] || die "input scroll <up|down|left|right> [amount] [x y]"
        local j
        j="$(jq -cn --arg d "$1" --argjson a "${2:-3}" '{direction: $d, amount: $a}')"
        [[ $# -ge 4 ]] && read_window && j="$(jq -c --argjson x "$3" --argjson y "$4" '. + {x: $x, y: $y}' <<<"$j")"
        cua scroll "$j"
        ;;
    drag)
        [[ $# -ge 4 ]] || die "input drag <x1> <y1> <x2> <y2>"
        read_window
        cua drag "$(jq -cn --argjson a "$1" --argjson b "$2" --argjson c "$3" --argjson d "$4" \
            '{from_x: $a, from_y: $b, to_x: $c, to_y: $d}')"
        ;;
    state)
        local query="" png=""
        while [[ $# -gt 0 ]]; do
            case "$1" in
            --png) png="$(abspath "$2")"; shift 2 ;;
            *) query="$1"; shift ;;
            esac
        done
        local j='{}'
        [[ -n "$query" ]] && j="$(jq -cn --arg q "$query" '{query: $q}')"
        cua get_window_state "$j" "$png"
        ;;
    call)
        [[ $# -ge 1 ]] || die "input call <tool> [json]"
        cua "$1" "${2:-}"
        ;;
    *) die "input: unknown '$what'" ;;
    esac
}

# --- iOS Simulator ---------------------------------------------------------

sim() { xcrun simctl "$@"; }

# The session's Simulator, created and booted when it has none: the
# newest iOS runtime's newest iPhone (the runtime lists it first), or
# MLC_IOS_DEVICE (a device type's name or identifier).
ios_boot() {
    local udid runtime device
    udid="$(get sim)"
    if [[ -z "$udid" ]] || ! sim list devices | grep -q "$udid"; then
        runtime="$(sim list runtimes -j |
            jq -r '[.runtimes[] | select(.platform == "iOS" and .isAvailable)] | last | .identifier // empty')"
        [[ -n "$runtime" ]] || die "no iOS Simulator runtime installed (Xcode › Settings › Components)"
        device="${MLC_IOS_DEVICE:-$(sim list runtimes -j | jq -r --arg r "$runtime" \
            '.runtimes[] | select(.identifier == $r) | [.supportedDeviceTypes[] | select(.name | startswith("iPhone"))] | first | .identifier')}"
        mkdir -p "$STATE"
        udid="$(sim create "mac-live-check-$SESSION" "$device" "$runtime")" || die "could not create a $device Simulator"
        put sim "$udid"
        say "simulator $udid ($device, $runtime)"
    fi
    sim boot "$udid" 2>/dev/null || true
    sim bootstatus "$udid" -b >/dev/null || die "simulator $udid did not boot"
}

# A scheme of the Xcode project or workspace here, built for the Simulator.
ios_build_scheme() {
    local scheme="$1" udid="$2" where=()
    local ws proj
    ws="$(ls -d ./*.xcworkspace 2>/dev/null | head -1 || true)"
    proj="$(ls -d ./*.xcodeproj 2>/dev/null | head -1 || true)"
    if [[ -n "$ws" ]]; then where=(-workspace "$ws"); elif [[ -n "$proj" ]]; then where=(-project "$proj"); else
        die "'$scheme' is not an .app or an installed bundle id, and there is no Xcode project here"
    fi
    xcodebuild "${where[@]}" -scheme "$scheme" -destination "id=$udid" \
        -derivedDataPath "$STATE/derived" build >"$STATE/xcodebuild.log" 2>&1 ||
        die "xcodebuild failed (log: $STATE/xcodebuild.log)"
    ls -d "$STATE"/derived/Build/Products/*-iphonesimulator/*.app | head -1
}

cmd_ios() {
    [[ $# -ge 1 ]] || die "ios <App.app | bundle-id | scheme> [args…]"
    local what="$1" udid bid app=""
    shift
    ios_boot
    udid="$(get sim)"
    if [[ -d "$what" && "$what" == *.app ]]; then
        app="$(abspath "$what")"
    elif sim get_app_container "$udid" "$what" >/dev/null 2>&1; then
        bid="$what"
    else
        app="$(ios_build_scheme "$what" "$udid")"
    fi
    if [[ -n "$app" ]]; then
        bid="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Info.plist")"
        sim install "$udid" "$app"
        say "installed $bid"
    fi
    put bundle "$bid"
    # Its control socket in a short folder on the Mac (a Simulator app's
    # files are the Mac's; its own temporary folder is too deep for a
    # socket's path): FTS_CONTROL_DIR, for `ios-act`.
    local iotmp
    iotmp="$(get iostmp)"
    [[ -n "$iotmp" ]] || { iotmp="$(mktemp -d /tmp/mlc-ios.XXXXXX)"; put iostmp "$iotmp"; }
    SIMCTL_CHILD_FTS_CONTROL=1 SIMCTL_CHILD_FTS_CONTROL_DIR="$iotmp" \
        sim launch --terminate-running-process "$udid" "$bid" "$@" >/dev/null
    say "launched $bid; waiting ${MLC_IOS_SETTLE:-8}s"
    sleep "${MLC_IOS_SETTLE:-8}"
    cmd_ios_shot "${MLC_IOS_SHOT:-/tmp/mlc-$SESSION-ios.png}"
}

# A tap on the Simulator's screen, in points (a screenshot's pixels over
# its scale, 3 on an iPhone): idb (`idb ui tap`; MLC_IDB names it).
cmd_ios_tap() {
    [[ $# -ge 2 ]] || die "ios-tap <x> <y> (points)"
    local idb udid
    idb="${MLC_IDB:-$(command -v idb || true)}"
    [[ -n "$idb" ]] || die "ios-tap needs idb (pip install fb-idb, and idb_companion; MLC_IDB=<path>)"
    udid="$(get sim)"
    [[ -n "$udid" ]] || die "no Simulator in session '$SESSION' (ios first)"
    local companion=()
    command -v idb_companion >/dev/null && companion=(--companion-path "$(command -v idb_companion)")
    "$idb" ${companion[@]+"${companion[@]}"} ui tap --udid "$udid" "$1" "$2"
}

# `actions --app <its socket> run …` against the Simulator's app, with an
# architect app's `actions` CLI (MLC_CLI, else the session's launched one).
cmd_ios_act() {
    [[ $# -ge 1 ]] || die "ios-act <ACTION> [--arg k=v…]"
    local cli sock
    cli="${MLC_CLI:-$(get bin)}"
    [[ -x "$cli" ]] || die "ios-act needs an architect CLI: MLC_CLI=target/debug/architect-desktop"
    sock="$(ls "$(get iostmp)"/*.sock 2>/dev/null | head -1 || true)"
    [[ -n "$sock" ]] || die "the Simulator's app serves no control socket (a debug build, FTS_CONTROL=1)"
    "$cli" actions --app "$sock" run "$@"
}

cmd_ios_shot() {
    [[ $# -ge 1 ]] || die "ios-shot <out.png>"
    local udid out
    udid="$(get sim)"
    [[ -n "$udid" ]] || die "no Simulator in session '$SESSION' (ios first)"
    out="$(abspath "$1")"
    sim io "$udid" screenshot --mask=black "$out" >/dev/null 2>&1
    say "simulator screenshot → $out"
    echo "$out"
}

cmd_status() {
    [[ -d "$STATE" ]] || { echo "session '$SESSION': nothing running"; return; }
    echo "session '$SESSION' ($STATE)"
    if alive; then echo "  app     pid $(get pid) $(get bin)"; else echo "  app     not running"; fi
    [[ -n "$(get pid)" ]] && echo "  socket  $(socket)" && echo "  window  $(get window)" && echo "  config  $(get config)"
    [[ -n "$(get sim)" ]] && echo "  ios     $(get sim) $(get bundle)"
    return 0
}

cmd_stop() {
    local pid udid
    pid="$(get pid)"
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
        # What it started (an extension's process) first, while it is
        # still their parent.
        pkill -TERM -P "$pid" 2>/dev/null || true
        kill -TERM "$pid" 2>/dev/null || true
        for _ in $(seq 20); do kill -0 "$pid" 2>/dev/null || break; sleep 0.25; done
        kill -KILL "$pid" 2>/dev/null || true
        say "quit pid $pid"
    fi
    udid="$(get sim)"
    if [[ -n "$udid" ]]; then
        # idb's companion for it, if `ios-tap` started one.
        pkill -f "idb_companion --udid $udid" 2>/dev/null || true
        sim shutdown "$udid" 2>/dev/null || true
        sim delete "$udid" 2>/dev/null || true
        say "shut down and deleted simulator $udid"
    fi
    if [[ "${MLC_KEEP:-0}" == 1 ]]; then
        say "kept $(get config) and $STATE"
        return
    fi
    for d in "$(get tmp)" "$(get config)" "$(get iostmp)"; do [[ "$d" == /tmp/mlc* ]] && rm -rf "$d"; done
    rm -rf "$STATE"
}

[[ $# -ge 1 ]] || { sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 2; }
command="$1"
shift
case "$command" in
launch) cmd_launch "$@" ;;
act) cmd_act "$@" ;;
cli) app_cli "$@" ;;
shot) cmd_shot "$@" ;;
input) cmd_input "$@" ;;
ios) cmd_ios "$@" ;;
ios-shot) cmd_ios_shot "$@" ;;
ios-act) cmd_ios_act "$@" ;;
ios-tap) cmd_ios_tap "$@" ;;
status) cmd_status ;;
stop) cmd_stop ;;
-h | --help | help) sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//' ;;
*) die "unknown command '$command' (launch, act, cli, shot, input, ios, ios-act, ios-tap, ios-shot, status, stop)" ;;
esac
