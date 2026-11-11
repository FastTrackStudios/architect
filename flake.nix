{
  description = "Architect: packaging only. The headless node, the web front end, and their pure-Nix OCI images.";

  # The dev environment is devenv (devenv.yaml, devenv.nix; `devenv shell`
  # or direnv through .envrc). This flake only packages what is deployed:
  # the Architect app built without its window (`architect-desktop`,
  # `--no-default-features --features vox`), which runs as
  # `architect-desktop serve`, the always-on node (apps/desktop/src/node).
  # Its image is assembled by `dockerTools.streamLayeredImage` and pushed
  # with skopeo: no Docker, no buildx (Task's images are made the same way).
  #
  #   nix build .#architect-image            # pure: compiles the node in the sandbox
  #   ARCHITECT_PREBUILT_BIN=target/release/architect-desktop \
  #     nix build --impure .#architect-image-prebuilt   # wraps a binary built in the dev shell
  #   ./result | skopeo copy docker-archive:/dev/stdin docker://ghcr.io/fasttrackstudios/architect:<v>
  #
  # The release workflow (.github/workflows/release.yml) uses the
  # prebuilt one: the runner's warm target dir builds the binary in
  # minutes, and the same binary is attached to the release.
  #
  # The web front end (#431): the landing page, the docs and the web app
  # (apps/web) as one static site (scripts/web-bundle.sh) behind
  # static-web-server (apps/web/sws.toml), its own image:
  #
  #   nix build .#architect-web-image        # pure: the wasm and the docs' CLI built in the sandbox
  #   ARCHITECT_WEB_BUNDLE=target/web-bundle \
  #     nix build --impure .#architect-web-image-prebuilt  # wraps a bundle built in the dev shell

  inputs = {
    # The fleet's pins: nixpkgs and rust-overlay at devenv-fleet's
    # revisions, the same as devenv.yaml's, and `lib.toolchainFor` for the
    # shell's rustc. Move them only with devenv-fleet.
    devenv-fleet.url = "github:FastTrackStudios/devenv-fleet/c4ddec850ad97b5da74c746bff39a988f02b13a4";
    nixpkgs.follows = "devenv-fleet/nixpkgs";
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      devenv-fleet,
      crane,
      ...
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAll =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ devenv-fleet.overlays.default ];
            }
          )
        );
      fromEnv =
        name:
        let
          v = builtins.getEnv name;
        in
        if v == "" then null else v;
    in
    {
      packages = forAll (
        pkgs:
        let
          inherit (pkgs) lib;
          craneLib = (crane.mkLib pkgs).overrideToolchain (devenv-fleet.lib.toolchainFor pkgs);
          src = lib.cleanSourceWith {
            src = ./.;
            filter =
              path: _type:
              !(builtins.elem (baseNameOf (toString path)) [
                "target"
                ".git"
                "result"
                "node_modules"
                ".devenv"
              ]);
          };
          version = (lib.importTOML ./Cargo.toml).workspace.package.version;
          # Git dependencies whose locked commit is on no branch any more
          # (`builtins.fetchGit` cannot find it; cargo fetches it by sha):
          # fetched by rev with a fixed hash instead. Blitz's stylo pin.
          cargoVendorDir = craneLib.vendorCargoDeps {
            inherit src;
            # FastTrackStudios/vendor commits crates' `Cargo.toml.orig`,
            # which `cargo package -l` (how crane lists a git crate's
            # files) refuses as a reserved name; dropped before listing.
            overrideVendorGitCheckout =
              _: drv:
              drv.overrideAttrs (old: {
                preInstall = (old.preInstall or "") + ''
                  find . -name Cargo.toml.orig -delete
                '';
              });
            outputHashes = {
              "git+https://github.com/nicoburns/stylo?rev=78734bf554bb40af9dc11c0eedbe724f6055697c#78734bf554bb40af9dc11c0eedbe724f6055697c" =
                "sha256-lbhr0hkTHF9DHQRVWjrd5t8ZpKP6zCmK3OQ81KJVDUI=";
            };
          };
          # The release's own version when the workflow stamps it, else the
          # workspace's.
          release = let v = fromEnv "ARCHITECT_RELEASE_VERSION"; in if v == null then version else v;
          buildRev =
            let
              v = fromEnv "ARCHITECT_BUILD_REV";
            in
            if v != null then v else self.rev or self.dirtyRev or "unknown";

          # The node, without the window. `no_workspace_hack` drops the
          # hakari crate, which would otherwise pull the GUI stack's
          # features into every member (Cargo.toml's workspace-hack lines).
          architect-node = craneLib.buildPackage {
            pname = "architect-node";
            inherit version src cargoVendorDir;
            strictDeps = true;
            cargoArtifacts = null;
            doCheck = false;
            cargoExtraArgs = "--package architect-desktop --bin architect-desktop --no-default-features --features vox,node-wasi";
            RUSTFLAGS = "--cfg no_workspace_hack";
            ARCHITECT_BUILD_REV = buildRev;
            nativeBuildInputs = [ pkgs.pkg-config ];
          };

          # A binary built outside the sandbox (the release runner's dev
          # shell), copied in; Nix finds its store references (glibc) and
          # carries them into the image.
          prebuiltBin = fromEnv "ARCHITECT_PREBUILT_BIN";
          # Another distribution's node (#491, `cargo xtask dist node-image`):
          # its binary's name, and the image's.
          imageBin = let v = fromEnv "ARCHITECT_IMAGE_BIN"; in if v == null then "architect-desktop" else v;
          imageName = fromEnv "ARCHITECT_IMAGE_NAME";
          architect-node-prebuilt = pkgs.runCommand "architect-node-prebuilt" { } ''
            install -Dm755 ${/. + prebuiltBin} $out/bin/${imageBin}
          '';

          # ── The web front end ────────────────────────────────────────
          # The web app's wasm (apps/web), for wasm32-unknown-unknown.
          architect-web-wasm = craneLib.buildPackage {
            pname = "architect-web-wasm";
            inherit version src cargoVendorDir;
            strictDeps = true;
            cargoArtifacts = null;
            doCheck = false;
            CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
            cargoExtraArgs = "--package architect-web --lib";
            installPhaseCommand = ''
              install -Dm644 target/wasm32-unknown-unknown/release/architect_web.wasm $out/architect_web.wasm
            '';
          };
          # The `architect` CLI (apps/cli): it builds the docs site.
          architect-cli = craneLib.buildPackage {
            pname = "architect-cli";
            inherit version src cargoVendorDir;
            strictDeps = true;
            cargoArtifacts = null;
            doCheck = false;
            cargoExtraArgs = "--package architect-cli --bin architect";
            RUSTFLAGS = "--cfg no_workspace_hack";
            nativeBuildInputs = [ pkgs.pkg-config ];
          };
          # The bundle: the same script `just web` runs, given the pieces;
          # which distribution it is (`/app/distribution.json`), from the
          # Architect app's node (`architect-desktop distribution`, #491).
          architect-web-bundle =
            pkgs.runCommand "architect-web-bundle-${release}"
              {
                nativeBuildInputs = [ pkgs.wasm-bindgen-cli_0_2_127 ];
                ARCHITECT_WEB_WASM = "${architect-web-wasm}/architect_web.wasm";
                ARCHITECT_CLI = "${architect-cli}/bin/architect";
                WASM_BINDGEN = "wasm-bindgen";
              }
              ''
                ${architect-node}/bin/architect-desktop distribution > distribution.json
                ARCHITECT_DISTRIBUTION=$PWD/distribution.json bash ${src}/scripts/web-bundle.sh $out
              '';
          prebuiltBundle = fromEnv "ARCHITECT_WEB_BUNDLE";
          architect-web-bundle-prebuilt = pkgs.runCommand "architect-web-bundle-prebuilt" { } ''
            cp -r ${/. + prebuiltBundle} $out
          '';

          # static-web-server on :8080 with the site at /srv/www: real
          # files, not store symlinks (it refuses a path resolving outside
          # its root), so a ConfigMap can replace /srv/www/app/config.json.
          mkWebImage =
            {
              bundle,
              name ? "architect-web",
            }:
            pkgs.dockerTools.streamLayeredImage {
              inherit name;
              tag = "latest";
              contents = [
                pkgs.static-web-server
                pkgs.cacert
              ];
              extraCommands = ''
                mkdir -p srv etc/architect-web tmp
                cp -r ${bundle} srv/www
                chmod -R u+w srv/www
                echo '{"rev":"${buildRev}","version":"${release}"}' > srv/www/version.json
                cp ${./apps/web/sws.toml} etc/architect-web/sws.toml
              '';
              config = {
                Entrypoint = [ "/bin/static-web-server" ];
                Cmd = [
                  "--config-file"
                  "/etc/architect-web/sws.toml"
                  "--root"
                  "/srv/www"
                  "--host"
                  "0.0.0.0"
                  "--port"
                  "8080"
                ];
                ExposedPorts = {
                  "8080/tcp" = { };
                };
                Env = [ "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt" ];
                Labels = {
                  "org.opencontainers.image.source" = "https://github.com/FastTrackStudios/architect";
                  "org.opencontainers.image.revision" = buildRev;
                  "org.opencontainers.image.version" = release;
                  "org.opencontainers.image.description" = "Architect's web front end: the landing page, the docs and the web app, static";
                };
              };
            };

          mkImage =
            {
              node,
              name ? "architect",
              bin ? "architect-desktop",
            }:
            pkgs.dockerTools.streamLayeredImage {
              inherit name;
              tag = "latest";
              contents = [
                node
                pkgs.cacert
              ];
              extraCommands = ''
                mkdir -p data tmp
              '';
              config = {
                Entrypoint = [
                  "/bin/${bin}"
                  "serve"
                ];
                Env = [
                  "ARCHITECT_BIND=0.0.0.0:8080"
                  "ARCHITECT_DATA_DIR=/data"
                  "ARCHITECT_IROH_BIND=0.0.0.0:7777"
                  "ARCHITECT_BUILD_REV=${buildRev}"
                  "RUST_LOG=info"
                  "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
                ];
                ExposedPorts = {
                  "8080/tcp" = { };
                  "7777/udp" = { };
                };
                Volumes = {
                  "/data" = { };
                };
                WorkingDir = "/data";
                Labels = {
                  "org.opencontainers.image.source" = "https://github.com/FastTrackStudios/architect";
                  "org.opencontainers.image.revision" = buildRev;
                  "org.opencontainers.image.version" = release;
                  "org.opencontainers.image.description" = "Architect, headless: the always-on node (architect-desktop serve)";
                };
              };
            };
        in
        {
          inherit
            architect-node
            architect-cli
            architect-web-wasm
            architect-web-bundle
            ;
        }
        // lib.optionalAttrs pkgs.stdenv.isLinux (
          {
            architect-image = mkImage { node = architect-node; };
            architect-web-image = mkWebImage { bundle = architect-web-bundle; };
          }
          // lib.optionalAttrs (prebuiltBin != null) {
            architect-image-prebuilt = mkImage {
              node = architect-node-prebuilt;
              name = if imageName == null then "architect" else imageName;
              bin = imageBin;
            };
          }
          // lib.optionalAttrs (prebuiltBundle != null) {
            architect-web-image-prebuilt = mkWebImage {
              bundle = architect-web-bundle-prebuilt;
              name = if imageName == null then "architect-web" else imageName;
            };
          }
        )
      );
    };

  nixConfig = {
    extra-trusted-public-keys = [
      "fasttrackstudio.cachix.org-1:r7v7WXBeSZ7m5meL6w0wttnvsOltRvTpXeVNItcy9f4="
    ];
    extra-substituters = [
      "https://fasttrackstudio.cachix.org"
    ];
  };
}
