# One extension for a Nix-built deployment (#583): a signed `.archext`,
# fetched by pinned version and hash, its signature checked at BUILD time
# against the publisher keys given, unpacked into a store path that is the
# extension's package folder (`extension.toml` at its top). Nothing is
# downloaded when the node runs, and nothing unsigned or unpinned builds.
#
#   architect.lib.architectExtension {
#     inherit pkgs;
#     id = "fts.ledger";
#     version = "0.3.0";
#     registry = "https://github.com/FastTrackStudios/extensions/raw/main";
#     hash = "sha256-…";                       # the archive's
#     publishers.FastTrackStudios = "RWR…";    # minisign public keys
#   }
#
# The archive is `url`, or `<registry>/archives/<id>-<version>.archext`
# (a registry's layout, examples/showcase-registry), or `src` (a path: a
# vendored archive, the checks' fixtures). The check is `archext install`
# (features/extensions/archive/src/bin/archext.rs): the same
# `architect_extension_archive::open` every app runs on an install.
{
  archext,
  pkgs,
  id,
  version,
  hash,
  url ? null,
  registry ? null,
  src ? null,
  # Publisher keys, name → the base64 line of a minisign `.pub`.
  publishers ? { },
  # Also trust the FastTrackStudios release key (compiled into archext).
  trustOrg ? true,
}:
let
  inherit (pkgs) lib;
  archive =
    if src != null then
      src
    else
      pkgs.fetchurl {
        url = if url != null then url else "${registry}/archives/${id}-${version}.archext";
        inherit hash;
      };
  hex = builtins.convertHash {
    inherit hash;
    toHashFormat = "base16";
  };
  keys = lib.concatStringsSep " " (
    lib.mapAttrsToList (name: key: "--key ${lib.escapeShellArg "${name}=${key}"}") publishers
  );
in
assert lib.assertMsg (
  src != null || url != null || registry != null
) "architectExtension ${id}: give url, registry or src";
assert lib.assertMsg (
  trustOrg || publishers != { }
) "architectExtension ${id}: no key to trust (publishers, or trustOrg)";
pkgs.runCommand "architect-extension-${id}-${version}"
  {
    nativeBuildInputs = [ archext ];
    passthru = { inherit id version archive; };
    meta.description = "Architect extension ${id} ${version}, signature-checked at build time";
  }
  ''
    archext install ${archive} \
      --id ${lib.escapeShellArg id} --version ${lib.escapeShellArg version} \
      --sha256 ${hex} ${lib.optionalString trustOrg "--org"} ${keys} \
      $out
  ''
