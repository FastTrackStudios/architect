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
            # Blitz's system fonts (fontique) link fontconfig: a desktop
            # test binary needs it at run time.
            fontconfig
          ];
          common = with pkgs; [
            pkg-config
            # Stylo's native build script generates Rust from Python.
            # GitHub's Ubuntu image supplied this implicitly; NixOS does not.
            python3
            openssl
            zlib
            sqlite
            # Guide media (`just media`, features/guide): frames to WebM and
            # MP4, and to GIF. Run as programs, never linked.
            ffmpeg
            gifski
          ];
          shell = extra: pkgs.mkShell {
            packages = common ++ linux ++ extra;
            # Point the loader at the same libraries the linker found, so
            # a desktop test binary can also *run* from this shell. The C++
            # runtime too: Luau (architect-scripting, -canvas) is C++, and a
            # test binary linking it needs libstdc++ at run time on NixOS.
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (common ++ linux ++ [ pkgs.stdenv.cc.cc.lib ]);
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
