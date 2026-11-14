//! A vault, read into a site: its pages in reading order, the navigation
//! tree, tags and backlinks. Pure data once read; [`crate::write()`]
//! writes it, [`crate::ui`] lays each page out.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

use vault::obsidian::{LinkIndex, OutgoingLink, Vault, VaultPage, page_tags};

use crate::bases::BaseBlocks;
use crate::blocks::Blocks;
use crate::frontmatter::{self, Meta};
use crate::markdown::{self, Heading, Input, Section, Target};
use crate::scope::Scope;
use crate::url::{
    decode_percent, encode_path, folder_url, folders, is_index_name, is_md, normalise_base,
    slug_segment,
};

/// How a site is built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// The vault (a folder of notes).
    pub vault: PathBuf,
    /// The site's name, in its header and every page's `<title>`. Empty:
    /// its home page's title, else its folder's name.
    pub title: String,
    /// One line about it, for the home page's `<meta>`.
    pub description: Option<String>,
    /// Where it is served (`/`, `/docs/`).
    pub base: String,
    /// The theme family: `<theme>-light` and `<theme>-dark` of
    /// architect's built-in themes.
    pub theme: String,
    /// Where it is published (`https://architect.fasttrackstudio.app`),
    /// for its sitemap. None: no sitemap.
    pub url: Option<String>,
    /// A live-reload event stream to listen on (`architect site serve`).
    pub live_reload: Option<String>,
    /// Fail on any [`Problem`] instead of reporting it.
    pub strict: bool,
    /// Folders copied into the site as they are (a docs `static/`).
    pub static_dirs: Vec<PathBuf>,
    /// Draw ```` ```base ```` fences as their views over the vault
    /// ([`crate::BaseBlocks`]), unless a renderer the caller gave claims
    /// them first. On by default.
    pub bases: bool,
    /// Which notes are published ([`Scope`]): the whole vault by
    /// default. A link to a note outside it is plain text.
    pub scope: Scope,
    /// A Bases filter expression over the scope's notes (#510,
    /// [`crate::listing::parse_filter`]): only the notes it holds for
    /// are published, the rest are outside the site as notes outside
    /// the scope are. None: every note of the scope.
    pub filter: Option<String>,
    /// Attachments (vault paths) the site holds back (#528: a share link's
    /// files above its layer): never copied into it, and no link, embed,
    /// player, lyrics, search entry or listing of the site mentions one; a
    /// link to one is dropped with its text. Nothing held back is ever on
    /// disk under the site, so no request can reach its bytes there.
    pub hold_back: BTreeSet<String>,
    /// Write each page's prose as plain blocks too (`<page>/_note.json`
    /// in the data folder beside the site, [`crate::data_dir`],
    /// [`crate::plain`]): what a share link's guest reads in the app
    /// (#629). Off by default.
    pub page_data: bool,
}

impl Options {
    /// The vault at `vault`, published at `/` under `title`, in
    /// architect's own theme.
    #[must_use]
    pub fn new(vault: impl Into<PathBuf>, title: impl Into<String>) -> Self {
        Self {
            vault: vault.into(),
            title: title.into(),
            description: None,
            base: "/".to_owned(),
            theme: "fasttrackstudio".to_owned(),
            url: None,
            live_reload: None,
            strict: false,
            static_dirs: Vec::new(),
            bases: true,
            scope: Scope::Vault,
            filter: None,
            hold_back: BTreeSet::new(),
            page_data: false,
        }
    }
}

/// One page of the site: a note, or a folder with no note of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// The note's vault path; `None` for a folder's generated page.
    pub rel_path: Option<String>,
    /// Its URL, ending in `/`.
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    /// Lower case, sorted.
    pub tags: Vec<String>,
    /// The finished prose.
    pub html: String,
    pub headings: Vec<Heading>,
    /// Its text by heading, for search.
    pub sections: Vec<Section>,
    pub words: u32,
    /// Pages linking here (indices into [`Site::pages`]), in reading order.
    pub backlinks: Vec<usize>,
    /// The folders above it, outermost first, home included.
    pub crumbs: Vec<Crumb>,
    /// A folder's page: what is in it, in reading order.
    pub children: Vec<usize>,
    /// Whether it is a folder's page.
    pub is_section: bool,
    /// Its text as a share link's guest's view is handed it (#648, #669):
    /// written only with [`Options::page_data`], and only as the page
    /// shows it (`crate::guest_text`).
    pub source: Option<String>,
}

