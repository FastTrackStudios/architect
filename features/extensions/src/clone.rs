//! `EXT_CLONE` and the extensions' problems (#268 H4, H7).
//!
//! What the app ships (a built-in extension) and what came from a
//! registry (an installed one) is not edited where it is: a built-in is
//! linked into the build, an installed copy is replaced on update.
//! [`Extensions::clone_to_dev`] copies one into the app's development
//! folder ([`Extensions::with_dev_root`]) under a new id — its manifest
//! rewritten to that id, its action ids and setting keys moved to the new
//! namespace — and runs it as a development copy, which shadows nothing
//! (a new id) and reloads when its files change.
//!
//! [`Extensions::problems`] is what is wrong with them now, for
//! `APP_STATUS`: a folder whose manifest does not read (its loaded copy,
//! if any, keeps running), and the last load of each that failed.

use std::path::{Path, PathBuf};

use architect::action::Problem;

use crate::extensions::Extensions;
use crate::host::{Event, LoadError};
use crate::manifest::{MANIFEST, Manifest, action_prefix};
use crate::package::{Files, PackageError};

/// What [`Extensions::clone_to_dev`] made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cloned {
    /// The copy's id.
    pub id: String,
    /// Its folder.
    pub folder: PathBuf,
    /// What loading it did.
    pub events: Vec<Event>,
}

impl Extensions {
    /// Where [`Self::clone_to_dev`] puts copies: `<root>/<id>`.
    #[must_use]
    pub fn with_dev_root(self, root: impl Into<PathBuf>) -> Self {
        self.set_dev_root(Some(root.into()));
        self
    }

    /// [`Self::with_dev_root`] on extensions already shared.
    pub fn set_dev_root(&self, root: Option<PathBuf>) {
        *self.dev_root_slot() = root;
    }

    /// The development folder, when the app has one.
    #[must_use]
    pub fn dev_root(&self) -> Option<PathBuf> {
        self.dev_root_slot().clone()
    }

    /// Copies extension `id` (the copy that runs: built in, installed or
    /// in development) into the development folder as `as_id`
    /// (`<id>-copy` unless given) and runs the copy.
    ///
    /// # Errors
    ///
    /// No development folder, no such extension, a process extension with
    /// actions (its program answers them by their old ids), the new id
    /// taken or not an id, or a file that could not be copied.
    pub fn clone_to_dev(&self, id: &str, as_id: Option<&str>) -> Result<Cloned, String> {
        let root = self
            .dev_root()
            .ok_or("this app has no development folder for copies")?;
        let chosen = self.chosen();
        let from = chosen
            .iter()
            .find(|c| c.runs.id == id)
            .ok_or_else(|| format!("no extension {id} (EXT_LIST lists them)"))?;
        if from.runs.package.manifest.kind == crate::manifest::Kind::Process
            && !from.runs.package.manifest.actions.is_empty()
        {
            return Err(format!(
                "{id} is a process extension: its program answers its actions by their ids, so a copy \
                 under a new id would not run; copy its source and build it under the new id instead"
            ));
        }
        let new_id = as_id.map_or_else(|| format!("{id}-copy"), |a| a.trim().to_owned());
        if chosen.iter().any(|c| c.runs.id == new_id) {
            return Err(format!("an extension {new_id} exists: give `as`"));
        }
        let folder = root.join(&new_id);
        if folder.exists() {
            return Err(format!("{} exists: give `as`", folder.display()));
        }
        let package = &from.runs.package;
        let text = package.text(MANIFEST).map_err(|e| e.to_string())?;
        let manifest = renamed(&text, &package.manifest, &new_id)?;
        let made = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(&folder)?;
            match &package.files {
                Files::Dir(dir) => copy_dir(dir, &folder)?,
                Files::Embedded(files) => {
                    for (path, text) in files {
                        let to = folder.join(path);
                        if let Some(parent) = to.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        std::fs::write(to, text)?;
                    }
                }
            }
            std::fs::write(folder.join(MANIFEST), manifest)
        })();
        if let Err(e) = made {
            let _ = std::fs::remove_dir_all(&folder);
            return Err(format!("copying {id} to {}: {e}", folder.display()));
        }
        let events = self.dev(folder.clone());
        Ok(Cloned {
            id: new_id,
            folder,
            events,
        })
    }

    /// Puts [`Self::problems`] among `problems` (an app's list's,
    /// `ActionList::problems`), as the `extensions` source: what
    /// `APP_STATUS` lists.
    pub fn report_problems(&self, problems: &architect::action::Problems) {
        let me = self.clone();
        problems.source("extensions", move || me.problems());
    }

    /// What is wrong with the extensions now: a folder that does not read,
    /// and each one's last failed load. Each is a [`Problem`] from
    /// `extension`, its file and field when they are known.
    #[must_use]
    pub fn problems(&self) -> Vec<Problem> {
        let mut out: Vec<Problem> = self
            .unreadable()
            .iter()
            .filter_map(|event| match event {
                Event::Failed { id, error } => Some(problem(id, error)),
                _ => None,
            })
            .collect();
        for (id, error) in self.failed_loads() {
            out.push(problem(&id, &error));
        }
        out
    }
}

