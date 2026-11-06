# architect's dev environment. The toolchain (rust 1.99.1, the
# rust-toolchain.toml pin), mold, cargo-hakari, cargo-rail (change files,
# .changes/README) and prek come from devenv-fleet's `rust` module; dx,
# wasm-bindgen, binaryen and tailwindcss from its `dioxus` module
# (devenv.yaml). This file adds what this workspace links against and runs.
{ pkgs, lib, config, ... }:
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
    # What a Blitz window (winit, wgpu) dlopens at run time: without them the
    # app panics "Error creating DeviceHandle" (no GPU adapter) under
    # `devenv up` / `just app`. Studio's shell carries the same.
    wayland
    libxkbcommon
    vulkan-loader
    libGL
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
    # `just test` and main CI's test job (.github/workflows/checks.yml).
    cargo-nextest
  ]);

  # Point the loader at the libraries the linker found, so a desktop test
  # binary can also run from this shell. The C++ runtime too: Luau
  # (architect-scripting, -canvas) is C++, and a test binary linking it needs
  # libstdc++ at run time on NixOS.
  env.LD_LIBRARY_PATH = lib.makeLibraryPath (common ++ linux ++ [ pkgs.stdenv.cc.cc.lib ]);

  # `devenv up`: the Architect app (apps/desktop) under the fleet's dx,
  # plus the telemetry module's Teley (:8787) and collector (:4318), which
  # the app's spans go to. `just app` runs the app alone, without dx, as a
  # person runs it (its first run asks for a vault).
  # The app's build as a task the app process waits on, so `devenv up`'s TUI
  # shows the app waiting (as auth-dev waits on mock-oauth) while it
  # compiles, with the build's own log. The same flags as `dx serve` below,
  # so its build after this is a quick up-to-date check. A failed build
  # fails the task and the app does not start.
  tasks."app:build" = {
    exec = "cd apps/desktop && dx build --platform desktop --renderer native";
    before = [ "devenv:processes:app" ];
  };
  # Without Rust hot-patching: it builds a different ("fat") binary from
  # `dx build`, so the task's build was thrown away (86s twice), and dx's
  # own help calls it experimental and segfault-prone. rsx hot reload stays.
  # The app opens a scratch copy of the demo vault (apps/ui/demo-vault),
  # seeded under devenv's state when it is missing, so a run never edits the
  # repo's copy or a vault of the person's; `just vault-reset` seeds it again.
  processes.app = {
    exec = "cd apps/desktop && exec dx serve --platform desktop --renderer native --hot-patch false";
    env = config.fleet.telemetry.processEnv // {
      ARCHITECT_VAULT = "${config.devenv.state}/demo-vault";
      ARCHITECT_VAULT_SEED = "${config.devenv.root}/apps/ui/demo-vault";
    };
    # Ready only while the built app itself runs, not while dx is still
    # compiling it: the TUI shows the app waiting during the build, running
    # once its window is up, and unhealthy if the app dies under dx. dx
    # launches it from target/dx/architect-desktop/… (the [p] keeps pgrep
    # from matching this probe's own command line).
    ready = {
      exec = "${pkgs.procps}/bin/pgrep -f 'target/dx/architect-deskto[p]/' >/dev/null";
      period = 2;
      failure_threshold = 2;
    };
  };
  # The dev sandbox (apps/sandbox, made-up data and the example
  # extensions) the same way, off by default: `devenv up sandbox`.
  processes.sandbox = {
    exec = "cd apps/sandbox && exec dx serve --platform desktop --renderer native --hot-patch false";
    env = config.fleet.telemetry.processEnv // {
      ARCHITECT_VAULT = "${config.devenv.state}/demo-vault";
      ARCHITECT_VAULT_SEED = "${config.devenv.root}/apps/ui/demo-vault";
    };
    start.enable = false;
  };

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

  # `devenv --profile wasi shell`: the fleet toolchain plus `wasm32-wasip2`,
  # what a WASI extension (`features/extensions/wasi-guest`, the sandbox's
  # `apps/sandbox/wasi-counter`) builds for. A profile, not the default shell:
  # an extra target changes the rustc store path, so kache shares nothing
  # with the other repos' builds in it (devenv-fleet `fleet.rust.extraTargets`).
  # Node too: jco transpiles a component for the browser
  # (features/extensions/web, `just web`) and its tests run under node.
  # The current release, not 24: the transpiled components suspend through
  # JSPI (`WebAssembly.Suspending`), which nixpkgs' node 24 lacks
  # (features/extensions/tests/wasi_web.rs runs node from here).
  profiles.wasi.module = {
    fleet.rust.extraTargets = [ "wasm32-wasip2" ];
    packages = [ pkgs.nodejs_latest ];
  };
}