/// A step of a page's breadcrumbs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    pub title: String,
    pub url: String,
}

/// The navigation tree: a page and, for a folder's, what is in it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NavNode {
    /// Index into [`Site::pages`].
    pub page: usize,
    pub children: Vec<Self>,
}

/// Something a build should not ship, against the note it is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The note's vault path.
    pub page: String,
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.page, self.message)
    }
}

/// Why a vault could not be read into a site.
#[derive(Debug, thiserror::Error)]
pub enum SiteError {
    #[error("no vault at {0}")]
    Missing(PathBuf),
    #[error("could not read the vault at {0}: {1}")]
    Unreadable(PathBuf, String),
    #[error("the vault at {0} has no notes to publish")]
    Empty(PathBuf),
    #[error("{} problem(s) in a strict build:\n{}", .0.len(), .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"))]
    Problems(Vec<Problem>),
    #[error("the filter is not a Bases filter expression: {0}")]
    Filter(String),
}

/// A built site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    pub title: String,
    pub description: Option<String>,
    /// Starts and ends in `/`.
    pub base: String,
    pub theme: String,
    pub url: Option<String>,
    pub live_reload: Option<String>,
    /// Whether each page's plain blocks are written beside it
    /// ([`Options::page_data`]).
    pub page_data: bool,
    /// Every page, in reading order; the home page first.
    pub pages: Vec<Page>,
    /// The tree the sidebar draws; its root is the home page.
    pub nav: NavNode,
    /// Every tag, alphabetical, with its pages in reading order.
    pub tags: Vec<(String, Vec<usize>)>,
    pub problems: Vec<Problem>,
    /// The vault (attachments are copied from it).
    pub root: PathBuf,
    /// Attachments linked or embedded from a page, by vault path.
    pub attachments: Vec<String>,
    pub static_dirs: Vec<PathBuf>,
}

/// A folder of the vault while the tree is built.
#[derive(Default)]
struct Folder {
    /// Its own note (`_index.md`, `index.md`, `README.md`).
    index: Option<usize>,
    notes: Vec<usize>,
    folders: BTreeMap<String, Self>,
}

/// What a folder holds, while it is laid out.
enum Item<'a> {
    Note(usize),
    Folder(&'a str, &'a Folder),
}

/// A note while the site is read: the vault's page and what it says.
struct Note<'v> {
    page: &'v VaultPage,
    meta: Meta,
    title: String,
}

