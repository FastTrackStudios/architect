//! `sandbox-media`: the sandbox's guide scenarios as media (`--out`, by default
//! `target/media`), or as tests (`--test`). Run from the workspace's root:
//! `cargo run -p architect-sandbox-media -- showcase-layout`. A tour is a
//! scenario too: `-- extensions/projects/tours/projects-tour.styx` films
//! it, over the Architect app (its demo vault) with
//! `SANDBOX_MEDIA_APP=architect`.

fn main() -> std::process::ExitCode {
    let make = if std::env::var("SANDBOX_MEDIA_APP").as_deref() == Ok("architect") {
        architect_sandbox_media::architect_app
    } else {
        architect_sandbox_media::app
    };
    architect_guide::cli::main(
        make,
        std::path::Path::new(architect_sandbox_media::SCENARIOS),
    )
}
