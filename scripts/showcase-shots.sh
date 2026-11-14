#!/usr/bin/env bash
# The showcase's desktop screenshots (#710, docs/content/guide/showcase.md):
# the app on a fresh scratch copy of the showcase vault (scripts/showcase.sh
# env), in an isolated Loft test session per width, each note named shot
# light and dark, drawn by the app itself (APP_SHOT).
#
#   scripts/showcase-shots.sh [plain|standard] [STEP…]
#
# Each STEP, in order (default: Showcase.md):
#   <vault path>             opens it (EXT_OPEN) and shoots it
#   <ACTION>[@<json args>]   runs the action, then shoots
#   +<ACTION>[@<json args>]  runs the action, no shot
# A shot is named for its step (the note's or the action's name). Writes
# target/showcase/desktop/<name>-<width>-<light|dark>.png at the widths in
# SHOWCASE_WIDTHS (default "390 1280"). Builds nothing: build the binary
# first (`cargo build -p architect-desktop`, or the standard distribution's
# with --manifest-path dist/standard/Cargo.toml). Needs the Loft checkout
# (LOFT, else a `loft` beside this checkout or a parent) and runs in the dev shell (its
# libraries).
#
# The window opens at its width from its first frame (a niri rule set
# before launch): a resize after it crashes the desktop app in stylo, so
# nothing here resizes. The sessions are stopped when it ends.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$here"

flavour=plain
case "${1:-}" in
    standard | plain)
        flavour="$1"
        shift
        ;;
esac
steps=("$@")
[ "${#steps[@]}" -gt 0 ] || steps=(Showcase.md)
widths=(${SHOWCASE_WIDTHS:-390 1280})
height="${SHOWCASE_HEIGHT:-844}"
# The Loft checkout: LOFT, else a `loft` folder beside this checkout or
# beside one of its parents (worktrees live deeper).
loft="${LOFT:-}"
if [ -z "$loft" ]; then
    d="$here"
    while [ "$d" != / ]; do
        d="$(dirname "$d")"
        [ -f "$d/loft/flake.nix" ] && { loft="$d/loft"; break; }
    done
fi
out="$here/target/showcase/desktop"
mkdir -p "$out"

case "$flavour" in
    plain) bin="$here/target/debug/architect-desktop" ;;
    standard) bin="$here/dist/standard/target/debug/architect-desktop" ;;
esac
[ -x "$bin" ] || { echo "showcase-shots: build $bin first" >&2; exit 1; }
[ -d "$loft" ] || { echo "showcase-shots: no Loft checkout at $loft (LOFT)" >&2; exit 1; }

