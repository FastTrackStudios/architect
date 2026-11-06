//! `SHARE_*` and `PUBLISH_DOMAIN` on an Action List (feature `actions`):
//! share links on the node that hosts the current org (#374), for MCP,
//! ⌘K, a file's context menu and the Share sheet.
//!
//! A share is a scope of one of the host's vaults (`vault`, its only one
//! when unset): `folder`, `tag`, `notes` (or one `note`), `base`, or the
//! whole vault when none is given. Its link is `<host>/<code>/`.

use std::sync::Arc;

use architect::action::{
    ActionCaller, ActionError, ActionList, ActionMeta, Effect, InvocationSource,
};
use architect_spaces::{Audience, NewShare, Share, ShareAccess, ShareChange, ShareScope};
use facet::Facet;

use crate::controller::{Accounts, Sheet};
use crate::shares::ShareDraft;

/// The actions' ids.
pub mod ids {
    pub const SHARE_CREATE: &str = "SHARE_CREATE";
    pub const SHARE_LIST: &str = "SHARE_LIST";
    pub const SHARE_UPDATE: &str = "SHARE_UPDATE";
    pub const SHARE_REVOKE: &str = "SHARE_REVOKE";
    pub const SHARE_COMMENTS: &str = "SHARE_COMMENTS";
    pub const SHARE_SHEET: &str = "SHARE_SHEET";
    pub const PUBLISH_DOMAIN: &str = "PUBLISH_DOMAIN";
    /// Every id, in menu order.
    pub const ALL: [&str; 7] = [
        SHARE_SHEET,
        SHARE_CREATE,
        SHARE_LIST,
        SHARE_UPDATE,
        SHARE_REVOKE,
        SHARE_COMMENTS,
        PUBLISH_DOMAIN,
    ];
}

macro_rules! metas {
    ($group:literal; $($name:ident = $id:ident, $display:literal, $desc:literal;)*) => {
        $(static $name: ActionMeta = ActionMeta {
            id: ids::$id,
            trait_name: "",
            method_name: "",
            display_name: $display,
            description: $desc,
            category: $group,
            group: $group,
            shortcut: "",
            toggleable: false,
            undo: false,
        };)*
    };
}

metas! { "Share";
    SHARE_SHEET = SHARE_SHEET, "Share…", "Opens the Share sheet on a scope: `folder`, `tag`, `note`/`notes`, `base`, or the whole `vault` when none is given; from a file's context menu, that file (a note) or folder. The sheet picks the audience, access and expiry, makes the link, copies it, and lists and revokes the org's shares.";
    SHARE_CREATE = SHARE_CREATE, "Create a share link", "Makes a share link on the node that hosts the current org (`host`, or the one kept): a scope of `vault` (the host's only vault when unset) given as `folder`, `tag`, `note`, `notes` or `base` (else the whole vault); `audience` public, link (default: anyone with the link), org (the org's signed-in members) or invited:<email>,<email>; `access` read (default) or comment; `expires` never (default), 1h, 24h, 7d, 30d or a date YYYY-MM-DD; `title`. Answers the share and its `url`.";
    SHARE_LIST = SHARE_LIST, "Share links", "The share links of the current org's host: code, url, scope, audience, access, expiry, whether it is live, expired or revoked.";
    SHARE_UPDATE = SHARE_UPDATE, "Change a share link", "Changes share `code`: `audience`, `access`, `expires` (never, 1h, 24h, 7d, 30d, YYYY-MM-DD) or `title`. The next request to the link is answered by the change.";
    SHARE_REVOKE = SHARE_REVOKE, "Revoke a share link", "Takes share `code` down: its link answers 404 from the next request on. Final.";
    SHARE_COMMENTS = SHARE_COMMENTS, "Share comments", "The comments left on share `code` (page, by, text, when).";
    PUBLISH_DOMAIN = PUBLISH_DOMAIN, "Publish at a domain", "Serves share `code` at the root of `domain` (album.example.com) too, as its own site; no `domain` stops it. Point the domain's DNS at the host and add it to its ingress.";
}

/// A scope, as the actions take it.
#[derive(Debug, Clone, Default, Facet)]
pub struct ScopeArgs {
    pub vault: Option<String>,
    pub folder: Option<String>,
    pub tag: Option<String>,
    pub note: Option<String>,
    pub notes: Option<Vec<String>>,
    pub base: Option<String>,
}

