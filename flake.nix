{
  description = "architect — dev shell with everything the workspace links against";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" "x86_64-darwin" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      devShells = forAll (pkgs:
        let
          # The Dioxus desktop crates (`architect-atom`, `architect-form`,
          # the story workbench, the launcher UI, the ui showcase) link
          # against the native GUI stack even in their unit tests. Without
          # these the workspace builds but `cargo test --workspace` fails
          # at link time with `unable to find library -lxdo`.
          linux = with pkgs; lib.optionals stdenv.isLinux [
            xdotool            # libxdo — the one the linker names first
            webkitgtk_4_1
            gtk3
            libsoup_3
            glib
            cairo
            pango
            gdk-pixbuf
            atk
            harfbuzz
            dbus
            libappindicator-gtk3
            libayatana-appindicator
            xorg.libX11
            xorg.libxcb
          ];
          common = with pkgs; [
            pkg-config
            openssl
            zlib
            sqlite
          ];
          shell = extra: pkgs.mkShell {
            packages = common ++ linux ++ extra;
            # Point the loader at the same libraries the linker found, so
            # a desktop test binary can also *run* from this shell.
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (common ++ linux);
          };
        in
        {
          # Libraries only: layers under whatever rust toolchain you already
          # have (`nix develop -c cargo test --workspace`).
          default = shell [ ];
          # Self-contained: nixpkgs' stable toolchain on top, for CI or a
          # machine with no rust of its own.
          ci = shell (with pkgs; [ cargo rustc rustfmt clippy rust-analyzer ]);
        });
    };
}
