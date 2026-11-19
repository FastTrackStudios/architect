//! A project on a share link: its session documents and exactly the
//! files they reference.
//!
//! A share may carry projects beside its notes
//! (`architect_spaces::ShareProject`): a file root
//! `<org>/Projects/<project>` and the session documents of it that are
//! shared. What a guest gets of the root is decided here and nowhere
//! else:
//!
//! - **The session documents** named (every one in the root when none
//!   is), at the link's `reference` layer and above.
//! - **What they reference**, at `full`. A session is an Architect
//!   document (`crdt::file`); the node reads it whatever its kind and
//!   takes every string in it. One that is a path beside the session, a
//!   path in the root, or a file's sha-256 names that file. Nothing of
//!   the session's format is known here: whoever writes sessions names a
//!   file one of those three ways and it is carried.
//! - **Nothing else of the root**, by any name: a file is served by its
//!   sha-256 (`<link>_blob/<sha256>/<name>`), and only a hash in the set
//!   above is answered. A hash outside it is the same `404` as a path
//!   that is nowhere.
//!
//! The answer is worked out again on every request from the root's
//! manifest as it is now, so an edit of a session changes what the link
//! serves with no rebuild, and the link's URL never moves.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use architect::blob::Sha256;
use architect_spaces::share::is_session_document;
use architect_spaces::{
    GuestProject, GuestProjects, Listen, ProjectAccess, ProjectFile, ProjectRef, ProjectSession,
    Share, ShareProject,
};

use super::holders::{FileAt, Holders};

/// The largest session document read for its references.
pub const SESSION_MAX: u64 = 64 << 20;
/// The longest string of a session taken for a reference.
const NAMED_MAX: usize = 1024;
/// How many sessions' strings are kept.
const KEPT: usize = 256;

/// The layer a project's session documents are in.
pub const SESSIONS: Listen = Listen::Reference;
/// The layer the files they reference are in.
pub const MEDIA: Listen = Listen::Full;

/// One project of a link, resolved against its root as it is now.
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    /// Each shared session: its path, its file, and what it references
    /// (the string in the document, the file's path).
    pub sessions: Vec<(String, FileAt, Vec<(String, String)>)>,
    /// Every referenced file, by root-relative path.
    pub files: BTreeMap<String, FileAt>,
}

/// Every string of the document `bytes` are, when they are an Architect
/// document; none otherwise (an older session format names nothing the
/// node can read: its file is still shared).
async fn strings_of(bytes: Vec<u8>) -> Vec<String> {
    let Ok(doc) = crdt::file::read_main(bytes).await else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect(&doc.get_deep_value(), &mut out);
    out.sort();
    out.dedup();
    out
}

fn collect(value: &crdt::loro::LoroValue, out: &mut Vec<String>) {
    use crdt::loro::LoroValue;
    match value {
        LoroValue::String(s) if !s.is_empty() && s.len() <= NAMED_MAX => out.push(s.to_string()),
        LoroValue::List(items) => items.iter().for_each(|v| collect(v, out)),
        LoroValue::Map(map) => map.values().for_each(|v| collect(v, out)),
        _ => {}
    }
}

/// `named`, read as a path from the folder `dir` of the root (`""` its
/// top), without `.` and `..`; `None` when it leaves the root.
fn under(dir: &str, named: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for part in named.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// The file of `files` that `named` (a string of the session in `dir`)
/// names: by its sha-256, beside the session, or from the root's top.
fn names<'a>(
    named: &str,
    dir: &str,
    files: &'a BTreeMap<String, FileAt>,
    by_hash: &'a HashMap<String, &'a str>,
) -> Option<&'a str> {
    if named.len() == 64 && named.bytes().all(|b| b.is_ascii_hexdigit()) {
        return by_hash.get(&named.to_ascii_lowercase()).copied();
    }
    // An absolute path is some machine's own: not a name in the root.
    if named.starts_with('/') || named.contains(':') {
        return None;
    }
    [under(dir, named), under("", named)]
        .into_iter()
        .flatten()
        .find_map(|path| files.get_key_value(&path).map(|(k, _)| k.as_str()))
}

