//! `sandbox-media`: the sandbox's guide scenarios as media (`--out`, by default
//! `target/media`), or as tests (`--test`). Run from the workspace's root:
//! `cargo run -p architect-sandbox-media -- showcase-layout`.

fn main() -> std::process::ExitCode {
    architect_guide::cli::main(
        architect_sandbox_media::app,
        std::path::Path::new(architect_sandbox_media::SCENARIOS),
    )
}
