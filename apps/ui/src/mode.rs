//! Which app is running: the Architect app, on the person's own vaults,
//! or the dev sandbox, on made-up data.
//!
//! Both are this crate. The binary says which before it launches
//! ([`Mode::set`]): `architect-desktop` and `architect-mobile` are the
//! Architect app, `architect-sandbox` (`apps/sandbox`) the sandbox. A test
//! or a film that mounts the app in-process sets nothing and gets the
//! sandbox, so the goldens, scenarios and films run on its made-up data.
//!
//! Each has a folder of its own ([`Mode::folder`] under the platform's
//! config dir, `$ARCHITECT_CONFIG_DIR` / `$ARCHITECT_SANDBOX_CONFIG_DIR`
//! overriding it) and an identity of its own ([`Mode::bundle_id`]), so the
//! two run side by side and the sandbox never touches the person's
//! settings.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Which app is running.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// The Architect app: the person's vault, opened or made on first run.
    Architect,
    /// The dev sandbox: the made-up Inbox, Tasks, Schedule, Board and
    /// Clicker, for framework demos, goldens, scenarios and films.
    Sandbox,
}

static MODE: OnceLock<Mode> = OnceLock::new();

/// The lab's folder before it became the Architect app, under the
/// platform's config dir: copied into [`Mode::Architect`]'s once
/// ([`migrate_lab_folder`]).
pub const LAB_FOLDER: &str = "architect-lab";

/// The lab's override of its folder, still read by the Architect app
/// when `$ARCHITECT_CONFIG_DIR` is not set ([`Mode::set`]).
pub const LAB_CONFIG_ENV: &str = "ARCHITECT_LAB_CONFIG_DIR";

impl Mode {
    /// Says which app this process is, once, before launch; a second
    /// call is ignored. For the Architect app it also adopts the lab's
    /// `$ARCHITECT_LAB_CONFIG_DIR` as `$ARCHITECT_CONFIG_DIR` when only
    /// the old name is set, so every store under the app's folder reads it.
    pub fn set(self) {
        if MODE.set(self).is_err() {
            return;
        }
        if self == Self::Architect {
            let new = architect::settings::env_var(self.folder());
            let unset = std::env::var_os(&new).is_none_or(|v| v.is_empty());
            if let Some(old) = std::env::var_os(LAB_CONFIG_ENV).filter(|v| !v.is_empty() && unset) {
                // SAFETY: called from `main` before the app starts any
                // thread (the binaries' first act).
                unsafe { std::env::set_var(new, old) };
            }
        }
    }

    /// The app this process is: what [`Mode::set`] said, else (a test or
    /// a film mounting the app) the sandbox.
    #[must_use]
    pub fn current() -> Self {
        MODE.get().copied().unwrap_or(Self::Sandbox)
    }

    /// The sandbox's made-up data and example extensions are on.
    #[must_use]
    pub const fn is_sandbox(self) -> bool {
        matches!(self, Self::Sandbox)
    }

    /// Its name, for the window's title and the about.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Architect => "Architect",
            Self::Sandbox => "Architect Sandbox",
        }
    }

    /// Its folder's name, under the platform's config dir, and the app id
    /// its stores open (`architect::settings::app_dir`):
    /// `$ARCHITECT_CONFIG_DIR` or `$ARCHITECT_SANDBOX_CONFIG_DIR` names
    /// another.
    #[must_use]
    pub const fn folder(self) -> &'static str {
        match self {
            Self::Architect => "architect",
            Self::Sandbox => "architect-sandbox",
        }
    }

    /// Its bundle identifier: the one instance, the login item, the Dock.
    #[must_use]
    pub const fn bundle_id(self) -> &'static str {
        match self {
            Self::Architect => "app.fasttrackstudio.architect",
            Self::Sandbox => "app.fasttrackstudio.architect-sandbox",
        }
    }
}

/// The running app's folder: its settings, themes, layouts, extensions in
/// development and the agents' guide (`AGENTS.md`).
#[must_use]
pub fn app_dir() -> Option<PathBuf> {
    architect::settings::app_dir(Mode::current().folder()).ok()
}