fn kept() -> &'static Mutex<HashMap<Sha256, Arc<Vec<String>>>> {
    static KEPT_STRINGS: OnceLock<Mutex<HashMap<Sha256, Arc<Vec<String>>>>> = OnceLock::new();
    KEPT_STRINGS.get_or_init(Mutex::default)
}

/// A session's strings, read once per content.
async fn session_strings(holders: &Holders, at: FileAt) -> Arc<Vec<String>> {
    let have = kept()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&at.hash)
        .cloned();
    if let Some(have) = have {
        return have;
    }
    let Some(bytes) = holders.read_small(at, SESSION_MAX).await else {
        // Not readable now: not kept, so the next request asks again.
        return Arc::default();
    };
    let strings = Arc::new(strings_of(bytes).await);
    let mut kept = kept().lock().unwrap_or_else(PoisonError::into_inner);
    if kept.len() >= KEPT {
        kept.clear();
    }
    kept.insert(at.hash, Arc::clone(&strings));
    strings
}

/// `project` against its root as `holders` knows it now; `None` when
/// this node does not know the root.
pub async fn resolve(holders: &Holders, project: &ShareProject) -> Option<Resolved> {
    let files = holders.root_files(&project.root).await?;
    let by_hash: HashMap<String, &str> = files
        .iter()
        .map(|(path, at)| (at.hash.to_hex(), path.as_str()))
        .collect();
    let shared: Vec<&String> = if project.sessions.is_empty() {
        files.keys().filter(|p| is_session_document(p)).collect()
    } else {
        project
            .sessions
            .iter()
            .filter(|p| files.contains_key(*p))
            .collect()
    };
    let mut out = Resolved::default();
    for path in shared {
        let Some(at) = files.get(path).copied() else {
            continue;
        };
        let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
        let mut references = Vec::new();
        for named in session_strings(holders, at).await.iter() {
            let Some(file) = names(named, dir, &files, &by_hash) else {
                continue;
            };
            // Another session is shared only when the link names it.
            if file == path || is_session_document(file) {
                continue;
            }
            if let Some(found) = files.get(file) {
                out.files.insert(file.to_owned(), *found);
                references.push((named.clone(), file.to_owned()));
            }
        }
        out.sessions.push((path.clone(), at, references));
    }
    Some(out)
}

/// A file a link serves by its hash: what it is and whether it is
/// offered as a download.
#[derive(Debug, Clone)]
pub struct Served {
    pub at: FileAt,
    /// Its name (the last part of its path): the response's type.
    pub name: String,
    pub stream_only: bool,
}

/// Every file of `share`'s projects a link that plays to `cap` serves,
/// by sha-256 (hex).
pub async fn served(holders: &Holders, share: &Share, cap: Listen) -> HashMap<String, Served> {
    let mut out = HashMap::new();
    for project in &share.projects {
        let Some(resolved) = resolve(holders, project).await else {
            continue;
        };
        let name = |path: &str| path.rsplit('/').next().unwrap_or(path).to_owned();
        if cap >= SESSIONS {
            for (path, at, _) in &resolved.sessions {
                out.insert(
                    at.hash.to_hex(),
                    Served {
                        at: *at,
                        name: name(path),
                        stream_only: !share.download.offers(path),
                    },
                );
            }
        }
        if cap >= MEDIA {
            for (path, at) in &resolved.files {
                out.insert(
                    at.hash.to_hex(),
                    Served {
                        at: *at,
                        name: name(path),
                        stream_only: !share.download.offers(path),
                    },
                );
            }
        }
    }
    out
}

/// The path of a file served by hash, below a link: `_blob/<sha256>/<name>`.
#[must_use]
pub fn blob_path(sha256: &str, name: &str) -> String {
    format!("{BLOB}/{sha256}/{}", architect_sites::encode_path(name))
}

/// The folder files served by hash are under, below a link.
pub const BLOB: &str = "_blob";
/// The folder a session is saved back to, below a link:
/// `_project/<n>/<path>`, `n` the project's place in the share's list.
pub const SAVE: &str = "_project";

