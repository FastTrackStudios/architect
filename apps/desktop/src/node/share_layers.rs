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
use architect_spaces::{KeptLayers, LayersReport, Listen};

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
    /// What is kept, as in the kept file.
    file: KeptFile,
    /// The kept file did not read: it was set aside, and nothing is
    /// written over it or placed until the owner acknowledges it.
    fault: Option<String>,
    current: Arc<Current>,
}

/// The kept file (`share-layers.json`).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct KeptFile {
    /// Each extension's last declared rules, by its folder.
    kept: BTreeMap<String, Kept>,
    /// What a reinstall lowered, until acknowledged.
    #[serde(default)]
    lowered: Vec<String>,
}

/// One extension's last declared rules, as kept.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Kept {
    /// The id its manifest claimed.
    id: String,
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
    /// Why where files are is not known now: a manifest that does not
    /// read, two folders claiming one id, the kept file set aside. While
    /// there is one, every attached file is held back below Full.
    pub broken: Vec<String>,
    /// The installed extensions' folders and ids.
    installed: BTreeSet<String>,
    /// What the owner should know, until acknowledged.
    notices: Vec<String>,
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

/// The marker a kept file that did not read is set aside as.
const CORRUPT: &str = ".corrupt-";
/// What an acknowledged marker ends with.
const ACKNOWLEDGED: &str = ".acknowledged";

/// The kept file's markers set aside and not acknowledged.
fn faults(kept_at: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (kept_at.parent(), kept_at.file_name()) else {
        return Vec::new();
    };
    let prefix = format!("{}{CORRUPT}", name.to_string_lossy());
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with(&prefix) && !n.ends_with(ACKNOWLEDGED)
        })
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

/// What `old` declared that `new` does not, in the layers above the page:
/// each a line (`song: Media/Proxies/** no longer full`).
fn lowered(old: &[ShareLayersDecl], new: &[ShareLayersDecl]) -> Vec<String> {
    let items = |rules: &[ShareLayersDecl]| -> BTreeSet<(String, &'static str, String)> {
        rules
            .iter()
            .flat_map(|r| {
                let kind = r.note_type.trim().to_lowercase();
                [("reference", &r.reference), ("full", &r.full)]
                    .into_iter()
                    .flat_map(move |(layer, patterns)| {
                        let kind = kind.clone();
                        patterns
                            .iter()
                            .map(move |p| (kind.clone(), layer, p.clone()))
                    })
            })
            .collect()
    };
    let now = items(new);
    items(old)
        .into_iter()
        .filter(|item| !now.contains(item))
        .map(|(kind, layer, pattern)| format!("{kind}: `{pattern}` no longer {layer}"))
        .collect()
}

