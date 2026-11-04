//! Where a view's page is, as a file.
//!
//! A view's page comes from one of three places, the first found winning:
//!
//! 1. **An override in the app's folder**: `<app dir>/pages/<extension>/<view>.<ext>`
//!    (or `<app dir>/pages/<view>.<ext>`), where `<ext>` is `tsx`, `ts`,
//!    `jsx`, `js`, `luau` or `a2ui.json` ([`PageKind`]). It replaces a
//!    compiled view live; deleting it brings the built-in back
//!    ([`find_override`]).
//! 2. **The extension's own page**: a view that says `page = "pages/board.tsx"`
//!    in its manifest ships its page as a file.
//! 3. **The compiled view**, when there is no file at all.
//!
//! A built-in's page file is read-only: it is written under
//! `<app dir>/pages/.builtin/<extension>/` for reading ([`write_builtin`]), and
//! edited by cloning it to the override's place ([`clone_to_edit`]).
//!
//! A page that fails to load or render keeps the last good one on screen
//! and writes why beside it, `<file>.error` ([`write_error`]); the next
//! good save removes it.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

/// The folder under the app's folder that pages live in.
pub const PAGES: &str = "pages";

/// The folder under [`PAGES`] built-ins' read-only page files go in.
pub const BUILTIN: &str = ".builtin";

/// What a page file is, by its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PageKind {
    /// TypeScript with JSX: oxc strips its types and turns its JSX into
    /// `h(...)` calls, then `QuickJS` runs it.
    Tsx,
    /// TypeScript.
    Ts,
    /// JavaScript with JSX.
    Jsx,
    /// JavaScript (an ES module).
    Js,
    /// Luau elements (`div():child(...)`).
    Luau,
    /// A2UI JSON, bound to its extension's data and actions.
    A2ui,
}

impl PageKind {
    /// Every kind, in the order an override is looked for.
    pub const ALL: [Self; 6] = [
        Self::Tsx,
        Self::Ts,
        Self::Jsx,
        Self::Js,
        Self::Luau,
        Self::A2ui,
    ];

    /// Its file name's ending, without the dot.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Tsx => "tsx",
            Self::Ts => "ts",
            Self::Jsx => "jsx",
            Self::Js => "js",
            Self::Luau => "luau",
            Self::A2ui => "a2ui.json",
        }
    }

    /// The kind `path` names, by its ending.
    #[must_use]
    pub fn of(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?;
        if name.ends_with(".a2ui.json") {
            return Some(Self::A2ui);
        }
        match path.extension()?.to_str()? {
            "tsx" => Some(Self::Tsx),
            "ts" => Some(Self::Ts),
            "jsx" => Some(Self::Jsx),
            "js" | "mjs" => Some(Self::Js),
            "luau" | "lua" => Some(Self::Luau),
            _ => None,
        }
    }

    /// Whether `QuickJS` runs it.
    #[must_use]
    pub const fn is_js(self) -> bool {
        matches!(self, Self::Tsx | Self::Ts | Self::Jsx | Self::Js)
    }
}

/// Where a page's file came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Origin {
    /// An override in the app's folder: the person's (or an agent's),
    /// editable.
    Override,
    /// Its extension's own page file, from the package (a development
    /// copy's is editable in its folder).
    Extension,
    /// A built-in's page, read-only: clone it to edit it.
    Builtin,
}

/// A view's page, as a file.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PageFile {
    pub path: PathBuf,
    pub kind: PageKind,
    pub origin: Origin,
}

impl PageFile {
    /// The page at `path`, if its ending names a kind.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, origin: Origin) -> Option<Self> {
        let path = path.into();
        let kind = PageKind::of(&path)?;
        Some(Self { path, kind, origin })
    }

    /// Whether it may be edited where it is.
    #[must_use]
    pub const fn editable(&self) -> bool {
        !matches!(self.origin, Origin::Builtin)
    }

    /// The folder its imports are kept inside.
    #[must_use]
    pub fn root(&self) -> PathBuf {
        self.path
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    }

    /// Where why it failed is written ([`write_error`]).
    #[must_use]
    pub fn error_path(&self) -> PathBuf {
        error_path(&self.path)
    }
}

/// An override for `extension`'s view `view` in `app_dir`, if there is
/// one: `pages/<extension>/<view>.<ext>`, then `pages/<view>.<ext>`.
#[must_use]
pub fn find_override(app_dir: &Path, extension: &str, view: &str) -> Option<PageFile> {
    let pages = app_dir.join(PAGES);
    [pages.join(extension), pages]
        .into_iter()
        .flat_map(|dir| {
            PageKind::ALL
                .into_iter()
                .map(move |k| dir.join(format!("{view}.{}", k.extension())))
        })
        .find(|p| p.is_file())
        .and_then(|p| PageFile::new(p, Origin::Override))
}

