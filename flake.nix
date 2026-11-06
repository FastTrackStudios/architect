{
  description = "Architect: packaging only. The headless node's binary and its pure-Nix OCI image.";

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
            cargoExtraArgs = "--package architect-desktop --bin architect-desktop --no-default-features --features vox";
            RUSTFLAGS = "--cfg no_workspace_hack";
            ARCHITECT_BUILD_REV = buildRev;
            nativeBuildInputs = [ pkgs.pkg-config ];
          };

          # A binary built outside the sandbox (the release runner's dev
          # shell), copied in; Nix finds its store references (glibc) and
          # carries them into the image.
          prebuiltBin = fromEnv "ARCHITECT_PREBUILT_BIN";
          architect-node-prebuilt = pkgs.runCommand "architect-node-prebuilt" { } ''
            install -Dm755 ${/. + prebuiltBin} $out/bin/architect-desktop
          '';

          mkImage =
            { node }:
            pkgs.dockerTools.streamLayeredImage {
              name = "architect";
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
                  "/bin/architect-desktop"
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
          inherit architect-node;
        }
        // lib.optionalAttrs pkgs.stdenv.isLinux (
          {
            architect-image = mkImage { node = architect-node; };
          }
          // lib.optionalAttrs (prebuiltBin != null) {
            architect-image-prebuilt = mkImage { node = architect-node-prebuilt; };
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
