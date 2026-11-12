//! The standard Architect (#631): with `gui` (the default) the Architect
//! app's window; without it, the headless node (`serve`) a hosted
//! Architect runs.

fn main() -> std::process::ExitCode {
    architect_desktop::run(architect_standard::distribution())
}