/// Every page file under `app_dir`'s `pages/`, as `(extension, view,
/// file)`.
///
/// `pages/<extension>/<view>.<ext>`, and `pages/<view>.<ext>` with no
/// extension (it overrides that view of any). The built-ins'
/// read-only copies (`pages/.builtin/`) are not overrides and are left
/// out, as are `.error` files and names no kind ends with.
#[must_use]
pub fn overrides(app_dir: &Path) -> Vec<(String, String, PageFile)> {
    let pages = app_dir.join(PAGES);
    let mut out = Vec::new();
    let mut at = |dir: &Path, extension: &str| {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let view = PageKind::ALL
                .into_iter()
                .find_map(|k| name.strip_suffix(&format!(".{}", k.extension())));
            if let (Some(view), Some(page)) = (view, PageFile::new(&path, Origin::Override))
                && !view.is_empty()
                && !view.starts_with('.')
            {
                out.push((extension.to_owned(), view.to_owned(), page));
            }
        }
    };
    at(&pages, "");
    if let Ok(entries) = std::fs::read_dir(&pages) {
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for dir in dirs {
            let Some(name) = dir.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.starts_with('.') {
                at(&dir, name);
            }
        }
    }
    out.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    out
}

/// Removes the override `page` (and its `.error`): the view shows its own
/// page, or its compiled view, again. What it imports stays.
///
/// # Errors
///
/// It is not an override, or it could not be removed.
pub fn remove_override(page: &PageFile) -> std::io::Result<()> {
    if page.origin != Origin::Override {
        return Err(std::io::Error::other(format!(
            "{} is not an override",
            page.path.display()
        )));
    }
    std::fs::remove_file(&page.path)?;
    clear_error(page);
    Ok(())
}

/// Where an override for `extension`'s view `view` of kind `kind` goes.
#[must_use]
pub fn override_path(app_dir: &Path, extension: &str, view: &str, kind: PageKind) -> PathBuf {
    app_dir
        .join(PAGES)
        .join(extension)
        .join(format!("{view}.{}", kind.extension()))
}

/// Writes a built-in's page files, read-only: the folder they are in.
///
/// Each is `(name, source)` — its pages, what they import, its workers'
/// scripts — under `<app dir>/pages/.builtin/<extension>/`, for a person
/// or an agent to read and clone.
///
/// # Errors
///
/// A file could not be written.
pub fn write_builtin(
    app_dir: &Path,
    extension: &str,
    files: &[(&str, &str)],
) -> std::io::Result<PathBuf> {
    let dir = app_dir.join(PAGES).join(BUILTIN).join(extension);
    std::fs::create_dir_all(&dir)?;
    for (name, source) in files {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(*source) {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if path.exists() {
                set_read_only(&path, false)?;
            }
            std::fs::write(&path, source)?;
        }
        set_read_only(&path, true)?;
    }
    Ok(dir)
}

/// Copies `page` (a built-in's, read-only) to the override's place for
/// `extension`'s view `view`, editable: the override, which wins from
/// then on. One there already is kept (an edit is never overwritten).
///
/// # Errors
///
/// The copy could not be made.
pub fn clone_to_edit(
    page: &PageFile,
    app_dir: &Path,
    extension: &str,
    view: &str,
) -> std::io::Result<PageFile> {
    let to = override_path(app_dir, extension, view, page.kind);
    if !to.exists() {
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&to, std::fs::read(&page.path)?)?;
        set_read_only(&to, false)?;
        // What it imports and spawns comes along (none overwritten).
        if let (Some(from), Some(into)) = (page.path.parent(), to.parent())
            && let Ok(entries) = std::fs::read_dir(from)
        {
            for entry in entries.flatten() {
                let (path, there) = (entry.path(), into.join(entry.file_name()));
                if path.is_file() && path != page.path && !there.exists() {
                    std::fs::write(&there, std::fs::read(&path)?)?;
                    set_read_only(&there, false)?;
                }
            }
        }
    }
    Ok(PageFile {
        path: to,
        kind: page.kind,
        origin: Origin::Override,
    })
}

fn set_read_only(path: &Path, read_only: bool) -> std::io::Result<()> {
    let mut perms = std::fs::metadata(path)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = if read_only { 0o444 } else { 0o644 };
        perms.set_mode(mode);
    }
    #[cfg(not(unix))]
    perms.set_readonly(read_only);
    std::fs::set_permissions(path, perms)
}

