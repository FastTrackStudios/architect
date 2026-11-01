//! `lab-media`: the lab's guide scenarios as media (`--out`, by default
//! `target/media`), or as tests (`--test`). Run from the workspace's root:
//! `cargo run -p architect-lab-media -- showcase-layout`.

fn main() -> std::process::ExitCode {
    architect_guide::cli::main(
        architect_lab_media::app,
        std::path::Path::new(architect_lab_media::SCENARIOS),
    )
}
