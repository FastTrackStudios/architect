//! `cargo xtask tags` — the release-tag ordering guard.
//!
//! ## Why this exists
//!
//! Six repos pin architect by git tag. A tag is the only thing they can
//! reason about, and the only useful question they ask of it is "am I
//! ahead of or behind that other repo?". That question has an answer only
//! if the numbers order the commits the same way the graph does.
//!
//! In September 2026 they didn't. `v0.0.2` names a commit that sits
//! *between* `v0.5.0` and `v0.6.0`, so `signal` — pinned to `v0.0.2` —
//! looked eight minor versions behind `patchbay`'s `v0.2.0` while
//! actually being three ahead of it. Nobody could see that without
//! running `git merge-base --is-ancestor` by hand, which nobody does.
//!
//! So: every release tag must be a descendant of every lower-numbered
//! one. This checks it, and CI runs it.

use std::fmt::Write as _;

use crate::git::{self, Tag, Version};

/// Known historical violations, with the reason they're grandfathered.
///
/// This list must never grow. Each entry is a tag whose number does not
/// match its position in history, kept only because other repos already
/// pin it and deleting it would break their resolution.
const GRANDFATHERED: &[(&str, &str)] = &[(
    "v0.0.2",
    "Numbered before the v0.1.0 line but cut from a commit between \
     v0.5.0 and v0.6.0. `signal` pins it. Retire by re-pinning signal to \
     a correctly-numbered tag for the same commit, then deleting this.",
)];

/// Print the ordering report; return `Err` on a new violation.
pub fn run(verbose: bool) -> Result<(), String> {
    let tags = git::release_tags()?;
    if tags.is_empty() {
        println!("no `vX.Y.Z` release tags found");
        return Ok(());
    }

    if verbose {
        println!("release tags, in semver order:\n");
        for tag in &tags {
            let short = tag.commit.get(..8).unwrap_or(&tag.commit);
            println!(
                "  {:<10} {short}  {}  {}",
                tag.name,
                tag.date.get(..10).unwrap_or(&tag.date),
                truncate(&tag.subject, 52)
            );
        }
        println!();
    }

    let mut violations = Vec::new();
    let mut grandfathered_seen = Vec::new();
    for pair in tags.windows(2) {
        let (Some(lower), Some(higher)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        if git::is_ancestor(&lower.commit, &higher.commit)? {
            continue;
        }
        if let Some((_, why)) = GRANDFATHERED.iter().find(|(name, _)| *name == lower.name) {
            grandfathered_seen.push((lower.clone(), *why));
        } else {
            violations.push((lower.clone(), higher.clone()));
        }
    }

    for (tag, why) in &grandfathered_seen {
        println!("grandfathered: {} — {why}\n", tag.name);
    }

    if violations.is_empty() {
        let checked = tags.len().saturating_sub(1);
        println!("ok: {checked} tag transition(s) are monotonic in history");
        return Ok(());
    }

    let mut msg = String::from("release tags are out of order:\n");
    for (lower, higher) in &violations {
        // Writing to a `String` is infallible.
        let _ = write!(
            msg,
            "\n  {} is numbered below {}, but its commit is NOT an ancestor of it.\n    {}  {}\n    {}  {}\n",
            lower.name, higher.name, lower.name, lower.date, higher.name, higher.date,
        );
        if let Some(placement) = suggest_placement(&tags, lower)? {
            let _ = writeln!(
                msg,
                "    `{}`'s commit really sits just after `{placement}`.",
                lower.name
            );
        }
    }
    msg.push_str(
        "\nA tag is the only thing a consuming repo can reason about. Cut a new\n\
         correctly-numbered tag rather than moving or renumbering a published one,\n\
         and re-pin the consumers with `cargo xtask fleet-bump`.\n",
    );
    Err(msg)
}

/// The highest-numbered tag that IS an ancestor of `tag` — i.e. where it
/// really belongs in the sequence.
fn suggest_placement(tags: &[Tag], tag: &Tag) -> Result<Option<String>, String> {
    let mut best: Option<&Tag> = None;
    for candidate in tags {
        if candidate.name == tag.name {
            continue;
        }
        if git::is_ancestor(&candidate.commit, &tag.commit)? {
            let better = best.is_none_or(|current| {
                Version::parse(&candidate.name) > Version::parse(&current.name)
            });
            if better {
                best = Some(candidate);
            }
        }
    }
    Ok(best.map(|t| t.name.clone()))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let head: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}