impl LayerRules {
    /// The rules of the extensions installed in `dir`, kept at `kept_at`
    /// once read, and `given` (a distribution's compiled-in ones).
    ///
    /// A kept file that does not read is set aside
    /// (`share-layers.json.corrupt-<time>`), warned of once, and every
    /// attached file is held back below Full until the owner acknowledges
    /// it ([`Self::acknowledge`]), across restarts: nothing is written over
    /// it meanwhile.
    #[must_use]
    pub fn new(
        dir: Option<PathBuf>,
        given: Vec<ShareLayersDecl>,
        kept_at: Option<PathBuf>,
    ) -> Self {
        let mut state = State::default();
        if let Some(path) = &kept_at {
            match std::fs::read_to_string(path) {
                Ok(text) => match serde_json::from_str::<KeptFile>(&text) {
                    Ok(file) => state.file = file,
                    Err(error) => {
                        let aside = path.with_file_name(format!(
                            "{}{CORRUPT}{}",
                            path.file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                            unix_now()
                        ));
                        let moved = std::fs::rename(path, &aside);
                        tracing::warn!(
                            path = %path.display(),
                            aside = %aside.display(),
                            moved = moved.is_ok(),
                            %error,
                            "the kept share layer rules do not read: set aside; share links below Full hold back every attached file until the owner acknowledges it (SHARE_LAYERS_ACKNOWLEDGE)"
                        );
                        if moved.is_err() {
                            state.fault = Some(format!("{}: {error}", path.display()));
                        }
                    }
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => state.fault = Some(format!("{}: {error}", path.display())),
            }
            if let Some(marker) = faults(path).into_iter().next() {
                state.fault = Some(format!(
                    "the kept share layer rules did not read and were set aside as {}",
                    marker.display()
                ));
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
    /// changed, each extension's declared rules kept by its folder.
    #[must_use]
    pub fn current(&self) -> Arc<Current> {
        let read = self.0.installed.as_ref().map(InstalledLayers::read);
        let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        let now = read
            .as_ref()
            .map_or(Read::NoFolder, |r| Read::At(r.generation));
        if state.read != now {
            state.read = now;
            let packages = read.iter().flat_map(|r| r.packages.iter());
            // One id, one folder: two folders claiming one id is a typo
            // or a shadow, and neither is believed.
            let mut by_id: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
            for p in packages.clone() {
                by_id
                    .entry(p.id.as_str())
                    .or_default()
                    .push(p.folder.as_str());
            }
            let mut broken: Vec<String> = read
                .iter()
                .flat_map(|r| r.broken.iter())
                .map(|(path, why)| format!("{}: {why}", path.display()))
                .collect();
            for (id, folders) in by_id.iter().filter(|(_, f)| f.len() > 1) {
                broken.push(format!(
                    "{} extensions claim the id {id}: {}",
                    folders.len(),
                    folders.join(", ")
                ));
            }
            let mut changed = false;
            let mut installed = BTreeSet::new();
            for p in packages {
                installed.insert(p.folder.clone());
                installed.insert(p.id.clone());
                if by_id.get(p.id.as_str()).is_some_and(|f| f.len() > 1) {
                    continue;
                }
                let before = state.file.kept.get(&p.folder).cloned();
                if before
                    .as_ref()
                    .is_some_and(|k| k.id == p.id && k.rules == p.rules)
                {
                    continue;
                }
                if let Some(before) = before.filter(|k| k.id == p.id) {
                    let less = lowered(&before.rules, &p.rules);
                    if !less.is_empty() {
                        tracing::warn!(
                            folder = %p.folder,
                            id = %p.id,
                            lowered = ?less,
                            "an extension now declares fewer share layer rules: these files may open on links that play lower"
                        );
                        state.file.lowered.extend(
                            less.into_iter()
                                .map(|l| format!("{} ({}): {l}", p.id, p.folder)),
                        );
                    }
                }
                // Exactly what it declares now, and kept for when it is
                // gone.
                if p.rules.is_empty() {
                    state.file.kept.remove(&p.folder);
                } else {
                    state.file.kept.insert(
                        p.folder.clone(),
                        Kept {
                            id: p.id.clone(),
                            rules: p.rules.clone(),
                            since: unix_now(),
                        },
                    );
                }
                changed = true;
            }
            if changed {
                self.save(&mut state);
            }
            broken.extend(state.fault.clone());
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
            .chain(
                state
                    .file
                    .kept
                    .values()
                    .flat_map(|k| k.rules.iter().cloned()),
            )
            .collect();
        let notices = broken
            .iter()
            .cloned()
            .chain(state.file.lowered.iter().map(|l| format!("lowered: {l}")))
            .collect();
        state.current = Arc::new(Current {
            generation: state.current.generation.wrapping_add(1),
            rules,
            broken,
            installed,
            notices,
        });
    }

    /// Write the kept file, never while one is set aside unacknowledged.
    ///
    /// A write that fails is a fault like a file that does not read:
    /// every attached file is held back below Full, the status says so,
    /// and a marker with what could not be written is left beside it when
    /// the folder takes one, so the fault holds across a restart until
    /// acknowledged.
    fn save(&self, state: &mut State) {
        let Some(path) = &self.0.kept_at else { return };
        if state.fault.is_some() {
            return;
        }
        let bytes = serde_json::to_vec_pretty(&state.file).map_err(std::io::Error::other);
        let written = bytes
            .as_ref()
            .map_err(|e| std::io::Error::other(e.to_string()))
            .and_then(|bytes| {
                let tmp = path.with_extension("json.tmp");
                std::fs::write(&tmp, bytes)?;
                std::fs::rename(&tmp, path)
            });
        if let Err(error) = written {
            let marker = path.with_file_name(format!(
                "{}{CORRUPT}{}-unsaved",
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                unix_now()
            ));
            let left_beside = bytes
                .map_err(|e| std::io::Error::other(e.to_string()))
                .and_then(|bytes| std::fs::write(&marker, bytes));
            tracing::warn!(
                path = %path.display(),
                marker = %marker.display(),
                marked = left_beside.is_ok(),
                %error,
                "the share layer rules could not be kept: share links below Full hold back every attached file until the owner acknowledges it (SHARE_LAYERS_ACKNOWLEDGE)"
            );
            state.fault = Some(format!(
                "the kept share layer rules could not be written ({}): {error}",
                path.display()
            ));
        }
    }

    /// The rules kept for extensions no longer installed, and the notices.
    #[must_use]
    pub fn report(&self) -> LayersReport {
        let current = self.current();
        let state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        let kept = state
            .file
            .kept
            .iter()
            .filter(|(folder, _)| !current.installed.contains(*folder))
            .map(|(folder, kept)| {
                let types: BTreeSet<String> = kept
                    .rules
                    .iter()
                    .map(|r| r.note_type.trim().to_lowercase())
                    .collect();
                KeptLayers {
                    extension: kept.id.clone(),
                    folder: folder.clone(),
                    note_types: types.into_iter().collect(),
                    since: kept.since,
                }
            })
            .collect();
        drop(state);
        LayersReport {
            kept,
            notices: current.notices.clone(),
        }
    }

    /// What the node's status says of its layer rules: one line per
    /// kind of notice, with nothing of the vault or the extensions.
    #[must_use]
    pub fn status(&self) -> Vec<String> {
        let current = self.current();
        let (fault, lowered) = {
            let state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            (state.fault.is_some(), state.file.lowered.len())
        };
        let mut out = Vec::new();
        if fault {
            out.push("share layers: the kept rules could not be read or written; links below Full hold every attached file back until acknowledged (SHARE_LAYERS_ACKNOWLEDGE)".to_owned());
        }
        let manifests = current.broken.len().saturating_sub(usize::from(fault));
        if manifests > 0 {
            out.push(format!("share layers: {manifests} extension manifest problem(s); links below Full hold every attached file back until fixed (SHARE_LAYERS_KEPT)"));
        }
        if lowered > 0 {
            out.push(format!("share layers: a reinstall lowered {lowered} rule(s) (SHARE_LAYERS_KEPT, SHARE_LAYERS_ACKNOWLEDGE)"));
        }
        out
    }

    /// Forget the kept rules of `extension` (its id or its folder), no
    /// longer installed; what is kept after.
    ///
    /// # Errors
    ///
    /// It is installed (its rules are its manifest's), or nothing is kept
    /// of it.
    pub fn forget(&self, extension: &str) -> Result<LayersReport, String> {
        let current = self.current();
        if current.installed.contains(extension) {
            return Err(format!(
                "{extension} is installed: its rules are its manifest's; uninstall it first"
            ));
        }
        {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            let before = state.file.kept.len();
            state
                .file
                .kept
                .retain(|folder, k| folder != extension && k.id != extension);
            if state.file.kept.len() == before {
                return Err(format!("no layer rules are kept for {extension}"));
            }
            self.save(&mut state);
            // Read the folder again on the next look (with any fault).
            state.read = Read::Not;
            drop(state);
        }
        Ok(self.report())
    }

    /// Acknowledge the notices: a kept file set aside (marked
    /// acknowledged; what it held is not read again) and what a reinstall
    /// lowered. Files are placed again by the rules there are now.
    pub fn acknowledge(&self) -> LayersReport {
        {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(path) = &self.0.kept_at {
                for marker in faults(path) {
                    let mut done = marker.clone().into_os_string();
                    done.push(ACKNOWLEDGED);
                    if let Err(error) = std::fs::rename(&marker, &done) {
                        tracing::warn!(path = %marker.display(), %error, "a set-aside kept rules file could not be marked acknowledged");
                    }
                }
                if faults(path).is_empty() {
                    state.fault = None;
                }
            } else {
                state.fault = None;
            }
            state.file.lowered.clear();
            self.save(&mut state);
            // Read the folder again on the next look.
            state.read = Read::Not;
        }
        self.report()
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
    use std::fmt::Write as _;

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

    /// A `[[favourites]]` declaration that breaks a rule (a label too
    /// long, six tabs) is dropped, never the manifest: its share layers
    /// still place files, so no link holds back everything for it.
    #[test]
    fn a_bad_favourites_declaration_never_makes_placement_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        let mut tabs = String::new();
        for i in 0..6 {
            let _ = write!(
                tabs,
                "[[favourites.tabs]]\nid = \"t{i}\"\nlabel = \"Tab {i}\"\nicon = \"music\"\ntarget = {{ view = \"page\" }}\n"
            );
        }
        install(
            tmp.path(),
            "demo.songs",
            &format!("{OWNER}\n[[favourites]]\nnote_type = \"song\"\n{tabs}"),
        );
        let rules = LayerRules::new(Some(tmp.path().to_owned()), Vec::new(), None);
        let current = rules.current();
        assert!(current.broken.is_empty(), "{:?}", current.broken);
        let entries = [entry("Joy/Joy.md", Some("song"), &["Joy/Media/Bass.ogg"])];
        assert_eq!(
            LayerRules::place(&current, &entries)
                .unwrap()
                .get("Joy/Media/Bass.ogg"),
            Some(&Listen::Full)
        );
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
        assert!(rules.report().kept.is_empty(), "installed: its own");
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
        assert_eq!(rules.report().kept[0].extension, "demo.songs");
        assert_eq!(rules.report().kept[0].folder, "demo.songs");
        // A restarted node keeps them too.
        let again = LayerRules::new(Some(dir.clone()), Vec::new(), Some(kept_at.clone()));
        assert_eq!(again.report().kept.len(), 1);
        assert!(again.forget("demo.unknown").is_err());
        assert!(again.forget("demo.songs").unwrap().kept.is_empty());
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

    /// A kept file that does not read holds everything back, and goes on
    /// doing so after a restart: it is never written over (#540's second
    /// review: the first `current()` used to save the installed rules
    /// alone over it, and after a restart an uninstalled extension's
    /// stems opened).
    #[test]
    fn a_corrupt_kept_file_holds_back_across_restarts() {
        let tmp = tempfile::tempdir().unwrap();
        let (dir, kept_at) = (
            tmp.path().join("extensions"),
            tmp.path().join("share-layers.json"),
        );
        install(
            &dir,
            "demo.songs",
            "[[share_layers]]\nnote_type = \"song\"\nfull = [\"**\"]\n",
        );
        install(
            &dir,
            "demo.other",
            "[[share_layers]]\nnote_type = \"live\"\nfull = [\"**\"]\n",
        );
        let entries = [entry("Joy/Joy.md", Some("song"), &["Joy/Media/Bass.ogg"])];
        let first = LayerRules::new(Some(dir.clone()), Vec::new(), Some(kept_at.clone()));
        let _ = first.current();
        std::fs::remove_dir_all(dir.join("demo.songs")).unwrap();
        assert_eq!(
            LayerRules::place(&first.current(), &entries)
                .unwrap()
                .get("Joy/Media/Bass.ogg"),
            Some(&Listen::Full),
            "kept after the uninstall"
        );
        std::fs::write(&kept_at, "{ half").unwrap();
        for restart in 0..3 {
            let rules = LayerRules::new(Some(dir.clone()), Vec::new(), Some(kept_at.clone()));
            assert!(
                LayerRules::place(&rules.current(), &entries).is_none(),
                "restart {restart}: the stem must stay held back"
            );
            assert!(!rules.status().is_empty(), "the node's status says so");
            assert!(
                rules
                    .report()
                    .notices
                    .iter()
                    .any(|n| n.contains("set aside"))
            );
        }
        // Set aside, never written over.
        let aside: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("share-layers.json.corrupt-"))
            .collect();
        assert_eq!(aside.len(), 1, "{aside:?}");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join(&aside[0])).unwrap(),
            "{ half"
        );
        // The owner acknowledges: placed by the rules there are now (the
        // uninstalled one's were in the file that did not read).
        let rules = LayerRules::new(Some(dir.clone()), Vec::new(), Some(kept_at.clone()));
        let report = rules.acknowledge();
        assert!(report.notices.is_empty(), "{:?}", report.notices);
        assert!(rules.status().is_empty());
        assert!(LayerRules::place(&rules.current(), &entries).is_some());
        let restarted = LayerRules::new(Some(dir), Vec::new(), Some(kept_at));
        assert!(LayerRules::place(&restarted.current(), &entries).is_some());
    }

    #[test]
    fn a_reinstall_that_declares_less_says_what_it_lowered() {
        let tmp = tempfile::tempdir().unwrap();
        let (dir, kept_at) = (
            tmp.path().join("extensions"),
            tmp.path().join("share-layers.json"),
        );
        install(
            &dir,
            "demo.songs",
            "[[share_layers]]\nnote_type = \"song\"\nreference = [\"*.session\"]\nfull = [\"Media/**\"]\n",
        );
        let rules = LayerRules::new(Some(dir.clone()), Vec::new(), Some(kept_at.clone()));
        assert!(rules.status().is_empty());
        install(
            &dir,
            "demo.songs",
            "[[share_layers]]\nnote_type = \"song\"\nreference = [\"*.session\"]\n",
        );
        let report = rules.report();
        assert_eq!(report.notices.len(), 1, "{:?}", report.notices);
        assert!(
            report.notices[0].contains("`Media/**` no longer full"),
            "{:?}",
            report.notices
        );
        assert_eq!(rules.status().len(), 1);
        // Kept until acknowledged, across a restart.
        let restarted = LayerRules::new(Some(dir), Vec::new(), Some(kept_at));
        assert_eq!(restarted.report().notices.len(), 1);
        assert!(restarted.acknowledge().notices.is_empty());
    }

    #[test]
    fn two_folders_claiming_one_id_hold_everything_back() {
        let tmp = tempfile::tempdir().unwrap();
        install(&tmp.path(), "demo.songs", OWNER);
        let rules = LayerRules::new(Some(tmp.path().to_owned()), Vec::new(), None);
        let entries = [entry("Joy/Joy.md", Some("song"), &["Joy/Media/Bass.ogg"])];
        assert!(LayerRules::place(&rules.current(), &entries).is_some());
        // Another folder, claiming the owner's id, with no rules.
        let shadow = tmp.path().join("zzz.shadow");
        std::fs::create_dir_all(&shadow).unwrap();
        std::fs::write(
            shadow.join("extension.toml"),
            "id = \"demo.songs\"\nname = \"X\"\nversion = \"0.1.0\"\nkind = \"data\"\n",
        )
        .unwrap();
        assert!(LayerRules::place(&rules.current(), &entries).is_none());
    }

    #[test]
    fn a_misspelled_layer_holds_everything_back() {
        let tmp = tempfile::tempdir().unwrap();
        install(
            &tmp.path(),
            "demo.songs",
            "[[share_layers]]\nnote_type = \"song\"\nfulll = [\"Media/**\"]\n",
        );
        let rules = LayerRules::new(Some(tmp.path().to_owned()), Vec::new(), None);
        let entries = [entry("Joy/Joy.md", Some("song"), &["Joy/Media/Bass.ogg"])];
        assert!(LayerRules::place(&rules.current(), &entries).is_none());
    }

    /// A kept file that cannot be written is a fault like one that does
    /// not read: everything held back, and the status says so.
    #[cfg(unix)]
    #[test]
    fn a_kept_file_that_cannot_be_written_holds_everything_back() {
        use std::os::unix::fs::PermissionsExt as _;
        let tmp = tempfile::tempdir().unwrap();
        let (dir, data) = (tmp.path().join("extensions"), tmp.path().join("data"));
        std::fs::create_dir_all(&data).unwrap();
        let rules = LayerRules::new(
            Some(dir.clone()),
            Vec::new(),
            Some(data.join("share-layers.json")),
        );
        let entries = [entry("Joy/Joy.md", Some("song"), &["Joy/Media/Bass.ogg"])];
        assert!(LayerRules::place(&rules.current(), &entries).is_some());
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o555)).unwrap();
        install(&dir, "demo.songs", OWNER);
        let current = rules.current();
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(LayerRules::place(&current, &entries).is_none());
        assert!(!rules.status().is_empty());
    }
}
