//! Which layer of a share link each attached file is in (#528).
//!
//! The `[[share_layers]]` rules of the extensions installed on the node
//! and of a distribution's compiled-in ones
//! (`architect_extensions::share_layers`).
//!
//! The node holds no extension running; it reads their manifests
//! (`architect_sites::InstalledLayers`, read again when the folder
//! changes). Nothing an extension does, or fails to do, opens a link:
//!
//! - **The highest wins.** A file is in the highest layer any rule for
//!   its note's type places it in, every extension's rules together: no
//!   rule lowers another's.
//! - **Unknown is closed.** While a manifest in the folder does not read
//!   (half-written, not a manifest), where a file is is unknown
//!   ([`LayerRules::place`] answers None) and the node holds every
//!   attached file back on a link below Full until it is fixed.
//! - **Removed is kept.** The rules each extension declared are kept in
//!   the node's data dir (`share-layers.json`) and still place files after
//!   it is uninstalled, until the owner forgets them
//!   ([`LayerRules::forget`], `ShareLinks::forget_layers`). An installed
//!   extension's rules are exactly what its manifest declares now.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use architect_sites::listing::Entry;
use architect_sites::{InstalledLayers, ShareLayersDecl};
use architect_spaces::{KeptLayers, Listen};

/// How many times this process has placed a vault's files.
static PLACEMENTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many times this process has placed a vault's files: a placement
/// is made once per vault change or rules change, never per request.
#[must_use]
pub fn placements() -> u64 {
    PLACEMENTS.load(std::sync::atomic::Ordering::Relaxed)
}

/// The rules a node places files by. Cheap to clone.
#[derive(Clone, Default)]
pub struct LayerRules(Arc<Inner>);

#[derive(Default)]
struct Inner {
    /// The installed extensions'; None: none read.
    installed: Option<InstalledLayers>,
    /// A distribution's compiled-in extensions' rules.
    given: Vec<ShareLayersDecl>,
    /// Where each extension's last declared rules are kept; None: kept
    /// in memory only.
    kept_at: Option<PathBuf>,
    state: Mutex<State>,
}

/// How far the installed folder has been read.
#[derive(Default, PartialEq, Eq)]
enum Read {
    #[default]
    Not,
    /// No folder to read.
    NoFolder,
    /// The folder, at this generation.
    At(u64),
}

#[derive(Default)]
struct State {
    /// The installed folder as last read.
    read: Read,
    /// Each extension's last declared rules, by id.
    kept: BTreeMap<String, Kept>,
    /// The kept file could not be read: unknown until it is.
    kept_broken: Option<String>,
    current: Arc<Current>,
}

/// One extension's last declared rules, as kept.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Kept {
    rules: Vec<ShareLayersDecl>,
    /// When they were read from its manifest (unix seconds).
    since: i64,
}

/// The rules in force at one moment.
#[derive(Debug, Default)]
pub struct Current {
    /// Counts up each time the rules change.
    pub generation: u64,
    rules: Vec<ShareLayersDecl>,
    /// What does not read: manifests, or the kept rules' file. While
    /// there is one, placement is unknown.
    pub broken: Vec<String>,
    installed: BTreeSet<String>,
}

