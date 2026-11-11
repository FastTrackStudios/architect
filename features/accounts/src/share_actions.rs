//! `SHARE_*` and `PUBLISH_DOMAIN` on an Action List (feature `actions`):
//! share links on the node that hosts the current org (#374), for MCP,
//! ⌘K, a file's context menu and the Share sheet.
//!
//! A share is a scope of one of the host's vaults (`vault`, its only one
//! when unset): `folder`, `tag`, `notes` (or one `note`), `base`, or the
//! whole vault when none is given. Its link is `<host>/s/<code>/`.
//!
//! A share for many people has named audiences (#510): each a Bases
//! filter over the scope on a link of its own (`SHARE_AUDIENCE_ADD`,
//! `SHARE_AUDIENCE_REMOVE`, `SHARE_AUDIENCES`). `SHARE_ARCHIVE` saves the
//! zip a link's "Download all" gives.
//!
//! A link plays to a layer (#528, `listen`: page, reference or full; a
//! new link's default is reference), set on the share (`SHARE_CREATE`,
//! `SHARE_UPDATE`) and narrowed per audience (`SHARE_AUDIENCE_ADD`,
//! `SHARE_AUDIENCE_UPDATE`); the host refuses a file above it.

use std::sync::Arc;

use architect::action::{
    ActionCaller, ActionError, ActionList, ActionMeta, Effect, InvocationSource,
};
use architect_spaces::{
    Audience, Listen, NewAudience, NewShare, Share, ShareAccess, ShareAudience, ShareChange,
    ShareScope,
};
use facet::Facet;

use crate::controller::{Accounts, Sheet};
use crate::shares::{ArchiveAsk, ShareDraft};

