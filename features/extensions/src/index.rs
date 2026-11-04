//! A registry's index: what it offers, at which versions, and where each
//! package is — `index.toml` at the root of a registry's git repository
//! (Zed's `extensions` repository is the shape), kept by its CI.
//!
//! ```toml
//! name = "FastTrackStudio"
//!
//! [[extensions]]
//! id = "fts.metronome"
//! name = "Metronome"
//! version = "0.2.0"
//! kind = "luau"
//! description = "A click, and the actions to count in."
//! path = "metronome"            # a folder of this repository
//!
//! [[extensions]]
//! id = "acme.lyrics"
//! name = "Lyrics lane"
//! version = "1.4.0"
//! kind = "wasm"
//! git = "https://github.com/acme/lyrics-lane"
//! rev = "v1.4.0"                # its own repository, at a tag or commit
//!
//! [[extensions]]
//! id = "fts.studio"
//! name = "Studio"
//! version = "1.2.0"
//! kind = "process"
//! archive = "https://github.com/FastTrackStudios/studio/releases/download/ext-v1.2.0/fts.studio-1.2.0.archext"
//! sha256 = "…"                  # the archive's: a signed `.archext`
//! platforms = ["aarch64-apple-darwin", "x86_64-unknown-linux-gnu"]
//! changelog = "The mixer view."  # what an update shows
//! ```
//!
//! A `ReaPack` repository's `index.xml` reads into the same entries
//! ([`crate::reapack`]).

use architect_commands::fuzzy;
use semver::{Version, VersionReq};
use serde::Deserialize;

/// Where a listed package's files are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    /// A folder of the registry itself.
    Path(String),
    /// A git repository of its own, at a tag or a commit.
    Git { url: String, rev: String },
    /// Files to fetch (a `ReaPack` package): each its URL and where it goes
    /// in the package.
    Files(Vec<(String, String)>),
    /// A signed `.archext` archive: a path in the registry or a URL, and
    /// the archive's SHA-256 when the index pins it. Installed only once
    /// its signature is by a trusted key (`architect-extension-archive`).
    Archive { url: String, sha256: Option<String> },
}

/// One package at one version, as a registry lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub id: String,
    pub name: String,
    pub version: Version,
    /// `data`, `luau`, `wasm`, `process`; a `ReaPack` package's own type
    /// (`reapack.effect`, `reapack.script`) for the app's package kinds.
    pub kind: String,
    pub description: String,
    /// What it is filed under (`ReaPack`'s category).
    pub category: String,
    pub location: Location,
    /// The target triples it has builds for (a process extension's
    /// `bin/<triple>/`); empty for one that runs anywhere.
    pub platforms: Vec<String>,
    /// What this version changed, a line: what "update available" shows.
    pub changelog: String,
}

/// The target triples this platform runs, best first: what a listing's
/// [`Listing::platforms`] are matched against (the same table as
/// `architect_extension_archive::host_targets`).
#[must_use]
pub fn host_targets() -> &'static [&'static str] {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => &["x86_64-unknown-linux-gnu", "x86_64-unknown-linux-musl"],
        ("linux", "aarch64") => &["aarch64-unknown-linux-gnu", "aarch64-unknown-linux-musl"],
        ("macos", "aarch64") => &["aarch64-apple-darwin"],
        ("macos", "x86_64") => &["x86_64-apple-darwin"],
        ("windows", "x86_64") => &["x86_64-pc-windows-msvc", "x86_64-pc-windows-gnu"],
        ("windows", "aarch64") => &["aarch64-pc-windows-msvc"],
        _ => &[],
    }
}

impl Listing {
    /// Why it cannot run on a platform running `targets` (best first):
    /// it has builds, and none for this platform. `None` when it can.
    #[must_use]
    pub fn no_build_for(&self, targets: &[&str]) -> Option<String> {
        if self.platforms.is_empty() || self.platforms.iter().any(|p| targets.contains(&p.as_str())) {
            return None;
        }
        Some(format!(
            "no build for this platform ({}); it has builds for {}",
            targets.first().copied().unwrap_or("unknown"),
            self.platforms.join(", ")
        ))
    }
}

/// A registry's index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Index {
    pub name: String,
    pub listings: Vec<Listing>,
}

#[derive(Deserialize)]
struct IndexFile {
    #[serde(default)]
    name: String,
    #[serde(default)]
    extensions: Vec<ListingFile>,
}

#[derive(Deserialize)]
struct ListingFile {
    id: String,
    #[serde(default)]
    name: String,
    version: String,
    kind: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    git: Option<String>,
    #[serde(default)]
    rev: Option<String>,
    #[serde(default)]
    archive: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    platforms: Vec<String>,
    #[serde(default)]
    changelog: String,
}

/// Why an index could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexError(pub String);

impl std::fmt::Display for IndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the registry's index cannot be read: {}", self.0)
    }
}

impl std::error::Error for IndexError {}

/// One search result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub listing: Listing,
    pub score: i64,
}

