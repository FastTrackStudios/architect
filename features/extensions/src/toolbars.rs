//! The app's toolbars (feature `host`): every extension's `[[toolbars]]`
//! and a person's toolbar files, the files over the extensions'
//! (`docs/content/architecture/extensions.md`, "Toolbars").
//!
//! A person's files are `*.toml` in a folder the app names
//! ([`Toolbars::set_dir`]: `toolbars/` in its folder), one toolbar each
//! ([`ToolbarDecl::parse`]); [`Toolbars::poll`] reads them again when one
//! changes, so an edit shows at once. A file whose `id` is an extension
//! toolbar's key (`fts.metronome/transport`) is the person's copy of it,
//! used in its place while the file is there ([`Toolbars::reset`] takes it
//! away); another id is a toolbar of the person's own (`user/<id>`). A file
//! that does not read or check is a problem (`APP_STATUS`), the rest load.
//!
//! It keeps which action is armed, too ([`Toolbars::arm`]): a button whose
//! right-click arms its action shows it armed until it is disarmed; what
//! an armed action does next (to what is clicked) is the app's.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use architect::action::Problem;

use crate::contrib::{By, Contributions};
use crate::toolbar::ToolbarDecl;

/// Who a person's own toolbars are owned by (`user/<id>`).
pub const PERSON: &str = "user";

/// Where a toolbar in force comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// An extension's manifest, as it is.
    Extension,
    /// A person's file in place of an extension's.
    Override,
    /// A person's file of their own.
    Person,
}

/// A toolbar in force.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toolbar {
    /// `<extension>/<id>`, or `user/<id>`.
    pub key: String,
    /// The extension whose it is (whose views its widgets are), or `user`.
    pub owner: String,
    pub decl: ToolbarDecl,
    pub source: Source,
    /// The person's file it is read from.
    pub file: Option<PathBuf>,
}

#[derive(Default)]
struct State {
    dir: Option<PathBuf>,
    /// The files as last read: by key, each with its path.
    files: BTreeMap<String, (ToolbarDecl, PathBuf)>,
    problems: Vec<Problem>,
    /// What was read: each file and when it last changed.
    seen: Vec<(PathBuf, Option<SystemTime>)>,
    armed: Option<String>,
    revision: u64,
    next_listener: u64,
    listeners: Vec<(u64, Arc<dyn Fn(u64) + Send + Sync>)>,
}

/// The toolbars. Cheap to clone: every clone is the same.
#[derive(Clone, Default)]
pub struct Toolbars(Arc<Mutex<State>>);

impl std::fmt::Debug for Toolbars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.lock();
        f.debug_struct("Toolbars")
            .field("dir", &s.dir)
            .field("files", &s.files.keys().collect::<Vec<_>>())
            .field("armed", &s.armed)
            .finish()
    }
}

/// Takes its listener off when dropped.
pub struct Listening {
    state: std::sync::Weak<Mutex<State>>,
    id: u64,
}

impl Drop for Listening {
    fn drop(&mut self) {
        if let Some(s) = self.state.upgrade() {
            s.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .listeners
                .retain(|(id, _)| *id != self.id);
        }
    }
}

impl std::fmt::Debug for Listening {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Listening").field(&self.id).finish()
    }
}

/// A key as a file name: `fts.metronome/transport` → `fts.metronome--transport.toml`.
fn file_name(key: &str) -> String {
    let safe: String = key
        .chars()
        .map(|c| match c {
            '/' => '-',
            c if c.is_alphanumeric() || matches!(c, '.' | '_' | '-') => c,
            _ => '_',
        })
        .collect();
    format!("{safe}.toml")
}

/// What a person's file checks against: any action (theirs to run), a
/// widget's view named whole (`<extension>/<view>`).
fn check_file(t: &ToolbarDecl) -> Result<(), String> {
    t.check(
        |_| Ok(()),
        |view| {
            if view
                .split_once('/')
                .is_some_and(|(e, v)| !e.is_empty() && !v.is_empty())
            {
                Ok(())
            } else {
                Err(format!(
                    "{view:?}: a person's widget names <extension>/<view>"
                ))
            }
        },
    )
}

impl Toolbars {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Tells the listeners, after the lock.
    fn changed(mut s: MutexGuard<'_, State>) {
        s.revision = s.revision.wrapping_add(1);
        let r = s.revision;
        let listeners: Vec<_> = s.listeners.iter().map(|(_, l)| Arc::clone(l)).collect();
        drop(s);
        for l in listeners {
            l(r);
        }
    }

    /// Calls `listener` with the revision after every change (a file read
    /// again, an action armed).
    pub fn subscribe(&self, listener: impl Fn(u64) + Send + Sync + 'static) -> Listening {
        let mut s = self.lock();
        let id = s.next_listener;
        s.next_listener = id.wrapping_add(1);
        s.listeners.push((id, Arc::new(listener)));
        drop(s);
        Listening {
            state: Arc::downgrade(&self.0),
            id,
        }
    }

