//! A path inside a root, checked in one place.
//!
//! [`RootPath::parse`] normalises and rejects: `..`, empty components and
//! NUL bytes. Holding one means the value passed that check — except a
//! value that arrived over the wire, which the decoder built without
//! calling `parse`; a server re-checks those with [`RootPath::validate`].

use std::fmt;

use facet::Facet;
use thiserror::Error;

/// Why a path was refused.
#[derive(Debug, Clone, PartialEq, Eq, Facet, Error)]
#[repr(u8)]
pub enum PathError {
    #[error("path escapes its root: {0}")]
    Escapes(String),
    #[error("path has an empty component: {0}")]
    EmptyComponent(String),
    #[error("path contains a NUL byte")]
    Nul,
}

/// A `/`-separated path relative to a root; `""` is the root itself.
/// Leading and trailing slashes and `.` components are dropped,
/// backslashes read as slashes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Facet)]
#[facet(transparent)]
#[repr(C)]
pub struct RootPath(String);

impl RootPath {
    /// Normalise and check. The only checked constructor.
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, PathError> {
        let raw = raw.as_ref();
        if raw.contains('\0') {
            return Err(PathError::Nul);
        }
        let unified = raw.replace('\\', "/");
        let trimmed = unified.trim_matches('/');
        let mut out: Vec<&str> = Vec::new();
        if !trimmed.is_empty() {
            for part in trimmed.split('/') {
                match part {
                    "" => return Err(PathError::EmptyComponent(raw.to_owned())),
                    "." => {}
                    ".." => return Err(PathError::Escapes(raw.to_owned())),
                    name => out.push(name),
                }
            }
        }
        Ok(Self(out.join("/")))
    }

    /// The root itself.
    #[must_use]
    pub const fn root() -> Self {
        Self(String::new())
    }

    /// Check again a value the wire decoded without [`Self::parse`].
    pub fn validate(&self) -> Result<Self, PathError> {
        Self::parse(&self.0)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub const fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// The last component; `None` at the root.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.0.rsplit('/').next().filter(|n| !n.is_empty())
    }

    /// The folder holding this path; `None` at the root.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        Some(Self(
            self.0
                .rsplit_once('/')
                .map_or_else(String::new, |(head, _)| head.to_owned()),
        ))
    }

    /// Every folder above this path, outermost first, the root excluded.
    #[must_use]
    pub fn ancestors(&self) -> Vec<Self> {
        let mut out = Vec::new();
        let mut at = self.parent();
        while let Some(p) = at.filter(|p| !p.is_root()) {
            at = p.parent();
            out.push(p);
        }
        out.reverse();
        out
    }

    /// This path with `child` (one component, or several) under it.
    pub fn join(&self, child: impl AsRef<str>) -> Result<Self, PathError> {
        let child = Self::parse(child)?;
        Ok(match (self.is_root(), child.is_root()) {
            (true, _) => child,
            (_, true) => self.clone(),
            _ => Self(format!("{}/{}", self.0, child.0)),
        })
    }

    /// The same folder, another name.
    pub fn with_name(&self, name: &str) -> Result<Self, PathError> {
        if name.contains('/') || name.contains('\\') {
            return Err(PathError::Escapes(name.to_owned()));
        }
        self.parent().unwrap_or_default().join(name)
    }

    /// Whether this is `other` or lies beneath it — by component, so
    /// `Audio 2/x` is not within `Audio`.
    #[must_use]
    pub fn is_within(&self, other: &Self) -> bool {
        other.is_root()
            || self.0 == other.0
            || self
                .0
                .strip_prefix(other.0.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    }

    /// This path moved from under `from` to under `to`; `None` when it
    /// is not within `from`.
    #[must_use]
    pub fn rebase(&self, from: &Self, to: &Self) -> Option<Self> {
        if !self.is_within(from) {
            return None;
        }
        let rest = self
            .0
            .get(from.0.len()..)
            .unwrap_or_default()
            .trim_start_matches('/');
        Some(if rest.is_empty() {
            to.clone()
        } else if to.is_root() {
            Self(rest.to_owned())
        } else {
            Self(format!("{}/{rest}", to.0))
        })
    }
}

impl fmt::Display for RootPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<&str> for RootPath {
    type Error = PathError;
    fn try_from(raw: &str) -> Result<Self, PathError> {
        Self::parse(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> RootPath {
        RootPath::parse(s).unwrap()
    }

    #[test]
    fn traversal_and_nul_are_refused() {
        assert!(matches!(
            RootPath::parse("a/../../etc"),
            Err(PathError::Escapes(_))
        ));
        assert!(matches!(
            RootPath::parse("a//b"),
            Err(PathError::EmptyComponent(_))
        ));
        assert_eq!(RootPath::parse("a\0b"), Err(PathError::Nul));
    }

    #[test]
    fn paths_normalise() {
        assert_eq!(p("/a/./b/").as_str(), "a/b");
        assert_eq!(p("a\\b").as_str(), "a/b");
        assert!(p("").is_root());
    }

    #[test]
    fn paths_walk_by_component() {
        let take = p("Sessions/Song/Audio Files");
        assert_eq!(take.name(), Some("Audio Files"));
        assert_eq!(take.parent(), Some(p("Sessions/Song")));
        assert_eq!(take.ancestors(), vec![p("Sessions"), p("Sessions/Song")]);
        assert!(take.is_within(&p("Sessions")));
        assert!(!p("Audio 2/x").is_within(&p("Audio")));
        assert_eq!(p("a").parent(), Some(RootPath::root()));
        assert_eq!(p("a/b/c").rebase(&p("a/b"), &p("z")), Some(p("z/c")));
        assert_eq!(p("a/b").rebase(&p("a/b"), &p("z")), Some(p("z")));
        assert_eq!(p("a/b").with_name("c").unwrap(), p("a/c"));
        assert!(p("a/b").with_name("../c").is_err());
    }
}
