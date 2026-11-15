//! Which files are the vault's notes: `.md`, and the file types
//! extensions declare (`[[file_types]]` in an `extension.toml`).
//!
//! A note is a file of text with optional YAML `---` frontmatter: its
//! frontmatter is its properties, the rest is its body. That is all the
//! vault needs to know of a type an extension adds, so the vault's
//! walkers, its index, the documents API and the navigator ask this one
//! registry ([`is_note_path`]) where they used to ask for `.md`, and the
//! extension host [`register`]s each declared type as it loads its
//! manifest. A type stays registered for the life of the process: its
//! files stay notes after the extension is turned off.
//!
//! The registry is process-wide and holds no more than each extension
//! and the `type` property its files read as when their frontmatter has
//! none ([`note_type_of`]). It is plain `std`, so it works the same in a
//! browser.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{PoisonError, RwLock};

/// The note extension every vault has.
pub const MARKDOWN: &str = "md";

/// A declared note file type: its extension (lower-case, without the
/// dot) and the `type` its files read as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteFileType {
    pub extension: String,
    pub note_type: String,
    /// Whether its body is markdown (inline `#tags`, task lines and
    /// headings read as such), or text the vault reads only for its
    /// `[[links]]` ([`body_is_markdown`]).
    pub markdown: bool,
}

static DECLARED: RwLock<Vec<NoteFileType>> = RwLock::new(Vec::new());
/// Whether anything was ever declared: the walkers ask for every file in
/// a vault, and most processes never take the lock.
static ANY: AtomicBool = AtomicBool::new(false);

/// Makes files ending in `.{extension}` notes, reading as `type:
/// {note_type}` when their frontmatter names none. `extension` is taken
/// without a leading dot and in any case. The first registration of an
/// extension wins; `md` (and an empty one) is never registered. Answers
/// whether it was new.
// Registering is what it is for: its answer is often not wanted.
#[allow(clippy::must_use_candidate)]
pub fn register(extension: &str, note_type: &str) -> bool {
    register_with(extension, note_type, true)
}

/// [`register`], saying whether its body is markdown (`false`: a body in
/// another language, Cooklang's `#cookware{}` say, whose `#` is no tag:
/// only its `[[links]]` are read).
pub fn register_with(extension: &str, note_type: &str, markdown: bool) -> bool {
    let ext = extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    if ext.is_empty() || ext == MARKDOWN {
        return false;
    }
    {
        let mut declared = DECLARED.write().unwrap_or_else(PoisonError::into_inner);
        if declared.iter().any(|t| t.extension == ext) {
            return false;
        }
        declared.push(NoteFileType {
            extension: ext,
            note_type: note_type.trim().to_owned(),
            markdown,
        });
    }
    ANY.store(true, Ordering::Release);
    true
}

/// Every declared type, in the order they were registered (`md` is not
/// among them).
#[must_use]
pub fn declared() -> Vec<NoteFileType> {
    DECLARED
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

fn with_declared<R>(ext: &str, f: impl FnOnce(&NoteFileType) -> R) -> Option<R> {
    if !ANY.load(Ordering::Acquire) {
        return None;
    }
    DECLARED
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|t| t.extension.eq_ignore_ascii_case(ext))
        .map(f)
}

/// Whether a file with extension `ext` (without the dot, any case) is a
/// note: `md` or a declared one.
#[must_use]
pub fn is_note_extension(ext: &str) -> bool {
    ext.eq_ignore_ascii_case(MARKDOWN) || with_declared(ext, |_| ()).is_some()
}

/// The extension of a path's last segment, without the dot (`None` for
/// a name with none, or a dotfile's bare name).
fn extension_of(path: &str) -> Option<&str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some(("", _)) | None => None,
        Some((_, ext)) => Some(ext),
    }
}

/// Whether the file at `path` (a vault path or a name) is a note.
#[must_use]
pub fn is_note_path(path: &str) -> bool {
    extension_of(path).is_some_and(is_note_extension)
}

/// The `type` a declared file type's files read as when their
/// frontmatter names none; `None` for `.md` and anything not declared.
#[must_use]
pub fn note_type_of(path: &str) -> Option<String> {
    let ext = extension_of(path)?;
    with_declared(ext, |t| t.note_type.clone()).filter(|t| !t.is_empty())
}

/// Whether the note at `path` has a markdown body: `.md` and every type
/// declared so (the default), not one declared with a text body.
#[must_use]
pub fn body_is_markdown(path: &str) -> bool {
    extension_of(path)
        .and_then(|ext| with_declared(ext, |t| t.markdown))
        .unwrap_or(true)
}

/// `name` without its note extension (`Songs/A.md` → `Songs/A`,
/// `Pancakes.cook` → `Pancakes` once `cook` is declared); anything else
/// as it is.
#[must_use]
pub fn note_stem(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.contains('/') && is_note_extension(ext) => {
            stem
        }
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test: the registry is the process's, and tests run at once.
    #[test]
    fn markdown_always_and_declared_types_once_registered() {
        assert!(is_note_path("Notes/A.md") && is_note_path("B.MD"));
        assert!(!is_note_path("Recipes/Pancakes.zzcook"));
        assert!(!is_note_path(".zzcook") && !is_note_path("README"));
        assert_eq!(
            note_stem("Recipes/Pancakes.zzcook"),
            "Recipes/Pancakes.zzcook"
        );
        assert_eq!(note_type_of("Notes/A.md"), None);

        assert!(register(".ZZCook", "recipe"));
        assert!(!register("zzcook", "other"), "the first wins");
        assert!(!register("md", "x") && !register("  ", "x"));

        assert!(is_note_path("Recipes/Pancakes.zzcook"));
        assert!(is_note_path("Recipes/Pancakes.ZZCOOK"));
        assert!(is_note_extension("zzcook"));
        assert!(!is_note_path("Recipes/zzcook"));
        assert!(!is_note_path("Recipes/.zzcook"));
        assert_eq!(
            note_type_of("Recipes/Pancakes.zzcook").as_deref(),
            Some("recipe")
        );
        assert_eq!(note_stem("Recipes/Pancakes.zzcook"), "Recipes/Pancakes");
        assert_eq!(note_stem("Songs/A.md"), "Songs/A");
        assert_eq!(note_stem("Art/x.png"), "Art/x.png");
        assert_eq!(note_stem("a.b/c"), "a.b/c");
        assert!(
            declared()
                .iter()
                .any(|t| t.extension == "zzcook" && t.note_type == "recipe")
        );
        assert!(register_with("zztext", "recipe", false));
        assert!(!body_is_markdown("R/Toast.zztext"));
        assert!(body_is_markdown("Notes/A.md") && body_is_markdown("x.unknown"));
    }
}