impl Site {
    /// Read the vault in `opts` and draw every page.
    ///
    /// # Errors
    ///
    /// The vault is missing, unreadable or has nothing to publish, or a
    /// strict build has problems.
    pub fn read(opts: &Options, blocks: &Blocks) -> Result<Self, SiteError> {
        if !opts.vault.is_dir() {
            return Err(SiteError::Missing(opts.vault.clone()));
        }
        let vault = Vault::open(&opts.vault)
            .map_err(|e| SiteError::Unreadable(opts.vault.clone(), e.to_string()))?;
        let notes: Vec<Note<'_>> = vault
            .pages
            .iter()
            .map(|page| {
                let meta = Meta::of(page);
                let title = title_of(page, &meta);
                Note { page, meta, title }
            })
            .filter(|n| !n.meta.hidden)
            .collect();
        let filter = opts
            .filter
            .as_deref()
            .map(crate::listing::parse_filter)
            .transpose()
            .map_err(SiteError::Filter)?
            .filter(|f| *f != vault::bases::FilterNode::None);
        let scoped = !opts.scope.is_vault() || filter.is_some();
        let notes = if scoped {
            let wanted: BTreeSet<String> = filter.as_ref().map_or_else(
                || opts.scope.notes(&vault),
                |filter| {
                    crate::listing::select(&vault, &opts.scope, filter)
                        .into_iter()
                        .map(|e| e.path)
                        .collect()
                },
            );
            notes
                .into_iter()
                .filter(|n| wanted.contains(&n.page.rel_path))
                .collect()
        } else {
            notes
        };
        if notes.is_empty() {
            return Err(SiteError::Empty(opts.vault.clone()));
        }

        let base = normalise_base(&opts.base);
        let title = if opts.title.trim().is_empty() {
            notes
                .iter()
                .find(|n| {
                    opts.scope.is_one_note()
                        || !opts.scope.shown(&n.page.rel_path).contains('/')
                            && is_index_name(&n.page.basename)
                })
                .map(|n| n.title.clone())
                .or_else(|| opts.scope.folder()?.rsplit('/').next().map(deslug))
                .or_else(|| {
                    let root = opts.vault.canonicalize().ok()?;
                    Some(root.file_name()?.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "Site".to_owned())
        } else {
            opts.title.clone()
        };
        let mut site = Self {
            title,
            description: opts.description.clone(),
            base: base.clone(),
            theme: opts.theme.clone(),
            url: opts
                .url
                .as_ref()
                .map(|u| u.trim_end_matches('/').to_owned()),
            live_reload: opts.live_reload.clone(),
            page_data: opts.page_data,
            pages: Vec::new(),
            nav: NavNode::default(),
            tags: Vec::new(),
            problems: Vec::new(),
            root: opts.vault.clone(),
            attachments: Vec::new(),
            static_dirs: opts.static_dirs.clone(),
        };

        // The folder tree, then the pages in reading order.
        let mut tree = Folder::default();
        for (i, note) in notes.iter().enumerate() {
            let mut at = &mut tree;
            for folder in folders(opts.scope.shown(&note.page.rel_path)) {
                at = at.folders.entry(folder.to_owned()).or_default();
            }
            let home = is_index_name(&note.page.basename) || opts.scope.is_one_note();
            if home && at.index.is_none() {
                at.index = Some(i);
            } else {
                at.notes.push(i);
            }
        }
        let mut note_of: Vec<Option<usize>> = Vec::new();
        site.nav = site.lay_out(&tree, &notes, &[], None, &mut note_of);

        // Every URL is one page's.
        let mut url_of: HashMap<&str, String> = HashMap::new();
        let mut taken: HashMap<String, usize> = HashMap::new();
        for (i, page) in site.pages.iter().enumerate() {
            if let Some(&first) = taken.get(&page.url) {
                let other = site.pages.get(first).and_then(|p| p.rel_path.clone());
                site.problems.push(Problem {
                    page: page.rel_path.clone().unwrap_or_default(),
                    message: format!(
                        "published at {}, which {} already is",
                        page.url,
                        other.unwrap_or_default()
                    ),
                });
            } else {
                taken.insert(page.url.clone(), i);
            }
        }
        for (i, note) in note_of.iter().enumerate() {
            if let (Some(n), Some(page)) = (note, site.pages.get(i)) {
                if let Some(note) = notes.get(*n) {
                    url_of.insert(note.page.rel_path.as_str(), page.url.clone());
                }
            }
        }

        // Task's rule, kept for a wikilink the index leaves unresolved: a
        // note answers to its file name and its title, any case, spaces
        // and dashes alike (`[[Key Meter Changes]]` is
        // `key-meter-changes.md`). The file name wins over a title.
        let mut by_name: HashMap<String, String> = HashMap::new();
        for note in &notes {
            if let Some(url) = url_of.get(note.page.rel_path.as_str()) {
                by_name
                    .entry(loose(&note.page.basename))
                    .or_insert_with(|| url.clone());
            }
        }
        for note in &notes {
            if let Some(url) = url_of.get(note.page.rel_path.as_str()) {
                by_name
                    .entry(loose(&note.title))
                    .or_insert_with(|| url.clone());
            }
        }

        // Bases are the vault's own: drawn over it, after the caller's
        // renderers, so an extension claiming `fence:base` wins.
        let mut blocks = blocks.clone();
        if opts.bases {
            let pages = notes
                .iter()
                .filter_map(|n| {
                    let url = url_of.get(n.page.rel_path.as_str())?;
                    Some((n.page.rel_path.clone(), (url.clone(), n.title.clone())))
                })
                .collect();
            let bases = BaseBlocks::new(&vault, pages);
            let bases = if scoped {
                bases.published_only()
            } else {
                bases
            };
            blocks.add(std::sync::Arc::new(bases));
        }
        // Synced lyrics (```` ```lrc ````), after the caller's too (#511).
        let blocks = &crate::LyricsBlocks::added_to(&blocks);

        // Draw every note, its links resolved by the vault's index.
        let index = LinkIndex::build(&vault);
        let attachments: BTreeSet<&str> = vault
            .attachments
            .iter()
            .map(|a| a.rel_path.as_str())
            .collect();
        let mut used_attachments: BTreeSet<String> = BTreeSet::new();
        let page_of_rel: HashMap<&str, usize> = note_of
            .iter()
            .enumerate()
            .filter_map(|(i, n)| Some((notes.get((*n)?)?.page.rel_path.as_str(), i)))
            .collect();
        let mut linked: Vec<(usize, Vec<String>)> = Vec::new();
        // The notes with a page: what a listing fence's row may name.
        let published: Vec<&str> = url_of.keys().copied().collect();
        for (i, note_index) in note_of.iter().enumerate() {
            let Some(note) = note_index.and_then(|n| notes.get(n)) else {
                continue;
            };
            let rel = note.page.rel_path.as_str();
            // Its listing fences (a collection's rows, #668) cut before
            // anything is drawn from them: a row naming a note outside
            // the site is gone, a file the site holds back is emptied
            // from a row kept (`architect_media::fence::within`). The
            // page, its source, its plain blocks and an extension's drawn
            // block all read the cut text.
            let folder = architect_media::link::folder_of(rel);
            let body = architect_media::fence::within(
                frontmatter::body(&note.page.raw),
                &|p| architect_media::link::resolve(p, folder, published.iter().copied()).is_some(),
                &|p| {
                    architect_media::link::resolve(p, folder, attachments.iter().copied())
                        .is_some_and(|t| opts.scope.holds_file(t) && !opts.hold_back.contains(t))
                },
            );
            let outgoing = index.outgoing(rel);
            let resolve = |dest: &str, wiki: bool| match resolve_link(
                dest,
                wiki,
                &outgoing,
                &url_of,
                &attachments,
                &base,
                &opts.scope,
                &opts.hold_back,
            ) {
                Target::Broken if wiki => {
                    let name = dest.split('#').next().unwrap_or_default();
                    by_name
                        .get(&loose(name))
                        .map_or(Target::Broken, |url| Target::Page(url.clone()))
                }
                other => other,
            };
            // An embedded `.lrc` the scope holds: its text, to draw its
            // lines (#511).
            let text = |dest: &str, _wiki: bool| {
                let decoded = decode_percent(dest);
                let wanted = decoded.split('#').next().unwrap_or_default().trim();
                let target = outgoing
                    .iter()
                    .find(|l| l.linkpath == dest || l.linkpath == decoded || l.linkpath == wanted)
                    .and_then(|l| l.resolved)
                    .filter(|t| {
                        crate::media::is_lyrics(t)
                            && attachments.contains(t)
                            && opts.scope.holds_file(t)
                            && !opts.hold_back.contains(*t)
                    })?;
                // Through the vault's own open: a lyrics file swapped for a
                // symlink since the walk is left out, as every other read.
                let mut text = String::new();
                std::io::Read::read_to_string(
                    &mut vault::obsidian::open_own(&opts.vault, target).ok()?,
                    &mut text,
                )
                .ok()?;
                Some(text)
            };
            // The text a guest's view is handed, from the same resolution
            // the page is drawn with: nothing the page does not show.
            let source = opts
                .page_data
                .then(|| crate::guest_text::guest_text(&body, &resolve));
            let rendered = markdown::render(&Input {
                body: &body,
                page: rel,
                title: &note.title,
                resolve: &resolve,
                blocks,
                text: &text,
            });
            for broken in &rendered.broken {
                site.problems.push(Problem {
                    page: rel.to_owned(),
                    message: format!("links to `{broken}`, which is not in the site"),
                });
            }
            for (claim, why) in &rendered.block_errors {
                site.problems.push(Problem {
                    page: rel.to_owned(),
                    message: format!("its {claim} block could not be drawn: {why}"),
                });
            }
            for link in &outgoing {
                if let Some(target) = link.resolved.filter(|t| {
                    attachments.contains(t)
                        && opts.scope.holds_file(t)
                        && !opts.hold_back.contains(*t)
                }) {
                    used_attachments.insert(target.to_owned());
                }
            }
            // Its art is one of its files, as a link in its body is.
            if let Some(art) = note
                .meta
                .art
                .as_deref()
                .and_then(|link| {
                    crate::art::resolve(
                        link,
                        crate::art::folder_of(rel),
                        attachments.iter().copied(),
                    )
                })
                .filter(|t| opts.scope.holds_file(t) && !opts.hold_back.contains(*t))
            {
                used_attachments.insert(art.to_owned());
            }
            // So is the file it plays as a media item (`media:`, else
            // `recording:`; #668), as the listing names it
            // (`crate::listing::Entry::files`): held back, as any file, from
            // a link below its layer.
            if let Some(media) = note
                .meta
                .media
                .as_deref()
                .and_then(|link| {
                    architect_media::link::resolve(
                        link,
                        architect_media::link::folder_of(rel),
                        attachments.iter().copied(),
                    )
                })
                .filter(|t| {
                    crate::media::media_kind(t).is_some()
                        && opts.scope.holds_file(t)
                        && !opts.hold_back.contains(*t)
                })
            {
                used_attachments.insert(media.to_owned());
            }
            let mut tags: BTreeSet<String> = page_tags(note.page)
                .into_iter()
                .map(|t| t.to_lowercase())
                .collect();
            tags.extend(note.meta.tags.iter().cloned());
            let backlinks: BTreeSet<usize> = index
                .backlinks(rel)
                .into_iter()
                .filter_map(|from| page_of_rel.get(from).copied())
                .filter(|&from| from != i)
                .collect();
            linked.push((i, rendered.links));
            if let Some(page) = site.pages.get_mut(i) {
                page.html = rendered.html;
                page.headings = rendered.headings;
                page.sections = rendered.sections;
                page.words = rendered.words;
                page.tags = tags.into_iter().collect();
                page.backlinks = backlinks.into_iter().collect();
                page.source = source;
            }
        }
        site.attachments = used_attachments.into_iter().collect();

        // A link the index resolved is already a backlink; one resolved by
        // name above is added here, so the two never disagree.
        let page_at: HashMap<String, usize> = site
            .pages
            .iter()
            .enumerate()
            .map(|(i, p)| (p.url.clone(), i))
            .collect();
        for (from, urls) in linked {
            for url in urls {
                let Some(&to) = page_at.get(&url) else {
                    continue;
                };
                if let Some(page) = site.pages.get_mut(to) {
                    if to != from && !page.backlinks.contains(&from) {
                        page.backlinks.push(from);
                        page.backlinks.sort_unstable();
                    }
                }
            }
        }

        let mut tags: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, page) in site.pages.iter().enumerate() {
            for tag in &page.tags {
                tags.entry(tag.clone()).or_default().push(i);
            }
        }
        site.tags = tags.into_iter().collect();

        if opts.strict && !site.problems.is_empty() {
            return Err(SiteError::Problems(site.problems));
        }
        Ok(site)
    }

    /// Push `folder`'s page and everything in it, in reading order; its
    /// node in the navigation tree.
    fn lay_out(
        &mut self,
        folder: &Folder,
        notes: &[Note<'_>],
        path: &[&str],
        parent: Option<usize>,
        note_of: &mut Vec<Option<usize>>,
    ) -> NavNode {
        let own = folder.index.and_then(|i| notes.get(i));
        let title = own.map_or_else(
            || {
                path.last()
                    .map_or_else(|| self.title.clone(), |name| deslug(name))
            },
            |n| n.title.clone(),
        );
        let at = self.push_page(
            Page {
                rel_path: own.map(|n| n.page.rel_path.clone()),
                url: folder_url(&self.base, path),
                title,
                description: own.and_then(|n| n.meta.description.clone()),
                is_section: true,
                ..self.blank_page()
            },
            parent,
        );
        note_of.push(folder.index);

        // What is in it: notes and folders, by weight then title.
        let mut items: Vec<(i64, String, Item<'_>)> = Vec::new();
        for &n in &folder.notes {
            if let Some(note) = notes.get(n) {
                items.push((
                    note.meta.weight.unwrap_or(i64::MAX),
                    note.title.to_lowercase(),
                    Item::Note(n),
                ));
            }
        }
        for (name, sub) in &folder.folders {
            let own = sub.index.and_then(|i| notes.get(i));
            items.push((
                own.and_then(|n| n.meta.weight).unwrap_or(i64::MAX),
                own.map_or_else(|| deslug(name), |n| n.title.clone())
                    .to_lowercase(),
                Item::Folder(name, sub),
            ));
        }
        items.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));

        let mut node = NavNode {
            page: at,
            children: Vec::new(),
        };
        for (_, _, item) in items {
            let child = match item {
                Item::Note(n) => {
                    let Some(entry) = notes.get(n) else { continue };
                    let mut url = folder_url(&self.base, path);
                    url.push_str(&slug_segment(&entry.page.basename));
                    url.push('/');
                    let i = self.push_page(
                        Page {
                            rel_path: Some(entry.page.rel_path.clone()),
                            url,
                            title: entry.title.clone(),
                            description: entry.meta.description.clone(),
                            ..self.blank_page()
                        },
                        Some(at),
                    );
                    note_of.push(Some(n));
                    NavNode {
                        page: i,
                        children: Vec::new(),
                    }
                }
                Item::Folder(name, sub) => {
                    let mut deeper = path.to_vec();
                    deeper.push(name);
                    self.lay_out(sub, notes, &deeper, Some(at), note_of)
                }
            };
            if let Some(page) = self.pages.get_mut(at) {
                page.children.push(child.page);
            }
            node.children.push(child);
        }
        node
    }

    /// Push `page` under `parent`, with its breadcrumbs; its index.
    fn push_page(&mut self, mut page: Page, parent: Option<usize>) -> usize {
        if let Some(up) = parent.and_then(|p| self.pages.get(p)) {
            page.crumbs.clone_from(&up.crumbs);
            page.crumbs.push(Crumb {
                title: up.title.clone(),
                url: up.url.clone(),
            });
        }
        self.pages.push(page);
        self.pages.len().saturating_sub(1)
    }

    fn blank_page(&self) -> Page {
        Page {
            rel_path: None,
            url: self.base.clone(),
            title: String::new(),
            description: None,
            tags: Vec::new(),
            html: String::new(),
            headings: Vec::new(),
            sections: Vec::new(),
            words: 0,
            backlinks: Vec::new(),
            crumbs: Vec::new(),
            children: Vec::new(),
            is_section: false,
            source: None,
        }
    }

    /// The page before `i` in reading order.
    #[must_use]
    pub fn previous(&self, i: usize) -> Option<&Page> {
        self.pages.get(i.checked_sub(1)?)
    }

    /// The page after `i` in reading order.
    #[must_use]
    pub fn next(&self, i: usize) -> Option<&Page> {
        self.pages.get(i.checked_add(1)?)
    }

    /// A tag's page.
    #[must_use]
    pub fn tag_url(&self, tag: &str) -> String {
        format!("{}tags/{}/", self.base, slug_segment(tag))
    }

    /// The page at `url`, if any.
    #[must_use]
    pub fn page_at(&self, url: &str) -> Option<&Page> {
        self.pages.iter().find(|p| p.url == url)
    }
}

/// Where the link `dest` from a note goes. The vault's index resolved
/// it (matched by what was written, then without its anchor); this maps
/// what it resolved to onto the site. In a scoped site a note that is
/// not published, or an attachment the scope does not hold, is outside
/// it: plain text, never reported.
fn resolve_link(
    dest: &str,
    wiki: bool,
    outgoing: &[OutgoingLink<'_>],
    url_of: &HashMap<&str, String>,
    attachments: &BTreeSet<&str>,
    base: &str,
    scope: &Scope,
    hold_back: &BTreeSet<String>,
) -> Target {
    let scoped = !scope.is_vault();
    let decoded = decode_percent(dest);
    let path_of = |s: &str| s.split('#').next().unwrap_or_default().trim().to_owned();
    let wanted = path_of(&decoded);
    let link = outgoing
        .iter()
        .find(|l| l.linkpath == dest || l.linkpath == decoded)
        .or_else(|| outgoing.iter().find(|l| path_of(l.linkpath) == wanted));
    if let Some(target) = link.and_then(|l| l.resolved) {
        if let Some(url) = url_of.get(target) {
            return Target::Page(url.clone());
        }
        if attachments.contains(target) {
            if !scope.holds_file(target) {
                return Target::Outside;
            }
            if hold_back.contains(target) {
                return Target::Held;
            }
            return Target::File(format!("{base}{}", encode_path(target)));
        }
        // A note that is not published: a draft, or outside the scope.
        return if scoped {
            Target::Outside
        } else {
            Target::Broken
        };
    }
    let external = dest.contains("://") || dest.starts_with("mailto:") || dest.starts_with("//");
    if external {
        Target::Keep
    } else if wiki || is_md(&wanted) || wanted.starts_with("@/") {
        Target::Broken
    } else {
        Target::Keep
    }
}

/// A name as a loose key: lower case, `-` and `_` as spaces, one space
/// between words.
fn loose(name: &str) -> String {
    name.to_lowercase()
        .replace(['-', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A note's title: its frontmatter's, else its first heading, else its
/// file name as words.
pub fn title_of(page: &VaultPage, meta: &Meta) -> String {
    meta.title
        .clone()
        .or_else(|| first_h1(frontmatter::body(&page.raw)))
        .unwrap_or_else(|| deslug(&page.basename))
}

/// A note's first `# heading`, as its title when it names none. A line
/// inside fenced code is no heading: a listing fence's `# ` row (a private
/// set's name, cut from the page) must not become the page's title.
fn first_h1(body: &str) -> Option<String> {
    // The open fence's character and length, while inside one.
    let mut fence: Option<(char, usize)> = None;
    for line in body.lines() {
        let trimmed = line.trim_start_matches(' ');
        let marker = (line.len().saturating_sub(trimmed.len()) <= 3)
            .then(|| trimmed.chars().next())
            .flatten()
            .filter(|c| *c == '`' || *c == '~')
            .map(|c| (c, trimmed.chars().take_while(|x| *x == c).count()))
            .filter(|(_, n)| *n >= 3);
        match (fence, marker) {
            (None, Some(open)) => fence = Some(open),
            (Some((c, n)), Some((m, k))) if c == m && k >= n => fence = None,
            (Some(_), _) => {}
            (None, None) => {
                if let Some(t) = line.strip_prefix("# ") {
                    let t = t.trim();
                    if !t.is_empty() {
                        return Some(t.to_owned());
                    }
                }
            }
        }
    }
    None
}

/// A file or folder name as a title: `getting-started` → `Getting started`.
fn deslug(name: &str) -> String {
    let spaced = name.replace(['-', '_'], " ");
    let mut chars = spaced.trim().chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_from_names() {
        assert_eq!(deslug("getting-started"), "Getting started");
        assert_eq!(first_h1("Intro\n# The UI\n"), Some("The UI".to_owned()));
        // A heading inside fenced code is not the title.
        assert_eq!(
            first_h1("```studio-setlist\n# Secret Encore\n1 | x\n```\n"),
            None
        );
        assert_eq!(
            first_h1("~~~~\n# Hidden\n~~~\n# Not yet\n~~~~\n# Shown\n"),
            Some("Shown".to_owned())
        );
    }
}
