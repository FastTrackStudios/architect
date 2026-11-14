# Architect's headless node as a NixOS service (`<binary> serve`,
# docs/content/guide/deploy.md), and the extensions it runs (#583):
#
#   services.architect-node = {
#     enable = true;
#     environment.ARCHITECT_HOST_ORG = "fasttrackstudio";
#     extensions = [ (architect.lib.architectExtension { inherit pkgs; … }) ];
#     extensionSecrets = "/run/secrets/architect-extensions";  # <ext>/<secret>
#   };
#
# Each extension is a store path built by `lib.architectExtension`: pinned
# by version and hash, its signature checked against the publisher keys
# when the system is built. The node's extension folder
# (`ARCHITECT_EXTENSIONS`) is one store path linking them by id, read-only,
# switched with the system's generation; a change restarts the node, which
# reads the folder at start. Nothing is fetched at run time.
{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.architect-node;
  inherit (lib) mkOption types;
  extensionsDir = pkgs.linkFarm "architect-node-extensions" (
    map (e: {
      name = e.passthru.id or (throw "services.architect-node.extensions: ${e.name} is not from lib.architectExtension");
      path = e;
    }) cfg.extensions
  );
in
{
  options.services.architect-node = {
    enable = lib.mkEnableOption "Architect's headless node (`<binary> serve`)";
    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.architect-node;
      defaultText = lib.literalExpression "architect.packages.\${system}.architect-node";
      description = "The node: Architect's, or another distribution's (`studio-architect`)";
    };
    binary = mkOption {
      type = types.str;
      default = "architect-desktop";
      description = "Its program's name in the package's bin/";
    };
    dataDir = mkOption {
      type = types.str;
      default = "/var/lib/architect-node";
      description = "ARCHITECT_DATA_DIR: its documents, iroh key and built shares";
    };
    environment = mkOption {
      type = types.attrsOf types.str;
      default = { };
      example = {
        ARCHITECT_HOST_ORG = "fasttrackstudio";
        ARCHITECT_ISSUER = "https://auth.fasttrackstudio.app";
      };
      description = "Its other ARCHITECT_* settings (docs/content/guide/deploy.md). Never a secret: those are files";
    };
    extensions = mkOption {
      type = types.listOf types.package;
      default = [ ];
      description = ''
        The extensions it runs, each from `lib.architectExtension`: pinned by
        version and hash, its signature checked at build time. Their share
        blocks draw on its share links and Sites; their node actions
        (`runs = "node"`) are served on its org lane.
      '';
    };
    extensionsDir = mkOption {
      type = types.path;
      readOnly = true;
      default = extensionsDir;
      defaultText = lib.literalMD "a store path linking each of `extensions` by id";
      description = "ARCHITECT_EXTENSIONS: its extension folder, read-only, from the store";
    };
    extensionSecrets = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "/run/secrets/architect-extensions";
      description = ''
        ARCHITECT_EXTENSION_SECRETS: its extensions' node secrets, a file each
        at `<dir>/<ext id>/<secret id>` (sops-nix's files). Null: an org admin
        sets them on the org lane, kept in its data dir.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = lib.hasPrefix "/var/lib/" cfg.dataDir;
        message = "services.architect-node.dataDir is under /var/lib (its StateDirectory)";
      }
    ];
    systemd.services.architect-node = {
      description = "Architect node (${cfg.binary} serve)";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      environment = {
        ARCHITECT_DATA_DIR = cfg.dataDir;
        ARCHITECT_EXTENSIONS = "${cfg.extensionsDir}";
      }
      // lib.optionalAttrs (cfg.extensionSecrets != null) {
        ARCHITECT_EXTENSION_SECRETS = cfg.extensionSecrets;
      }
      // cfg.environment;
      restartTriggers = [ cfg.extensionsDir ];
      serviceConfig = {
        ExecStart = "${lib.getExe' cfg.package cfg.binary} serve";
        DynamicUser = true;
        StateDirectory = lib.removePrefix "/var/lib/" cfg.dataDir;
        WorkingDirectory = cfg.dataDir;
        Restart = "on-failure";
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
      };
    };
  };
}
