//! The three git questions this tool asks, and nothing else.

use std::process::Command;

/// A tag name plus the commit it resolves to.
#[derive(Clone, Debug)]
pub struct Tag {
    pub name: String,
    pub commit: String,
    pub date: String,
    pub subject: String,
}

/// Run `git` with `args`, returning stdout on success.
fn git(args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .output()
        .map_err(|e| format!("running `git {}`: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!(
            "`git {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("git output was not utf-8: {e}"))
}

/// Every tag matching `v<major>.<minor>.<patch>`, in **semver** order.
///
/// Tags that don't parse as a three-part `v`-prefixed version are skipped:
/// this repo's release contract is about the numbered line, and a
/// `pre-phon-base`-style marker tag is deliberately outside it.
pub fn release_tags() -> Result<Vec<Tag>, String> {
    let listing = git(&["tag", "--list", "v*"])?;
    let mut tags: Vec<(Version, Tag)> = Vec::new();
    for name in listing.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Some(version) = Version::parse(name) else {
            continue;
        };
        let commit = git(&["rev-list", "-n1", name])?.trim().to_owned();
        let meta = git(&["log", "-1", "--format=%ci%x1f%s", name])?;
        let trimmed = meta.trim();
        let (date, subject) = trimmed.split_once('\u{1f}').unwrap_or((trimmed, ""));
        tags.push((
            version,
            Tag {
                name: name.to_owned(),
                commit,
                date: date.to_owned(),
                subject: subject.to_owned(),
            },
        ));
    }
    tags.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(tags.into_iter().map(|(_, tag)| tag).collect())
}

/// Is `ancestor` reachable from `descendant`?
pub fn is_ancestor(ancestor: &str, descendant: &str) -> Result<bool, String> {
    let out = Command::new("git")
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .output()
        .map_err(|e| format!("running git merge-base: {e}"))?;
    match out.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(format!(
            "git merge-base --is-ancestor {ancestor} {descendant}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

/// A `vMAJOR.MINOR.PATCH` release version.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Version(pub u64, pub u64, pub u64);

impl Version {
    /// Parse `vX.Y.Z`, or `None` for anything else.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        let rest = name.strip_prefix('v')?;
        let mut parts = rest.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self(major, minor, patch))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "v{}.{}.{}", self.0, self.1, self.2)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::Version;

    #[test]
    fn parses_and_orders_release_tags() {
        assert_eq!(Version::parse("v0.7.1"), Some(Version(0, 7, 1)));
        assert_eq!(Version::parse("v1.10.0"), Some(Version(1, 10, 0)));
        assert!(Version::parse("pre-phon-base").is_none());
        assert!(Version::parse("v0.7").is_none());
        assert!(Version::parse("v0.7.1.2").is_none());
        // The whole point: numeric, not lexicographic.
        assert!(Version::parse("v0.10.0").unwrap() > Version::parse("v0.9.0").unwrap());
        assert!(Version::parse("v0.0.2").unwrap() < Version::parse("v0.2.0").unwrap());
    }
}