/// [`app_dir`], or a folder of its name under the temp dir where the
/// platform has no config dir.
#[must_use]
pub fn app_dir_or_temp() -> PathBuf {
    app_dir().unwrap_or_else(|| std::env::temp_dir().join(Mode::current().folder()))
}

/// Copies the lab's folder into the Architect app's, once.
///
/// `<config dir>/architect-lab` goes into `<config dir>/architect` when
/// that one is missing or empty: its settings, themes, layouts and tour
/// place carry over. The lab's folder is left as it was. Nothing happens when either folder
/// is overridden by its environment variable (a test, a session of the
/// person's choosing). Returns the folder copied into.
#[must_use]
pub fn migrate_lab_folder() -> Option<PathBuf> {
    let overridden = |app: &str| {
        std::env::var_os(architect::settings::env_var(app)).is_some_and(|v| !v.is_empty())
    };
    let to = Mode::Architect.folder();
    if overridden(to) || overridden(LAB_FOLDER) {
        return None;
    }
    let from = architect::settings::app_dir(LAB_FOLDER).ok()?;
    let to = architect::settings::app_dir(to).ok()?;
    match migrate(&from, &to) {
        Ok(true) => {
            tracing::info!(from = %from.display(), to = %to.display(), "app.config.migrated");
            Some(to)
        }
        Ok(false) => None,
        Err(e) => {
            tracing::warn!(from = %from.display(), to = %to.display(), "app.config.migrate_failed: {e}");
            None
        }
    }
}

/// Copies `from` into `to` when `from` is a folder and `to` is missing or
/// empty. Links are copied as links (an extension's development copy is
/// one); anything else that is neither a file nor a folder is skipped.
///
/// # Errors
///
/// A file cannot be read or written.
pub fn migrate(from: &Path, to: &Path) -> std::io::Result<bool> {
    if !from.is_dir() {
        return Ok(false);
    }
    if to.is_dir() && std::fs::read_dir(to)?.next().is_some() {
        return Ok(false);
    }
    let mut todo = vec![(from.to_path_buf(), to.to_path_buf())];
    while let Some((src, dst)) = todo.pop() {
        std::fs::create_dir_all(&dst)?;
        for e in std::fs::read_dir(&src)? {
            let e = e?;
            let (path, into) = (e.path(), dst.join(e.file_name()));
            let kind = e.file_type()?;
            if kind.is_symlink() {
                #[cfg(unix)]
                std::os::unix::fs::symlink(std::fs::read_link(&path)?, &into)?;
            } else if kind.is_dir() {
                todo.push((path, into));
            } else if kind.is_file() {
                std::fs::copy(&path, &into)?;
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::migrate;

    #[test]
    fn the_lab_folder_is_copied_into_an_empty_app_folder_once() {
        let tmp = tempfile::tempdir().unwrap();
        let (lab, app) = (
            tmp.path().join("architect-lab"),
            tmp.path().join("architect"),
        );
        std::fs::create_dir_all(lab.join("themes")).unwrap();
        std::fs::write(lab.join("settings.json"), "{}").unwrap();
        std::fs::write(lab.join("themes/mine.json"), "{\"id\":\"mine\"}").unwrap();
        std::fs::create_dir_all(&app).unwrap();
        assert!(
            migrate(&lab, &app).unwrap(),
            "an empty folder takes the lab's"
        );
        assert_eq!(
            std::fs::read_to_string(app.join("settings.json")).unwrap(),
            "{}"
        );
        assert!(app.join("themes/mine.json").is_file());
        assert!(
            lab.join("settings.json").is_file(),
            "the lab's folder is left"
        );
        std::fs::write(lab.join("settings.json"), "{\"changed\":true}").unwrap();
        assert!(
            !migrate(&lab, &app).unwrap(),
            "only once: the app's folder has files now"
        );
        assert_eq!(
            std::fs::read_to_string(app.join("settings.json")).unwrap(),
            "{}"
        );
    }

    #[test]
    fn no_lab_folder_means_nothing_to_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("architect");
        assert!(!migrate(&tmp.path().join("architect-lab"), &app).unwrap());
        assert!(!app.exists());
    }
}
