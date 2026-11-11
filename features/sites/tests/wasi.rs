//! A WASI component's embed view drawing a site's block (#488): the
//! sandbox's counter (`apps/sandbox/wasi-counter`), built for
//! `wasm32-wasip2` here, draws a ```` ```bars ```` fence as an SVG that
//! follows the playhead, at build time, with no window and nothing of it
//! linked in.
//!
//! Built by this toolchain when it has the target, else in `devenv
//! --profile wasi shell`; `ARCHITECT_WASI_GUESTS=<dir>` names a folder it
//! was built into instead. With none of those the test says so and passes;
//! `ARCHITECT_WASI_REQUIRED=1` (CI) makes that a failure.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use architect_sites::{Block, BlockRenderer, Claim, ComponentBlocks, InstalledBlocks};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn has_wasip2() -> bool {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    Command::new(rustc)
        .args(["--print", "sysroot"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .is_some_and(|s| {
            Path::new(s.trim())
                .join("lib/rustlib/wasm32-wasip2")
                .is_dir()
        })
}

/// Where the counter's component was built, once.
fn guests() -> Option<&'static Path> {
    static BUILT: OnceLock<Option<PathBuf>> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            if let Ok(dir) = std::env::var("ARCHITECT_WASI_GUESTS") {
                return Some(PathBuf::from(dir));
            }
            let target = root().join("target/wasi-guest");
            let build = [
                "build",
                "-p",
                "architect-sandbox-wasi-counter",
                "--target",
                "wasm32-wasip2",
                "--target-dir",
                target.to_str().unwrap(),
            ];
            let mut cmd = if has_wasip2() {
                let mut c =
                    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned()));
                c.args(build);
                c
            } else {
                let mut c = Command::new("devenv");
                c.args(["--profile", "wasi", "shell", "--", "cargo"])
                    .args(build);
                c
            };
            for var in [
                "CARGO_TARGET_DIR",
                "CARGO_BUILD_TARGET",
                "RUSTC",
                "CARGO_ENCODED_RUSTFLAGS",
            ] {
                cmd.env_remove(var);
            }
            let exit = {
                let _devenv = devenv_lock();
                cmd.current_dir(root()).status()
            };
            match exit {
                Ok(s) if s.success() => Some(target.join("wasm32-wasip2/debug")),
                other => {
                    let why = format!("the WASI counter did not build ({other:?})");
                    assert!(std::env::var("ARCHITECT_WASI_REQUIRED").is_err(), "{why}");
                    eprintln!("skipped: {why}");
                    None
                }
            }
        })
        .as_deref()
}

/// The counter's package, as an install writes it, under `dir`.
fn install(dir: &Path, built: &Path) -> PathBuf {
    let pkg = dir.join("lab.wasi-counter");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::copy(
        root().join("apps/sandbox/wasi-counter/extension.toml"),
        pkg.join("extension.toml"),
    )
    .unwrap();
    let wasm = "architect_sandbox_wasi_counter.wasm";
    std::fs::copy(built.join(wasm), pkg.join(wasm)).unwrap();
    pkg
}

#[test]
fn a_components_embed_view_draws_a_fence_as_an_svg_that_follows() {
    let Some(built) = guests() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let pkg = install(tmp.path(), built);
    let blocks = ComponentBlocks::load(&pkg)
        .unwrap()
        .expect("it draws a fence");
    assert_eq!(blocks.id(), "lab.wasi-counter");
    assert_eq!(blocks.fences(), vec!["bars".to_owned()]);

    let bars = Claim::Fence("bars".into());
    assert!(blocks.claims(&bars));
    assert!(!blocks.claims(&Claim::Fence("kf".into())));
    let block = |source| Block {
        claim: &bars,
        info: "bars",
        source,
        page: "Songs/joy.md",
    };
    let html = blocks
        .render(&block("| C | G | Am |"))
        .unwrap()
        .into_html()
        .unwrap();
    assert!(html.starts_with("<svg class=\"wasi-bars\""), "{html}");
    assert!(html.ends_with("</svg>"), "{html}");
    for kept in [
        "viewBox=\"0 0 240 40\"",
        "width=\"240\"",
        "data-follow-playhead=\"\"",
        "<rect class=\"wasi-bar\" y=\"0\" height=\"40\" fill=\"transparent\" data-at=\"0\" x=\"0\" width=\"80\"></rect>",
        "data-at=\"2\" x=\"160\"",
        ">Am</text>",
    ] {
        assert!(html.contains(kept), "lost {kept}: {html}");
    }
    // Each block is its own mount: the next draws its own input.
    let next = blocks.render(&block("| F |")).unwrap().into_html().unwrap();
    assert!(next.contains("viewBox=\"0 0 80 40\""), "{next}");
    assert!(!next.contains("Am"), "{next}");

    // The same, found in a folder of installed extensions (the node's).
    let installed = InstalledBlocks::new(tmp.path());
    assert!(installed.claims(&bars));
    assert_eq!(installed.drawn_by(&bars), "lab.wasi-counter");
}

/// Held while `devenv` runs. Under nextest every test is a process of its
/// own, and each `devenv` run rewrites its profile's bootstrap files: two
/// at once and one reads a half-written `resolve-lock.nix` ("unexpected
/// end of file"), so the components "did not build". One lock file for
/// every test binary that enters the `wasi` profile.
#[allow(
    clippy::incompatible_msrv,
    reason = "a test: it builds with the workspace toolchain, not the MSRV"
)]
fn devenv_lock() -> Option<std::fs::File> {
    let dir = root().join("target");
    std::fs::create_dir_all(&dir).ok()?;
    let file = std::fs::File::create(dir.join(".devenv-wasi.lock")).ok()?;
    file.lock().ok()?;
    Some(file)
}
