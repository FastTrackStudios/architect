//! A site or share build follows no symlink: not one to a file outside
//! the vault, not one inside it naming a private or ignored folder (or
//! `.git`), not a symlinked folder, and not a file swapped for a symlink
//! after the vault was read. No symlink is listed, built or copied.

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::os::unix::fs::symlink;
use std::path::Path;

use architect_sites::listing::{self, parse_filter};
use architect_sites::{Blocks, Options, Scope, Site, build, write};

const SECRET: &str = "outside secret";
const PRIVATE: &str = "private plans";
const GIT: &str = "[remote \"origin\"] token";
const IGNORED: &str = "ignored draft";

fn touch(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// Every file under `dir`, by its path below it, and its bytes.
fn files_under(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.push((rel, std::fs::read(&path).unwrap()));
            }
        }
    }
    out
}

/// Nothing under `out` holds any of `secrets`.
fn assert_none_of(out: &Path, secrets: &[&str]) {
    for (rel, bytes) in files_under(out) {
        let text = String::from_utf8_lossy(&bytes);
        for secret in secrets {
            assert!(
                !text.contains(secret),
                "{rel} holds `{secret}`, which the vault never shares"
            );
        }
    }
}

/// The reviewer's scratch vault (#673 review): an image and a note that
/// are symlinks to files outside the vault, linked from the home note.
#[test]
fn a_build_copies_and_renders_nothing_a_symlink_out_of_the_vault_names() {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    let outside = tmp.path().join("outside");
    touch(&outside, "photo.png", SECRET);
    touch(
        &outside,
        "other.md",
        &format!("---\ntitle: Other\n---\n{SECRET}\n"),
    );
    touch(
        &vault,
        "index.md",
        "---\ntitle: Home\n---\n![[art.png]]\n\nSee [[other]].\n",
    );
    symlink(outside.join("photo.png"), vault.join("art.png")).unwrap();
    symlink(outside.join("other.md"), vault.join("other.md")).unwrap();
    let out = tmp.path().join("site");
    build(&Options::new(&vault, "Home"), &Blocks::new(), &out).unwrap();
    assert!(
        !out.join("art.png").exists(),
        "the symlinked image was copied"
    );
    assert!(
        !out.join("other/index.html").exists(),
        "the outside note was rendered"
    );
    assert_none_of(&out, &[SECRET]);
}

/// A vault whose `Shared/` folder holds symlinks that stay inside the
/// vault yet name what it never shares: a private folder's note, `.git`'s
/// config, an ignored folder's note, and a symlinked folder leading to
/// the private one; and one out of the vault, and a symlinked folder out.
fn inside_links() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    let outside = tmp.path().join("outside");
    touch(&outside, "secret.png", SECRET);
    touch(&vault, "Private/.fts-private", "");
    touch(
        &vault,
        "Private/plans.md",
        &format!("---\ntitle: Plans\n---\n{PRIVATE}\n"),
    );
    touch(&vault, "Private/plan.png", PRIVATE);
    touch(&vault, ".git/config", GIT);
    touch(
        &vault,
        ".obsidian/app.json",
        r#"{"userIgnoreFilters": ["Drafts/"]}"#,
    );
    touch(
        &vault,
        "Drafts/draft.md",
        &format!("---\ntitle: Draft\n---\n{IGNORED}\n"),
    );
    touch(
        &vault,
        "Shared/index.md",
        "---\ntitle: Shared\n---\n[[notes]] [[draft]] [[evil]]\n\n![[x.png]]\n\n![[out/secret.png]]\n\n![[priv/plan.png]]\n\n![[art.png]]\n",
    );
    touch(&vault, "Shared/art.png", "shared png");
    symlink("../Private/plans.md", vault.join("Shared/notes.md")).unwrap();
    symlink("../.git/config", vault.join("Shared/x.png")).unwrap();
    symlink("../Drafts/draft.md", vault.join("Shared/draft.md")).unwrap();
    symlink("../Private", vault.join("Shared/priv")).unwrap();
    symlink(outside.join("secret.png"), vault.join("Shared/evil.png")).unwrap();
    symlink(&outside, vault.join("Shared/out")).unwrap();
    tmp
}

#[test]
fn a_share_listing_lists_no_symlink_inside_or_outside_the_vault() {
    let tmp = inside_links();
    let root = tmp.path().join("vault");
    for (vault, how) in [
        (listing::open(&root).unwrap(), "open"),
        (listing::open_reporting(&root).unwrap().0, "open_reporting"),
    ] {
        let pages: Vec<&str> = vault.pages.iter().map(|p| p.rel_path.as_str()).collect();
        assert_eq!(
            pages
                .iter()
                .filter(|p| p.starts_with("Shared/"))
                .collect::<Vec<_>>(),
            [&"Shared/index.md"],
            "{how}: {pages:?}"
        );
        let entries = listing::select(
            &vault,
            &Scope::Folder("Shared".to_owned()),
            &parse_filter("").unwrap(),
        );
        let files: Vec<&str> = entries
            .iter()
            .flat_map(|e| e.files.iter().map(String::as_str))
            .collect();
        assert_eq!(files, ["Shared/art.png"], "{how}");
    }
}

#[test]
fn a_build_follows_no_symlink_inside_or_outside_the_vault() {
    let tmp = inside_links();
    let out = tmp.path().join("site");
    let mut opts = Options::new(tmp.path().join("vault"), "Shared");
    opts.scope = Scope::Folder("Shared".to_owned());
    let site = Site::read(&opts, &Blocks::new()).unwrap();
    assert_eq!(site.attachments, ["Shared/art.png"]);
    write(&site, &out).unwrap();
    assert_none_of(&out, &[SECRET, PRIVATE, GIT, IGNORED]);
    assert_eq!(
        std::fs::read_to_string(out.join("Shared/art.png")).unwrap(),
        "shared png"
    );
}

/// Between the read and the copy, a file, or the folder above it, is
/// swapped for a symlink: neither is followed.
#[test]
fn a_file_swapped_for_a_symlink_after_the_read_is_not_copied() {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    touch(&tmp.path().join("outside"), "art.png", SECRET);
    touch(&tmp.path().join("outside"), "media/song.png", SECRET);
    touch(
        &vault,
        "Shared/index.md",
        "---\ntitle: Shared\n---\n![[art.png]]\n\n![[media/song.png]]\n",
    );
    touch(&vault, "Shared/art.png", "shared png");
    touch(&vault, "Shared/media/song.png", "song png");
    let mut opts = Options::new(&vault, "Shared");
    opts.scope = Scope::Folder("Shared".to_owned());
    let site = Site::read(&opts, &Blocks::new()).unwrap();
    assert_eq!(site.attachments.len(), 2, "{:?}", site.attachments);
    std::fs::remove_file(vault.join("Shared/art.png")).unwrap();
    symlink(
        tmp.path().join("outside/art.png"),
        vault.join("Shared/art.png"),
    )
    .unwrap();
    std::fs::remove_dir_all(vault.join("Shared/media")).unwrap();
    symlink(tmp.path().join("outside/media"), vault.join("Shared/media")).unwrap();
    let out = tmp.path().join("site");
    write(&site, &out).unwrap();
    assert_none_of(&out, &[SECRET]);
    assert!(!out.join("Shared/art.png").exists());
    assert!(!out.join("Shared/media/song.png").exists());
}
