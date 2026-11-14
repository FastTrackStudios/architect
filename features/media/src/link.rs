//! A link as a collection or an item writes it, and the vault path it
//! names.
//!
//! Resolution is against the paths the caller passes and nothing else: a
//! member's vault, or the notes a share link lists to its guest. A link
//! to anything outside them resolves to nothing.

/// What `link` names, as written.
///
/// `[[Songs/Joy|Joy]]` → `Songs/Joy`,
/// `![[cover.png]]` → `cover.png`, `[Joy](Songs/Joy.md)` → `Songs/Joy.md`
/// (percent-decoded), a plain path as it is; its alias (`|…`) and anchor
/// (`#…`) dropped. `None` for a URL, an absolute path, a `..` segment or
/// nothing.
#[must_use]
pub fn target(link: &str) -> Option<String> {
    let link = link.trim().trim_start_matches('!');
    let inner = if let Some(w) = link.strip_prefix("[[").and_then(|l| l.strip_suffix("]]")) {
        w.split(['|', '#']).next().unwrap_or_default().to_owned()
    } else if let Some(md) = markdown_target(link) {
        decode(md.split('#').next().unwrap_or_default())?
    } else {
        link.split('#').next().unwrap_or_default().to_owned()
    };
    let inner = inner.trim();
    let inner = inner.strip_prefix("./").unwrap_or(inner);
    let bad = inner.is_empty()
        || inner.contains("://")
        || inner.starts_with(['/', '\\'])
        || inner.contains(':')
        || inner.split(['/', '\\']).any(|s| s == "..");
    (!bad).then(|| inner.to_owned())
}

/// The alias a wikilink gives (`[[Songs/Joy|Joy]]` → `Joy`) or a
/// markdown link's text; `None` for a plain path.
#[must_use]
pub fn alias(link: &str) -> Option<String> {
    let link = link.trim().trim_start_matches('!');
    if let Some(w) = link.strip_prefix("[[").and_then(|l| l.strip_suffix("]]")) {
        return w
            .split_once('|')
            .map(|(_, a)| a.trim().to_owned())
            .filter(|a| !a.is_empty());
    }
    link.strip_prefix('[')
        .and_then(|l| l.split_once("]("))
        .map(|(text, _)| text.trim().to_owned())
        .filter(|a| !a.is_empty())
}

/// `[text](target)` → `target`.
fn markdown_target(link: &str) -> Option<&str> {
    let (_, rest) = link.strip_prefix('[')?.split_once("](")?;
    rest.strip_suffix(')')
        .map(|t| t.trim().trim_matches(['<', '>']))
}

/// `%XX` decoded; `None` when it does not decode to UTF-8.
fn decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hex = s.get(i.saturating_add(1)..i.saturating_add(3))?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i = i.saturating_add(3);
        } else {
            out.push(b);
            i = i.saturating_add(1);
        }
    }
    String::from_utf8(out).ok()
}

/// The folder of vault path `path` (`A/B` for `A/B/C.md`).
#[must_use]
pub fn folder_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// The path among `paths` that `link` (written in a note in `folder`)
/// names, the vault's way.
///
/// The path as written, or with `.md` added;
/// else the same under `folder`; else, for a link that names more than a
/// file, the one path ending in it; else the one file of that name. Two
/// that match equally: none (never a guess).
pub fn resolve<'a, I>(link: &str, folder: &str, paths: I) -> Option<&'a str>
where
    I: IntoIterator<Item = &'a str>,
    I::IntoIter: Clone,
{
    let want = target(link)?;
    let paths = paths.into_iter();
    let names = [want.clone(), format!("{want}.md")];
    let find = |name: &str| paths.clone().find(|p| *p == name);
    if let Some(p) = names.iter().find_map(|n| find(n)) {
        return Some(p);
    }
    if !folder.is_empty()
        && let Some(p) = names.iter().find_map(|n| find(&format!("{folder}/{n}")))
    {
        return Some(p);
    }
    let ends = |p: &&str| ends_in(p, &names);
    let mut found = paths.filter(ends);
    match (found.next(), found.next()) {
        (Some(one), None) => Some(one),
        _ => None,
    }
}

/// Whether `path` ends in one of `names` after a `/`.
fn ends_in(path: &str, names: &[String; 2]) -> bool {
    names.iter().any(|n| {
        path.strip_suffix(n.as_str())
            .is_some_and(|b| b.ends_with('/'))
    })
}

/// The last segment of a path.
fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `paths` indexed for many links: [`resolve`]'s answer for each, without
/// a walk of every path per link (a long list in a large vault).
#[derive(Debug, Clone, Default)]
pub struct Index<'a> {
    exact: std::collections::HashSet<&'a str>,
    by_name: std::collections::HashMap<&'a str, Vec<&'a str>>,
}

