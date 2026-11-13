//! Walk a vault root in the shape Obsidian uses: arbitrary folder
//! tree under one root, with metadata in `.obsidian/`. Filters out
//! the conventional ignore set (`.obsidian`, `.trash`, `.git`,
//! `.DS_Store`, dotfiles) and honors `userIgnoreFilters` from
//! `.obsidian/app.json` (prefix / suffix patterns only in v1).

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::config::ObsidianConfig;

/// What an entry maps to in our entity model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaultEntryKind {
    /// `.md` / `.markdown`. Parsed via `crate::obsidian_parse::parse_page`.
    Markdown,
    /// `.base` Obsidian Bases definition. Parsed via `vault_live::bases::parse`.
    Base,
    /// `.canvas`. v1 stores raw JSON on a placeholder page; full
    /// rendering is a later arc.
    Canvas,
    /// Anything else inside the vault — images, PDFs, audio, etc.
    /// Recorded so links can be resolved; no per-block import.
    Attachment,
}

#[derive(Clone, Debug)]
pub struct VaultEntry {
    /// Full disk path.
    pub abs_path: PathBuf,
    /// Vault-relative path with extension. Forward-slash separated.
    pub rel_path: String,
    pub kind: VaultEntryKind,
}

/// Synchronous walk. Returns entries in walkdir's default order
/// (folders before children, lexicographic within a folder). v1
/// loads the whole list into memory — vaults under ~50k files are
/// fine; we'll stream if/when that becomes a constraint.
#[must_use]
pub fn walk_vault(root: &Path, cfg: &ObsidianConfig) -> Vec<VaultEntry> {
    walk_vault_reporting(root, cfg).0
}

/// [`walk_vault`], and what it could not read on the way (a folder or an
/// entry it skipped), one line each: for a caller that must not take a
/// partial walk for the whole vault.
#[must_use]
pub fn walk_vault_reporting(root: &Path, cfg: &ObsidianConfig) -> (Vec<VaultEntry>, Vec<String>) {
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    let ignore = build_ignore(cfg);
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            !is_hard_skip(e.path(), root)
                // t[impl vault.private.node-only]
                && !(e.file_type().is_dir() && vault_live::private::is_private_dir(e.path()))
        })
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(?e, "walk error — skipping");
                skipped.push(e.to_string());
                continue;
            }
        };
        if entry.file_type().is_dir() {
            continue;
        }
        // A symlink is never the vault's own entry: inside or outside the
        // vault, it can name what the walk skips (a private or ignored
        // folder, `.git`, a file elsewhere), so it is not listed, built
        // or shared. A symlinked folder is not walked into either
        // (`follow_links(false)`).
        if entry.path_is_symlink() {
            tracing::debug!(path = %entry.path().display(), "a symlink in the vault; skipped");
            continue;
        }
        let rel = match entry.path().strip_prefix(root) {
            Ok(r) => vault_live::walker::rel_string(r),
            Err(_) => continue,
        };
        if ignore.matches(&rel) {
            continue;
        }
        let kind = classify(entry.path());
        out.push(VaultEntry {
            // Moved out of the walkdir entry, not copied from it.
            abs_path: entry.into_path(),
            rel_path: rel,
            kind,
        });
    }
    (out, skipped)
}

fn classify(p: &Path) -> VaultEntryKind {
    // Compared case-insensitively in place, not by lowering a copy of
    // the extension of every file in the vault.
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or_default();
    let is = |want: &str| ext.eq_ignore_ascii_case(want);
    if is("md") || is("markdown") {
        VaultEntryKind::Markdown
    } else if is("base") {
        VaultEntryKind::Base
    } else if is("canvas") {
        VaultEntryKind::Canvas
    } else {
        VaultEntryKind::Attachment
    }
}

