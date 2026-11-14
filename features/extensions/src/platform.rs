//! Where an extension runs (#578): a manifest's `platforms`, and the
//! platform this app runs on.
//!
//! `platforms = ["linux"]` in `extension.toml` names the operating systems
//! an extension runs on, for every kind: Rust's `target_os` values
//! (`linux`, `macos`, `windows`, `ios`, `android`) and `web` for a browser.
//! Absent (or empty) means everywhere. One that cannot run here is kept,
//! listed as "Linux only", never loaded, its data untouched
//! ([`crate::host::LoadError::NotHere`]); the store refuses to install it
//! here with the same words ([`crate::install::InstallError::NotHere`]).

/// Every value `platforms` takes.
pub const PLATFORMS: &[&str] = &["linux", "macos", "windows", "ios", "android", "web"];

/// The platform this build runs on: `web` in a browser, else Rust's
/// `target_os`.
#[must_use]
pub const fn here() -> &'static str {
    if cfg!(all(target_arch = "wasm32", not(target_os = "wasi"))) {
        "web"
    } else {
        std::env::consts::OS
    }
}

/// What a person calls `platform`: `macOS`, `Linux`, `iOS`, the web.
#[must_use]
pub fn name(platform: &str) -> String {
    match platform {
        "linux" => "Linux".to_owned(),
        "macos" => "macOS".to_owned(),
        "windows" => "Windows".to_owned(),
        "ios" => "iOS".to_owned(),
        "android" => "Android".to_owned(),
        "web" => "the web".to_owned(),
        other => other.to_owned(),
    }
}

/// Whether an extension declaring `platforms` runs on `platform`.
#[must_use]
pub fn runs_on(platforms: &[String], platform: &str) -> bool {
    platforms.is_empty() || platforms.iter().any(|p| p == platform)
}

/// Why an extension declaring `platforms` cannot run on `platform`, in a
/// person's words ("Linux only", "macOS and Windows only"); `None` when it
/// can.
#[must_use]
pub fn not_here(platforms: &[String], platform: &str) -> Option<String> {
    if runs_on(platforms, platform) {
        return None;
    }
    let names: Vec<String> = platforms.iter().map(|p| name(p)).collect();
    let list = match names.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    Some(format!("{list} only"))
}

/// The unknown values in `platforms`, for the manifest's check.
pub(crate) fn unknown(platforms: &[String]) -> Option<&str> {
    platforms
        .iter()
        .map(String::as_str)
        .find(|p| !PLATFORMS.contains(p))
}

#[cfg(test)]
mod tests {
    use super::{not_here, runs_on, unknown};

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn absent_platforms_mean_everywhere() {
        assert!(runs_on(&[], "ios"));
        assert_eq!(not_here(&[], "web"), None);
    }

    #[test]
    fn a_linux_only_extension_says_so_elsewhere() {
        assert!(runs_on(&v(&["linux"]), "linux"));
        assert_eq!(not_here(&v(&["linux"]), "macos").as_deref(), Some("Linux only"));
        assert_eq!(
            not_here(&v(&["linux", "macos", "windows"]), "ios").as_deref(),
            Some("Linux, macOS and Windows only")
        );
    }

    #[test]
    fn an_unknown_platform_is_named() {
        assert_eq!(unknown(&v(&["linux", "darwin"])), Some("darwin"));
        assert_eq!(unknown(&v(&["web"])), None);
    }
}
