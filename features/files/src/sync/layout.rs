//! Where roots are on disk: `<files_dir>/<org>/Projects/<project>` and
//! `<files_dir>/<org>/Assets/<kind>`, the tree starcommand already has.
//!
//! A root's id is the `id` of its `.fts-root.json` (Task's marker), so
//! Task's ids carry over; a folder without one gets an id derived from
//! its place. Nothing here writes to a root it found.

use std::path::{Path, PathBuf};

use facet::Facet;
use uuid::Uuid;

use super::manifest::{RootMeta, Section};
use crate::id::RootId;

/// Task's root marker, read for the id.
pub const MARKER: &str = ".fts-root.json";
/// Task's store inside a root: ignored, never read, migrated or deleted.
pub const TASK_STORE: &str = ".fts-files";
/// The first line of a file Task dehydrated: never indexed as content.
pub const TASK_STUB: &[u8] = b"#fts-stub v1\n";
/// The suffix of this engine's own temp files: one convention, the
/// blob layer's ([`architect::blob::STAGING`]).
pub const PART: &str = architect::blob::STAGING;
/// The engine's own folder in a tree (`<files_dir>/.architect`: a holding
/// node's incoming bytes and kept versions). Never a root, never synced.
pub const AREA: &str = ".architect";

/// The file that marks `.architect` as the node's own folder, and tells
/// whoever finds it on the share what it is.
pub const AREA_MARK: &str = "README.txt";
const AREA_MARK_TEXT: &str = "This folder belongs to the Architect node that holds this tree.\n\
incoming/  bytes it fetched from peers, about to be renamed into the tree\n\
versions/  the previous versions of files it replaced (history), by sha-256\n\
Anyone who mounts this share sees it. Do not edit or delete what is in it:\n\
the versions are the only copy of some old versions.\n";

/// What [`claim_area`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claimed {
    /// `.architect` is the node's own folder.
    Ready,
    /// The share cannot be written here (read-only, root-squashed): the
    /// node starts and serves reads, keeps no history, and every
    /// replacement or delete waits and says this.
    Unwritable(String),
}

/// Make `<files_dir>/.architect` the node's own folder.
///
/// An absent or empty folder is claimed: its mark is written beside it
/// and renamed in, so a crash leaves either no mark (claimed again next
/// start) or a whole one. A share the node cannot write is
/// [`Claimed::Unwritable`].
///
/// # Errors
///
/// Something else already has that name (a file, a link, a folder with
/// other things in it and no mark of the node's): a holding node refuses
/// to start, and the message names what to move.
pub fn claim_area(files_dir: &Path) -> Result<Claimed, String> {
    let area = files_dir.join(AREA);
    let mark = area.join(AREA_MARK);
    let unwritable = |e: &std::io::Error| {
        matches!(
            e.kind(),
            std::io::ErrorKind::ReadOnlyFilesystem | std::io::ErrorKind::PermissionDenied
        )
    };
    let write_mark = || -> std::io::Result<()> {
        let tmp = area.join(format!("{AREA_MARK}{PART}"));
        {
            use std::io::Write as _;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(AREA_MARK_TEXT.as_bytes())?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &mark)
    };
    let claim = |made: std::io::Result<()>| match made.and_then(|()| write_mark()) {
        Ok(()) => Ok(Claimed::Ready),
        Err(e) if unwritable(&e) => Ok(Claimed::Unwritable(format!(
            "{} cannot be written ({e}): no history is kept, replacements and deletes wait",
            area.display()
        ))),
        Err(e) => Err(format!(
            "{}: cannot make the node's folder: {e}",
            area.display()
        )),
    };
    match std::fs::symlink_metadata(&area) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => claim(std::fs::create_dir_all(&area)),
        Err(e) => Err(format!("{}: {e}", area.display())),
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => Err(format!(
            "{} exists and is not a folder of the node's (a file or a link): move it away, \
             the node keeps its history there",
            area.display()
        )),
        Ok(_) => {
            if std::fs::read_to_string(&mark).is_ok_and(|text| {
                text.starts_with(AREA_MARK_TEXT.lines().next().unwrap_or_default())
            }) {
                return Ok(Claimed::Ready);
            }
            // Empty but for a half-written mark: a claim a crash cut short.
            let empty = std::fs::read_dir(&area).is_ok_and(|entries| {
                entries
                    .flatten()
                    .all(|e| e.file_name().to_string_lossy().starts_with(AREA_MARK))
            });
            if empty {
                return claim(Ok(()));
            }
            Err(format!(
                "{} exists and is not the node's own folder (no {AREA_MARK} of its own): \
                 move it away, the node keeps its history there",
                area.display()
            ))
        }
    }
}

