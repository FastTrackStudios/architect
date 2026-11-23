#!/usr/bin/env bash
# Makes the showcase registry again (#710 L1-L4): packs each package in
# packages/ (and each extension the showcase harness built, given as more
# folders), signs it with a fresh throwaway key whose secret is thrown away,
# writes index.toml and keys/showcase.pub, and the fixture vault's carried
# archive. Run from an architect checkout:
#
#   examples/showcase-registry/make.sh [more package folders...]
#
# Then check it: cargo xtask ext-index examples/showcase-registry --key examples/showcase-registry/keys/showcase.pub
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
cd "$here/../.."
key="$(mktemp -d)"
trap 'rm -rf "$key"' EXIT

cargo xtask ext-keygen "$key"
pub="$(command ls "$key"/*.pub)"
secret="$(command ls "$key"/*.key)"
mkdir -p "$here/keys" "$here/archives" "$here/vault/.architect/extensions"
rm -f "$here"/archives/*.archext "$here"/vault/.architect/extensions/*.archext
cp "$pub" "$here/keys/showcase.pub"

field() { sed -n "s/^$2 = \"\(.*\)\"$/\1/p" "$1/extension.toml" | head -n 1; }

index="$here/index.toml"
{
  echo '#:schema ../registry/index.schema.json'
  echo '# The showcase registry (#710): signed showcase extensions, made by make.sh.'
  echo '# Its key is keys/showcase.pub; the secret half was thrown away.'
  echo 'name = "Showcase"'
} > "$index"

pack() {
  local dir="$1" out="$2" listed="$3"
  local id version kind name platforms
  id="$(field "$dir" id)"
  version="$(field "$dir" version)"
  kind="$(field "$dir" kind)"
  name="$(field "$dir" name)"
  local file="$out/$id-$version.archext"
  cargo xtask ext-pack "$dir" --out "$file"
  cargo xtask ext-sign "$file" --key "$secret"
  if [ "$listed" = yes ]; then
    local rel="archives/$id-$version.archext"
    {
      echo
      echo '[[extensions]]'
      echo "id = \"$id\""
      echo "name = \"$name\""
      echo "version = \"$version\""
      echo "kind = \"$kind\""
      echo "description = \"$(field "$dir" description)\""
      echo "archive = \"$rel\""
      echo "sha256 = \"$(sha256sum "$file" | cut -d' ' -f1)\""
      if [ "$id" = showcase.greeter ] && [ "$version" = 0.2.0 ]; then
        echo 'changelog = "Copies the greeting to the clipboard (asks for the clipboard)."'
      fi
    } >> "$index"
  fi
}

for dir in greeter-0.1.0 greeter-0.2.0 word-count mac-only; do
  pack "$here/packages/$dir" "$here/archives" yes
done
for dir in "$@"; do
  pack "$dir" "$here/archives" yes
done
# The fixture vault carries this one; no registry lists it.
pack "$here/packages/tiny" "$here/vault/.architect/extensions" no
