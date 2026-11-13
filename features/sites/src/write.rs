//! A [`Site`] written out as files.
//!
//! The site is written beside `out` (`<out>.next`) and swapped in whole,
//! so a server reading `out` (`architect site serve`, or the architect
//! server's site mount) never serves half a build. `out` is only ever
//! replaced if it is a site this wrote (it holds [`MARKER`]) or empty:
//! a mistyped `--out .` cannot delete a checkout.

use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use crate::assets::{fingerprint, site_css, site_js, theme_css};
use crate::search;
use crate::site::{Problem, Site};
use crate::ui::{self, Assets, View};

/// The file that marks a folder as a site this wrote.
pub const MARKER: &str = ".architect-site";

/// Where fingerprinted assets go (`architect::host::SITE_ASSETS`).
const ASSETS: &str = "_site";

/// What [`write()`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub out: PathBuf,
    /// Pages, tag pages included.
    pub pages: usize,
    pub tags: usize,
    /// Files copied as they are (attachments, static folders).
    pub files: usize,
    /// What the build reported, for the caller to show.
    pub problems: Vec<Problem>,
}

/// Why a site could not be written.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("{0} is not empty and is not a site `architect site` wrote; refusing to replace it")]
    NotASite(PathBuf),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
}

/// Called between the two renames that swap a written site into place,
/// with the folder being replaced: a test's way to hold the moment the
/// folder is not there (#540's review). None in use.
#[doc(hidden)]
pub type SwapHook = std::sync::Arc<dyn Fn(&Path) + Send + Sync>;

static SWAP_HOOK: std::sync::Mutex<Option<SwapHook>> = std::sync::Mutex::new(None);

/// Set (or clear) the [`SwapHook`]. For tests only.
#[doc(hidden)]
pub fn set_swap_hook(hook: Option<SwapHook>) {
    *SWAP_HOOK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = hook;
}

/// Write `site` to `out`.
///
/// # Errors
///
/// `out` holds something else, or a file could not be written.
pub fn write(site: &Site, out: &Path) -> Result<Report, WriteError> {
    ensure_replaceable(out)?;
    let name = out
        .file_name()
        .map_or_else(|| "site".to_owned(), |n| n.to_string_lossy().into_owned());
    let staging = out.with_file_name(format!("{name}.next"));
    if staging.exists() {
        ensure_replaceable(&staging)?;
        remove(&staging)?;
    }
    let mut report = Report {
        out: out.to_path_buf(),
        pages: 0,
        tags: site.tags.len(),
        files: 0,
        problems: site.problems.clone(),
    };
    write_into(site, &staging, &mut report)?;

    if out.exists() {
        let previous = out.with_file_name(format!("{name}.prev"));
        if previous.exists() {
            remove(&previous)?;
        }
        rename(out, &previous)?;
        let hook = SWAP_HOOK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(hook) = hook {
            hook(out);
        }
        rename(&staging, out)?;
        remove(&previous)?;
    } else {
        rename(&staging, out)?;
    }
    Ok(report)
}