started=()
pids=()
cleanup() {
    if [ "${SHOWCASE_KEEP_SESSION:-0}" = 1 ]; then
        echo "showcase-shots: kept ${started[*]} (app pids ${pids[*]})"
        return
    fi
    for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done
    for s in "${started[@]}"; do
        (cd "$loft" && nix run .#dev -- --stop "$s" >/dev/null 2>&1) || true
        # A compositor can outlive a failed stop: say so rather than leave
        # it running unnoticed.
        if (cd "$loft" && nix run .#dev -- --list 2>/dev/null) | awk -v s="$s" '$1 == s && $2 == "running" { found = 1 } END { exit !found }'; then
            echo "showcase-shots: session $s is still listed after --stop; stop it with loft-dev --stop $s" >&2
            continue
        fi
        # A stopped session leaves its folder (about 200 MB) on the
        # runtime tmpfs: this script made it, so it goes too.
        rm -rf "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/loft-dev-$s"
    done
}
trap cleanup EXIT

for width in "${widths[@]}"; do
    session="showcase-$width-$$"
    envf="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/loft-dev-$session/session.env"
    # Recorded first, so a session that half-starts is still stopped.
    started+=("$session")
    # Headless and isolated, said rather than left to the defaults: its own
    # output, D-Bus and XDG_RUNTIME_DIR, so nothing here can reach the
    # person's desktop or their running Architect.
    (cd "$loft" && setsid -f nix run .#dev -- --session "$session" --headless --isolated "$loft" </dev/null >"$out/loft-$width.log" 2>&1)
    for _ in $(seq 1 240); do [ -f "$envf" ] && break; sleep 0.5; done
    [ -f "$envf" ] || { echo "showcase-shots: session $session did not start ($out/loft-$width.log)" >&2; exit 1; }
    # Every action below reaches the app's control socket under the
    # session's XDG_RUNTIME_DIR: refuse a session that would hand us the
    # host's (the person's real app).
    session_runtime="$(set -a; . "$envf"; printf '%s' "${XDG_RUNTIME_DIR:-}")"
    host_runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
    if [ -z "$session_runtime" ] || [ "$session_runtime" = "$host_runtime" ]; then
        echo "showcase-shots: session $session is not isolated (XDG_RUNTIME_DIR $session_runtime)" >&2
        exit 1
    fi
    rules="$(dirname "$envf")/overrides/niri.kdl"
    for _ in $(seq 1 60); do [ -f "$rules" ] && break; sleep 0.5; done
    sleep 2
    # Floating at its size from the first frame (Loft can rewrite the file
    # as it starts: check the rule is there before launch).
    for _ in 1 2 3; do
        grep -q "showcase-shots: $width" "$rules" 2>/dev/null && break
        printf '\n// showcase-shots: %s\nwindow-rule {\n    match title="^Architect"\n    open-floating true\n    default-column-width { fixed %s; }\n    default-window-height { fixed %s; }\n}\n' \
            "$width" "$width" "$height" >>"$rules"
        sleep 1
    done

    no_opener=0
    # A fresh scratch vault and app folder for each width.
    eval "$(SHOWCASE_DIR="$here/target/showcase/run-$width" bash scripts/showcase.sh env)"
    (
        set -a
        . "$envf"
        set +a
        unset DISPLAY
        export FTS_CONTROL=1
        exec "$bin" >"$out/app-$width.log" 2>&1
    ) &
    pids+=("$!")
    act() { (set -a; . "$envf"; set +a; "$bin" actions "$@"); }
    up=0
    for _ in $(seq 1 120); do
        if act run APP_STATUS >/dev/null 2>&1; then up=1; break; fi
        sleep 0.5
    done
    [ "$up" = 1 ] || { echo "showcase-shots: the app did not come up ($out/app-$width.log)" >&2; exit 1; }
    sleep 3
    # Nothing over the page: no tour, no inbox deck; the Vault tab.
    act run GUIDE_TOUR_SKIP >/dev/null 2>&1 || true
    act run CAPTURE_CLOSE >/dev/null 2>&1 || true
    act run LAB_GO --arg tab=vault >/dev/null 2>&1 || true

    shoot() {
        local name="$1" scheme want
        for scheme in light dark; do
            act run THEME_SET --json "{\"appearance\":\"$scheme\",\"layer\":\"pick\"}" >/dev/null
            # The shot is the window as last laid out: wait for the scheme,
            # then shoot twice (the first after a change can be a frame old).
            for _ in $(seq 1 20); do
                want="$(act run THEME_GET 2>/dev/null | python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("scheme",""))' || true)"
                [ "$want" = "$scheme" ] && break
                sleep 0.25
            done
            sleep 1
            act run APP_SHOT --arg "path=$out/$name-$width-$scheme.png" >/dev/null
            sleep 1
            act run APP_SHOT --arg "path=$out/$name-$width-$scheme.png" >/dev/null
            echo "showcase-shots: $out/$name-$width-$scheme.png"
        done
    }
    slug() { printf '%s' "$1" | sed 's/\.[a-z]*$//; s#.*/##' | tr 'A-Z' 'a-z' | tr -cs 'a-z0-9' '-' | sed 's/-$//'; }

    for step in "${steps[@]}"; do
        quiet=0
        case "$step" in +*) quiet=1; step="${step#+}" ;; esac
        if [[ "$step" =~ ^[A-Z][A-Z0-9_]*(@.*)?$ ]]; then
            id="${step%%@*}"
            json="{}"
            [ "$id" != "$step" ] && json="${step#*@}"
            act run "$id" --json "$json" >/dev/null || echo "showcase-shots: $id failed" >&2
            sleep 1.5
            [ "$quiet" = 1 ] || shoot "$(slug "$id")"
        else
            # Openers come with the extensions: wait for them on the first.
            opened=0
            tries=60
            [ "${no_opener:-0}" = 1 ] && tries=0
            for _ in $(seq 1 "$tries"); do
                act run EXT_OPEN --arg "path=$step" >/dev/null 2>&1 && { opened=1; break; }
                sleep 0.5
            done
            if [ "$opened" = 0 ]; then
                # The phone's shell has no path opener (an extension's
                # `open` of a note does nothing there): the note's pane in
                # the centre group, as the Vault opens one.
                group="$(act run WORKSPACE_LAYOUT | python3 -I -c '
import json, sys
def first(n):
    if isinstance(n, dict):
        g = n.get("group")
        if isinstance(g, dict) and "id" in g:
            return g["id"]
        for v in n.values():
            r = first(v)
            if r is not None:
                return r
    if isinstance(n, list):
        for v in n:
            r = first(v)
            if r is not None:
                return r
print(first(json.load(sys.stdin)["windows"][0]["center"]))')"
                args="$(python3 -I -c 'import json,sys; p=sys.argv[1]; print(json.dumps({"view":"vault.note","id":"showcase-"+str(abs(hash(p))),"title":p.rsplit("/",1)[-1].rsplit(".",1)[0],"state":p,"at":{"group":{"group":int(sys.argv[2])}},"focus":True}))' "$step" "$group")"
                act run WORKSPACE_OPEN --json "$args" >/dev/null || { echo "showcase-shots: nothing opens $step" >&2; exit 1; }
                no_opener=1
            fi
            sleep 1.5
            [ "$quiet" = 1 ] || shoot "$(slug "$step")"
        fi
    done
    [ "${SHOWCASE_KEEP_SESSION:-0}" = 1 ] || kill "${pids[-1]}" 2>/dev/null || true
done