/// Where a holding node keeps what it fetches and the versions it
/// replaces, on the tree's own filesystem.
#[must_use]
pub fn areas(files_dir: &Path) -> (PathBuf, PathBuf) {
    let area = files_dir.join(AREA);
    (area.join("incoming"), area.join("versions"))
}

/// A root found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundRoot {
    pub meta: RootMeta,
    pub dir: PathBuf,
}

#[derive(Facet)]
struct Marker {
    id: String,
}

/// Every root under `files_dir`, by org, projects then libraries.
#[must_use]
pub fn discover(files_dir: &Path) -> Vec<FoundRoot> {
    let mut out = Vec::new();
    for org in dirs(files_dir) {
        let Some(org_name) = org.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        for section in [Section::Projects, Section::Assets] {
            for dir in dirs(&org.join(section.dir())) {
                let Some(name) = dir.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
                    continue;
                };
                let place = format!("{org_name}/{}/{name}", section.dir());
                out.push(FoundRoot {
                    meta: RootMeta {
                        id: root_id(&dir, &place),
                        org: org_name.clone(),
                        section: section.clone(),
                        name,
                    },
                    dir,
                });
            }
        }
    }
    out
}

/// Where a root at `place` goes under `files_dir`.
#[must_use]
pub fn dir_for(files_dir: &Path, meta: &RootMeta) -> PathBuf {
    files_dir
        .join(&meta.org)
        .join(meta.section.dir())
        .join(&meta.name)
}

/// The id in `dir`'s marker, else one derived from `place`.
#[must_use]
pub fn root_id(dir: &Path, place: &str) -> RootId {
    std::fs::read_to_string(dir.join(MARKER))
        .ok()
        .and_then(|json| facet_json::from_str::<Marker>(&json).ok())
        .and_then(|m| Uuid::parse_str(&m.id).ok())
        .map_or_else(
            || {
                RootId(Uuid::new_v5(
                    &Uuid::NAMESPACE_URL,
                    format!("architect-root:{place}").as_bytes(),
                ))
            },
            RootId,
        )
}

