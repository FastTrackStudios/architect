//! Where an app's control socket is, and whether it serves one.
//!
//! One socket per running app, named after it: `<runtime dir>/fts/<app>.sock`
//! on Unix (`$XDG_RUNTIME_DIR`, else the temporary directory, which is the
//! user's own on macOS), in a directory only the user can enter, the
//! socket itself `0600`. On Windows a named pipe,
//! `\\.\pipe\fts-<user>-<app>`, that takes no remote clients.

use std::io;
use std::path::{Path, PathBuf};

/// `FTS_CONTROL=0` serves no control socket; `FTS_CONTROL=1` serves one
/// even where the app would not (a release build).
pub const ENABLE_ENV: &str = "FTS_CONTROL";

/// Whether an app serves its control socket: what `FTS_CONTROL` says,
/// else `default` — what [`dev_default`] is for an app that serves it in
/// development only.
#[must_use]
pub fn wanted(default: bool) -> bool {
    wanted_by(std::env::var(ENABLE_ENV).ok().as_deref(), default)
}

/// [`wanted`] for a value of `FTS_CONTROL`: `0` is off, `1` on, anything
/// else (or nothing) `default`.
#[must_use]
pub fn wanted_by(value: Option<&str>, default: bool) -> bool {
    match value.map(str::trim) {
        Some("0") => false,
        Some("1") => true,
        _ => default,
    }
}

/// On in a debug build, off in a release one: the control socket's
/// default.
#[must_use]
pub const fn dev_default() -> bool {
    cfg!(debug_assertions)
}

/// The directory app sockets are in: the user's runtime directory, else
/// the temporary one, then `fts`.
#[must_use]
pub fn socket_dir() -> PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("fts")
}

/// Where app `name`'s control socket is.
///
/// # Errors
///
/// `name` is not a plain name (empty, a path, a dot first).
pub fn socket_path(name: &str) -> io::Result<PathBuf> {
    check_name(name)?;
    Ok(path_for(name))
}

#[cfg(unix)]
fn path_for(name: &str) -> PathBuf {
    socket_dir().join(format!("{name}.sock"))
}

#[cfg(windows)]
fn path_for(name: &str) -> PathBuf {
    let user = std::env::var("USERNAME").unwrap_or_default();
    PathBuf::from(format!(r"\\.\pipe\fts-{user}-{name}"))
}

#[cfg(not(any(unix, windows)))]
fn path_for(name: &str) -> PathBuf {
    socket_dir().join(format!("{name}.sock"))
}

/// What `--app` names: a socket's path as it is (one with a separator in
/// it, or ending `.sock`), else an app's name.
///
/// # Errors
///
/// It is neither a path nor a plain name.
pub fn resolve(app: &str) -> io::Result<PathBuf> {
    let is_path = app.contains('/')
        || app.contains('\\')
        || Path::new(app)
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("sock"));
    if is_path {
        Ok(PathBuf::from(app))
    } else {
        socket_path(app)
    }
}

/// The app a socket's path names (`task` for `…/fts/task.sock`).
#[must_use]
pub fn app_of(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let name = name.strip_suffix(".sock").unwrap_or(name);
    #[cfg(windows)]
    let name = name
        .strip_prefix("fts-")
        .and_then(|rest| rest.split_once('-').map(|(_, app)| app))
        .unwrap_or(name);
    Some(name.to_owned())
}

/// Every socket in [`socket_dir`]: the apps that may be running (a socket
/// left by one that crashed is here too until the next start removes it).
/// Empty on Windows, whose pipes are not files.
#[must_use]
pub fn sockets() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(socket_dir()) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "sock"))
        .collect();
    out.sort();
    out
}

/// An app's name is one plain file-name part.
fn check_name(name: &str) -> io::Result<()> {
    let plain = !name.is_empty()
        && !name.starts_with('.')
        && !name
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':') || c.is_control() || c.is_whitespace());
    if plain {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name:?} is not a plain app name for a control socket"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{app_of, resolve, socket_path, wanted_by};

    #[test]
    fn only_zero_and_one_say_anything() {
        assert!(!wanted_by(Some("0"), true));
        assert!(wanted_by(Some(" 1 "), false));
        assert!(wanted_by(Some("yes"), true));
        assert!(!wanted_by(None, false));
    }

    #[cfg(unix)]
    #[test]
    fn a_name_is_a_socket_in_the_runtime_dir_and_a_path_is_itself() {
        let task = socket_path("task").expect("a plain name");
        assert!(task.ends_with("fts/task.sock"), "{}", task.display());
        assert_eq!(app_of(&task).as_deref(), Some("task"));
        assert_eq!(
            resolve("/tmp/x.sock").expect("a path"),
            std::path::PathBuf::from("/tmp/x.sock")
        );
        assert_eq!(resolve("task").expect("a name"), task);
        assert!(socket_path("../x").is_err());
        assert!(socket_path("").is_err());
        assert!(socket_path("a b").is_err());
    }
}