/// `<link>_project.json`: `share`'s projects for a guest at `asked`
/// (never past `cap`), the link mounted at `base` (`/s/<code>`).
pub async fn listed(
    holders: Option<&Holders>,
    share: &Share,
    base: &str,
    cap: Listen,
    asked: Listen,
) -> GuestProjects {
    let at = asked.min(cap);
    let mut projects = Vec::new();
    for (n, project) in share.projects.iter().enumerate() {
        let resolved = match holders {
            Some(holders) => resolve(holders, project).await,
            None => None,
        }
        .unwrap_or_default();
        let url = |sha: &str, path: &str| {
            let name = path.rsplit('/').next().unwrap_or(path);
            format!("{base}/{}", blob_path(sha, name))
        };
        let edit = project.access == ProjectAccess::Edit;
        let sessions = if at >= SESSIONS {
            resolved
                .sessions
                .iter()
                .map(|(path, file, references)| ProjectSession {
                    path: path.clone(),
                    sha256: file.hash.to_hex(),
                    size: file.size,
                    url: url(&file.hash.to_hex(), path),
                    save: edit
                        .then(|| format!("{base}/{SAVE}/{n}/{}", architect_sites::encode_path(path))),
                    references: if at >= MEDIA {
                        references
                            .iter()
                            .filter_map(|(named, path)| {
                                Some(ProjectRef {
                                    named: named.clone(),
                                    sha256: resolved.files.get(path)?.hash.to_hex(),
                                })
                            })
                            .collect()
                    } else {
                        Vec::new()
                    },
                })
                .collect()
        } else {
            Vec::new()
        };
        let files = if at >= MEDIA {
            resolved
                .files
                .iter()
                .map(|(path, file)| ProjectFile {
                    path: path.clone(),
                    sha256: file.hash.to_hex(),
                    size: file.size,
                    url: url(&file.hash.to_hex(), path),
                    stream_only: !share.download.offers(path),
                })
                .collect()
        } else {
            Vec::new()
        };
        projects.push(GuestProject {
            root: project.root.clone(),
            name: project
                .root
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_owned(),
            access: match project.access {
                ProjectAccess::Read => "read",
                ProjectAccess::Edit => "edit",
            }
            .to_owned(),
            sessions,
            files,
        });
    }
    GuestProjects {
        listen: at.label().to_owned(),
        max: cap.label().to_owned(),
        projects,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(n: u8) -> FileAt {
        FileAt {
            hash: Sha256::from_bytes([n; 32]),
            size: u64::from(n),
        }
    }

    #[test]
    fn a_string_names_a_file_beside_the_session_in_the_root_or_by_its_hash() {
        let files: BTreeMap<String, FileAt> = [
            ("songs/joy/Samples/kick.wav", at(1)),
            ("songs/joy/Media/Reference.wav", at(2)),
            ("shared/click.wav", at(3)),
            ("songs/other/Samples/kick.wav", at(4)),
        ]
        .into_iter()
        .map(|(p, a)| (p.to_owned(), a))
        .collect();
        let by_hash: HashMap<String, &str> = files
            .iter()
            .map(|(p, a)| (a.hash.to_hex(), p.as_str()))
            .collect();
        let named = |s: &str| names(s, "songs/joy", &files, &by_hash);
        assert_eq!(named("Samples/kick.wav"), Some("songs/joy/Samples/kick.wav"));
        assert_eq!(named("./Media/Reference.wav"), Some("songs/joy/Media/Reference.wav"));
        assert_eq!(named("Samples\\kick.wav"), Some("songs/joy/Samples/kick.wav"));
        assert_eq!(named("../../shared/click.wav"), Some("shared/click.wav"));
        assert_eq!(named("shared/click.wav"), Some("shared/click.wav"));
        assert_eq!(named(&at(4).hash.to_hex()), Some("songs/other/Samples/kick.wav"));
        // Not a file of the root, a path out of it, or some machine's own.
        assert_eq!(named("Samples/snare.wav"), None);
        assert_eq!(named("../../../etc/passwd"), None);
        assert_eq!(named("/songs/joy/Samples/kick.wav"), None);
        assert_eq!(named("C:\\Sessions\\kick.wav"), None);
        assert_eq!(named("kick"), None);
        assert_eq!(under("a/b", "../../.."), None);
    }
}
