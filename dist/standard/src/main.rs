//! The standard Architect (#631): with `gui` (the default) the Architect
//! app's window; without it, the headless node (`serve`) a hosted
//! Architect runs. Built for iOS (`cargo xtask dist ios`), the iPhone and
//! iPad app (#640).

fn main() -> std::process::ExitCode {
    #[cfg(not(target_os = "ios"))]
    return architect_desktop::run(architect_standard::distribution());
    #[cfg(target_os = "ios")]
    return architect_mobile::run(architect_standard::distribution());
}
