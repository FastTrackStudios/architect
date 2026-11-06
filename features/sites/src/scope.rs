//! Which notes of a vault a site publishes: all of them, or one part
//! ([`Scope`]). A share link (#374) is a scoped site.
//!
//! A scoped site is read from the whole vault, so its links still
//! resolve, but only the notes in the scope are pages. Everything else
//! is outside it, and never reaches the site: a link to a note outside
//! is plain text (no URL, no title), a backlink from one is not listed,
//! a base's rows are only the scope's notes, and an attachment is
//! copied only when a note in the scope links or embeds it.

use std::collections::BTreeSet;

use vault::bases::view_table;
use vault::obsidian::base_query::vault_to_rows;
use vault::obsidian::{Vault, VaultPage, page_tags};

use crate::frontmatter::Meta;

/// The part of a vault a site publishes.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// Every note.
    #[default]
    Vault,
    /// A folder (`Projects/Alpha`) and everything under it. The site is
    /// rooted there: the folder's own note is its home page.
    Folder(String),
    /// Every note with this tag (`#mix`, or a nested `#mix/final`
    /// under `mix`), any case.
    Tag(String),
    /// These notes, by vault path.
    Notes(Vec<String>),
    /// The notes a `.base` file's views select (by its vault path or
    /// file name).
    Base(String),
}

impl Scope {
    /// Whether this is the whole vault.
    #[must_use]
    pub const fn is_vault(&self) -> bool {
        matches!(self, Self::Vault)
    }

    /// The folder a [`Scope::Folder`] names, without slashes at either
    /// end; `None` for the vault root or any other scope.
    #[must_use]
    pub fn folder(&self) -> Option<&str> {
        match self {
            Self::Folder(f) => Some(f.trim_matches('/')).filter(|f| !f.is_empty()),
            _ => None,
        }
    }

    /// The vault paths of the notes in the scope.
    #[must_use]
    pub fn notes(&self, vault: &Vault) -> BTreeSet<String> {
        let base_rows = match self {
            Self::Base(name) => Some(base_paths(vault, name)),
            _ => None,
        };
        vault
            .pages
            .iter()
            .filter(|page| {
                base_rows
                    .as_ref()
                    .map_or_else(|| self.holds(page), |rows| rows.contains(&page.rel_path))
            })
            .map(|page| page.rel_path.clone())
            .collect()
    }

    /// Whether `page` is in the scope (not for [`Scope::Base`], which
    /// needs the whole vault: [`Self::notes`]).
    fn holds(&self, page: &VaultPage) -> bool {
        match self {
            Self::Vault => true,
            Self::Folder(_) => self.folder().is_none_or(|folder| {
                page.rel_path
                    .strip_prefix(folder)
                    .is_some_and(|rest| rest.starts_with('/'))
            }),
            Self::Tag(tag) => {
                let want = tag.trim().trim_start_matches('#').to_lowercase();
                !want.is_empty()
                    && page_tags(page)
                        .into_iter()
                        .chain(Meta::of(page).tags)
                        .map(|t| t.trim_start_matches('#').to_lowercase())
                        .any(|t| {
                            t == want || t.strip_prefix(&want).is_some_and(|r| r.starts_with('/'))
                        })
            }
            Self::Notes(paths) => paths.iter().any(|p| {
                let p = p.trim_matches('/');
                p == page.rel_path
                    || p.strip_suffix(".md").is_none() && format!("{p}.md") == page.rel_path
            }),
            Self::Base(_) => false,
        }
    }

    /// Whether it is one note, which is then the site's home page.
    #[must_use]
    pub const fn is_one_note(&self) -> bool {
        matches!(self, Self::Notes(n) if n.len() == 1)
    }

    /// `rel_path` as the site lays it out: under a folder scope, its path
    /// below that folder; a one-note scope's note at the root.
    #[must_use]
    pub fn shown<'a>(&self, rel_path: &'a str) -> &'a str {
        if self.is_one_note() {
            return rel_path.rsplit('/').next().unwrap_or(rel_path);
        }
        self.folder()
            .and_then(|folder| rel_path.strip_prefix(folder))
            .and_then(|rest| rest.strip_prefix('/'))
            .unwrap_or(rel_path)
    }

    /// Whether attachment `rel_path` may be published: under a folder
    /// scope only the folder's own; under any other, any one a note in
    /// the scope links to.
    #[must_use]
    pub fn holds_file(&self, rel_path: &str) -> bool {
        self.folder().is_none_or(|folder| {
            rel_path
                .strip_prefix(folder)
                .is_some_and(|rest| rest.starts_with('/'))
        })
    }
}

/// The vault paths every view of base `name` selects.
fn base_paths(vault: &Vault, name: &str) -> BTreeSet<String> {
    let name = name.trim().trim_matches('/');
    let file = vault.bases.iter().find(|b| {
        b.rel_path == name
            || format!("{}.base", b.basename) == name
            || b.basename == name
            || b.rel_path.strip_suffix(".base") == Some(name)
    });
    let Some(Ok(base)) = file.map(|b| &b.parsed) else {
        return BTreeSet::new();
    };
    let rows = vault_to_rows(vault);
    base.views
        .iter()
        .flat_map(|view| {
            view_table(base, view, &rows)
                .groups
                .into_iter()
                .flat_map(|g| g.rows.into_iter().map(|r| r.path))
        })
        .collect()
}