/// `<file>.error`.
#[must_use]
pub fn error_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".error");
    PathBuf::from(name)
}

/// Writes why `page` failed beside it (`<file>.error`); a built-in's goes
/// nowhere (its folder is read-only), and is only reported.
pub fn write_error(page: &PageFile, why: &str) {
    if page.editable() {
        let _ = std::fs::write(page.error_path(), format!("{why}\n"));
    }
}

/// Removes `page`'s `.error`: it loads again.
pub fn clear_error(page: &PageFile) {
    let path = page.error_path();
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
}

/// A fingerprint of the page's folder: every file's name, size and
/// modification time (its `.error` files left out, so writing one never
/// reloads it). It moves when the page or anything it imports is saved.
#[must_use]
pub fn fingerprint(page: &PageFile) -> u64 {
    let mut h = DefaultHasher::new();
    page.path.hash(&mut h);
    let mut files = Vec::new();
    walk(&page.root(), 0, &mut files);
    files.sort();
    for (path, len, modified) in files {
        (path, len, modified).hash(&mut h);
    }
    h.finish()
}

/// A fingerprint of which overrides `app_dir` has (their names, not
/// what is in them: a page's own thread follows its edits).
#[must_use]
pub fn overrides_print(app_dir: &Path) -> u64 {
    let mut h = DefaultHasher::new();
    let mut names = Vec::new();
    names_in(&app_dir.join(PAGES), 0, &mut names);
    names.sort();
    names.hash(&mut h);
    h.finish()
}

fn names_in(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 2 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name.ends_with(".error") {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            names_in(&path, depth.saturating_add(1), out);
        } else {
            out.push(path);
        }
    }
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<(PathBuf, u64, u128)>) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name.ends_with(".error") || name == "node_modules" {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            walk(&path, depth.saturating_add(1), out);
        } else {
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            out.push((path, meta.len(), modified));
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_kind_is_known_by_its_ending() {
        assert_eq!(PageKind::of(Path::new("a/board.tsx")), Some(PageKind::Tsx));
        assert_eq!(PageKind::of(Path::new("x.a2ui.json")), Some(PageKind::A2ui));
        assert_eq!(PageKind::of(Path::new("x.luau")), Some(PageKind::Luau));
        assert_eq!(PageKind::of(Path::new("x.json")), None);
        assert_eq!(
            error_path(Path::new("p/x.tsx")),
            PathBuf::from("p/x.tsx.error")
        );
    }

    #[test]
    fn an_override_wins_and_its_removal_restores_the_builtin() {
        let app = tempfile::tempdir().unwrap();
        assert!(find_override(app.path(), "lab.board", "board").is_none());
        let dir = write_builtin(
            app.path(),
            "lab.board",
            &[("board.tsx", "x"), ("job.js", "y")],
        )
        .unwrap();
        let builtin = PageFile::new(dir.join("board.tsx"), Origin::Builtin).unwrap();
        assert!(!builtin.editable());
        assert!(
            std::fs::metadata(&builtin.path)
                .unwrap()
                .permissions()
                .readonly()
        );
        // A built-in's file is not an override.
        assert!(find_override(app.path(), "lab.board", "board").is_none());
        let edit = clone_to_edit(&builtin, app.path(), "lab.board", "board").unwrap();
        assert!(edit.editable());
        // Its worker came along, editable.
        let job = app.path().join(PAGES).join("lab.board").join("job.js");
        assert!(!std::fs::metadata(&job).unwrap().permissions().readonly());
        assert_eq!(
            find_override(app.path(), "lab.board", "board"),
            Some(edit.clone())
        );
        std::fs::remove_file(&edit.path).unwrap();
        assert!(find_override(app.path(), "lab.board", "board").is_none());
        // The short form.
        std::fs::write(app.path().join(PAGES).join("board.luau"), "").unwrap();
        assert_eq!(
            find_override(app.path(), "lab.board", "board").map(|p| p.kind),
            Some(PageKind::Luau)
        );
    }

    #[test]
    fn writing_an_error_never_moves_the_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.tsx");
        std::fs::write(&path, "a").unwrap();
        let page = PageFile::new(&path, Origin::Override).unwrap();
        let before = fingerprint(&page);
        write_error(&page, "boom");
        assert!(page.error_path().exists());
        assert_eq!(fingerprint(&page), before);
        std::fs::write(&path, "ab").unwrap();
        assert_ne!(fingerprint(&page), before);
        clear_error(&page);
        assert!(!page.error_path().exists());
    }
}
