//! Feature `wpe` (Linux): compiles the WPE WebKit glue (`src/wpe/glue.c`)
//! against WPE WebKit and the WPE Platform API, found with pkg-config
//! (`devenv --profile wpe shell`; `nix/wpewebkit.nix`).

fn main() {
    println!("cargo::rerun-if-changed=src/wpe/glue.c");
    println!("cargo::rerun-if-changed=src/wpe/glue.h");
    #[cfg(feature = "wpe")]
    wpe();
}

#[cfg(feature = "wpe")]
fn wpe() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }
    let mut build = cc::Build::new();
    build
        .file("src/wpe/glue.c")
        .warnings(true)
        // GObject vfuncs and signal handlers take what they do not use.
        .flag_if_supported("-Wno-unused-parameter");
    for lib in ["wpe-webkit-2.0", "wpe-platform-2.0", "epoxy", "gbm"] {
        match pkg_config::Config::new().probe(lib) {
            Ok(found) => {
                for path in found.include_paths {
                    build.include(path);
                }
            }
            Err(e) => panic!(
                "architect-webview feature `wpe` needs {lib} (WPE WebKit with the WPE Platform API): \
                 run in `devenv --profile wpe shell` (nix/wpewebkit.nix). {e}"
            ),
        }
    }
    build.compile("architect_wpe_glue");
}
