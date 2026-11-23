//! Extensions' `process` views (#838): a pane drawn by a program in the
//! extension's package, whichever copy of it is loaded.
//!
//! ```toml
//! [capabilities]
//! programs = ["bin/room"]
//!
//! [[views]]
//! id = "room"
//! title = "Room"
//! kind = "process"
//! program = "bin/room"
//! args = ["--process-view"]
//! ```
//!
//! The app draws such a view with its process-view pane
//! (`architect-process-view`'s `ProcessViewPane`,
//! `docs/content/architecture/process-views.md`): the program renders on
//! its own GPU device and hands each frame to the app's window. This
//! module is what the app asks before it starts one, [`launch`]: the
//! program's file, checked, and why not when it cannot run.
//!
//! - **Platform.** Linux and macOS for now ([`not_here`]): elsewhere the
//!   extension loads and the pane says why, as other platform limits do.
//! - **Consent.** Running a native program is a capability: the manifest
//!   asks for it (`capabilities.programs`, `Manifest::check` refuses a
//!   process view whose program is not asked for), a person grants it to
//!   an installed copy, and what was granted is checked each time a pane
//!   opens ([`Host::capabilities`]). A built-in or development copy runs
//!   with what it asks, as for every capability. A share link's guest
//!   never gets one (`share_needs::guest_capabilities`).
//! - **The package.** The program is a file in the package: never a path
//!   out of it (`Manifest::check`), nor a link inside it that points out
//!   ([`package_file`]).
//! - **Settings.** An argument may name one of its settings,
//!   `{{setting:<key>}}`, as a fetch's header names a secret: its value
//!   (the one set, else the default) is put in when the program starts, and
//!   a change starts it again (`architect-process-view`'s
//!   `extension::refresh`). An argument that is a setting alone and comes
//!   out empty is left out, so `--show={{setting:x.show}}` is the form for
//!   a value that may be empty.
//! - **Its version.** [`Launch::stamp`] is the file's size and time and the
//!   copy loaded: a development copy rebuilt (`EXT_DEV` reloads it) or
//!   another copy loaded starts the program again.

use std::path::{Path, PathBuf};

use crate::Origin;
use crate::host::Host;

/// The platforms the app draws process views on.
pub const PLATFORMS: &[&str] = &["linux", "macos"];

/// Why process views cannot be shown on `platform` (`crate::platform::here`,
/// `Host::platform`), in a person's words; `None` where they can.
#[must_use]
pub fn not_here(platform: &str) -> Option<String> {
    if PLATFORMS.contains(&platform) {
        return None;
    }
    let on = crate::platform::name(platform);
    Some(match platform {
        "web" => {
            "a browser does not start programs: process views run in the desktop app".to_owned()
        }
        "windows" => format!(
            "process views run on Linux and macOS only for now: on {on} the program's frames \
             need a shared DXGI texture, designed and not built"
        ),
        _ => format!("process views run on Linux and macOS only for now, not on {on}"),
    })
}

/// A process view's program, ready to start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub extension: String,
    pub view: String,
    /// The program's file, resolved inside the package.
    pub program: PathBuf,
    pub args: Vec<String>,
    /// The package's folder: the program's working directory.
    pub dir: PathBuf,
    /// Which copy is loaded.
    pub origin: Origin,
    /// The program's file as it is now (its size and time) and the copy
    /// loaded: a new stamp starts the program again.
    pub stamp: String,
}

/// Extension `extension`'s process view `view`, ready to start: or why it
/// cannot be, in a person's words.
///
/// # Errors
///
/// It is not loaded, or `view` is not one of its process views; process
/// views do not run on this platform; it may not run the program (not
/// granted); its copy has no package folder; the program is not a file in
/// the package.
pub fn launch(host: &Host, extension: &str, view: &str) -> Result<Launch, String> {
    let loaded = host
        .get(extension)
        .ok_or_else(|| format!("{extension} is not loaded"))?;
    let decl = loaded
        .manifest
        .views
        .iter()
        .find(|v| v.id == view && v.kind == architect_views::PROCESS)
        .ok_or_else(|| format!("{extension} has no process view {view}"))?;
    if let Some(why) = not_here(&host.platform()) {
        return Err(why);
    }
    let program = decl
        .program
        .as_deref()
        .ok_or_else(|| format!("its view {view} names no program"))?;
    let may = host
        .capabilities(extension)
        .is_some_and(|c| c.may_run_program(program));
    if !may {
        return Err(format!(
            "{extension} may not run {program} on this computer: it is not granted \
             (capabilities.programs). Grant it on the Extensions page to show this view."
        ));
    }
    let dir = loaded
        .dir
        .clone()
        .ok_or_else(|| format!("{extension} has no package folder to run {program} from"))?;
    let file = package_file(&dir, program)?;
    let stamp = stamp(&file, loaded.origin);
    let args = decl
        .args
        .iter()
        .filter_map(|arg| {
            crate::manifest::with_settings(arg, |key| {
                arg_text(host.setting_value(extension, key).as_ref())
            })
        })
        .collect();
    Ok(Launch {
        extension: extension.to_owned(),
        view: view.to_owned(),
        program: file,
        args,
        dir,
        origin: loaded.origin,
        stamp,
    })
}

