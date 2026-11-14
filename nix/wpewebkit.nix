# WPE WebKit with the WPE Platform API (architect-webview, feature `wpe`;
# docs/content/architecture/embedded-views.md, "Web views").
#
# nixpkgs ships WebKit only as the GTK port (webkitgtk_6_0); this is the same
# derivation built from the WPE release tarball as `-DPORT=WPE`, with the
# WPE Platform API (headless and Wayland; no DRM/KMS backend, no MiniBrowser,
# no introspection or docs). About 160 MB installed.
{ pkgs }:
pkgs.webkitgtk_6_0.overrideAttrs (o: {
  pname = "wpewebkit";
  version = "2.52.6";
  src = pkgs.fetchurl {
    url = "https://wpewebkit.org/releases/wpewebkit-2.52.6.tar.xz";
    sha256 = "13l5bjjv4cfri6lw741mpbpg0bsl1zzk1wihypyvf98nfprgxfmj";
  };
  outputs = [ "out" "dev" ];
  buildInputs = (o.buildInputs or [ ]) ++ (with pkgs; [ libwpe libdrm mesa libinput udev ]);
  cmakeFlags = [
    "-DPORT=WPE"
    "-DENABLE_WPE_PLATFORM=ON"
    "-DENABLE_WPE_PLATFORM_HEADLESS=ON"
    "-DENABLE_WPE_PLATFORM_WAYLAND=ON"
    "-DENABLE_WPE_PLATFORM_DRM=OFF"
    "-DENABLE_INTROSPECTION=OFF"
    "-DENABLE_DOCUMENTATION=OFF"
    "-DENABLE_MINIBROWSER=OFF"
    "-DENABLE_API_TESTS=OFF"
    "-DENABLE_JOURNALD_LOG=OFF"
    "-DUSE_LIBBACKTRACE=OFF"
    "-DENABLE_SPEECH_SYNTHESIS=OFF"
    "-DUSE_LIBSECRET=OFF"
    "-DBWRAP_EXECUTABLE=${pkgs.lib.getExe pkgs.bubblewrap}"
    "-DDBUS_PROXY_EXECUTABLE=${pkgs.lib.getExe pkgs.xdg-dbus-proxy}"
  ];
  postFixup = "";
  doCheck = false;
  passthru = { };
})
