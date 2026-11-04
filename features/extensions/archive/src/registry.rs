//! A registry's `index.toml`, checked the way its CI checks it.
//!
//! `cargo xtask ext-index` runs it: the schema (`index.schema.json` in
//! `examples/registry/` says the same), and for each archive the hash the
//! index pins, the signature against the trusted keys, and that the
//! archive is what its entry says (id, version, kind, platforms).
//!
//! ```toml
//! name = "FastTrackStudios"
//!
//! [[extensions]]
//! id = "fts.studio"
//! name = "Studio"
//! version = "1.2.0"
//! kind = "process"
//! description = "REAPER, from the app."
//! archive = "https://github.com/FastTrackStudios/studio/releases/download/ext-v1.2.0/fts.studio-1.2.0.archext"
//! sha256 = "…"                     # the archive's, lower-case hex
//! platforms = ["aarch64-apple-darwin", "x86_64-unknown-linux-gnu"]
//! changelog = "The mixer view; faster project open."
//! ```

use sha2::Digest as _;

use crate::{Trust, open};

/// The keys an `[[extensions]]` entry may have.
pub const KEYS: &[&str] = &[
    "id",
    "name",
    "version",
    "kind",
    "description",
    "category",
    "path",
    "git",
    "rev",
    "archive",
    "sha256",
    "platforms",
    "changelog",
];

/// The kinds an entry may be.
pub const KINDS: &[&str] = &["data", "luau", "wasm", "process"];

/// What a check found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Entries read.
    pub entries: usize,
    /// Archives fetched and verified.
    pub archives: usize,
    /// Each problem, `<id> <version>: what`.
    pub problems: Vec<String>,
}

impl Report {
    #[must_use]
    pub const fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

fn is_id(id: &str) -> bool {
    id.contains('.')
        && !id.starts_with('.')
        && !id.ends_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c))
}

fn string<'a>(t: &'a toml::Table, key: &str) -> Option<&'a str> {
    t.get(key).and_then(toml::Value::as_str)
}

/// Checks an `index.toml`'s text. `fetch` gets an archive's bytes by the
/// entry's `archive` (a path relative to the registry, or a URL).
///
/// # Errors
///
/// It is not TOML: nothing more can be checked.
pub fn check(
    text: &str,
    fetch: &dyn Fn(&str) -> Result<Vec<u8>, String>,
    trust: &Trust,
) -> Result<Report, String> {
    let doc: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut report = Report::default();
    for key in doc.keys() {
        if key != "name" && key != "extensions" {
            report.problems.push(format!("the index: unknown key `{key}`"));
        }
    }
    if doc.get("name").is_some_and(|n| !n.is_str()) {
        report.problems.push("the index: `name` is not a string".to_owned());
    }
    let entries = match doc.get("extensions") {
        None => return Ok(report),
        Some(toml::Value::Array(a)) => a.clone(),
        Some(_) => {
            report
                .problems
                .push("the index: `extensions` is not an array of tables".to_owned());
            return Ok(report);
        }
    };
    let mut seen: Vec<(String, String)> = Vec::new();
    for (n, entry) in entries.iter().enumerate() {
        report.entries = report.entries.saturating_add(1);
        let Some(t) = entry.as_table() else {
            report.problems.push(format!("entry {n}: not a table"));
            continue;
        };
        let id = string(t, "id").unwrap_or_default().to_owned();
        let version = string(t, "version").unwrap_or_default().to_owned();
        let at = format!("{} {}", if id.is_empty() { "?" } else { &id }, version);
        let mut problem = |what: String| report.problems.push(format!("{at}: {what}"));
        for key in t.keys() {
            if !KEYS.contains(&key.as_str()) {
                problem(format!("unknown key `{key}`"));
            }
        }
        for key in KEYS {
            if *key != "platforms" && t.get(*key).is_some_and(|v| !v.is_str()) {
                problem(format!("`{key}` is not a string"));
            }
        }
        if !is_id(&id) {
            problem("`id` is missing or not lower-case reverse-DNS (`org.name`)".to_owned());
        }
        if semver::Version::parse(&version).is_err() {
            problem("`version` is missing or not semver".to_owned());
        }
        let kind = string(t, "kind").unwrap_or_default();
        if !KINDS.contains(&kind) {
            problem(format!("`kind` is {kind:?}, not one of {}", KINDS.join(", ")));
        }
        if seen.contains(&(id.clone(), version.clone())) {
            problem("listed twice".to_owned());
        }
        seen.push((id.clone(), version.clone()));
        let platforms: Option<Vec<String>> = match t.get("platforms") {
            None => None,
            Some(toml::Value::Array(a)) => {
                let p: Vec<String> = a
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect();
                if p.len() != a.len() {
                    problem("`platforms` holds a non-string".to_owned());
                }
                Some(p)
            }
            Some(_) => {
                problem("`platforms` is not an array".to_owned());
                None
            }
        };
        let located = [
            t.contains_key("path"),
            t.contains_key("git") && t.contains_key("rev"),
            t.contains_key("archive"),
        ];
        if located.iter().filter(|l| **l).count() != 1
            || (t.contains_key("git") != t.contains_key("rev"))
        {
            problem("give exactly one of `path`, `git` + `rev`, `archive`".to_owned());
            continue;
        }
        let Some(archive) = string(t, "archive") else {
            continue;
        };
        let Some(want) = string(t, "sha256").filter(|h| {
            h.len() == 64
                && h.chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        }) else {
            problem("an `archive` needs its `sha256` (64 lower-case hex)".to_owned());
            continue;
        };
        let bytes = match fetch(archive) {
            Ok(b) => b,
            Err(e) => {
                problem(format!("{archive} cannot be fetched: {e}"));
                continue;
            }
        };
        report.archives = report.archives.saturating_add(1);
        let got = hex::encode(sha2::Sha256::digest(&bytes));
        if got != want {
            report.problems.push(format!(
                "{at}: {archive}'s sha256 is {got}, the index says {want}"
            ));
            continue;
        }
        match open(&bytes, trust) {
            Err(e) => report.problems.push(format!("{at}: {archive}: {e}")),
            Ok(v) => {
                let i = &v.index;
                let mut differs = |what: &str, archive_says: &str, entry_says: &str| {
                    if archive_says != entry_says {
                        report.problems.push(format!(
                            "{at}: the archive's {what} is {archive_says:?}, the entry says {entry_says:?}"
                        ));
                    }
                };
                differs("id", &i.id, &id);
                differs("version", &i.version, &version);
                differs("kind", &i.kind, kind);
                if let Some(p) = platforms {
                    let mut p = p;
                    p.sort();
                    differs("platforms", &i.platforms.join(","), &p.join(","));
                }
            }
        }
    }
    Ok(report)
}
