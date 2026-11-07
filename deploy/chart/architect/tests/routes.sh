#!/usr/bin/env bash
# The chart's Ingress routes every path the node owns to the node and the
# rest to the web front end (#431). Any unrouted node path would land on
# the static site and answer 404 HTML, so each one is checked here.
#
#   deploy/chart/architect/tests/routes.sh      (helm and yq from nixpkgs when missing)
set -euo pipefail
chart="$(cd "$(dirname "$0")/.." && pwd)"
helm() { if type -P helm >/dev/null; then command helm "$@"; else nix run --quiet nixpkgs#kubernetes-helm -- "$@"; fi; }
yq() { if type -P yq >/dev/null && command yq --version 2>&1 | grep -q mikefarah; then command yq "$@"; else nix run --quiet nixpkgs#yq-go -- "$@"; fi; }

# Every prefix the node serves (apps/desktop/src/node: /vox and /health from
# EngineHost, /org/<slug>/vox the org lane, /mcp, /s share links, /node the
# HTTP face) and the two it reserves. A prefix taken out of values.yaml
# fails here.
want=(/vox /org /mcp /health /s /node /blobs /.well-known)

out="$(helm template t "$chart" --set ingress.enabled=true --set host=a.test \
    --set node.s3Face.enabled=true --set node.s3Face.host=s3.a.test --set node.extraPaths[0]=/notes)"
ing="$(printf '%s\n' "$out" | yq 'select(.kind == "Ingress")')"
backend() { # <host> <path>
    printf '%s\n' "$ing" | yq ".spec.rules[] | select(.host == \"$1\") | .http.paths[] | select(.path == \"$2\") | .backend.service.name + \":\" + .backend.service.port.name"
}
fail=0
check() { # <host> <path> <want>
    got="$(backend "$1" "$2")"
    if [ "$got" = "$3" ]; then echo "ok    $1$2 -> $got"; else echo "FAIL  $1$2 -> '${got:-unrouted}', want $3"; fail=1; fi
}
for p in "${want[@]}" /notes; do check a.test "$p" t-architect-node:http; done
check a.test / t-architect-web:http
check s3.a.test / t-architect-node:s3

# Nothing else goes to the node, and nothing is listed twice.
extra="$(printf '%s\n' "$ing" | yq '.spec.rules[] | select(.host == "a.test") | .http.paths[] | select(.backend.service.name == "t-architect-node") | .path' | sort)"
expect="$(printf '%s\n' "${want[@]}" /notes | sort)"
[ "$extra" = "$expect" ] || { echo "FAIL  node paths are"; echo "$extra"; echo "      want"; echo "$expect"; fail=1; }

# The web app's settings follow the issuer; a refused combination fails.
printf '%s\n' "$out" | yq 'select(.kind == "ConfigMap") | .data["config.json"]' | grep -q '"client_id":"architect"' \
    || { echo "FAIL  config.json has no client_id architect"; fail=1; }
if helm template t "$chart" --set node.replicaCount=2 >/dev/null 2>&1; then
    echo "FAIL  two node replicas on one volume were accepted"; fail=1
fi
if helm template t "$chart" --set node.s3.enabled=true --set node.s3.bucket=b >/dev/null 2>&1; then
    echo "FAIL  S3 without a credentials Secret was accepted"; fail=1
fi
# Images: a bare repository gets image.registry; a full one (what
# argocd-image-updater writes) is used as is.
images="$(helm template t "$chart" --set node.image.repository=ghcr.io/x/architect --set node.image.tag=1.2.3 | yq 'select(.kind == "Deployment") | .spec.template.spec.containers[0].image' | grep -v '^---$' | sort | tr '\n' ' ')"
case "$images" in
    *"ghcr.io/fasttrackstudios/architect-web:"*"ghcr.io/x/architect:1.2.3 "*) echo "ok    images: $images" ;;
    *) echo "FAIL  images: $images"; fail=1 ;;
esac
[ "$fail" = 0 ] && echo "routes: all good"
exit "$fail"
