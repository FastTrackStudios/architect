#!/usr/bin/env bash
# The extension showcase on the desk (#710, docs/content/guide/showcase.md):
# the Architect app on a fresh scratch copy of apps/ui/showcase-vault, with
# a folder of its own and every showcase extension loaded as a development
# copy (reloaded on save): examples/showcase, the sandbox's Luau
# extensions, its process extensions and WASI component (built first), the
# example counter and the web view board (its npm build first).
#
#   scripts/showcase.sh [standard|plain] [ARGS…]
#   scripts/showcase.sh env      prepare the scratch folder, print the exports
#
# standard (the default) runs the standard distribution (dist/standard: the
# app with its core extensions, Studio's setlists among them); plain runs
# Architect's own app (apps/desktop), which builds faster. ARGS go to the
# app (`actions list`, …).
#
# SHOWCASE_DIR moves the scratch folder (default: devenv's state, else
# .devenv/state, under showcase/); SHOWCASE_KEEP=1 keeps the vault and the
# app's folder from the last run instead of starting over. The run never
# touches the keychain, the person's own app folder or a real vault.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$here"

flavour=standard
case "${1:-}" in
    standard | plain | env)
        flavour="$1"
        shift
        ;;
esac

scratch="${SHOWCASE_DIR:-${DEVENV_STATE:-$here/.devenv/state}/showcase}"
vault="$scratch/vault"
config="$scratch/config"
if [ "${SHOWCASE_KEEP:-0}" != "1" ]; then
    rm -rf "$vault" "$config"
fi
mkdir -p "$config"

# The development copies: every showcase extension, each reloaded on save.
# The showcase registry's packages (examples/showcase-registry) are left
# out on purpose: they show installing from a registry, through its
# consent prompt and the vault's "Install all", not a development copy.
dev=()
for manifest in examples/showcase/*/extension.toml; do
    [ -f "$manifest" ] && dev+=("$here/$(dirname "$manifest")")
done
# The sandbox's Luau extensions: the editor's and the control surface's
# (C11, H13), the metronome and tidy.
for d in apps/sandbox/extensions/{outline,surface,metronome,tidy}; do
    dev+=("$here/$d")
done

# The process extensions and the WASI component need their programs built:
# each is staged in the scratch folder, its package's files linked in (so
# an edit still reloads) beside its program.
stage="$scratch/dev"
mkdir -p "$stage"
link_package() { # <package dir> <staged dir>
    rm -rf "$2"
    mkdir -p "$2"
    for f in "$1"/*; do
        case "$(basename "$f")" in Cargo.toml | src | target | tests) continue ;; esac
        ln -s "$f" "$2/$(basename "$f")"
    done
}
in_shell() {
    if [ -n "${DEVENV_ROOT:-}" ]; then "$@"; else devenv shell -- "$@"; fi
}
processes=(apps/sandbox/kanban apps/sandbox/sketch apps/sandbox/board apps/sandbox/clicker examples/extension/counter)
echo "showcase: building the process extensions' programs (kanban, sketch, board, clicker, counter)…"
if in_shell cargo build -p architect-sandbox-kanban -p architect-sandbox-sketch -p architect-sandbox-board \
    -p architect-sandbox-clicker -p example-extension-counter; then
    for d in "${processes[@]}"; do
        program="$(sed -n '/^\[entry\]/,/^\[/s/^path = "\(.*\)"/\1/p' "$d/extension.toml" | head -n 1)"
        to="$stage/$(basename "$d")"
        link_package "$here/$d" "$to"
        ln -sf "$here/target/debug/$program" "$to/$program"
        dev+=("$to")
    done
else
    echo "showcase: the process extensions did not build; they are left out" >&2
fi
echo "showcase: building the WASI counter's component (wasm32-wasip2)…"
wasm=architect_sandbox_wasi_counter.wasm
if devenv --profile wasi shell -- cargo build -p architect-sandbox-wasi-counter --target wasm32-wasip2; then
    to="$stage/wasi-counter"
    link_package "$here/apps/sandbox/wasi-counter" "$to"
    ln -sf "$here/target/wasm32-wasip2/debug/$wasm" "$to/$wasm"
    dev+=("$to")
else
    echo "showcase: the WASI counter did not build; it is left out" >&2
fi
# The web view board (R6): a Vite + npm app, built into its package's dist/
# when that is missing or older than its sources (`just webview-example`
# also packs and signs it), then loaded as a development copy like any
# package: its view is `kind = "web"`, drawn by the app's web view provider.
board=examples/extension/webview-board
if [ ! -f "$board/package/dist/index.html" ] \
    || [ -n "$(find "$board/app" -path "$board/app/node_modules" -prune -o -newer "$board/package/dist/index.html" -type f -print -quit)" ]; then
    echo "showcase: building the web view board's app (npm, Vite)…"
    if ! (cd "$board/app" && in_shell sh -c 'npm ci --no-audit --no-fund && npm run build'); then
        echo "showcase: the web view board did not build (npm); it is left out" >&2
    fi
fi
[ -f "$board/package/dist/index.html" ] && dev+=("$here/$board/package")

python3 -I - "$config/extension-registries.json" "${dev[@]}" <<'PY'
import json, sys
out, folders = sys.argv[1], sys.argv[2:]
try:
    kept = json.load(open(out))
except (OSError, ValueError):
    kept = {}
kept["dev_installs"] = True
kept["dev"] = sorted(set(kept.get("dev", [])) | set(folders))
json.dump(kept, open(out, "w"), indent=2)
PY

export ARCHITECT_CONFIG_DIR="$config"
export ARCHITECT_VAULT="$vault"
export ARCHITECT_VAULT_SEED="$here/apps/ui/showcase-vault"
export ARCHITECT_FORBID_KEYCHAIN=1

if [ "$flavour" = env ]; then
    for v in ARCHITECT_CONFIG_DIR ARCHITECT_VAULT ARCHITECT_VAULT_SEED ARCHITECT_FORBID_KEYCHAIN; do
        printf 'export %s=%q\n' "$v" "${!v}"
    done
    exit 0
fi

echo "showcase: vault $vault"
echo "showcase: app folder $config"
echo "showcase: ${#dev[@]} development copies"

# Built first, with its progress shown: the first build of the standard
# distribution takes several minutes, and a quiet `cargo run` looks like
# nothing happening.
case "$flavour" in
    standard)
        echo "showcase: building the standard distribution (dist/standard); the first build takes several minutes…"
        in_shell cargo build --manifest-path dist/standard/Cargo.toml --bin architect-desktop
        cmd=(cargo run -q --manifest-path dist/standard/Cargo.toml --bin architect-desktop -- "$@")
        ;;
    plain)
        echo "showcase: building Architect's app (apps/desktop)…"
        in_shell cargo build -p architect-desktop
        cmd=(cargo run -q -p architect-desktop -- "$@")
        ;;
esac
echo "showcase: starting the app"
if [ -n "${DEVENV_ROOT:-}" ]; then
    exec "${cmd[@]}"
fi
exec devenv shell -- "${cmd[@]}"