fn dirs(parent: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

/// Whether a root-relative path is never synced.
///
/// Task's store and marker, the engine's own folder, the DAW's
/// regenerable peaks, the OS's litter, and every temp name the engine
/// writes (a name ending in [`PART`]). The one place that
/// decides it: the scan, the unpin check and the start-up clean-up all
/// ask here.
#[must_use]
pub fn ignored(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    rel.split('/').any(|seg| seg == TASK_STORE || seg == AREA)
        || name == MARKER
        || name == ".DS_Store"
        || name == "Thumbs.db"
        || name.starts_with("._")
        || is_temp(name)
        || has_ext(name, &["reapeaks", "tmp"])
}

/// Whether a file name has the engine's temp suffix, exactly.
///
/// The engine writes `<name>.architect-part` and nothing else. Never synced;
/// removed at start only when the engine's journal says it wrote it
/// (`super::pending`), otherwise left alone and reported.
#[must_use]
pub fn is_temp(name: &str) -> bool {
    name.ends_with(PART)
}

/// Whether saving this file is a save point: a DAW's session file.
#[must_use]
pub fn is_session_file(rel: &str) -> bool {
    has_ext(rel, &["rpp", "ptx", "als", "logicx", "cpr", "song", "aup3"])
}

fn has_ext(name: &str, exts: &[&str]) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_task_tree_is_found_and_its_ids_carry_over() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("codywright/Projects/Right Here");
        std::fs::create_dir_all(p.join(".fts-files")).unwrap();
        std::fs::write(
            p.join(MARKER),
            r#"{"id":"7392cd25-c5f0-487a-be42-df50d0666a36","name":"Right Here"}"#,
        )
        .unwrap();
        std::fs::create_dir_all(tmp.path().join("days-to-praise/Assets/Tracks")).unwrap();
        let found = discover(tmp.path());
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].meta.place(), "codywright/Projects/Right Here");
        assert_eq!(
            found[0].meta.id.to_string(),
            "7392cd25-c5f0-487a-be42-df50d0666a36"
        );
        assert_eq!(found[1].meta.section, Section::Assets);
        assert_eq!(found[1].meta.name, "Tracks");
    }

    /// Every temp name the engine can write in a tree is ignored, and
    /// only those (#527: a cross-filesystem copy's `.architect-part.part`
    /// was committed as a file and pushed to peers; that name is gone).
    #[test]
    fn every_temp_name_the_engine_writes_is_ignored() {
        let at = Path::new("Audio/take.wav");
        let staged = architect::blob::staging_path(at);
        let rel = |p: &Path| p.to_string_lossy().into_owned();
        assert!(ignored(&rel(&staged)), "write_file's staging name");
        assert!(ignored(&rel(&architect::blob::staging_path(&staged))));
        assert!(
            !ignored("Audio/take.wav.architect-part.part"),
            "the engine never writes that name: it is a user's file"
        );
        assert!(!ignored("Audio/an.architect-part of mine.txt"));
        assert!(ignored(".architect/versions/ab/abcd"));
        assert!(ignored(".architect/incoming/.partial/abcd.part"));
        assert!(!ignored("Audio/take.wav"));
        assert!(!ignored("Audio/architect-parts.wav"));
    }

    #[test]
    fn the_node_s_folder_is_its_own_or_it_does_not_start() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(claim_area(tmp.path()), Ok(Claimed::Ready));
        assert_eq!(claim_area(tmp.path()), Ok(Claimed::Ready));
        assert!(tmp.path().join(AREA).join(AREA_MARK).is_file());

        // A claim a crash cut short: an empty folder, or a half mark.
        let cut = tempfile::tempdir().unwrap();
        std::fs::create_dir(cut.path().join(AREA)).unwrap();
        assert_eq!(claim_area(cut.path()), Ok(Claimed::Ready));
        let half = tempfile::tempdir().unwrap();
        std::fs::create_dir(half.path().join(AREA)).unwrap();
        std::fs::write(
            half.path().join(AREA).join("README.txt.architect-part"),
            b"Th",
        )
        .unwrap();
        assert_eq!(claim_area(half.path()), Ok(Claimed::Ready));

        let theirs = tempfile::tempdir().unwrap();
        std::fs::create_dir(theirs.path().join(AREA)).unwrap();
        std::fs::write(theirs.path().join(AREA).join("notes.md"), b"mine").unwrap();
        assert!(
            claim_area(theirs.path())
                .unwrap_err()
                .contains("not the node's own")
        );
        let file = tempfile::tempdir().unwrap();
        std::fs::write(file.path().join(AREA), b"x").unwrap();
        assert!(
            claim_area(file.path())
                .unwrap_err()
                .contains("not a folder")
        );
    }

    #[test]
    fn task_files_and_peaks_are_not_synced() {
        assert!(ignored(".fts-files/store/chunks/a"));
        assert!(ignored(".fts-root.json"));
        assert!(ignored("Audio Files/Bass_1.wav.reapeaks"));
        assert!(!ignored("Right Here/Right Here.RPP"));
        assert!(is_session_file("Right Here/Right Here.RPP"));
        assert!(!is_session_file("Audio Files/Bass_1.wav"));
    }
}