impl Current {
    /// Whether no rule places anything and nothing is unknown.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.rules.is_empty() && self.broken.is_empty()
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl LayerRules {
    /// The rules of the extensions installed in `dir`, kept at `kept_at`
    /// once read, and `given` (a distribution's compiled-in ones).
    #[must_use]
    pub fn new(
        dir: Option<PathBuf>,
        given: Vec<ShareLayersDecl>,
        kept_at: Option<PathBuf>,
    ) -> Self {
        let mut state = State::default();
        if let Some(path) = &kept_at {
            match std::fs::read_to_string(path) {
                Ok(text) => match serde_json::from_str(&text) {
                    Ok(kept) => state.kept = kept,
                    Err(error) => {
                        tracing::warn!(path = %path.display(), %error, "the kept share layer rules do not read: share links below Full hold back every attached file until the file is fixed or removed");
                        state.kept_broken = Some(format!("{}: {error}", path.display()));
                    }
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => state.kept_broken = Some(format!("{}: {error}", path.display())),
            }
        }
        Self(Arc::new(Inner {
            installed: dir.map(InstalledLayers::new),
            given,
            kept_at,
            state: Mutex::new(state),
        }))
    }

    /// The rules in force now: the installed folder read again if it
    /// changed, each extension's declared rules kept.
    #[must_use]
    pub fn current(&self) -> Arc<Current> {
        let read = self.0.installed.as_ref().map(InstalledLayers::read);
        let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        let now = read
            .as_ref()
            .map_or(Read::NoFolder, |r| Read::At(r.generation));
        if state.read != now {
            state.read = now;
            let mut changed = false;
            let mut installed = BTreeSet::new();
            for (id, rules) in read.iter().flat_map(|r| r.packages.iter()) {
                installed.insert(id.clone());
                // Exactly what it declares now, and kept for when it is
                // gone.
                if rules.is_empty() {
                    changed |= state.kept.remove(id).is_some();
                } else if state.kept.get(id).is_none_or(|k| &k.rules != rules) {
                    state.kept.insert(
                        id.clone(),
                        Kept {
                            rules: rules.clone(),
                            since: unix_now(),
                        },
                    );
                    changed = true;
                }
            }
            if changed {
                self.save(&state.kept);
            }
            let broken = read
                .iter()
                .flat_map(|r| r.broken.iter())
                .map(|(path, why)| format!("{}: {why}", path.display()))
                .chain(state.kept_broken.clone())
                .collect();
            Self::refresh(&mut state, &self.0.given, installed, broken);
        }
        Arc::clone(&state.current)
    }

    /// `state.current` from its kept rules and `given`, a generation on.
    fn refresh(
        state: &mut State,
        given: &[ShareLayersDecl],
        installed: BTreeSet<String>,
        broken: Vec<String>,
    ) {
        let rules = given
            .iter()
            .cloned()
            .chain(state.kept.values().flat_map(|k| k.rules.iter().cloned()))
            .collect();
        state.current = Arc::new(Current {
            generation: state.current.generation.wrapping_add(1),
            rules,
            broken,
            installed,
        });
    }

    fn save(&self, kept: &BTreeMap<String, Kept>) {
        let Some(path) = &self.0.kept_at else { return };
        let written = serde_json::to_vec_pretty(kept)
            .map_err(std::io::Error::other)
            .and_then(|bytes| {
                let tmp = path.with_extension("json.tmp");
                std::fs::write(&tmp, bytes)?;
                std::fs::rename(&tmp, path)
            });
        if let Err(error) = written {
            tracing::warn!(path = %path.display(), %error, "the share layer rules could not be kept");
        }
    }

    /// The rules kept for extensions no longer installed.
    #[must_use]
    pub fn kept(&self) -> Vec<KeptLayers> {
        let installed = self.current().installed.clone();
        let state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .kept
            .iter()
            .filter(|(id, _)| !installed.contains(*id))
            .map(|(id, kept)| {
                let types: BTreeSet<String> = kept
                    .rules
                    .iter()
                    .map(|r| r.note_type.trim().to_lowercase())
                    .collect();
                KeptLayers {
                    extension: id.clone(),
                    note_types: types.into_iter().collect(),
                    since: kept.since,
                }
            })
            .collect()
    }

    /// Forget the kept rules of `extension`, no longer installed; what is
    /// kept after.
    ///
    /// # Errors
    ///
    /// It is installed (its rules are its manifest's), or nothing is kept
    /// of it.
    pub fn forget(&self, extension: &str) -> Result<Vec<KeptLayers>, String> {
        let current = self.current();
        if current.installed.contains(extension) {
            return Err(format!(
                "{extension} is installed: its rules are its manifest's; uninstall it first"
            ));
        }
        {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.kept.remove(extension).is_none() {
                return Err(format!("no layer rules are kept for {extension}"));
            }
            self.save(&state.kept);
            let broken = current.broken.clone();
            Self::refresh(&mut state, &self.0.given, current.installed.clone(), broken);
            drop(state);
        }
        Ok(self.kept())
    }

    /// The layer of each file `entries` attach that is above the page,
    /// by `current`'s rules: the highest any rule for any of its notes'
    /// types places it in. None while a manifest does not read: unknown.
    #[must_use]
    pub fn place(current: &Current, entries: &[Entry]) -> Option<HashMap<String, Listen>> {
        if !current.broken.is_empty() {
            return None;
        }
        PLACEMENTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut out: HashMap<String, Listen> = HashMap::new();
        if current.rules.is_empty() {
            return Some(out);
        }
        for entry in entries {
            // `type: song`, or a list of types (`type: [song, live]`).
            let kinds: Vec<&str> = match entry.properties.get("type") {
                Some(serde_json::Value::String(kind)) => vec![kind.as_str()],
                Some(serde_json::Value::Array(kinds)) => {
                    kinds.iter().filter_map(serde_json::Value::as_str).collect()
                }
                _ => continue,
            };
            let rules: Vec<&ShareLayersDecl> = current
                .rules
                .iter()
                .filter(|r| kinds.iter().any(|k| r.is_for(k)))
                .collect();
            if rules.is_empty() {
                continue;
            }
            for file in &entry.files {
                let layer = rules
                    .iter()
                    .filter_map(|r| r.layer_of(&entry.path, file))
                    .filter_map(|l| Listen::parse(l).ok())
                    .max()
                    .filter(|l| *l > Listen::Page);
                if let Some(layer) = layer {
                    let held = out.entry(file.clone()).or_insert(layer);
                    *held = (*held).max(layer);
                }
            }
        }
        Some(out)
    }
}

/// `entries` with every file above `cap` left out.
#[must_use]
pub fn within<S: std::hash::BuildHasher>(
    entries: &[Entry],
    placed: &HashMap<String, Listen, S>,
    cap: Listen,
) -> Vec<Entry> {
    entries
        .iter()
        .map(|e| {
            let mut e = e.clone();
            e.files
                .retain(|f| placed.get(f).is_none_or(|layer| *layer <= cap));
            e
        })
        .collect()
}

/// Whether `a` and `b` are the same file on disk, whatever each path's
/// spelling (a case-insensitive or normalising file system opens one
/// file by many names).
#[must_use]
pub fn same_file(a: &std::fs::Metadata, b: &Path) -> bool {
    let Ok(b) = std::fs::metadata(b) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        // No identity to compare: the same size and the same time
        // written is taken as the same file. It refuses too much at
        // worst, never too little: one file by two names has both.
        a.len() == b.len() && a.modified().ok() == b.modified().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, kind: Option<&str>, files: &[&str]) -> Entry {
        let mut properties = serde_json::Map::new();
        if let Some(kind) = kind {
            properties.insert("type".into(), kind.into());
        }
        Entry {
            path: path.into(),
            title: path.into(),
            properties,
            files: files.iter().map(|f| (*f).to_owned()).collect(),
        }
    }

    fn song() -> ShareLayersDecl {
        ShareLayersDecl {
            note_type: "song".into(),
            reference: vec!["Media/Reference*".into(), "*.session".into()],
            full: vec!["Media/**".into()],
            ..ShareLayersDecl::default()
        }
    }

    #[test]
    fn files_are_placed_by_their_notes_type_and_the_highest_wins() {
        let rules = LayerRules::new(None, vec![song()], None);
        let entries = [
            entry(
                "Joy/Joy.md",
                Some("song"),
                &[
                    "Joy/Joy.session",
                    "Joy/Media/Reference.ogg",
                    "Joy/Media/Bass.ogg",
                    "Joy/cover.png",
                ],
            ),
            // A plain note linking the stem: it stays Full.
            entry("Notes.md", None, &["Joy/Media/Bass.ogg", "Joy/cover.png"]),
        ];
        let placed = LayerRules::place(&rules.current(), &entries).unwrap();
        assert_eq!(placed.get("Joy/Joy.session"), Some(&Listen::Reference));
        assert_eq!(
            placed.get("Joy/Media/Reference.ogg"),
            Some(&Listen::Reference)
        );
        assert_eq!(placed.get("Joy/Media/Bass.ogg"), Some(&Listen::Full));
        assert_eq!(placed.get("Joy/cover.png"), None, "a page file");

        let files = |cap| -> Vec<Vec<String>> {
            within(&entries, &placed, cap)
                .into_iter()
                .map(|e| e.files)
                .collect()
        };
        assert_eq!(
            files(Listen::Page),
            [vec!["Joy/cover.png"], vec!["Joy/cover.png"]]
        );
        assert_eq!(
            files(Listen::Reference),
            [
                vec![
                    "Joy/Joy.session",
                    "Joy/Media/Reference.ogg",
                    "Joy/cover.png"
                ],
                vec!["Joy/cover.png"]
            ]
        );
        assert_eq!(files(Listen::Full).concat().len(), 6);
    }

    #[test]
    fn no_rules_place_nothing() {
        let entries = [entry("Joy/Joy.md", Some("song"), &["Joy/Media/Bass.ogg"])];
        let rules = LayerRules::default();
        assert!(
            LayerRules::place(&rules.current(), &entries)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            rules.current().generation,
            rules.current().generation,
            "read once"
        );
    }

    #[test]
    fn an_installed_extensions_rules_are_read_and_read_again() {
        let tmp = tempfile::tempdir().unwrap();
        let rules = LayerRules::new(Some(tmp.path().to_owned()), Vec::new(), None);
        let entries = [entry("Joy/Joy.md", Some("song"), &["Joy/Media/Bass.ogg"])];
        assert!(
            LayerRules::place(&rules.current(), &entries)
                .unwrap()
                .is_empty()
        );
        let pkg = tmp.path().join("fts.studio");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("extension.toml"),
            "id = \"fts.studio\"\nname = \"Studio\"\nversion = \"0.1.0\"\nkind = \"data\"\n\n\
             [[share_layers]]\nnote_type = \"song\"\nfull = [\"Media/**\"]\n",
        )
        .unwrap();
        assert_eq!(
            LayerRules::place(&rules.current(), &entries)
                .unwrap()
                .get("Joy/Media/Bass.ogg"),
            Some(&Listen::Full)
        );
    }

