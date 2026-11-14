//! The Architect app's vault: the folder of markdown it opens, chosen on
//! its first run ([`crate::first_run`]) and remembered in its folder
//! (`<app dir>/vaults.json`), so the next start opens it again.
//!
//! The first run offers three: a folder the person has, a new vault
//! ([`create`], `~/Documents/Architect`), or the demo vault ([`demo`]: the
//! made-up vault in `apps/ui/demo-vault`, built into the app and written
//! out to `<app dir>/Demo vault` to explore and edit).

use std::path::{Path, PathBuf};

/// Where the chosen vault is kept, in the app's folder.
const KEPT: &str = "vaults.json";

/// The demo vault, as built into the app (`apps/ui/demo-vault.py` makes it).
static DEMO: include_dir::Dir<'static> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/demo-vault");

/// The vault the app opened last, while its folder is there.
#[must_use]
pub fn remembered() -> Option<PathBuf> {
    let text = std::fs::read_to_string(crate::app_dir()?.join(KEPT)).ok()?;
    let kept: serde_json::Value = serde_json::from_str(&text).ok()?;
    let dir = PathBuf::from(kept.get("current")?.as_str()?);
    dir.is_dir().then_some(dir)
}

/// Remembers `dir` as the vault to open from now on.
///
/// # Errors
///
/// The app has no folder, or the file cannot be written.
pub fn remember(dir: &Path) -> std::io::Result<()> {
    let app = crate::app_dir().ok_or_else(|| std::io::Error::other("the app has no folder"))?;
    std::fs::create_dir_all(&app)?;
    let kept = serde_json::json!({ "current": dir.display().to_string() });
    std::fs::write(app.join(KEPT), kept.to_string())
}

/// One org's vault on this device (org = vault, #443): what the
/// navigator's Vaults/ shows beside the app's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgVault {
    /// The org's id (`codywright`): its mount is `@<org>`.
    pub org: String,
    /// The org's name, its badge's tooltip.
    pub name: String,
    /// Its badge's colour.
    pub colour: String,
    /// Its own icon (a Lucide name or an emoji), else its initial.
    pub icon: Option<String>,
    /// The vault's folder.
    pub dir: PathBuf,
}

impl OrgVault {
    /// Its badge in the navigator.
    #[must_use]
    pub fn badge(&self) -> architect_navigator::OrgBadge {
        let mut b = architect_navigator::OrgBadge::new(&self.org, &self.name, &self.colour);
        b.icon.clone_from(&self.icon);
        b
    }
}

/// The org vaults this device holds (#443).
///
/// `orgs` in `vaults.json`, and a run's own in the JSON file
/// `ARCHITECT_ORG_VAULTS` names (a test session's scratch vaults), as
/// `[{org, name, colour, icon, dir}]`. One of them may be the app's vault
/// ([`app_vault_org`]); the rest open beside it. A folder that is not
/// there is left out.
#[must_use]
pub fn catalog() -> Vec<OrgVault> {
    let mut texts = Vec::new();
    if let Some(app) = crate::app_dir()
        && let Ok(t) = std::fs::read_to_string(app.join(KEPT))
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&t)
        && let Some(orgs) = v.get("orgs")
    {
        texts.push(orgs.clone());
    }
    if let Some(path) = std::env::var_os("ARCHITECT_ORG_VAULTS")
        && let Ok(t) = std::fs::read_to_string(path)
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&t)
    {
        texts.push(v);
    }
    let mut out: Vec<OrgVault> = Vec::new();
    for list in texts {
        for e in list.as_array().into_iter().flatten() {
            let Some(v) = parse_entry(e) else { continue };
            if v.dir.is_dir() && !out.iter().any(|o| o.org == v.org) {
                out.push(v);
            }
        }
    }
    out
}