impl<'a> Index<'a> {
    /// The index of `paths`.
    pub fn new(paths: impl IntoIterator<Item = &'a str>) -> Self {
        let mut index = Self::default();
        for p in paths {
            if index.exact.insert(p) {
                index.by_name.entry(file_name(p)).or_default().push(p);
            }
        }
        index
    }

    /// [`resolve`] of `link`, written in a note in `folder`, among the
    /// indexed paths.
    #[must_use]
    pub fn resolve(&self, link: &str, folder: &str) -> Option<&'a str> {
        let want = target(link)?;
        let names = [want.clone(), format!("{want}.md")];
        let find = |name: &str| self.exact.get(name).copied();
        if let Some(p) = names.iter().find_map(|n| find(n)) {
            return Some(p);
        }
        if !folder.is_empty()
            && let Some(p) = names.iter().find_map(|n| find(&format!("{folder}/{n}")))
        {
            return Some(p);
        }
        let mut found = names
            .iter()
            .filter_map(|n| self.by_name.get(file_name(n)))
            .flatten()
            .copied()
            .filter(|p| ends_in(p, &names));
        let one = found.next()?;
        found.all(|p| p == one).then_some(one)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATHS: &[&str] = &[
        "Songs/Joy/Joy.md",
        "Songs/Noel/Noel.md",
        "Songs/Noel/cover.png",
        "Other/Noel.md",
        "Setlists/Set.md",
        "Recordings/Joy.mp3",
    ];

    fn r(link: &str, folder: &str) -> Option<&'static str> {
        resolve(link, folder, PATHS.iter().copied())
    }

    #[test]
    fn a_link_names_its_target_without_alias_or_anchor() {
        assert_eq!(
            target("[[Songs/Joy/Joy|Joy]]").as_deref(),
            Some("Songs/Joy/Joy")
        );
        assert_eq!(target("![[cover.png]]").as_deref(), Some("cover.png"));
        assert_eq!(target("[[Joy#Verse]]").as_deref(), Some("Joy"));
        assert_eq!(
            target("[Joy](Songs/Joy%20One.md)").as_deref(),
            Some("Songs/Joy One.md")
        );
        assert_eq!(
            target("Recordings/Joy.mp3").as_deref(),
            Some("Recordings/Joy.mp3")
        );
        assert_eq!(alias("[[Songs/Joy/Joy|Joy]]").as_deref(), Some("Joy"));
        assert_eq!(alias("[Joy](x.md)").as_deref(), Some("Joy"));
        assert_eq!(alias("[[Joy]]"), None);
    }

    #[test]
    fn a_url_an_absolute_path_or_a_climb_names_nothing() {
        for bad in [
            "https://example.com/a.mp3",
            "[[../secret]]",
            "/etc/passwd",
            "[x](data:text/plain,hi)",
            "C:\\x.md",
            "",
            "[[]]",
        ] {
            assert_eq!(target(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_link_resolves_the_vaults_way_and_never_guesses() {
        assert_eq!(r("[[Songs/Joy/Joy|Joy]]", ""), Some("Songs/Joy/Joy.md"));
        assert_eq!(r("[[Joy]]", "Setlists"), Some("Songs/Joy/Joy.md"));
        assert_eq!(
            r("[[cover.png]]", "Songs/Noel"),
            Some("Songs/Noel/cover.png")
        );
        assert_eq!(r("[[Noel/Noel]]", ""), Some("Songs/Noel/Noel.md"));
        // Two notes named Noel: no guess.
        assert_eq!(r("[[Noel]]", "Setlists"), None);
        // Beside the note first.
        assert_eq!(r("[[Noel]]", "Other"), Some("Other/Noel.md"));
        assert_eq!(r("[[Missing]]", ""), None);
    }

    #[test]
    fn an_index_resolves_as_a_walk_does() {
        let index = Index::new(PATHS.iter().copied());
        for (link, folder) in [
            ("[[Songs/Joy/Joy|Joy]]", ""),
            ("[[Joy]]", "Setlists"),
            ("[[cover.png]]", "Songs/Noel"),
            ("[[Noel/Noel]]", ""),
            ("[[Noel]]", "Setlists"),
            ("[[Noel]]", "Other"),
            ("[[Missing]]", ""),
            ("Recordings/Joy.mp3", ""),
            ("[[../x]]", ""),
        ] {
            assert_eq!(
                index.resolve(link, folder),
                r(link, folder),
                "{link} in {folder}"
            );
        }
    }

    #[test]
    fn only_the_paths_given_resolve() {
        // A guest's list: the notes a share lists, nothing else.
        let listed = ["Setlists/Set.md", "Songs/Joy/Joy.md"];
        assert_eq!(resolve("[[Songs/Noel/Noel]]", "", listed), None);
        assert_eq!(resolve("[[Joy]]", "", listed), Some("Songs/Joy/Joy.md"));
    }
}
