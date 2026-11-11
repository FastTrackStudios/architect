//! Which layer of a share link each attached file is in (#528): the
//! `[[share_layers]]` rules of the extensions installed on the node and
//! of a distribution's compiled-in ones
//! (`architect_extensions::share_layers`).
//!
//! The node holds no extension running; it reads their manifests
//! (`architect_sites::InstalledLayers`, read again when the folder
//! changes).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use architect_sites::listing::Entry;
use architect_sites::{InstalledLayers, ShareLayersDecl};
use architect_spaces::Listen;

/// The rules a node places files by. Cheap to clone.
#[derive(Clone, Default)]
pub struct LayerRules(Arc<Inner>);

#[derive(Default)]
struct Inner {
    /// The installed extensions'; None: none read.
    installed: Option<InstalledLayers>,
    /// A distribution's compiled-in extensions' rules, after the
    /// installed ones.
    given: Vec<ShareLayersDecl>,
}

impl LayerRules {
    /// The rules of the extensions installed in `dir`, then `given`.
    #[must_use]
    pub fn new(dir: Option<PathBuf>, given: Vec<ShareLayersDecl>) -> Self {
        Self(Arc::new(Inner {
            installed: dir.map(InstalledLayers::new),
            given,
        }))
    }

    /// Whether no extension gives a rule, installed or compiled in.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.given.is_empty()
            && self
                .0
                .installed
                .as_ref()
                .is_none_or(|i| i.rules().is_empty())
    }

    /// The layer of each file `entries` attach that is above the page:
    /// the highest any note attaching it places it in, so a file a rule
    /// keeps for Full is not served lower because another note links it
    /// too. An installed extension's rule for a note type is asked
    /// before a compiled-in one's, so its development copy says the last
    /// word.
    #[must_use]
    pub fn place(&self, entries: &[Entry]) -> HashMap<String, Listen> {
        let installed = self
            .0
            .installed
            .as_ref()
            .map(InstalledLayers::rules)
            .unwrap_or_default();
        let mut out: HashMap<String, Listen> = HashMap::new();
        if installed.is_empty() && self.0.given.is_empty() {
            return out;
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
            let Some(rule) = installed
                .iter()
                .chain(&self.0.given)
                .find(|r| kinds.iter().any(|k| r.is_for(k)))
            else {
                continue;
            };
            for file in &entry.files {
                let Some(layer) = rule
                    .layer_of(&entry.path, file)
                    .and_then(|l| Listen::parse(l).ok())
                    .filter(|l| *l > Listen::Page)
                else {
                    continue;
                };
                let held = out.entry(file.clone()).or_insert(layer);
                *held = (*held).max(layer);
            }
        }
        out
    }
}

/// `entries` with every file above `cap` left out.
#[must_use]
pub fn within(entries: &[Entry], placed: &HashMap<String, Listen>, cap: Listen) -> Vec<Entry> {
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
        let rules = LayerRules::new(None, vec![song()]);
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
        let placed = rules.place(&entries);
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
        assert!(LayerRules::default().place(&entries).is_empty());
    }

    #[test]
    fn an_installed_extensions_rules_are_read_and_read_again() {
        let tmp = tempfile::tempdir().unwrap();
        let rules = LayerRules::new(Some(tmp.path().to_owned()), Vec::new());
        let entries = [entry("Joy/Joy.md", Some("song"), &["Joy/Media/Bass.ogg"])];
        assert!(rules.place(&entries).is_empty());
        let pkg = tmp.path().join("fts.studio");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("extension.toml"),
            "id = \"fts.studio\"\nname = \"Studio\"\nversion = \"0.1.0\"\nkind = \"data\"\n\n\
             [[share_layers]]\nnote_type = \"song\"\nfull = [\"Media/**\"]\n",
        )
        .unwrap();
        assert_eq!(
            rules.place(&entries).get("Joy/Media/Bass.ogg"),
            Some(&Listen::Full)
        );
    }
}
