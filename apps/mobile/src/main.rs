//! The Architect app on an iPhone or an iPad (`architect_mobile::run`).
//! `ARCHITECT_SANDBOX=1` runs it as the dev sandbox instead, on made-up
//! data (one bundle on a phone, so no second binary). A build of someone's
//! own has the same one line with its own distribution (`dist/standard`).

fn main() -> std::process::ExitCode {
    let sandbox = std::env::var_os("ARCHITECT_SANDBOX").is_some_and(|v| v == "1");
    architect_mobile::run(if sandbox {
        architect_app::sandbox()
    } else {
        architect_mobile::distribution::architect()
    })
}
