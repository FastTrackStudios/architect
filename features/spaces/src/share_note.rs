//! A share as a note of its vault (feature `server`): how it is kept and
//! synced. The note is hidden from every Site (`publish: false`) and holds
//! the share as one JSON block, so a peer that syncs the vault carries it
//! like any other note, and the node that serves the link reads it back.

use crate::share::{Share, ShareComment};

const FENCE: &str = "```json\n";

/// The note's text for `share`.
#[must_use]
pub fn share_note(share: &Share) -> String {
    let json = facet_json::to_string(share).unwrap_or_default();
    format!(
        "---\npublish: false\n---\n# Share {code}\n\nA share link, kept by Architect (`SHARE_*`): {scope} of `{vault}`, for {audience}. Change it with `SHARE_UPDATE` or `SHARE_REVOKE`.\n\n{FENCE}{json}\n```\n",
        code = share.code,
        scope = share.scope.describe(),
        vault = share.vault,
        audience = share.audience.label(),
    )
}

/// The share a note holds, if it holds one.
#[must_use]
pub fn parse_share_note(text: &str) -> Option<Share> {
    let start = text.find(FENCE)?.checked_add(FENCE.len())?;
    let rest = text.get(start..)?;
    let json = rest.get(..rest.find("\n```")?)?;
    let share: Share = facet_json::from_str(json.trim()).ok()?;
    crate::share::is_code(&share.code).then_some(share)
}

/// The comments note's text, holding `comments`.
#[must_use]
pub fn comments_note(code: &str, comments: &[ShareComment]) -> String {
    let mut out = format!(
        "---\npublish: false\n---\n# Comments on share {code}\n\nOne JSON object per line.\n\n```jsonl\n"
    );
    for c in comments {
        if let Ok(line) = facet_json::to_string(c) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out.push_str("```\n");
    out
}

/// The comments a comments note holds, oldest first.
#[must_use]
pub fn parse_comments(text: &str) -> Vec<ShareComment> {
    text.lines()
        .filter(|l| l.starts_with('{'))
        .filter_map(|l| facet_json::from_str::<ShareComment>(l).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::{Audience, ShareAccess, ShareScope, code_from};

    #[test]
    fn a_share_round_trips_through_its_note() {
        let share = Share {
            code: code_from([3; 10]),
            vault: "notes".into(),
            scope: ShareScope::Notes(vec!["a.md".into(), "b/c.md".into()]),
            title: Some("Album \"one\"".into()),
            audience: Audience::Invited(vec!["ada@x.test".into()]),
            access: ShareAccess::Comment,
            expires_at: Some(1_900_000_000),
            revoked_at: None,
            created_at: 1,
            created_by: "u".into(),
            domain: None,
        };
        let note = share_note(&share);
        assert!(note.starts_with("---\npublish: false\n---\n"), "{note}");
        assert_eq!(parse_share_note(&note), Some(share));
        assert_eq!(parse_share_note("# nothing"), None);

        let c = ShareComment {
            page: "/".into(),
            by: "Ada".into(),
            text: "Nice\nmix".into(),
            at: 5,
        };
        assert_eq!(
            parse_comments(&comments_note("x", &[c.clone(), c.clone()])),
            vec![c.clone(), c]
        );
    }
}
