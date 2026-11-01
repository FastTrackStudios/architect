//! Which copy of an extension runs, when there is more than one.
//!
//! One is linked into the build, one installed from a registry, one in
//! development (`docs/content/architecture/extensions.md`, "A built-in can
//! be overridden").
//!
//! A development copy wins over everything until its session ends; an
//! installed copy wins over the built-in when it is newer (an older one
//! — the app updated past it — does not); among installed ones the
//! newest. What lost is kept, so the extensions page can say "built in,
//! overridden by 1.4.0-dev" with a button back.

use semver::Version;

/// Where a copy came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Origin {
    /// Linked into the app's build.
    Builtin,
    /// Installed from a registry or a file.
    Installed,
    /// A development copy (`architect ext dev <path>`), watched.
    Dev,
}

impl Origin {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Builtin => "built in",
            Self::Installed => "installed",
            Self::Dev => "in development",
        }
    }
}

/// One copy of an extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate<P> {
    pub id: String,
    pub version: Version,
    pub origin: Origin,
    /// The copy itself (a package, a path).
    pub package: P,
}

/// The copy that runs, and those it shadows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chosen<P> {
    pub runs: Candidate<P>,
    /// The others, best first: their origin and version.
    pub shadows: Vec<(Origin, Version)>,
}

impl<P> Chosen<P> {
    /// What the extensions page says of it: `1.2.0, installed`, or
    /// `built in, overridden by 1.4.0-dev (in development)`.
    #[must_use]
    pub fn describe(&self) -> String {
        let runs = &self.runs;
        match self.shadows.iter().find(|(o, _)| *o == Origin::Builtin) {
            Some((_, builtin)) if runs.origin != Origin::Builtin => format!(
                "built in ({builtin}), overridden by {} ({})",
                runs.version,
                runs.origin.name()
            ),
            _ => format!("{}, {}", runs.version, runs.origin.name()),
        }
    }
}

/// Picks the copy of each extension that runs, in id order.
#[must_use]
pub fn choose<P>(candidates: Vec<Candidate<P>>) -> Vec<Chosen<P>> {
    let mut by_id: std::collections::BTreeMap<String, Vec<Candidate<P>>> =
        std::collections::BTreeMap::new();
    for c in candidates {
        by_id.entry(c.id.clone()).or_default().push(c);
    }
    by_id
        .into_values()
        .filter_map(|mut copies| {
            copies.sort_by(|a, b| rank(b, a));
            let mut copies = copies.into_iter();
            let runs = copies.next()?;
            Some(Chosen {
                runs,
                shadows: copies.map(|c| (c.origin, c.version)).collect(),
            })
        })
        .collect()
}

/// Which of two copies of one extension is better.
fn rank<P>(a: &Candidate<P>, b: &Candidate<P>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a.origin, b.origin) {
        (Origin::Dev, Origin::Dev) => a.version.cmp(&b.version),
        (Origin::Dev, _) => Ordering::Greater,
        (_, Origin::Dev) => Ordering::Less,
        // The built-in wins a tie: it is the one this build was made with.
        (Origin::Builtin, Origin::Installed) => a.version.cmp(&b.version).then(Ordering::Greater),
        (Origin::Installed, Origin::Builtin) => a.version.cmp(&b.version).then(Ordering::Less),
        _ => a.version.cmp(&b.version),
    }
}

#[cfg(test)]
mod tests {
    use super::{Candidate, Origin, choose};
    use semver::Version;

    fn c(origin: Origin, v: &str) -> Candidate<()> {
        Candidate {
            id: "fts.metronome".into(),
            version: Version::parse(v).unwrap(),
            origin,
            package: (),
        }
    }

    #[test]
    fn a_newer_install_overrides_the_built_in_and_a_dev_copy_both() {
        let chosen = choose(vec![
            c(Origin::Builtin, "1.2.0"),
            c(Origin::Installed, "1.4.0"),
        ]);
        assert_eq!(chosen[0].runs.origin, Origin::Installed);
        assert_eq!(
            chosen[0].describe(),
            "built in (1.2.0), overridden by 1.4.0 (installed)"
        );

        let chosen = choose(vec![
            c(Origin::Builtin, "1.2.0"),
            c(Origin::Installed, "1.1.0"),
        ]);
        assert_eq!(
            chosen[0].runs.origin,
            Origin::Builtin,
            "the app updated past it"
        );
        let chosen = choose(vec![
            c(Origin::Installed, "1.2.0"),
            c(Origin::Builtin, "1.2.0"),
        ]);
        assert_eq!(
            chosen[0].runs.origin,
            Origin::Builtin,
            "a tie is the build's"
        );

        let chosen = choose(vec![
            c(Origin::Installed, "2.0.0"),
            c(Origin::Dev, "1.4.0-dev"),
            c(Origin::Builtin, "1.2.0"),
        ]);
        assert_eq!(chosen[0].runs.origin, Origin::Dev);
        assert_eq!(chosen[0].shadows.len(), 2);
        assert_eq!(chosen[0].shadows[0].0, Origin::Installed);
        assert_eq!(
            chosen[0].describe(),
            "built in (1.2.0), overridden by 1.4.0-dev (in development)"
        );
    }
}