impl ScopeArgs {
    fn scope(&self) -> ShareScope {
        let given = |s: &Option<String>| {
            s.as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        given(&self.folder)
            .map(ShareScope::Folder)
            .or_else(|| given(&self.tag).map(ShareScope::Tag))
            .or_else(|| given(&self.note).map(|n| ShareScope::Notes(vec![n])))
            .or_else(|| {
                self.notes
                    .clone()
                    .filter(|n| !n.is_empty())
                    .map(ShareScope::Notes)
            })
            .or_else(|| given(&self.base).map(ShareScope::Base))
            .unwrap_or(ShareScope::Vault)
    }

    fn vault(&self) -> String {
        self.vault.clone().unwrap_or_default().trim().to_owned()
    }
}

/// What [`ids::SHARE_CREATE`] takes.
#[derive(Debug, Clone, Facet)]
pub struct CreateArgs {
    pub vault: Option<String>,
    pub folder: Option<String>,
    pub tag: Option<String>,
    pub note: Option<String>,
    pub notes: Option<Vec<String>>,
    pub base: Option<String>,
    pub audience: Option<String>,
    pub access: Option<String>,
    pub expires: Option<String>,
    pub title: Option<String>,
    pub host: Option<String>,
}

/// What [`ids::SHARE_UPDATE`] takes.
#[derive(Debug, Clone, Facet)]
pub struct UpdateArgs {
    pub code: String,
    pub audience: Option<String>,
    pub access: Option<String>,
    pub expires: Option<String>,
    pub title: Option<String>,
    pub host: Option<String>,
}

/// What [`ids::SHARE_REVOKE`] and [`ids::SHARE_COMMENTS`] take.
#[derive(Debug, Clone, Facet)]
pub struct CodeArgs {
    pub code: String,
    pub host: Option<String>,
}

/// What [`ids::PUBLISH_DOMAIN`] takes.
#[derive(Debug, Clone, Facet)]
pub struct DomainArgs {
    pub code: String,
    pub domain: Option<String>,
    pub host: Option<String>,
}

/// What [`ids::SHARE_LIST`] takes.
#[derive(Debug, Clone, Facet)]
pub struct HostArgs {
    pub host: Option<String>,
}

/// One share, as the actions answer it.
#[derive(Debug, Clone, Facet)]
pub struct ShareRow {
    pub code: String,
    /// Its link, on the host.
    pub url: Option<String>,
    pub vault: String,
    /// In words: "the folder Album".
    pub scope: String,
    pub title: Option<String>,
    /// public, link, org or invited:<emails>.
    pub audience: String,
    /// read or comment.
    pub access: String,
    /// Unix seconds; none: until revoked.
    pub expires_at: Option<i64>,
    /// live, expired or revoked.
    pub state: String,
    pub domain: Option<String>,
}

impl ShareRow {
    fn of(share: &Share, accounts: &Accounts) -> Self {
        Self {
            code: share.code.clone(),
            url: accounts.share_url(share),
            vault: share.vault.clone(),
            scope: share.scope.describe(),
            title: share.title.clone(),
            audience: share.audience.label(),
            access: access_label(share.access).to_owned(),
            expires_at: share.expires_at,
            state: match share.state(now()) {
                architect_spaces::ShareState::Live => "live",
                architect_spaces::ShareState::Expired => "expired",
                architect_spaces::ShareState::Revoked => "revoked",
            }
            .to_owned(),
            domain: share.domain.clone(),
        }
    }
}

/// What [`ids::SHARE_LIST`] and [`ids::SHARE_REVOKE`] answer.
#[derive(Debug, Clone, Facet)]
pub struct SharesOut {
    pub host: Option<String>,
    pub shares: Vec<ShareRow>,
}

/// What [`ids::SHARE_CREATE`], [`ids::SHARE_UPDATE`] and
/// [`ids::PUBLISH_DOMAIN`] answer.
#[derive(Debug, Clone, Facet)]
pub struct ShareOut {
    pub share: ShareRow,
}

/// A comment, as [`ids::SHARE_COMMENTS`] answers it.
#[derive(Debug, Clone, Facet)]
pub struct CommentRow {
    pub page: String,
    pub by: String,
    pub text: String,
    pub at: i64,
}

/// What [`ids::SHARE_SHEET`] answers.
#[derive(Debug, Clone, Facet)]
pub struct SheetOut {
    pub sheet: String,
    pub scope: String,
}

/// `read` or `comment`.
#[must_use]
pub const fn access_label(access: ShareAccess) -> &'static str {
    match access {
        ShareAccess::Read => "read",
        ShareAccess::Comment => "comment",
    }
}

