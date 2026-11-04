# architect's dev environment. The toolchain (rust 1.99.1, the
# rust-toolchain.toml pin), mold, cargo-hakari, cargo-rail (change files,
# .changes/README) and prek come from devenv-fleet's `rust` module
# (devenv.yaml); this file adds what this workspace links against and runs.
{ pkgs, lib, ... }:
let
  # The Dioxus desktop crates (`architect-atom`, `architect-form`, the story
  # workbench, the launcher UI, the ui showcase) link against the native GUI
  # stack even in their unit tests. Without these the workspace builds but
  # `cargo test --workspace` fails at link time with
  # `unable to find library -lxdo`.
  linux = with pkgs; lib.optionals stdenv.isLinux [
    xdotool # libxdo, the one the linker names first
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
    # Blitz's system fonts (fontique) link fontconfig: a desktop test binary
    # needs it at run time.
    fontconfig
  ];
  common = with pkgs; [
    # Stylo's native build script generates Rust from Python.
    python3
    openssl
    zlib
    sqlite
  ];
in
{
  packages = common ++ linux ++ (with pkgs; [
    # Guide media (`just media`, features/guide): frames to WebM and MP4,
    # and to GIF. Run as programs, never linked.
    ffmpeg
    gifski
    # `just ui-css` / `just auth-ui-css`: the committed utility sheets.
    tailwindcss_4
    # `just test` and main CI's test job (.github/workflows/checks.yml).
    cargo-nextest
  ]);

  # Point the loader at the libraries the linker found, so a desktop test
  # binary can also run from this shell. The C++ runtime too: Luau
  # (architect-scripting, -canvas) is C++, and a test binary linking it needs
  # libstdc++ at run time on NixOS.
  env.LD_LIBRARY_PATH = lib.makeLibraryPath (common ++ linux ++ [ pkgs.stdenv.cc.cc.lib ]);

  # The justfile's auth demo as processes, off by default:
  # `devenv up auth-dev mock-oauth`.
  processes.mock-oauth = {
    exec = "cargo run -p mock-oauth";
    # Up once :4040 answers at all (curl exits 0 on any HTTP status).
    ready.exec = "${pkgs.curl}/bin/curl -s -o /dev/null http://127.0.0.1:4040/";
    start.enable = false;
  };
  processes.auth-dev = {
    exec = "just auth-dev";
    after = [ "devenv:processes:mock-oauth" ];
    start.enable = false;
  };
}