/// The program a process view lacks only a person's yes to run.
///
/// Extension `extension`'s view `view` asks to run it and a person has not
/// granted it (an installed copy whose grant was taken back, or an update
/// that asks for it): what its pane offers to allow.
#[must_use]
pub fn needs_grant(host: &Host, extension: &str, view: &str) -> Option<String> {
    let loaded = host.get(extension)?;
    let program = loaded
        .manifest
        .views
        .iter()
        .find(|v| v.id == view && v.kind == architect_views::PROCESS)?
        .program
        .clone()?;
    let asks = loaded.manifest.capabilities.may_run_program(&program);
    let may = host
        .capabilities(extension)
        .is_some_and(|c| c.may_run_program(&program));
    (asks && !may && host.grants().is_some() && not_here(&host.platform()).is_none())
        .then_some(program)
}

/// A person's yes, given in the app (the pane's Allow): extension
/// `extension` may run `program` on this device, added to what it was
/// granted here. Only a program its manifest asks for.
///
/// # Errors
///
/// It is not loaded, does not ask for `program`, or the app keeps no
/// grants.
pub fn grant_program(host: &Host, extension: &str, program: &str, by: &str) -> Result<(), String> {
    let loaded = host
        .get(extension)
        .ok_or_else(|| format!("{extension} is not loaded"))?;
    if !loaded.manifest.capabilities.may_run_program(program) {
        return Err(format!("{extension} does not ask to run {program}"));
    }
    let grants = host.grants().ok_or("this app keeps no grants")?;
    let mut caps = grants
        .of(extension)
        .into_iter()
        .find(|g| g.scope == crate::consent::Scope::Device)
        .map(|g| g.capabilities)
        .unwrap_or_default();
    if !caps.programs.iter().any(|p| p == program) {
        caps.programs.push(program.to_owned());
    }
    grants.grant(
        extension,
        &loaded.manifest.version.to_string(),
        caps,
        crate::consent::Scope::Device,
        by,
    );
    host.grants_changed(extension);
    Ok(())
}

/// `program` (a path the manifest names) as a file inside package `dir`,
/// resolved: a link inside the package that points out of it is outside
/// too, as for a web view's files.
///
/// # Errors
///
/// It is outside the package, missing, not a file, or (Unix) not
/// executable.
pub fn package_file(dir: &Path, program: &str) -> Result<PathBuf, String> {
    let (top, found) = match (dir.canonicalize(), dir.join(program).canonicalize()) {
        (Ok(top), Ok(found)) => (top, found),
        (Err(e), _) => return Err(format!("its package {}: {e}", dir.display())),
        (_, Err(_)) => {
            return Err(format!(
                "{program} is not in its package (build it, or check the view's program)"
            ));
        }
    };
    if !found.starts_with(&top) {
        return Err(format!(
            "{program} points out of its package: a program runs from the package only"
        ));
    }
    let meta = std::fs::metadata(&found).map_err(|e| format!("{program}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{program} is not a file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if meta.permissions().mode() & 0o111 == 0 {
            return Err(format!("{program} is not executable"));
        }
    }
    Ok(found)
}

/// A setting's value as an argument: text as it is, a switch `true` or
/// `false`, a number as written, nothing as empty, anything else as JSON.
fn arg_text(value: Option<&serde_json::Value>) -> String {
    match value {
        None | Some(serde_json::Value::Null) => String::new(),
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// `file`'s size and modification time, and `origin`.
fn stamp(file: &Path, origin: Origin) -> String {
    let meta = std::fs::metadata(file).ok();
    let len = meta.as_ref().map_or(0, std::fs::Metadata::len);
    let at = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    format!("{origin:?}:{len}:{at}")
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{not_here, package_file};

    fn program(dir: &std::path::Path, rel: &str) {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn linux_and_macos_draw_them_and_elsewhere_says_why() {
        assert_eq!(not_here("linux"), None);
        assert_eq!(not_here("macos"), None);
        let windows = not_here("windows").unwrap();
        assert!(
            windows.contains("Linux and macOS") && windows.contains("DXGI"),
            "{windows}"
        );
        assert!(not_here("web").unwrap().contains("browser"));
        assert!(not_here("ios").unwrap().contains("iOS"));
    }

    #[test]
    fn the_program_is_an_executable_file_in_the_package() {
        let dir = tempfile::tempdir().unwrap();
        program(dir.path(), "bin/room");
        let found = package_file(dir.path(), "bin/room").unwrap();
        assert!(found.ends_with("bin/room"));
        assert!(package_file(dir.path(), "bin/none").is_err());
        std::fs::write(dir.path().join("data"), "x").unwrap();
        let e = package_file(dir.path(), "data").unwrap_err();
        assert!(e.contains("not executable"), "{e}");
        let e = package_file(dir.path(), "bin").unwrap_err();
        assert!(e.contains("not a file"), "{e}");
    }

    #[test]
    fn a_link_out_of_the_package_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        program(other.path(), "tool");
        std::os::unix::fs::symlink(other.path().join("tool"), dir.path().join("tool")).unwrap();
        let e = package_file(dir.path(), "tool").unwrap_err();
        assert!(e.contains("out of its package"), "{e}");
        // A link that stays inside is the file it points to.
        program(dir.path(), "bin/real");
        std::os::unix::fs::symlink(dir.path().join("bin/real"), dir.path().join("bin/alias"))
            .unwrap();
        assert!(package_file(dir.path(), "bin/alias").is_ok());
    }
}