/// Hard skips applied at the directory level so `walkdir` doesn't
/// descend into them at all. Top-level only for `.obsidian` /
/// `.trash` / `.git` — those are anchored at the vault root, not
/// nested. NOTE: we do NOT skip `node_modules` — Obsidian itself
/// indexes those trees (confirmed against The Observatory, where
/// its `files total` includes `node_modules` contents).
fn is_hard_skip(path: &Path, root: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    // Only the first component is ever read: take it, rather than
    // collecting every component of every entry's path into a `Vec`.
    let Some(first) = rel.components().next() else {
        return false;
    };
    let first = first.as_os_str().to_string_lossy();
    if matches!(first.as_ref(), ".obsidian" | ".trash" | ".git") {
        return true;
    }
    // A dot-file or dot-folder anywhere is hidden, as Obsidian hides it
    // (`.empty-folders`, `.DS_Store`, `.stfolder`), and as vault-live's
    // walker skips it (#442).
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

/// Compiled user-ignore matcher. v1 = prefix / suffix only.
struct IgnoreMatcher {
    prefixes: Vec<String>,
    suffixes: Vec<String>,
    exact: Vec<String>,
}

impl IgnoreMatcher {
    fn matches(&self, rel: &str) -> bool {
        if self.exact.iter().any(|e| e == rel) {
            return true;
        }
        if self.prefixes.iter().any(|p| rel.starts_with(p)) {
            return true;
        }
        if self.suffixes.iter().any(|s| rel.ends_with(s)) {
            return true;
        }
        false
    }
}

fn build_ignore(cfg: &ObsidianConfig) -> IgnoreMatcher {
    let mut prefixes = Vec::new();
    let mut suffixes = Vec::new();
    let mut exact = Vec::new();
    for raw in &cfg.user_ignore_filters {
        // Obsidian stores either bare paths or regex-y patterns; v1
        // only recognizes the simple shapes.
        if let Some(rest) = raw.strip_prefix('*').and_then(|r| r.strip_suffix('*')) {
            tracing::warn!(pattern = %raw, "ignoring unsupported user_ignore_filter (contains: {rest})");
        } else if let Some(rest) = raw.strip_prefix('*') {
            suffixes.push(rest.to_string());
        } else if let Some(rest) = raw.strip_suffix('*') {
            prefixes.push(rest.to_string());
        } else if raw.ends_with('/') {
            // Folder-style ignore: treat as prefix.
            prefixes.push(raw.clone());
        } else if !raw.contains('*') {
            exact.push(raw.clone());
        } else {
            tracing::warn!(pattern = %raw, "ignoring unsupported user_ignore_filter");
        }
    }
    IgnoreMatcher {
        prefixes,
        suffixes,
        exact,
    }
}

/// The vault file at `rel` (forward-slash separated, below `root`)
/// opened for reading, never through a symlink.
///
/// Opened without following a link at its last component
/// (`O_NOFOLLOW`), then checked to be a plain file that `rel` names
/// with no symlink anywhere on the way: its resolved path is exactly the
/// root's joined with `rel`, and that path is the very file opened. A
/// file or a folder above it swapped for a symlink (out of the vault,
/// or to a private or ignored folder in it) between a walk and a read is
/// refused, never followed.
///
/// # Errors
///
/// The file could not be opened, or it is not the vault's own plain
/// file now (`InvalidData`), or `rel` is not a plain relative path:
/// absolute, or with `..`, `.` or a prefix (`InvalidInput`).
///
/// Unix only in full: elsewhere the opened file is compared with the path
/// by size and modification time, a weaker check.
pub fn open_own(root: &Path, rel: &str) -> std::io::Result<std::fs::File> {
    use std::io::{Error, ErrorKind};
    use std::path::Component;
    // `root.join` of an absolute path replaces the root.
    if rel.is_empty()
        || !Path::new(rel)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "not a path in the vault",
        ));
    }
    let path = root.join(rel);
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // Non-blocking too: a link swapped to a FIFO never hangs a read.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(&path)?;
    let opened = file.metadata()?;
    let not_own = || Error::new(ErrorKind::InvalidData, "not a plain file of the vault");
    if !opened.is_file() {
        return Err(not_own());
    }
    let expected = root.canonicalize()?.join(rel);
    let resolved = path.canonicalize()?;
    if resolved != expected {
        return Err(not_own());
    }
    let now = std::fs::symlink_metadata(&resolved)?;
    if now.file_type().is_symlink() || !same_file(&opened, &now) {
        return Err(not_own());
    }
    Ok(file)
}

#[cfg(unix)]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    a.dev() == b.dev() && a.ino() == b.ino()
}

