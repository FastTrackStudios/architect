# Architect's Linux desktop app from the release's own AppImage (#523): the
# build release.yml (job linux) makes once, not a second, from-source build
# of the workspace. Installing compiles nothing.
#
# The version and hash live in source.json; ./update.sh rewrites it from a
# release's latest-linux.json (the manifest the in-app updater reads, which
# carries the AppImage's sha256), so a bump downloads nothing.
#
# The store is read-only, so the app's updater writes into
# $ARCHITECT_UPDATE_DIR instead, and the launcher (launch.sh) runs the newer
# of the store's image and the downloaded one (choose.sh). The FHS env runs
# any type-2 AppImage, so it runs the downloaded image too, with the host
# libraries the AppImage leaves out (scripts/desktop/linux.sh: the graphics
# stack, glibc, and what winit opens at run time).
{
  lib,
  appimageTools,
  buildFHSEnv,
  fetchurl,
  replaceVars,
  runCommand,
}:

let
  source = lib.importJSON ./source.json;
  pname = "architect-desktop";
  inherit (source) version;

  src = fetchurl {
    url = "https://github.com/FastTrackStudios/architect/releases/download/v${version}/Architect-linux-x86_64.AppImage";
    inherit (source) hash;
  };

  contents = appimageTools.extract { inherit pname version src; };

  choose = ./choose.sh;

  launch = replaceVars ./launch.sh {
    inherit choose;
    storeVersion = version;
    storeImage = src;
    storeContents = contents;
  };

  fhsArgs = appimageTools.defaultFhsEnvArgs;
in
buildFHSEnv (
  lib.recursiveUpdate fhsArgs {
    inherit pname version;

    targetPkgs = pkgs: [ appimageTools.appimage-exec ] ++ fhsArgs.targetPkgs pkgs;
    runScript = "bash ${launch}";

    extraInstallCommands = ''
      for d in ${contents}/*.desktop; do
        [ -e "$d" ] || continue
        install -Dm444 "$d" "$out/share/applications/${pname}.desktop"
        sed -i -E 's|^Exec=[^ ]*|Exec=${pname}|' "$out/share/applications/${pname}.desktop"
      done
      if [ -d ${contents}/usr/share/icons ]; then
        mkdir -p $out/share
        cp -r ${contents}/usr/share/icons $out/share/
      fi
    '';

    passthru = {
      inherit src contents choose;
      tests.choose = runCommand "architect-desktop-choose" { } ''
        bash ${./test-choose.sh} ${choose}
        touch $out
      '';
    };

    meta = {
      description = "Architect desktop app (the release AppImage, wrapped)";
      homepage = "https://github.com/FastTrackStudios/architect";
      platforms = [ "x86_64-linux" ];
      mainProgram = pname;
      sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
    };
  }
)