fn write_into(site: &Site, dir: &Path, report: &mut Report) -> Result<(), WriteError> {
    create(dir)?;
    // Copied first, so a generated file wins over one of the same name.
    for folder in &site.static_dirs {
        report.files = report.files.saturating_add(copy_dir(folder, dir)?);
    }
    for rel in &site.attachments {
        // A folder's `index.html` (and the site's 404 page) is the
        // builder's own, served as a page this site wrote
        // (`architect::host::BUILT_PAGE`): a vault file is never copied
        // under that name.
        let name = rel.rsplit('/').next().unwrap_or(rel).to_ascii_lowercase();
        if name == "index.html" || name == "404.html" || name == crate::plain::NOTE_FILE {
            report.problems.push(crate::site::Problem {
                page: rel.clone(),
                message: "not published: a file named index.html or 404.html would be served as one of the site's own pages".to_owned(),
            });
            continue;
        }
        let from = site.root.join(rel);
        // The vault's own file as it is now, never through a symlink: one
        // swapped in since the vault was read is not followed.
        let Ok(mut source) = vault::obsidian::open_own(&site.root, rel) else {
            continue;
        };
        if let Some(to) = inside(dir, rel) {
            if let Some(parent) = to.parent() {
                create(parent)?;
            }
            let mut target =
                std::fs::File::create(&to).map_err(|e| WriteError::Io(to.clone(), e))?;
            std::io::copy(&mut source, &mut target).map_err(|e| WriteError::Io(from.clone(), e))?;
            report.files = report.files.saturating_add(1);
        }
    }

    let css = format!("{}\n{}", theme_css(&site.theme), site_css());
    let index = serde_json::to_string(&search::entries(site)).unwrap_or_else(|_| "[]".to_owned());
    let assets_dir = dir.join(ASSETS);
    create(&assets_dir)?;
    let asset = |stem: &str, ext: &str, body: &str| -> Result<String, WriteError> {
        let file = format!("{stem}.{}.{ext}", fingerprint(body.as_bytes()));
        put(&assets_dir.join(&file), body)?;
        Ok(format!("{}{ASSETS}/{file}", site.base))
    };
    let assets = Rc::new(Assets {
        css: asset("site", "css", &css)?,
        js: asset("site", "js", &site_js())?,
        search: asset("search", "json", &index)?,
    });

    let shared = Rc::new(site.clone());
    for (i, page) in site.pages.iter().enumerate() {
        let Some(at) = page_dir(dir, &site.base, &page.url) else {
            continue;
        };
        put(
            &at.join("index.html"),
            &ui::render(&shared, View::Page(i), &assets),
        )?;
        if site.page_data && page.rel_path.is_some() {
            let note = crate::plain::note(site, page);
            let json = facet_json::to_string(&note).map_err(|e| {
                WriteError::Io(
                    at.join(crate::plain::NOTE_FILE),
                    std::io::Error::other(e.to_string()),
                )
            })?;
            put(&at.join(crate::plain::NOTE_FILE), &json)?;
        }
        report.pages = report.pages.saturating_add(1);
    }
    let tags_dir = dir.join("tags");
    put(
        &tags_dir.join("index.html"),
        &ui::render(&shared, View::Tags, &assets),
    )?;
    for (t, (tag, _)) in site.tags.iter().enumerate() {
        if let Some(at) = page_dir(dir, &site.base, &site.tag_url(tag)) {
            put(
                &at.join("index.html"),
                &ui::render(&shared, View::Tag(t), &assets),
            )?;
            report.pages = report.pages.saturating_add(1);
        }
    }
    put(
        &dir.join("404.html"),
        &ui::render(&shared, View::NotFound, &assets),
    )?;
    if let Some(origin) = &site.url {
        put(&dir.join("sitemap.xml"), &sitemap(site, origin))?;
    }
    put(&dir.join(MARKER), "written by architect site build\n")?;
    Ok(())
}

/// Every page at its absolute URL.
fn sitemap(site: &Site, origin: &str) -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for page in &site.pages {
        let loc = crate::element_html::escape(&format!("{origin}{}", page.url));
        let _ = writeln!(xml, "  <url><loc>{loc}</loc></url>");
    }
    xml.push_str("</urlset>\n");
    xml
}

/// The folder a page at `url` is written to.
fn page_dir(dir: &Path, base: &str, url: &str) -> Option<PathBuf> {
    let rel = url.strip_prefix(base).unwrap_or(url);
    inside(dir, rel.trim_end_matches('/'))
}

/// `rel` under `dir`, refused if it would leave it.
fn inside(dir: &Path, rel: &str) -> Option<PathBuf> {
    let rel = Path::new(rel);
    rel.components()
        .all(|c| matches!(c, Component::Normal(_)))
        .then(|| dir.join(rel))
}

fn ensure_replaceable(out: &Path) -> Result<(), WriteError> {
    if !out.exists() || out.join(MARKER).exists() {
        return Ok(());
    }
    let empty = std::fs::read_dir(out)
        .map_err(|e| WriteError::Io(out.to_path_buf(), e))?
        .next()
        .is_none();
    if empty {
        Ok(())
    } else {
        Err(WriteError::NotASite(out.to_path_buf()))
    }
}

/// Copy `from`'s contents into `to`; how many files.
fn copy_dir(from: &Path, to: &Path) -> Result<usize, WriteError> {
    let mut copied = 0_usize;
    let entries = std::fs::read_dir(from).map_err(|e| WriteError::Io(from.to_path_buf(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| WriteError::Io(from.to_path_buf(), e))?;
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path.is_dir() {
            create(&target)?;
            copied = copied.saturating_add(copy_dir(&path, &target)?);
        } else {
            std::fs::copy(&path, &target).map_err(|e| WriteError::Io(path.clone(), e))?;
            copied = copied.saturating_add(1);
        }
    }
    Ok(copied)
}

fn put(path: &Path, body: &str) -> Result<(), WriteError> {
    if let Some(parent) = path.parent() {
        create(parent)?;
    }
    std::fs::write(path, body).map_err(|e| WriteError::Io(path.to_path_buf(), e))
}

fn create(dir: &Path) -> Result<(), WriteError> {
    std::fs::create_dir_all(dir).map_err(|e| WriteError::Io(dir.to_path_buf(), e))
}

fn remove(dir: &Path) -> Result<(), WriteError> {
    std::fs::remove_dir_all(dir).map_err(|e| WriteError::Io(dir.to_path_buf(), e))
}

fn rename(from: &Path, to: &Path) -> Result<(), WriteError> {
    std::fs::rename(from, to).map_err(|e| WriteError::Io(from.to_path_buf(), e))
}
