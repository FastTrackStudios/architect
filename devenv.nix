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
  # The stack's ports (dev-stack.toml: the one place they are set) and the
  # command every one of its processes starts through.
  stack = builtins.fromTOML (builtins.readFile ./dev-stack.toml);
  url = name: "http://127.0.0.1:${toString stack.ports.${name}}";
  curl = "${pkgs.curl}/bin/curl";
  xdev = "exec cargo run -q -p xtask -- dev run";
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

  # `devenv up`: the whole stack on this machine (#541,
  # docs/content/guide/local-stack.md), each process started by
  # `cargo xtask dev run <process>` (xtask/src/dev), which runs the guard
  # (nothing may name production or leave loopback), decides the fill (a
  # sanitised mirror of production on a machine with an admin token, the
  # demo seed everywhere else) and execs the binary with its environment.
  # The stack's test (xtask/tests/dev_stack.rs) starts the same command, so
  # the two cannot drift. Ports are dev-stack.toml's; state is under
  # devenv's (`$DEVENV_STATE/stack`, 0700), `just dev-reset` removes it.
  # Plus the telemetry module's Teley (:8787) and collector (:4318).
  tasks."app:build" = {
    # The app's build as a task the app process waits on, so `devenv up`'s
    # TUI shows the app waiting while it compiles, with the build's own
    # log. The same flags as the app's `dx serve`, so its build after this
    # is a quick up-to-date check. A failed build fails the task and the
    # app does not start.
    exec = "cd apps/desktop && dx build --platform desktop --renderer native";
    before = [ "devenv:processes:app" ];
  };

  # The stand-in for GitHub, Google and TONE3000 the auth server's social
  # buttons go to. Up once it answers at all (curl exits 0 on any status).
  processes.mock-oauth = {
    exec = "${xdev} mock-oauth";
    ready.exec = "${curl} -s -o /dev/null ${url "mock_oauth"}/";
  };
  # The auth server: on-disk SQLite and a secret of its own under devenv's
  # state, filled once at start (it prints which fill and why).
  processes.auth = {
    exec = "${xdev} auth";
    after = [ "devenv:processes:mock-oauth" ];
    ready.exec = "${curl} -sf -o /dev/null ${url "auth"}/health";
  };
  # The headless node (`architect-desktop serve`, built without the window)
  # signed in to that auth server, hosting the fill's org and its vault,
  # `ARCHITECT_IROH=local`: no relay, no lookup, loopback only.
  processes.node = {
    exec = "${xdev} node";
    after = [ "devenv:processes:auth" ];
    ready.exec = "${curl} -sf -o /dev/null ${url "node"}/health";
  };
  # The Architect app (apps/desktop) under the fleet's dx, its issuer the
  # local auth server and its orgs' node the local node, with a folder of
  # its own (its sessions a file there, never the keychain), on a scratch
  # copy of the demo vault
  # (`just vault-reset`). Without Rust hot-patching: it builds a different
  # ("fat") binary from `dx build`, and dx calls it experimental. rsx hot
  # reload stays.
  processes.app = {
    exec = "${xdev} app";
    after = [ "devenv:processes:node" ];
    env = config.fleet.telemetry.processEnv;
    # Ready only while the built app itself runs, not while dx is still
    # compiling it. dx launches it from target/dx/architect-desktop/… (the
    # [p] keeps pgrep from matching this probe's own command line).
    ready = {
      exec = "${pkgs.procps}/bin/pgrep -f 'target/dx/architect-deskto[p]/' >/dev/null";
      period = 2;
      failure_threshold = 2;
    };
  };
  # Off by default: `devenv up web` (the web app, its issuer the local
  # one) and `devenv up site` (apps/site's dev server).
  processes.web = {
    exec = "${xdev} web";
    after = [ "devenv:processes:auth" ];
    start.enable = false;
  };
  processes.site = {
    exec = "${xdev} site";
    start.enable = false;
  };
  # The dev sandbox (apps/sandbox, made-up data and the example
  # extensions), off by default: `devenv up sandbox`.
  processes.sandbox = {
    exec = "cd apps/sandbox && exec dx serve --platform desktop --renderer native --hot-patch false";
    env = config.fleet.telemetry.processEnv // {
      ARCHITECT_VAULT = "${config.devenv.state}/demo-vault";
      ARCHITECT_VAULT_SEED = "${config.devenv.root}/apps/ui/demo-vault";
    };
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