    /// Moves on with every change.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.lock().revision
    }

    /// Reads a person's toolbar files from `dir` (made when it is not
    /// there), and again whenever [`Self::poll`] sees one change.
    pub fn set_dir(&self, dir: impl Into<PathBuf>) {
        let dir = dir.into();
        let _ = std::fs::create_dir_all(&dir);
        self.lock().dir = Some(dir);
        self.reload();
    }

    /// The folder a person's files are in.
    #[must_use]
    pub fn dir(&self) -> Option<PathBuf> {
        self.lock().dir.clone()
    }

    /// The files and when each last changed.
    fn stamps(dir: &Path) -> Vec<(PathBuf, Option<SystemTime>)> {
        let mut out: Vec<_> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .map(|p| {
                let at = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
                (p, at)
            })
            .collect();
        out.sort();
        out
    }

    /// Reads the files again when one was added, changed or removed since
    /// the last read: whether it did.
    #[must_use = "whether the files changed"]
    pub fn poll(&self) -> bool {
        let Some(dir) = self.dir() else {
            return false;
        };
        if Self::stamps(&dir) == self.lock().seen {
            return false;
        }
        self.reload();
        true
    }

    /// Reads every file in the folder.
    pub fn reload(&self) {
        let Some(dir) = self.dir() else {
            return;
        };
        let seen = Self::stamps(&dir);
        let mut files = BTreeMap::new();
        let mut problems = Vec::new();
        for (path, _) in &seen {
            let shown = path.display().to_string();
            let read = std::fs::read_to_string(path)
                .map_err(|e| e.to_string())
                .and_then(|text| ToolbarDecl::parse(&text))
                .and_then(|t| check_file(&t).map(|()| t));
            match read {
                Ok(t) => {
                    let key = if t.id.contains('/') {
                        t.id.clone()
                    } else {
                        format!("{PERSON}/{}", t.id)
                    };
                    files.insert(key, (t, path.clone()));
                }
                Err(why) => {
                    problems.push(
                        Problem::new("toolbars", format!("{shown}: {why}")).file(shown.clone()),
                    );
                }
            }
        }
        let mut s = self.lock();
        s.files = files;
        s.problems = problems;
        s.seen = seen;
        Self::changed(s);
    }

    /// What is wrong with the person's files now: one problem a file.
    #[must_use]
    pub fn problems(&self) -> Vec<Problem> {
        self.lock().problems.clone()
    }

    /// Every toolbar in force: each extension's (a person's copy in its
    /// place), then the person's own.
    #[must_use]
    pub fn all(&self, contributions: &Contributions) -> Vec<Toolbar> {
        let files = self.lock().files.clone();
        let mut out: Vec<Toolbar> = contributions
            .toolbars()
            .into_iter()
            .map(|By { owner, item }| {
                let key = format!("{owner}/{}", item.id);
                match files.get(&key) {
                    Some((mine, path)) => Toolbar {
                        decl: ToolbarDecl {
                            id: item.id,
                            ..mine.clone()
                        },
                        key,
                        owner,
                        source: Source::Override,
                        file: Some(path.clone()),
                    },
                    None => Toolbar {
                        key,
                        owner,
                        decl: item,
                        source: Source::Extension,
                        file: None,
                    },
                }
            })
            .collect();
        for (key, (decl, path)) in &files {
            if let Some(id) = key.strip_prefix(&format!("{PERSON}/")) {
                out.push(Toolbar {
                    key: key.clone(),
                    owner: PERSON.to_owned(),
                    decl: ToolbarDecl {
                        id: id.to_owned(),
                        ..decl.clone()
                    },
                    source: Source::Person,
                    file: Some(path.clone()),
                });
            }
        }
        out
    }

    /// The toolbar in force under `key`.
    #[must_use]
    pub fn get(&self, contributions: &Contributions, key: &str) -> Option<Toolbar> {
        self.all(contributions).into_iter().find(|t| t.key == key)
    }

    /// The toolbars in the header of a pane of view kind `kind`.
    #[must_use]
    pub fn in_header_of(&self, contributions: &Contributions, kind: &str) -> Vec<Toolbar> {
        self.all(contributions)
            .into_iter()
            .filter(|t| t.decl.in_header_of(kind))
            .collect()
    }

    /// Writes `decl` as the person's toolbar under `key` (an extension's
    /// key: their copy of it; `user/<id>` or a bare id: their own), and
    /// reads the folder again: its file.
    ///
    /// # Errors
    ///
    /// No folder, a toolbar that does not check, a file that cannot be
    /// written.
    pub fn save(&self, key: &str, decl: &ToolbarDecl) -> Result<PathBuf, String> {
        let dir = self
            .dir()
            .ok_or("no folder for toolbars (Toolbars::set_dir)")?;
        let key = if key.contains('/') {
            key.to_owned()
        } else {
            format!("{PERSON}/{key}")
        };
        let mut decl = decl.clone();
        decl.id = key
            .strip_prefix(&format!("{PERSON}/"))
            .map_or_else(|| key.clone(), str::to_owned);
        check_file(&decl)?;
        let path = self
            .lock()
            .files
            .get(&key)
            .map_or_else(|| dir.join(file_name(&key)), |(_, p)| p.clone());
        std::fs::write(&path, decl.to_toml()?).map_err(|e| format!("{}: {e}", path.display()))?;
        self.reload();
        Ok(path)
    }

    /// Takes away the person's file under `key` (their copy: the
    /// extension's comes back): whether there was one.
    ///
    /// # Errors
    ///
    /// The file cannot be removed.
    pub fn reset(&self, key: &str) -> Result<bool, String> {
        let path = self.lock().files.get(key).map(|(_, p)| p.clone());
        let Some(path) = path else {
            return Ok(false);
        };
        std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.reload();
        Ok(true)
    }

    /// Arms `action` (one at a time): its buttons show it armed.
    pub fn arm(&self, action: &str) {
        let mut s = self.lock();
        if s.armed.as_deref() == Some(action) {
            return;
        }
        s.armed = Some(action.to_owned());
        Self::changed(s);
    }

    /// Disarms whatever is armed: what was.
    // What was armed is for whoever wants it.
    #[allow(clippy::must_use_candidate)]
    pub fn disarm(&self) -> Option<String> {
        let mut s = self.lock();
        let was = s.armed.take();
        if was.is_some() {
            Self::changed(s);
        }
        was
    }

    /// The action armed now.
    #[must_use]
    pub fn armed(&self) -> Option<String> {
        self.lock().armed.clone()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::contrib::Contribution;
    use crate::toolbar::ToolItem;

    fn bar(id: &str, action: &str) -> ToolbarDecl {
        ToolbarDecl {
            id: id.into(),
            title: "T".into(),
            at: "dock:right".into(),
            items: vec![ToolItem {
                action: action.into(),
                ..ToolItem::default()
            }],
            ..ToolbarDecl::default()
        }
    }

    #[test]
    fn a_persons_file_overrides_an_extensions_and_adds_their_own() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = Contributions::new();
        reg.put(
            "lab.x",
            Contribution {
                toolbars: vec![bar("t", "LAB_X_A")],
                ..Contribution::default()
            },
        );
        let bars = Toolbars::new();
        bars.set_dir(tmp.path());
        assert_eq!(bars.all(&reg)[0].source, Source::Extension);

        // Their copy, saved: in its place, the same key.
        let mut mine = bar("t", "LAB_X_B");
        mine.title = "Mine".into();
        let path = bars.save("lab.x/t", &mine).unwrap();
        assert!(path.ends_with("lab.x-t.toml"), "{}", path.display());
        let all = bars.all(&reg);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].source, Source::Override);
        assert_eq!(all[0].decl.title, "Mine");
        assert_eq!(all[0].decl.id, "t");
        // One of their own, written by hand: read on the next poll.
        std::fs::write(
            tmp.path().join("own.toml"),
            bar("own", "ANYTHING").to_toml().unwrap(),
        )
        .unwrap();
        assert!(bars.poll());
        assert!(!bars.poll(), "nothing changed since");
        let keys: Vec<String> = bars.all(&reg).into_iter().map(|t| t.key).collect();
        assert_eq!(keys, ["lab.x/t", "user/own"]);
        // A broken file is a problem; the rest stay.
        std::fs::write(tmp.path().join("broken.toml"), "id = ").unwrap();
        assert!(bars.poll());
        assert_eq!(bars.problems().len(), 1);
        assert!(bars.problems()[0].message.contains("broken.toml"));
        assert_eq!(bars.all(&reg).len(), 2);
        // Reset: the extension's comes back.
        assert!(bars.reset("lab.x/t").unwrap());
        assert_eq!(bars.all(&reg)[0].source, Source::Extension);
        assert!(!bars.reset("lab.x/t").unwrap());
    }

    #[test]
    fn one_action_is_armed_at_a_time() {
        let bars = Toolbars::new();
        let heard = Arc::new(Mutex::new(0_u32));
        let h = Arc::clone(&heard);
        let _l = bars.subscribe(move |_| *h.lock().unwrap() += 1);
        bars.arm("A");
        bars.arm("A");
        bars.arm("B");
        assert_eq!(bars.armed().as_deref(), Some("B"));
        assert_eq!(bars.disarm().as_deref(), Some("B"));
        assert_eq!(bars.disarm(), None);
        assert_eq!(
            *heard.lock().unwrap(),
            3,
            "a change each time, none for the same"
        );
    }
}