/// Parse `read` / `comment`.
///
/// # Errors
///
/// Anything else.
pub fn parse_access(s: &str) -> Result<ShareAccess, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "read" | "read-only" | "view" => Ok(ShareAccess::Read),
        "comment" | "comments" => Ok(ShareAccess::Comment),
        other => Err(format!("no access `{other}`: read or comment")),
    }
}

/// Unix seconds now.
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Parse an expiry from `now`: `never`, `<n>h`, `<n>d`, `<n>m` (minutes)
/// or a date `YYYY-MM-DD` (its midnight, UTC).
///
/// # Errors
///
/// Anything else.
pub fn parse_expiry(s: &str, now: i64) -> Result<Option<i64>, String> {
    let s = s.trim().to_ascii_lowercase();
    if s.is_empty() || s == "never" || s == "none" {
        return Ok(None);
    }
    let bad = || format!("no expiry `{s}`: never, 1h, 24h, 7d, 30d or YYYY-MM-DD");
    for (unit, secs) in [("m", 60), ("h", 3600), ("d", 86_400)] {
        if let Some(n) = s.strip_suffix(unit).and_then(|n| n.parse::<i64>().ok()) {
            return Ok(Some(now.saturating_add(n.max(1).saturating_mul(secs))));
        }
    }
    let mut parts = s.split('-').map(str::parse::<i64>);
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(Ok(y)), Some(Ok(m)), Some(Ok(d)), None)
            if (1..=12).contains(&m) && (1..=31).contains(&d) =>
        {
            Ok(Some(days_from_civil(y, m, d).saturating_mul(86_400)))
        }
        _ => Err(bad()),
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Hinnant's); `m`
/// and `d` already checked, `y` any year a person types.
#[allow(clippy::arithmetic_side_effects, clippy::integer_division)]
const fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The scope a file's context menu was opened on: a note, else a folder.
fn from_context(caller: &ActionCaller) -> Option<ScopeArgs> {
    let inv = caller.invocation()?;
    if inv.source != InvocationSource::ContextMenu {
        return None;
    }
    let path = inv.id.clone().or_else(|| inv.path.clone())?;
    let path = path.trim_matches('/').to_owned();
    let note = std::path::Path::new(&path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"));
    Some(if note {
        ScopeArgs {
            note: Some(path),
            ..ScopeArgs::default()
        }
    } else {
        ScopeArgs {
            folder: Some(path),
            ..ScopeArgs::default()
        }
    })
}

/// Registers `SHARE_*` and `PUBLISH_DOMAIN` on `list`, over `accounts`.
///
/// # Errors
///
/// An id already on the list.
pub fn register(list: &Arc<ActionList>, accounts: &Accounts) -> Result<(), ActionError> {
    let a = accounts.clone();
    list.register_with(
        &SHARE_SHEET,
        Effect::ReadOnly,
        move |caller: &ActionCaller, args: ScopeArgs| {
            let args = from_context(caller).map_or_else(
                || args.clone(),
                |c| ScopeArgs {
                    vault: args.vault.clone(),
                    ..c
                },
            );
            let scope = args.scope();
            a.open_share_sheet(ShareDraft {
                vault: args.vault(),
                scope: scope.clone(),
            });
            Ok(SheetOut {
                sheet: if a.sheet() == Some(Sheet::Share) {
                    "share".to_owned()
                } else {
                    "closed".to_owned()
                },
                scope: scope.describe(),
            })
        },
    )?;
    let a = accounts.clone();
    list.register_async(&SHARE_CREATE, Effect::Mutating, move |args: CreateArgs| {
        let a = a.clone();
        async move {
            let at = now();
            let audience = Audience::parse(args.audience.as_deref().unwrap_or("link"))?;
            let access = parse_access(args.access.as_deref().unwrap_or("read"))?;
            let expires_at = parse_expiry(args.expires.as_deref().unwrap_or("never"), at)?;
            let scope = ScopeArgs {
                vault: args.vault.clone(),
                folder: args.folder,
                tag: args.tag,
                note: args.note,
                notes: args.notes,
                base: args.base,
            };
            let share = a
                .share_create(
                    args.host.as_deref(),
                    NewShare {
                        vault: scope.vault(),
                        scope: scope.scope(),
                        title: args.title,
                        audience,
                        access,
                        expires_at,
                        domain: None,
                    },
                )
                .await?;
            Ok(ShareOut {
                share: ShareRow::of(&share, &a),
            })
        }
    })?;
    let a = accounts.clone();
    list.register_async(&SHARE_LIST, Effect::ReadOnly, move |args: HostArgs| {
        let a = a.clone();
        async move {
            let shares = a.shares(args.host.as_deref()).await?;
            Ok(SharesOut {
                host: a.org_host(),
                shares: shares.iter().map(|s| ShareRow::of(s, &a)).collect(),
            })
        }
    })?;
    let a = accounts.clone();
    list.register_async(&SHARE_UPDATE, Effect::Mutating, move |args: UpdateArgs| {
        let a = a.clone();
        async move {
            let mut changes = Vec::new();
            if let Some(audience) = &args.audience {
                changes.push(ShareChange::Audience(Audience::parse(audience)?));
            }
            if let Some(access) = &args.access {
                changes.push(ShareChange::Access(parse_access(access)?));
            }
            if let Some(expires) = &args.expires {
                changes.push(ShareChange::Expires(parse_expiry(expires, now())?));
            }
            if let Some(title) = &args.title {
                changes.push(ShareChange::Title(Some(title.clone())));
            }
            if changes.is_empty() {
                return Err("say what to change: audience, access, expires or title".to_owned());
            }
            let mut share = None;
            for change in changes {
                share = Some(
                    a.share_update(args.host.as_deref(), &args.code, change)
                        .await?,
                );
            }
            let share = share.ok_or_else(|| "nothing changed".to_owned())?;
            Ok(ShareOut {
                share: ShareRow::of(&share, &a),
            })
        }
    })?;
    let a = accounts.clone();
    list.register_async(&SHARE_REVOKE, Effect::Destructive, move |args: CodeArgs| {
        let a = a.clone();
        async move {
            a.share_revoke(args.host.as_deref(), &args.code).await?;
            let shares = a.shares_seen().and_then(Result::ok).unwrap_or_default();
            Ok(SharesOut {
                host: a.org_host(),
                shares: shares.iter().map(|s| ShareRow::of(s, &a)).collect(),
            })
        }
    })?;
    let a = accounts.clone();
    list.register_async(&SHARE_COMMENTS, Effect::ReadOnly, move |args: CodeArgs| {
        let a = a.clone();
        async move {
            let comments = a.share_comments(args.host.as_deref(), &args.code).await?;
            Ok(comments
                .into_iter()
                .map(|c| CommentRow {
                    page: c.page,
                    by: c.by,
                    text: c.text,
                    at: c.at,
                })
                .collect::<Vec<_>>())
        }
    })?;
    let a = accounts.clone();
    list.register_async(
        &PUBLISH_DOMAIN,
        Effect::Mutating,
        move |args: DomainArgs| {
            let a = a.clone();
            async move {
                let domain = args.domain.filter(|d| !d.trim().is_empty());
                let share = a
                    .share_update(
                        args.host.as_deref(),
                        &args.code,
                        ShareChange::Domain(domain),
                    )
                    .await?;
                Ok(ShareOut {
                    share: ShareRow::of(&share, &a),
                })
            }
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiries() {
        assert_eq!(parse_expiry("never", 10), Ok(None));
        assert_eq!(parse_expiry("2h", 10), Ok(Some(10 + 7200)));
        assert_eq!(parse_expiry("7d", 0), Ok(Some(7 * 86_400)));
        assert_eq!(parse_expiry("1970-01-02", 0), Ok(Some(86_400)));
        assert_eq!(parse_expiry("2026-11-06", 0), Ok(Some(1_793_923_200)));
        assert!(parse_expiry("soon", 0).is_err());
        assert_eq!(parse_access("Comment"), Ok(ShareAccess::Comment));
    }
}