/// One failure as a problem: a dev folder's id is its path.
fn problem(id: &str, error: &LoadError) -> Problem {
    let folder = Path::new(id).is_dir().then(|| Path::new(id).to_path_buf());
    match error {
        LoadError::Package(PackageError::Read { path, why }) => {
            Problem::new("extension", format!("{id}: {path} cannot be read: {why}"))
                .file(path.clone())
        }
        LoadError::Package(PackageError::Manifest(e)) => {
            let (field, line, message) = crate::check::manifest_fields(e);
            let mut p = Problem::new(
                "extension",
                line.map_or_else(
                    || format!("{id}: {message}"),
                    |line| format!("{id}: line {line}: {message}"),
                ),
            );
            if let Some(folder) = &folder {
                p = p.file(folder.join(MANIFEST).display().to_string());
            }
            if let Some(field) = field {
                p = p.field(field);
            }
            p
        }
        other => Problem::new("extension", format!("{id}: {other}")),
    }
}

/// `text` (a manifest) as extension `new_id`'s: its id, its name marked a
/// copy, its action ids, setting keys and settings sections in the new
/// namespace, and a line saying where it came from. Checked as a
/// manifest.
fn renamed(text: &str, manifest: &Manifest, new_id: &str) -> Result<String, String> {
    let mut table: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
    let (old_prefix, new_prefix) = (action_prefix(&manifest.id), action_prefix(new_id));
    let (old_ns, new_ns) = (format!("{}.", manifest.id), format!("{new_id}."));
    for (key, value) in &mut table {
        if key != "name" && key != "description" {
            rename_in(value, &old_prefix, &new_prefix, &old_ns, &new_ns);
        }
    }
    // A settings section's id is the app's, not the extension's: the
    // copy's sit beside the original's under ids of their own.
    if let Some(toml::Value::Array(sections)) = table.get_mut("sections") {
        for section in sections {
            if let Some(toml::Value::String(id)) = section.get_mut("id") {
                *id = format!("{new_id}.{id}");
            }
        }
    }
    table.insert("id".to_owned(), toml::Value::String(new_id.to_owned()));
    table.insert(
        "name".to_owned(),
        toml::Value::String(format!("{} (copy)", manifest.name)),
    );
    let body = toml::to_string(&table).map_err(|e| e.to_string())?;
    let out = format!(
        "# A copy of {} {} (EXT_CLONE): yours to change. The original is untouched.\n{body}",
        manifest.id, manifest.version
    );
    Manifest::parse(&out).map_err(|e| format!("the copy's manifest: {e}"))?;
    Ok(out)
}

/// Every string in `value` that names the old namespace, in the new one.
fn rename_in(
    value: &mut toml::Value,
    old_prefix: &str,
    new_prefix: &str,
    old_ns: &str,
    new_ns: &str,
) {
    match value {
        toml::Value::String(s) => {
            if let Some(rest) = s.strip_prefix(old_prefix) {
                *s = format!("{new_prefix}{rest}");
            } else if let Some(rest) = s.strip_prefix(old_ns) {
                *s = format!("{new_ns}{rest}");
            }
        }
        toml::Value::Array(items) => {
            for item in items {
                rename_in(item, old_prefix, new_prefix, old_ns, new_ns);
            }
        }
        toml::Value::Table(table) => {
            for (key, item) in table.iter_mut() {
                if key != "name" && key != "description" && key != "title" {
                    rename_in(item, old_prefix, new_prefix, old_ns, new_ns);
                }
            }
        }
        _ => {}
    }
}

/// `from`'s files into `to`, the manifest aside (written after); a link
/// is linked again to what it points at.
fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == MANIFEST || name.to_string_lossy().starts_with('.') {
            continue;
        }
        let (src, dst) = (entry.path(), to.join(&name));
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            let target = std::fs::canonicalize(&src)?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, &dst)?;
            #[cfg(not(unix))]
            std::fs::copy(target, &dst).map(drop)?;
        } else if kind.is_dir() {
            std::fs::create_dir_all(&dst)?;
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}
