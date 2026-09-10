//! `cargo xtask fleet-bump` — repoint a consumer at ONE architect tag.
//!
//! ## Why this exists
//!
//! A consuming repo doesn't depend on "architect", it depends on up to
//! fourteen crates that all live in this repo: `architect`,
//! `architect-ui`, `architect-telemetry`, `auth`, `auth-proto`,
//! `auth-db`, `auth-client`, `crdt`, and friends. Bumping them by hand
//! means fourteen edits, and missing one is silent.
//!
//! `signal` has three different checkouts of this repo in one lockfile:
//! `architect` at tag `v0.0.2`, `auth`/`auth-proto`/`auth-db`/`crdt` at
//! rev `5e4bb82`, and `auth-client` at rev `1b007bb`. That is what a
//! partial bump looks like, and it means the `architect` those `auth`
//! crates were compiled against is not the `architect` the app uses.
//!
//! So this rewrites every line at once, or tells you why it can't.

use std::path::{Path, PathBuf};

/// The `[workspace.dependencies]` key prefixes that resolve to this repo.
///
/// Matched as whole keys or as `prefix-*`, so a future `architect-foo`
/// is picked up without editing this list.
const OWNED_PREFIXES: &[&str] = &["architect", "auth", "crdt", "ui-snapshot"];

/// The repo URL a dependency must point at to be ours.
const REPO_MARKER: &str = "FastTrackStudios/architect";

/// One rewritten dependency line.
struct Change {
    line_no: usize,
    key: String,
    before: String,
    after: String,
}

/// Rewrite every architect-owned git dep in `manifest` to `tag`.
///
/// With `dry_run`, reports what it would do and touches nothing.
pub fn run(manifest: &Path, tag: &str, dry_run: bool) -> Result<(), String> {
    let path = resolve_manifest(manifest)?;
    let original =
        std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;

    let mut lines: Vec<String> = original.lines().map(ToOwned::to_owned).collect();
    let mut changes = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let Some(key) = dependency_key(line) else {
            continue;
        };
        if !is_owned(&key) || !line.contains(REPO_MARKER) {
            continue;
        }
        let Some(rewritten) = repoint(line, tag) else {
            return Err(format!(
                "{}:{}: `{key}` points at this repo but has neither `tag = \"…\"` \
                 nor `rev = \"…\"`; rewrite it by hand:\n  {}",
                path.display(),
                idx.saturating_add(1),
                line.trim()
            ));
        };
        if rewritten != *line {
            changes.push(Change {
                line_no: idx.saturating_add(1),
                key,
                before: line.trim().to_owned(),
                after: rewritten.trim().to_owned(),
            });
        }
    }

    if changes.is_empty() {
        println!(
            "{}: already on {tag} (no architect-owned git deps to change)",
            path.display()
        );
        return Ok(());
    }

    println!("{}\n", path.display());
    for change in &changes {
        println!("  {:>4}  {}", change.line_no, change.key);
        println!("        - {}", change.before);
        println!("        + {}\n", change.after);
    }

    if dry_run {
        println!("{} line(s) would change (--dry-run)", changes.len());
        return Ok(());
    }

    for change in &changes {
        if let Some(slot) = lines.get_mut(change.line_no.saturating_sub(1)) {
            let indent: String = slot.chars().take_while(|c| c.is_whitespace()).collect();
            *slot = format!("{indent}{}", change.after);
        }
    }
    let mut out = lines.join("\n");
    if original.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(&path, out).map_err(|e| format!("writing {}: {e}", path.display()))?;
    println!(
        "{} line(s) repointed to {tag}. Run `cargo update -p architect` (or just \
         `cargo check`) to refresh the lockfile.",
        changes.len()
    );
    Ok(())
}

/// Accept either a manifest path or the directory holding one.
fn resolve_manifest(given: &Path) -> Result<PathBuf, String> {
    if given.is_dir() {
        let candidate = given.join("Cargo.toml");
        if candidate.is_file() {
            return Ok(candidate);
        }
        return Err(format!("no Cargo.toml in {}", given.display()));
    }
    if given.is_file() {
        return Ok(given.to_path_buf());
    }
    Err(format!("no such path: {}", given.display()))
}

/// The dependency key a `key = …` line declares, if it is one.
fn dependency_key(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return None;
    }
    let (key, _) = trimmed.split_once('=')?;
    let key = key.trim();
    if key.is_empty() || key.contains(char::is_whitespace) || key.contains('.') {
        return None;
    }
    Some(key.to_owned())
}

/// Is `key` one of the crates this repo publishes?
fn is_owned(key: &str) -> bool {
    OWNED_PREFIXES
        .iter()
        .any(|prefix| key == *prefix || key.starts_with(&format!("{prefix}-")))
}

/// Replace a `tag = "…"` / `rev = "…"` with `tag = "<tag>"`.
fn repoint(line: &str, tag: &str) -> Option<String> {
    for field in ["tag", "rev", "branch"] {
        let needle = format!("{field} = \"");
        if let Some(start) = line.find(&needle) {
            let value_start = start.saturating_add(needle.len());
            let rest = line.get(value_start..)?;
            let end = value_start.saturating_add(rest.find('"')?);
            let head = line.get(..start)?;
            let tail = line.get(end.saturating_add(1)..)?;
            return Some(format!("{head}tag = \"{tag}\"{tail}"));
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{dependency_key, is_owned, repoint};

    #[test]
    fn recognises_the_crates_this_repo_publishes() {
        assert!(is_owned("architect"));
        assert!(is_owned("architect-ui"));
        assert!(is_owned("architect-story-parity"));
        assert!(is_owned("auth"));
        assert!(is_owned("auth-proto"));
        assert!(is_owned("crdt-seaorm"));
        // Not ours, and not a prefix match on a different word.
        assert!(!is_owned("authors"));
        assert!(!is_owned("architecture"));
        assert!(!is_owned("serde"));
    }

    #[test]
    fn rewrites_tag_rev_and_branch_alike() {
        let tagged = r#"architect = { git = "https://x/architect", tag = "v0.7.1" }"#;
        assert_eq!(
            repoint(tagged, "v0.9.0").unwrap(),
            r#"architect = { git = "https://x/architect", tag = "v0.9.0" }"#
        );
        // A raw rev — signal's actual failure mode — becomes a tag.
        let revved = r#"auth = { git = "https://x/architect", rev = "5e4bb82" }"#;
        assert_eq!(
            repoint(revved, "v0.9.0").unwrap(),
            r#"auth = { git = "https://x/architect", tag = "v0.9.0" }"#
        );
        // Features and other keys survive.
        let featured =
            r#"architect = { git = "https://x/architect", tag = "v0.7.1", features = ["iroh"] }"#;
        assert_eq!(
            repoint(featured, "v0.9.0").unwrap(),
            r#"architect = { git = "https://x/architect", tag = "v0.9.0", features = ["iroh"] }"#
        );
        // Nothing to repoint.
        assert!(repoint(r#"serde = "1.0""#, "v0.9.0").is_none());
    }

    #[test]
    fn reads_keys_but_not_comments_or_headers() {
        assert_eq!(
            dependency_key(r#"architect = { git = "…" }"#).as_deref(),
            Some("architect")
        );
        assert_eq!(dependency_key("  auth-db.workspace = true"), None);
        assert_eq!(dependency_key("# architect = { … }"), None);
        assert_eq!(dependency_key("[workspace.dependencies]"), None);
    }
}