/// One `[{org, name, colour, icon, dir}]` entry.
fn parse_entry(e: &serde_json::Value) -> Option<OrgVault> {
    let text = |k: &str| {
        e.get(k)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let org = text("org").filter(|o| !o.trim().is_empty())?;
    Some(OrgVault {
        name: text("name").unwrap_or_else(|| org.clone()),
        colour: text("colour")
            .or_else(|| text("color"))
            .unwrap_or_else(|| "#3f3f46".to_owned()),
        icon: text("icon"),
        dir: PathBuf::from(text("dir")?),
        org,
    })
}

/// The org the app's own vault (at `dir`) is: the catalog's entry for its
/// folder, else the person's own org on this device.
#[must_use]
pub fn app_vault_org(catalog: &[OrgVault], dir: Option<&Path>) -> OrgVault {
    let same = |a: &Path, b: &Path| {
        a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
    };
    if let Some(dir) = dir
        && let Some(v) = catalog.iter().find(|v| same(&v.dir, dir))
    {
        return v.clone();
    }
    let me = architect_accounts::device_org();
    OrgVault {
        org: me.id.0,
        name: me.name,
        colour: me.color,
        icon: None,
        dir: dir.map(Path::to_path_buf).unwrap_or_default(),
    }
}

/// The demo vault, written out to `<app dir>/Demo vault` when it is not
/// there yet (an edit made to it stays; delete the folder for a fresh one).
///
/// # Errors
///
/// The app has no folder, or a file cannot be written.
pub fn demo() -> std::io::Result<PathBuf> {
    let app = crate::app_dir().ok_or_else(|| std::io::Error::other("the app has no folder"))?;
    let dir = app.join("Demo vault");
    write_demo(&dir)?;
    Ok(dir)
}

/// Writes the built-in demo vault into `dir` when `dir` does not exist,
/// with the empty folders its `.empty-folders` lists (git keeps none).
///
/// # Errors
///
/// A file cannot be written.
pub fn write_demo(dir: &Path) -> std::io::Result<bool> {
    if dir.exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(dir)?;
    DEMO.extract(dir)?;
    write_demo_recordings(dir, &DEMO)?;
    let empty = dir.join(crate::notes::EMPTY_FOLDERS);
    if let Ok(list) = std::fs::read_to_string(&empty) {
        for line in list.lines().map(str::trim).filter(|l| !l.is_empty()) {
            std::fs::create_dir_all(dir.join(line))?;
        }
        std::fs::remove_file(empty)?;
    }
    Ok(true)
}

/// The demo's silent recording (`apps/web/app/demo-recording.mp3`): the
/// web demo plays it for every song's recording, and the demo vault on
/// disk has it at each (#668), so its setlist plays the same way.
const SILENT: &[u8] = include_bytes!("../../web/app/demo-recording.mp3");

/// The silent recording at every `recording:`/`media:` an `.md` note of
/// `demo` names (a vault path ending `.mp3`), under `dir`.
fn write_demo_recordings(dir: &Path, demo: &include_dir::Dir<'_>) -> std::io::Result<()> {
    for entry in demo.entries() {
        match entry {
            include_dir::DirEntry::Dir(d) => write_demo_recordings(dir, d)?,
            include_dir::DirEntry::File(f) => {
                let Some(text) = f.contents_utf8() else {
                    continue;
                };
                if f.path().extension().is_none_or(|e| e != "md") {
                    continue;
                }
                let path = f.path().to_string_lossy();
                let item =
                    architect_media::Item::read(&path, &architect_media::front::fields(text));
                let Some(media) = item.media.filter(|m| {
                    architect_playhead::model::vault_path(m)
                        && Path::new(m)
                            .extension()
                            .is_some_and(|e| e.eq_ignore_ascii_case("mp3"))
                }) else {
                    continue;
                };
                let to = dir.join(&media);
                if !to.exists() {
                    if let Some(parent) = to.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(to, SILENT)?;
                }
            }
        }
    }
    Ok(())
}

/// A new, empty vault: `~/Documents/Architect` (`Architect 2`, … when
/// that is taken; the home folder when there is no Documents), with a
/// welcome note.
///
/// # Errors
///
/// There is no home folder, or the folder cannot be made.
pub fn create() -> std::io::Result<PathBuf> {
    let home = std::env::home_dir().ok_or_else(|| std::io::Error::other("no home folder"))?;
    let documents = home.join("Documents");
    let parent = if documents.is_dir() { documents } else { home };
    create_in(&parent)
}

/// [`create`] under `parent`.
///
/// # Errors
///
/// The folder cannot be made.
pub fn create_in(parent: &Path) -> std::io::Result<PathBuf> {
    let dir = (1..=999)
        .map(|n: u32| {
            parent.join(if n == 1 {
                "Architect".to_owned()
            } else {
                format!("Architect {n}")
            })
        })
        .find(|d| !d.exists())
        .ok_or_else(|| std::io::Error::other("no free folder name"))?;
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("Welcome.md"), WELCOME)?;
    Ok(dir)
}

/// A new vault's first note.
const WELCOME: &str = "# Welcome\n\nThis is your vault: a folder of markdown files on this device. \
Every note is a file you can open in any editor, and Architect reads the folder as it is.\n\n\
Start writing here, or make a new note from the file list.\n";

#[cfg(test)]
mod tests {
    #[test]
    fn the_built_in_demo_vault_is_written_with_its_empty_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("Demo vault");
        assert!(super::write_demo(&dir).unwrap());
        assert!(dir.join("Welcome.md").is_file());
        assert!(dir.join("Archive/Empty").is_dir(), "an empty folder");
        assert!(!dir.join(crate::notes::EMPTY_FOLDERS).exists());
        assert!(!super::write_demo(&dir).unwrap(), "an edited copy is kept");
    }

    #[test]
    fn the_catalog_reads_each_org_vault_and_names_the_apps_own() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let list = serde_json::json!([
            {"org": "codywright", "name": "Cody Wright", "colour": "#047857", "dir": a},
            {"org": "fasttrackstudio", "name": "FastTrackStudio", "icon": "music", "dir": b},
            {"org": "gone", "dir": tmp.path().join("nowhere")},
        ]);
        let file = tmp.path().join("orgs.json");
        std::fs::write(&file, list.to_string()).unwrap();
        let entries: Vec<super::OrgVault> = list
            .as_array()
            .unwrap()
            .iter()
            .filter_map(super::parse_entry)
            .filter(|v| v.dir.is_dir())
            .collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].colour, "#3f3f46");
        assert_eq!(entries[1].badge().icon.as_deref(), Some("music"));
        let own = super::app_vault_org(&entries, Some(&b));
        assert_eq!(own.org, "fasttrackstudio");
        let none = super::app_vault_org(&entries, Some(tmp.path()));
        assert_eq!(none.org, architect_accounts::DEVICE_ORG);
    }

    #[test]
    fn a_new_vault_takes_the_next_free_name() {
        let tmp = tempfile::tempdir().unwrap();
        let first = super::create_in(tmp.path()).unwrap();
        let second = super::create_in(tmp.path()).unwrap();
        assert_eq!(first, tmp.path().join("Architect"));
        assert_eq!(second, tmp.path().join("Architect 2"));
        assert!(second.join("Welcome.md").is_file());
    }
}