/// The actions' ids.
pub mod ids {
    pub const SHARE_CREATE: &str = "SHARE_CREATE";
    pub const SHARE_LIST: &str = "SHARE_LIST";
    pub const SHARE_UPDATE: &str = "SHARE_UPDATE";
    pub const SHARE_REVOKE: &str = "SHARE_REVOKE";
    pub const SHARE_COMMENTS: &str = "SHARE_COMMENTS";
    pub const SHARE_SHEET: &str = "SHARE_SHEET";
    pub const PUBLISH_DOMAIN: &str = "PUBLISH_DOMAIN";
    pub const SHARE_AUDIENCE_ADD: &str = "SHARE_AUDIENCE_ADD";
    pub const SHARE_AUDIENCE_REMOVE: &str = "SHARE_AUDIENCE_REMOVE";
    pub const SHARE_AUDIENCE_UPDATE: &str = "SHARE_AUDIENCE_UPDATE";
    pub const SHARE_LAYERS_KEPT: &str = "SHARE_LAYERS_KEPT";
    pub const SHARE_LAYERS_FORGET: &str = "SHARE_LAYERS_FORGET";
    pub const SHARE_AUDIENCES: &str = "SHARE_AUDIENCES";
    pub const SHARE_ARCHIVE: &str = "SHARE_ARCHIVE";
    /// Every id, in menu order.
    pub const ALL: [&str; 14] = [
        SHARE_SHEET,
        SHARE_CREATE,
        SHARE_LIST,
        SHARE_UPDATE,
        SHARE_REVOKE,
        SHARE_COMMENTS,
        PUBLISH_DOMAIN,
        SHARE_AUDIENCE_ADD,
        SHARE_AUDIENCE_REMOVE,
        SHARE_AUDIENCE_UPDATE,
        SHARE_AUDIENCES,
        SHARE_ARCHIVE,
        SHARE_LAYERS_KEPT,
        SHARE_LAYERS_FORGET,
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
    SHARE_CREATE = SHARE_CREATE, "Create a share link", "Makes a share link on the node that hosts the current org (`host`, or the one kept): a scope of `vault` (the host's only vault when unset) given as `folder`, `tag`, `note`, `notes` or `base` (else the whole vault); `audience` public, link (default: anyone with the link), org (the org's signed-in members) or invited:<email>,<email>; `access` read (default) or comment; `expires` never (default), 1h, 24h, 7d, 30d or a date YYYY-MM-DD; `title`; `listen`, how far the link plays: page (the notes and files no extension rule places higher), reference (default: also the files a reference player needs, the song's mix and project read-only) or full (every file, the multitracks); the host refuses a file above it. Answers the share and its `url`.";
    SHARE_LIST = SHARE_LIST, "Share links", "The share links of the current org's host: code, url, scope, audience, access, expiry, how far it plays (`listen`), whether it is live, expired or revoked, and its audiences.";
    SHARE_UPDATE = SHARE_UPDATE, "Change a share link", "Changes share `code`: `audience`, `access`, `expires` (never, 1h, 24h, 7d, 30d, YYYY-MM-DD), `title` or `listen` (page, reference or full: how far its link plays, and the most any of its audiences may). The next request to the link is answered by the change.";
    SHARE_REVOKE = SHARE_REVOKE, "Revoke a share link", "Takes share `code` down: its link answers 404 from the next request on. Final.";
    SHARE_COMMENTS = SHARE_COMMENTS, "Share comments", "The comments left on share `code` (page, by, text, when).";
    SHARE_AUDIENCE_ADD = SHARE_AUDIENCE_ADD, "Add a share audience", "Adds a named audience to share `code`: `name` (violin), and `filter`, a Bases filter expression over the notes of the share's scope (`instrument == \"violin\"`, `file.hasTag(\"rhythm\")`; empty: every note). The audience gets a link of its own, `<host>/s/<its code>/`, that shows and serves only the notes the filter holds for and the files they attach; the share's own link stays the whole scope. `listen` (page, reference or full; unset: the share's) narrows how far its link plays, never past the share's. Answers the share and the audience's `url`.";
    SHARE_LAYERS_KEPT = SHARE_LAYERS_KEPT, "Kept share layer rules", "The `[[share_layers]]` rules the host keeps for extensions no longer installed (#528): each extension's id, the note types it placed and since when. They still hold files back on share links, so removing an extension never opens one, until SHARE_LAYERS_FORGET.";
    SHARE_LAYERS_FORGET = SHARE_LAYERS_FORGET, "Forget kept share layer rules", "Forgets the kept `[[share_layers]]` rules of `extension` (no longer installed) on the host: its note types' files are placed by the rules installed now, and may open on links that play lower. Refused for an installed extension. Answers what is kept after.";
    SHARE_AUDIENCE_UPDATE = SHARE_AUDIENCE_UPDATE, "Change a share audience", "Changes audience `name` of share `code`: `listen`, how far its link plays (page, reference or full, never past the share's; `share`: the share's own). Answers the share and the audience.";
    SHARE_AUDIENCE_REMOVE = SHARE_AUDIENCE_REMOVE, "Remove a share audience", "Removes audience `name` from share `code`: its link answers 404 from the next request on. The share's own link and its other audiences stay.";
    SHARE_AUDIENCES = SHARE_AUDIENCES, "Share audiences", "The named audiences of share `code`: name, filter, code, url and how far its link plays (`listen`, and `listen_set` when it narrows the share's) of each.";
    SHARE_ARCHIVE = SHARE_ARCHIVE, "Download a share's files", "Saves the zip a share link's Download all gives to the file `to`: the files of share `code`, or of its `audience` alone, in folders by `by` (order: numbered files; note (default): a numbered folder per note; property:<key>: a folder per value), all of them or one `group` (a note's title or a property's value). Streamed from the host; answers the file, its bytes and how many files it holds.";
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
    /// page, reference (default) or full.
    pub listen: Option<String>,
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
    /// page, reference or full.
    pub listen: Option<String>,
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

/// What [`ids::SHARE_AUDIENCE_ADD`] takes.
#[derive(Debug, Clone, Facet)]
pub struct AudienceAddArgs {
    pub code: String,
    pub name: String,
    /// A Bases filter expression; none: every note of the scope.
    pub filter: Option<String>,
    /// page, reference or full; none: the share's.
    pub listen: Option<String>,
    pub host: Option<String>,
}

/// What [`ids::SHARE_LAYERS_FORGET`] takes.
#[derive(Debug, Clone, Facet)]
pub struct LayersForgetArgs {
    /// The extension's id (`fts.studio`).
    pub extension: String,
    pub host: Option<String>,
}

/// One extension's kept layer rules, as the actions answer them.
#[derive(Debug, Clone, Facet)]
pub struct KeptRow {
    pub extension: String,
    pub note_types: Vec<String>,
    /// Unix seconds: when the host last read them from its manifest.
    pub since: i64,
}

/// What [`ids::SHARE_LAYERS_KEPT`] and [`ids::SHARE_LAYERS_FORGET`]
/// answer.
#[derive(Debug, Clone, Facet)]
pub struct KeptOut {
    pub kept: Vec<KeptRow>,
}

fn kept_out(kept: Vec<architect_spaces::KeptLayers>) -> KeptOut {
    KeptOut {
        kept: kept
            .into_iter()
            .map(|k| KeptRow {
                extension: k.extension,
                note_types: k.note_types,
                since: k.since,
            })
            .collect(),
    }
}

/// What [`ids::SHARE_AUDIENCE_UPDATE`] takes.
#[derive(Debug, Clone, Facet)]
pub struct AudienceUpdateArgs {
    pub code: String,
    pub name: String,
    /// page, reference or full; `share`: the share's own.
    pub listen: String,
    pub host: Option<String>,
}

/// What [`ids::SHARE_AUDIENCE_REMOVE`] takes.
#[derive(Debug, Clone, Facet)]
pub struct AudienceRemoveArgs {
    pub code: String,
    pub name: String,
    pub host: Option<String>,
}

/// What [`ids::SHARE_ARCHIVE`] takes.
#[derive(Debug, Clone, Facet)]
pub struct ArchiveArgs {
    pub code: String,
    /// The file to write (`~/Downloads/violins.zip`).
    pub to: String,
    pub audience: Option<String>,
    /// order, note (default) or property:<key>.
    pub by: Option<String>,
    pub group: Option<String>,
    pub host: Option<String>,
}

/// What [`ids::SHARE_ARCHIVE`] answers.
#[derive(Debug, Clone, Facet)]
pub struct ArchiveOut {
    /// The file written.
    pub path: String,
    pub bytes: u64,
    /// How many files the archive holds.
    pub files: u64,
    /// Where it came from.
    pub url: String,
}

/// One audience of a share, as the actions answer it.
#[derive(Debug, Clone, Facet)]
pub struct AudienceRow {
    pub name: String,
    /// Its Bases filter expression.
    pub filter: String,
    pub code: String,
    /// Its own link, on the host.
    pub url: Option<String>,
    /// How far its link plays: page, reference or full.
    pub listen: String,
    /// The layer it was set to, when it narrows the share's; none: the
    /// share's.
    pub listen_set: Option<String>,
}

impl AudienceRow {
    fn of(share: &Share, audience: &ShareAudience, accounts: &Accounts) -> Self {
        Self {
            name: audience.name.clone(),
            filter: audience.filter.clone(),
            code: audience.code.clone(),
            url: accounts.org_host().map(|host| audience.url(&host)),
            listen: share.listen_for(Some(audience)).label().to_owned(),
            listen_set: audience.listen.map(|l| l.label().to_owned()),
        }
    }
}

/// What [`ids::SHARE_AUDIENCES`] and [`ids::SHARE_AUDIENCE_REMOVE`] answer.
#[derive(Debug, Clone, Facet)]
pub struct AudiencesOut {
    pub code: String,
    pub audiences: Vec<AudienceRow>,
}

/// What [`ids::SHARE_AUDIENCE_ADD`] answers.
#[derive(Debug, Clone, Facet)]
pub struct AudienceOut {
    pub share: ShareRow,
    pub audience: AudienceRow,
}

/// How many files a zip's directory says it holds (its last 22 bytes,
/// when it has no comment).
fn zip_count(path: &std::path::Path) -> u64 {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let mut end = [0_u8; 22];
    let read = std::fs::File::open(path).and_then(|mut f| {
        f.seek(SeekFrom::End(-22))?;
        f.read_exact(&mut end)
    });
    match (read, end) {
        (Ok(()), [0x50, 0x4b, 0x05, 0x06, _, _, _, _, _, _, lo, hi, ..]) => {
            u64::from(u16::from_le_bytes([lo, hi]))
        }
        _ => 0,
    }
}

/// `~/x` under the home folder.
fn expand(path: &str) -> std::path::PathBuf {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    match (path.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => std::path::PathBuf::from(path),
    }
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
    /// How far its own link plays: page, reference or full.
    pub listen: String,
    /// Its named audiences, each with its own link.
    pub audiences: Vec<AudienceRow>,
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
            listen: share.listen.label().to_owned(),
            audiences: share
                .audiences
                .iter()
                .map(|a| AudienceRow::of(share, a, accounts))
                .collect(),
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
            let listen = args
                .listen
                .as_deref()
                .map_or_else(|| Ok(Listen::default()), Listen::parse)?;
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
                        listen,
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
            if let Some(listen) = &args.listen {
                changes.push(ShareChange::Listen(Listen::parse(listen)?));
            }
            if changes.is_empty() {
                return Err(
                    "say what to change: audience, access, expires, title or listen".to_owned(),
                );
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
        &SHARE_AUDIENCE_ADD,
        Effect::Mutating,
        move |args: AudienceAddArgs| {
            let a = a.clone();
            async move {
                let new = NewAudience {
                    name: args.name.clone(),
                    filter: args.filter.unwrap_or_default(),
                    listen: args.listen.as_deref().map(Listen::parse).transpose()?,
                };
                let share = a
                    .share_audience_add(args.host.as_deref(), &args.code, new)
                    .await?;
                let audience = share
                    .audience_named(&args.name)
                    .map(|made| AudienceRow::of(&share, made, &a))
                    .ok_or_else(|| "the audience was not added".to_owned())?;
                Ok(AudienceOut {
                    share: ShareRow::of(&share, &a),
                    audience,
                })
            }
        },
    )?;
    let a = accounts.clone();
    list.register_async(
        &SHARE_AUDIENCE_REMOVE,
        Effect::Destructive,
        move |args: AudienceRemoveArgs| {
            let a = a.clone();
            async move {
                let share = a
                    .share_audience_remove(args.host.as_deref(), &args.code, &args.name)
                    .await?;
                Ok(AudiencesOut {
                    code: share.code.clone(),
                    audiences: share
                        .audiences
                        .iter()
                        .map(|x| AudienceRow::of(&share, x, &a))
                        .collect(),
                })
            }
        },
    )?;
    let a = accounts.clone();
    list.register_async(
        &SHARE_AUDIENCE_UPDATE,
        Effect::Mutating,
        move |args: AudienceUpdateArgs| {
            let a = a.clone();
            async move {
                let listen = match args.listen.trim() {
                    "share" | "" => None,
                    other => Some(Listen::parse(other)?),
                };
                let share = a
                    .share_audience_listen(args.host.as_deref(), &args.code, &args.name, listen)
                    .await?;
                let audience = share
                    .audience_named(&args.name)
                    .map(|x| AudienceRow::of(&share, x, &a))
                    .ok_or_else(|| format!("no audience {}", args.name))?;
                Ok(AudienceOut {
                    share: ShareRow::of(&share, &a),
                    audience,
                })
            }
        },
    )?;
    let a = accounts.clone();
    list.register_async(
        &SHARE_LAYERS_KEPT,
        Effect::ReadOnly,
        move |args: HostArgs| {
            let a = a.clone();
            async move { Ok(kept_out(a.share_layers_kept(args.host.as_deref()).await?)) }
        },
    )?;
    let a = accounts.clone();
    list.register_async(
        &SHARE_LAYERS_FORGET,
        Effect::Destructive,
        move |args: LayersForgetArgs| {
            let a = a.clone();
            async move {
                Ok(kept_out(
                    a.share_layers_forget(args.host.as_deref(), &args.extension)
                        .await?,
                ))
            }
        },
    )?;
    let a = accounts.clone();
    list.register_async(&SHARE_AUDIENCES, Effect::ReadOnly, move |args: CodeArgs| {
        let a = a.clone();
        async move {
            let share = a.share(args.host.as_deref(), &args.code).await?;
            Ok(AudiencesOut {
                code: share.code.clone(),
                audiences: share
                    .audiences
                    .iter()
                    .map(|x| AudienceRow::of(&share, x, &a))
                    .collect(),
            })
        }
    })?;
    let a = accounts.clone();
    list.register_async(
        &SHARE_ARCHIVE,
        Effect::Mutating,
        move |args: ArchiveArgs| {
            let a = a.clone();
            async move {
                if args.to.trim().is_empty() {
                    return Err("say where to save it: `to`".to_owned());
                }
                let to = expand(args.to.trim());
                let ask = ArchiveAsk {
                    code: args.code.clone(),
                    audience: args.audience.clone(),
                    by: args.by.clone().unwrap_or_else(|| "note".to_owned()),
                    group: args.group.clone(),
                };
                let (url, bytes) = a.share_archive(args.host.as_deref(), &ask, &to).await?;
                Ok(ArchiveOut {
                    files: zip_count(&to),
                    path: to.to_string_lossy().into_owned(),
                    bytes,
                    url,
                })
            }
        },
    )?;
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