#[cfg(not(unix))]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    b.is_file() && a.len() == b.len() && a.modified().ok() == b.modified().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(p: &Path) {
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, "x").unwrap();
    }

    #[test]
    fn walk_finds_markdown_and_bases_skips_obsidian_dir() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("Note.md"));
        touch(&dir.path().join("Folder/Sub.md"));
        touch(&dir.path().join("Charts.base"));
        touch(&dir.path().join(".obsidian/app.json"));
        touch(&dir.path().join(".trash/old.md"));

        let cfg = ObsidianConfig::default();
        let entries = walk_vault(dir.path(), &cfg);
        let kinds: Vec<_> = entries
            .iter()
            .map(|e| (e.rel_path.clone(), e.kind))
            .collect();
        assert!(kinds.contains(&("Note.md".into(), VaultEntryKind::Markdown)));
        assert!(kinds.contains(&("Folder/Sub.md".into(), VaultEntryKind::Markdown)));
        assert!(kinds.contains(&("Charts.base".into(), VaultEntryKind::Base)));
        assert!(!kinds.iter().any(|(p, _)| p.starts_with(".obsidian")));
        assert!(!kinds.iter().any(|(p, _)| p.starts_with(".trash")));
    }

    #[test]
    fn dot_files_and_dot_folders_are_hidden() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join(".empty-folders"));
        touch(&dir.path().join("Folder/.DS_Store"));
        touch(&dir.path().join(".stfolder/x.md"));
        touch(&dir.path().join("Folder/Keep.md"));
        let entries = walk_vault(dir.path(), &ObsidianConfig::default());
        let rels: Vec<_> = entries.iter().map(|e| e.rel_path.clone()).collect();
        assert_eq!(rels, ["Folder/Keep.md"]);
    }

    #[test]
    fn node_modules_is_not_skipped() {
        // Obsidian indexes node_modules trees (confirmed against
        // The Observatory). We follow suit so file totals + the
        // backlink/orphan graph match.
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("node_modules/foo/README.md"));
        let cfg = ObsidianConfig::default();
        let entries = walk_vault(dir.path(), &cfg);
        let rels: Vec<_> = entries.iter().map(|e| e.rel_path.clone()).collect();
        assert!(rels.contains(&"node_modules/foo/README.md".into()));
    }

    #[test]
    fn user_ignore_prefix_and_suffix() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("Drafts/wip.md"));
        touch(&dir.path().join("Notes/keep.md"));
        touch(&dir.path().join("backup.tmp.md"));

        let cfg = ObsidianConfig {
            user_ignore_filters: vec!["Drafts/".into(), "*.tmp.md".into()],
            ..Default::default()
        };
        let entries = walk_vault(dir.path(), &cfg);
        let rels: Vec<_> = entries.iter().map(|e| e.rel_path.clone()).collect();
        assert!(rels.contains(&"Notes/keep.md".into()));
        assert!(!rels.iter().any(|r| r.starts_with("Drafts/")));
        assert!(!rels.iter().any(|r| r.ends_with(".tmp.md")));
    }

    #[cfg(unix)]
    #[test]
    fn walk_lists_no_symlink_inside_or_outside_the_vault() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("vault");
        touch(&root.join("Note.md"));
        touch(&root.join("Private/plans.md"));
        touch(&tmp.path().join("outside/secret.md"));
        symlink("Note.md", root.join("Alias.md")).unwrap();
        symlink("../outside/secret.md", root.join("Out.md")).unwrap();
        symlink("Private", root.join("Linked")).unwrap();
        let rels: Vec<String> = walk_vault(&root, &ObsidianConfig::default())
            .into_iter()
            .map(|e| e.rel_path)
            .collect();
        assert!(rels.contains(&"Note.md".to_owned()), "{rels:?}");
        for link in ["Alias.md", "Out.md", "Linked", "Linked/plans.md"] {
            assert!(!rels.contains(&link.to_owned()), "{link} listed: {rels:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn open_own_opens_a_plain_file_and_refuses_any_symlink_on_the_way() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("vault");
        touch(&root.join("a/note.md"));
        touch(&tmp.path().join("outside/b/note.md"));
        touch(&tmp.path().join("outside/x.md"));
        assert!(open_own(&root, "a/note.md").is_ok());
        symlink("a/note.md", root.join("alias.md")).unwrap();
        symlink(tmp.path().join("outside/x.md"), root.join("out.md")).unwrap();
        symlink(tmp.path().join("outside/b"), root.join("b")).unwrap();
        symlink("a", root.join("c")).unwrap();
        for rel in [
            "alias.md",
            "out.md",
            "b/note.md",
            "c/note.md",
            "a",
            "missing.md",
        ] {
            assert!(open_own(&root, rel).is_err(), "{rel} opened");
        }
        // Not a plain relative path: `root.join` would leave the vault.
        let outside = tmp.path().join("outside/x.md");
        for rel in [
            outside.to_str().unwrap(),
            "../outside/x.md",
            "a/../a/note.md",
            "./a/note.md",
            "",
        ] {
            let e = open_own(&root, rel).expect_err(rel);
            assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput, "{rel}");
        }
    }
}