impl Index {
    /// Reads `index.toml`'s text.
    ///
    /// # Errors
    ///
    /// It is not TOML, a version is not semver, or a listing says
    /// none of `path`, `git` + `rev`, `archive`.
    pub fn parse(text: &str) -> Result<Self, IndexError> {
        let file: IndexFile = toml::from_str(text).map_err(|e| IndexError(e.to_string()))?;
        let listings = file
            .extensions
            .into_iter()
            .map(|l| {
                let version = Version::parse(&l.version)
                    .map_err(|e| IndexError(format!("{} {}: {e}", l.id, l.version)))?;
                let location = match (l.path, l.git, l.rev, l.archive) {
                    (Some(path), None, _, None) => Location::Path(path),
                    (None, Some(url), Some(rev), None) => Location::Git { url, rev },
                    (None, None, _, Some(url)) => Location::Archive {
                        url,
                        sha256: l.sha256,
                    },
                    _ => {
                        return Err(IndexError(format!(
                            "{} {}: give `path`, `git` and `rev`, or `archive`",
                            l.id, l.version
                        )));
                    }
                };
                Ok(Listing {
                    name: if l.name.is_empty() {
                        l.id.clone()
                    } else {
                        l.name
                    },
                    id: l.id,
                    version,
                    kind: l.kind,
                    description: l.description,
                    category: l.category,
                    location,
                    platforms: l.platforms,
                    changelog: l.changelog,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            name: file.name,
            listings,
        })
    }

    /// The newest listing of `id` that `req` takes (any, for `*`).
    #[must_use]
    pub fn newest(&self, id: &str, req: &VersionReq) -> Option<&Listing> {
        self.listings
            .iter()
            .filter(|l| l.id == id && req.matches(&l.version))
            .max_by(|a, b| a.version.cmp(&b.version))
    }

    /// What `query` finds, best first, one hit an extension (its newest
    /// version): by name, then id, kind, category and description.
    /// Everything, newest of each, for an empty query.
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<Hit> {
        let query = query.trim();
        let mut newest: Vec<&Listing> = Vec::new();
        for l in &self.listings {
            match newest.iter_mut().find(|n| n.id == l.id) {
                Some(slot) if slot.version < l.version => *slot = l,
                Some(_) => {}
                None => newest.push(l),
            }
        }
        let mut hits: Vec<Hit> = newest
            .into_iter()
            .filter_map(|l| {
                let score = if query.is_empty() {
                    0
                } else {
                    let name = fuzzy(query, &l.name).map(|m| m.score.saturating_mul(2));
                    let id = fuzzy(query, &l.id).map(|m| m.score);
                    let words = [&l.kind, &l.category, &l.description]
                        .iter()
                        .any(|t| t.to_lowercase().contains(&query.to_lowercase()))
                        .then_some(8);
                    [name, id, words].into_iter().flatten().max()?
                };
                Some(Hit {
                    listing: l.clone(),
                    score,
                })
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.listing.name.cmp(&b.listing.name))
        });
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::{Index, IndexError, Location};
    use semver::VersionReq;

    const INDEX: &str = r#"
name = "Lab"

[[extensions]]
id = "fts.metronome"
name = "Metronome"
version = "0.1.0"
kind = "luau"
description = "A click"
path = "metronome"

[[extensions]]
id = "fts.metronome"
name = "Metronome"
version = "0.2.0"
kind = "luau"
description = "A click"
path = "metronome-0.2"

[[extensions]]
id = "acme.lyrics"
name = "Lyrics lane"
version = "1.4.0"
kind = "wasm"
git = "https://example.com/lyrics"
rev = "v1.4.0"
"#;

    #[test]
    fn an_index_finds_the_newest_that_fits() {
        let index = Index::parse(INDEX).unwrap();
        assert_eq!(index.name, "Lab");
        let any = VersionReq::STAR;
        assert_eq!(
            index
                .newest("fts.metronome", &any)
                .unwrap()
                .version
                .to_string(),
            "0.2.0"
        );
        let old = VersionReq::parse("=0.1").unwrap();
        assert_eq!(
            index.newest("fts.metronome", &old).unwrap().location,
            Location::Path("metronome".into())
        );
        assert!(index.newest("nope", &any).is_none());
    }

    #[test]
    fn search_lists_each_extension_once_best_first() {
        let index = Index::parse(INDEX).unwrap();
        let all = index.search("");
        assert_eq!(all.len(), 2);
        let hits = index.search("metro");
        assert_eq!(
            hits.first().map(|h| h.listing.id.as_str()),
            Some("fts.metronome")
        );
        assert_eq!(hits[0].listing.version.to_string(), "0.2.0");
        assert_eq!(index.search("wasm").len(), 1);
    }

    #[test]
    fn an_archive_listing_says_its_hash_platforms_and_changelog() {
        let text = format!(
            "{INDEX}\n[[extensions]]\nid = \"fts.studio\"\nversion = \"1.2.0\"\nkind = \"process\"\n\
             archive = \"archives/fts.studio-1.2.0.archext\"\nsha256 = \"ab\"\n\
             platforms = [\"aarch64-apple-darwin\"]\nchangelog = \"The mixer view.\"\n"
        );
        let index = Index::parse(&text).unwrap();
        let l = index.newest("fts.studio", &VersionReq::STAR).unwrap();
        assert_eq!(
            l.location,
            Location::Archive {
                url: "archives/fts.studio-1.2.0.archext".into(),
                sha256: Some("ab".into())
            }
        );
        assert_eq!(l.changelog, "The mixer view.");
        assert_eq!(l.no_build_for(&["aarch64-apple-darwin"]), None);
        assert_eq!(
            l.no_build_for(&["x86_64-unknown-linux-gnu"]).as_deref(),
            Some(
                "no build for this platform (x86_64-unknown-linux-gnu); it has builds for \
                 aarch64-apple-darwin"
            )
        );
    }

    #[test]
    fn a_listing_says_where_it_is() {
        let bad = INDEX.replace("path = \"metronome\"\n", "");
        assert!(matches!(Index::parse(&bad), Err(IndexError(_))));
    }
}
