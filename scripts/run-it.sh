#!/usr/bin/env bash
# `just dev`, `just demo`, `just phone` (justfile): Studio's local stack
# with this checkout as its Architect when a Studio checkout is beside
# this one, else (or with `plain`) Architect's own stack (`devenv up`,
# docs/content/guide/local-stack.md).
#
#   run-it.sh dev|demo|phone [plain] [ARGS…]
#
# Studio's checkout: LOCAL_STACK_STUDIO_PATH, else ../studio from this
# checkout or from its main checkout (a worktree's). Its stack keeps its
# state in that checkout's .devenv/state (docs/local-stack.md
# there).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
what="${1:?usage: run-it.sh dev|demo|phone [plain] [ARGS…]}"
shift
plain=0
args=()
for a in "$@"; do
    if [ "$a" = plain ]; then plain=1; else args+=("$a"); fi
done

studio=""
if [ -n "${LOCAL_STACK_STUDIO_PATH:-}" ]; then
    studio="$LOCAL_STACK_STUDIO_PATH"
else
    main="$(dirname "$(git -C "$root" rev-parse --path-format=absolute --git-common-dir)")"
    for base in "$root" "$main"; do
        if [ -f "$base/../studio/scripts/local-stack.sh" ]; then
            studio="$(cd "$base/../studio" && pwd)"
            break
        fi
    done
fi

if [ "$plain" = 0 ] && [ -n "$studio" ] && [ -f "$studio/scripts/local-stack.sh" ]; then
    echo "run-it: Studio's local stack ($studio), Architect from this checkout ($root)" >&2
    cd "$studio"
    case "$what" in
        phone) exec just phone "${args[@]}" ;;
        *) exec just "$what" architect="$root" "${args[@]}" ;;
    esac
fi

[ "$plain" = 1 ] || echo "run-it: no Studio checkout with the local stack beside this one (../studio, or LOCAL_STACK_STUDIO_PATH): Architect's own stack" >&2
case "$what" in
    # Architect's stack: mock-oauth, auth, the node (demo seed and its
    # share links), the app under dx serve, the web app and the site.
    dev) exec devenv up mock-oauth auth node app web site ;;
    # The issuer and the node only: the demo seed's share links, a signed-in
    # dev user (ada@local.test / development-password), nothing watched.
    demo)
        echo "run-it: the share links are listed in docs/content/guide/local-stack.md (e.g. http://127.0.0.1:7412/s/ksqx75iaaaaaaaae/)" >&2
        exec devenv up mock-oauth auth node
        ;;
    phone) echo "run-it: \`just phone\` serves Studio's stack; Architect's own has none" >&2; exit 1 ;;
    *) echo "run-it: dev, demo or phone" >&2; exit 64 ;;
esac
