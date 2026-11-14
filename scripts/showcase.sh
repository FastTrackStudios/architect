#!/usr/bin/env bash
# The extension showcase on the desk (#710, docs/content/guide/showcase.md):
# the Architect app on a fresh scratch copy of apps/ui/showcase-vault, with
# a folder of its own and every showcase extension loaded as a development
# copy (reloaded on save).
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

# The development copies: each showcase example, and the sandbox's
# editor and control-surface extensions (C11, H13).
dev=()
for manifest in examples/showcase/*/extension.toml; do
    [ -f "$manifest" ] && dev+=("$here/$(dirname "$manifest")")
done
for d in apps/sandbox/extensions/outline apps/sandbox/extensions/surface; do
    dev+=("$here/$d")
done
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

case "$flavour" in
    standard) cmd=(cargo run -q --manifest-path dist/standard/Cargo.toml --bin architect-desktop -- "$@") ;;
    plain) cmd=(cargo run -q -p architect-desktop -- "$@") ;;
esac
if [ -n "${DEVENV_ROOT:-}" ]; then
    exec "${cmd[@]}"
fi
exec devenv shell -- "${cmd[@]}"
