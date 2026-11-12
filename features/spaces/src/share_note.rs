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
    let json = layers_in_any_case(json.trim());
    let share: Share = facet_json::from_str(&json).ok()?;
    crate::share::is_code(&share.code).then_some(share)
}

/// `json` with each `"listen": "<layer>"` value spelled as the model
/// reads it, whatever case a hand edit gave it (`full`, `REFERENCE`). A
/// value that is no layer is left as it is, and the note does not read.
fn layers_in_any_case(json: &str) -> String {
    const KEY: &str = "\"listen\"";
    let mut out = String::with_capacity(json.len());
    let mut rest = json;
    while let Some(at) = rest.find(KEY) {
        let (before, after) = rest.split_at(at.saturating_add(KEY.len()));
        out.push_str(before);
        rest = after;
        let gap = rest.len().saturating_sub(rest.trim_start().len());
        let Some(value) = rest.trim_start().strip_prefix(':') else {
            continue;
        };
        let value = value.trim_start();
        let Some(quoted) = value.strip_prefix('"') else {
            continue;
        };
        let Some(end) = quoted.find('"') else {
            continue;
        };
        let (word, tail) = quoted.split_at(end);
        let canonical = crate::share::Listen::ALL
            .iter()
            .find(|l| l.label().eq_ignore_ascii_case(word))
            .map(|l| format!("{l:?}"));
        if let Some(canonical) = canonical {
            out.push_str(rest.get(..gap).unwrap_or_default());
            out.push_str(":\"");
            out.push_str(&canonical);
            rest = tail;
        }
    }
    out.push_str(rest);
    out
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
    use crate::share::{Audience, Listen, ShareAccess, ShareAudience, ShareScope, code_from};

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
            audiences: vec![ShareAudience {
                name: "violin".into(),
                filter: "instrument == \"violin\"".into(),
                code: code_from([4; 10]),
                created_at: 2,
                listen: Some(Listen::Page),
            }],
            listen: Listen::Reference,
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

    /// A share note written with audiences but with no layer (#528): the
    /// share reads as Reference, the default, each audience as the
    /// share's. Access fails closed: a note that lost the field never
    /// opens wider than a new link.
    #[test]
    fn a_share_note_without_a_layer_reads_as_reference() {
        let (code, other) = (code_from([5; 10]), code_from([6; 10]));
        let note = format!(
            "---\npublish: false\n---\n# Share {code}\n\n```json\n{{\"code\":\"{code}\",\"vault\":\"notes\",\"scope\":\"Vault\",\"title\":null,\"audience\":\"Link\",\"access\":\"Read\",\"expires_at\":null,\"revoked_at\":null,\"created_at\":7,\"created_by\":\"u1\",\"domain\":null,\"audiences\":[{{\"name\":\"band\",\"filter\":\"\",\"code\":\"{other}\",\"created_at\":8}}]}}\n```\n"
        );
        let share = parse_share_note(&note).expect("an old note reads");
        assert_eq!(share.listen, Listen::Reference);
        let band = share.audience_named("band").expect("its audience");
        assert_eq!(band.listen, None);
        assert_eq!(share.listen_for(Some(band)), Listen::Reference);
        assert_eq!(parse_share_note(&share_note(&share)), Some(share));
    }

    /// The note of a share made before audiences (#510), byte for byte
    /// what `share_note` wrote then: it still reads, with no audiences.
    #[test]
    fn a_share_note_written_before_audiences_still_reads() {
        let code = code_from([5; 10]);
        let note = format!(
            "---\npublish: false\n---\n# Share {code}\n\nA share link, kept by Architect (`SHARE_*`): the folder Album of `notes`, for link. Change it with `SHARE_UPDATE` or `SHARE_REVOKE`.\n\n```json\n{{\"code\":\"{code}\",\"vault\":\"notes\",\"scope\":{{\"Folder\":\"Album\"}},\"title\":null,\"audience\":\"Link\",\"access\":\"Read\",\"expires_at\":null,\"revoked_at\":null,\"created_at\":7,\"created_by\":\"u1\",\"domain\":null}}\n```\n"
        );
        let share = parse_share_note(&note).expect("an old note reads");
        assert_eq!(share.code, code);
        assert_eq!(share.scope, ShareScope::Folder("Album".into()));
        assert_eq!(share.audience, Audience::Link);
        assert_eq!(share.created_at, 7);
        assert!(share.audiences.is_empty());
        // No layer (#528): Reference, the default; never wider.
        assert_eq!(share.listen, Listen::Reference);
        // And written again it carries the (empty) list, and reads the same.
        assert_eq!(parse_share_note(&share_note(&share)), Some(share));
    }

    /// A hand-edited layer in any case reads; one that is no layer leaves
    /// the note unread (404: closed).
    #[test]
    fn a_hand_edited_layer_reads_in_any_case() {
        let (code, other) = (code_from([5; 10]), code_from([6; 10]));
        let note = |listen: &str, theirs: &str| {
            format!(
                "```json\n{{\"code\":\"{code}\",\"vault\":\"notes\",\"scope\":\"Vault\",\"title\":null,\"audience\":\"Link\",\"access\":\"Read\",\"expires_at\":null,\"revoked_at\":null,\"created_at\":7,\"created_by\":\"u1\",\"domain\":null,\"listen\": \"{listen}\",\"audiences\":[{{\"name\":\"band\",\"filter\":\"\",\"code\":\"{other}\",\"created_at\":8,\"listen\":\"{theirs}\"}}]}}\n```\n"
            )
        };
        let share = parse_share_note(&note("full", "PAGE")).expect("reads");
        assert_eq!(share.listen, Listen::Full);
        assert_eq!(share.audiences[0].listen, Some(Listen::Page));
        assert_eq!(
            parse_share_note(&note("Reference", "reference"))
                .unwrap()
                .listen,
            Listen::Reference
        );
        assert!(parse_share_note(&note("everything", "page")).is_none());
    }
}
