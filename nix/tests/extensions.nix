# The Nix side of extensions in production (#583): `lib.architectExtension`
# and `services.architect-node.extensions`, against a signed fixture (the
# showcase's Bank link, signed with a throwaway key whose secret half was
# discarded; nix/tests/extensions/).
#
# - node-with-extension: a NixOS node with the Bank link builds; its unit's
#   ARCHITECT_EXTENSIONS is a store path holding the package folder.
# - extension-wrong-hash: a wrong pinned hash fails the build.
# - extension-bad-signature: an archive signed by a key not among the
#   publishers fails the build.
# - extension-tampered: an archive changed after signing fails the build.
#
# The node in these is a stand-in program: compiling the real one is the
# release's job; what runs it is tested in apps/desktop/tests/node.rs.
{
  pkgs,
  nixpkgs,
  architectExtension,
  module,
}:
let
  inherit (pkgs) lib;
  fixtures = ./extensions;
  key = file: lib.last (lib.splitString "\n" (lib.removeSuffix "\n" (builtins.readFile file)));
  showcaseKey = key (fixtures + "/showcase.pub");
  otherKey = key (fixtures + "/other.pub");
  bankLink =
    args:
    architectExtension (
      {
        inherit pkgs;
        id = "showcase.bank-link";
        version = "0.1.0";
        src = fixtures + "/showcase.bank-link-0.1.0.archext";
        hash = "sha256-gGucnf0l3Cjr6finUjFhRXzPkAGwa/G0jc2iGRwurtI=";
        publishers.Showcase = showcaseKey;
        trustOrg = false;
      }
      // args
    );
  # A build that must fail, and say why.
  refused =
    name: drv: why:
    pkgs.runCommand name { failed = pkgs.testers.testBuildFailure drv; } ''
      grep -F ${lib.escapeShellArg why} $failed/testBuildFailure.log
      touch $out
    '';
  node =
    (nixpkgs.lib.nixosSystem {
      inherit (pkgs.stdenv.hostPlatform) system;
      modules = [
        module
        {
          boot.loader.grub.enable = false;
          fileSystems."/" = {
            device = "none";
            fsType = "tmpfs";
          };
          system.stateVersion = "25.05";
          services.architect-node = {
            enable = true;
            package = pkgs.writeShellScriptBin "architect-desktop" "exit 0";
            extensions = [ (bankLink { }) ];
            extensionSecrets = "/run/secrets/architect-extensions";
          };
        }
      ];
    }).config;
  unit = node.systemd.units."architect-node.service".unit;
in
{
  node-with-extension = pkgs.runCommand "node-with-extension" { } ''
    dir=${node.services.architect-node.extensionsDir}
    test -f $dir/showcase.bank-link/extension.toml
    test -f $dir/showcase.bank-link/node/exchange.luau
    grep -F 'runs = "node"' $dir/showcase.bank-link/extension.toml
    grep -F "ARCHITECT_EXTENSIONS=$dir" ${unit}/architect-node.service
    grep -F "ARCHITECT_EXTENSION_SECRETS=/run/secrets/architect-extensions" ${unit}/architect-node.service
    touch $out
  '';
  extension-wrong-hash = refused "extension-wrong-hash" (bankLink {
    hash = "sha256-Y9VaQvLvU9C/3FJGiRoz7rvs8C+UriyqCODK6yVw8e4=";
  }) "pinned 63d55a42";
  extension-bad-signature = refused "extension-bad-signature" (bankLink {
    publishers = {
      Other = otherKey;
    };
  }) "which this app does not trust";
  extension-tampered = refused "extension-tampered" (bankLink {
    src = fixtures + "/tampered.archext";
    hash = "sha256-Y9VaQvLvU9C/3FJGiRoz7rvs8C+UriyqCODK6yVw8e4=";
  }) "was changed after signing";
}
