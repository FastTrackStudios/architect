//! The web app as a program, for `dx serve` (`devenv up web`,
//! `cargo xtask dev run web`): dx builds and serves a bin target. The
//! release bundle is the library (`scripts/web-bundle.sh`), whose start
//! is the same function.

fn main() {
    #[cfg(target_arch = "wasm32")]
    architect_web::run(architect_distribution::architect());
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!(
        "architect-web runs in a browser: `devenv up web` (dx serve), or `just web` (the bundle)"
    );
}