    fn install(dir: &Path, id: &str, rules: &str) {
        let pkg = dir.join(id);
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("extension.toml"),
            format!("id = \"{id}\"\nname = \"X\"\nversion = \"0.1.0\"\nkind = \"data\"\n\n{rules}"),
        )
        .unwrap();
    }

    const OWNER: &str = "[[share_layers]]\nnote_type = \"song\"\nfull = [\"Media/**\"]\n";

    #[test]
    fn no_rule_lowers_another() {
        let tmp = tempfile::tempdir().unwrap();
        install(tmp.path(), "demo.songs", OWNER);
        install(
            tmp.path(),
            "aaa.other",
            "[[share_layers]]\nnote_type = \"Song\"\npage = [\"**\"]\n",
        );
        let rules = LayerRules::new(Some(tmp.path().to_owned()), Vec::new(), None);
        let entries = [entry("Joy/Joy.md", Some("song"), &["Joy/Media/Bass.ogg"])];
        let placed = LayerRules::place(&rules.current(), &entries).unwrap();
        assert_eq!(placed.get("Joy/Media/Bass.ogg"), Some(&Listen::Full));
    }

    #[test]
    fn a_manifest_that_does_not_read_makes_placement_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        install(tmp.path(), "demo.songs", OWNER);
        let rules = LayerRules::new(Some(tmp.path().to_owned()), Vec::new(), None);
        let entries = [entry("Joy/Joy.md", Some("song"), &["Joy/Media/Bass.ogg"])];
        assert!(LayerRules::place(&rules.current(), &entries).is_some());
        // Half-written.
        std::fs::create_dir_all(tmp.path().join("other")).unwrap();
        std::fs::write(tmp.path().join("other/extension.toml"), "id = ").unwrap();
        let current = rules.current();
        assert_eq!(current.broken.len(), 1, "{:?}", current.broken);
        assert!(current.broken[0].contains("other/extension.toml"));
        assert!(LayerRules::place(&current, &entries).is_none());
        assert!(!current.is_empty());
    }

    #[test]
    fn rules_are_kept_after_an_uninstall_until_forgotten() {
        let tmp = tempfile::tempdir().unwrap();
        let (dir, kept_at) = (
            tmp.path().join("extensions"),
            tmp.path().join("share-layers.json"),
        );
        install(&dir, "demo.songs", OWNER);
        let entries = [entry("Joy/Joy.md", Some("song"), &["Joy/Media/Bass.ogg"])];
        let rules = LayerRules::new(Some(dir.clone()), Vec::new(), Some(kept_at.clone()));
        let first = rules.current().generation;
        assert!(rules.kept().is_empty(), "installed: its own");
        assert!(rules.forget("demo.songs").is_err());
        std::fs::remove_dir_all(dir.join("demo.songs")).unwrap();
        let after = rules.current();
        assert!(after.generation > first);
        assert_eq!(
            LayerRules::place(&after, &entries)
                .unwrap()
                .get("Joy/Media/Bass.ogg"),
            Some(&Listen::Full)
        );
        assert_eq!(rules.kept()[0].extension, "demo.songs");
        // A restarted node keeps them too.
        let again = LayerRules::new(Some(dir.clone()), Vec::new(), Some(kept_at.clone()));
        assert_eq!(again.kept().len(), 1);
        assert!(again.forget("demo.unknown").is_err());
        assert!(again.forget("demo.songs").unwrap().is_empty());
        assert!(
            LayerRules::place(&again.current(), &entries)
                .unwrap()
                .is_empty()
        );
        // A kept file that does not read: unknown, not empty.
        std::fs::write(&kept_at, "{ not json").unwrap();
        let corrupt = LayerRules::new(Some(dir), Vec::new(), Some(kept_at));
        assert!(LayerRules::place(&corrupt.current(), &entries).is_none());
    }
}
