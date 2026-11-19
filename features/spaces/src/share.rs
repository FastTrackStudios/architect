//! Share links (#197, #374): part of a vault on a link.
//!
//! A [`Share`] is a scope of one vault (the whole vault, a folder, a tag,
//! a list of notes or a base: [`ShareScope`]), a code (the link is
//! `<node>/s/<code>/`, [`LINKS`]), who may open it ([`Audience`]), until when
//! ([`Share::expires_at`]) and what they may do there ([`ShareAccess`]).
//! The always-on node serves it as a scoped Site, and refuses it once
//! it is revoked or expired.
//!
//! A share for many people carries named slices of its scope (#510,
//! [`ShareAudience`]: `violin`, `rhythm`), each a filter over the scope's
//! notes in the Bases filter language, each on a link of its own
//! (`<node>/s/<its code>/`) that is removed alone. The share's own link
//! stays the whole scope. Who may open any of them is still the share's
//! [`Audience`].
//!
//! A share is org data: it is a note of the vault it shares
//! ([`share_note_path`], hidden from every Site), so it syncs to every
//! device that holds the vault, the way any note does, and the node that
//! serves the link learns of it there. This module is the model alone:
//! pure, wasm-clean. The wire is [`crate::ShareLinks`] (feature `vox`),
//! the note's text [`crate::share_note`] (feature `server`).

use facet::Facet;

/// The path every share link is under: `<node>/s/<code>/`.
///
/// A fixed prefix keeps codes from clashing with the web front end's own paths
/// (`/docs/`, `/app/`, its SPA fallback) on a host that serves both, and
/// gives a reverse proxy one prefix to send to the node. The node's share
/// sign-in is under it too (`/s/_signin`).
pub const LINKS: &str = "/s";

/// The folder of a vault its shares are kept in (one note each).
pub const SHARES_FOLDER: &str = "_architect/shares";

/// The note share `code` is kept in, in its vault.
#[must_use]
pub fn share_note_path(code: &str) -> String {
    format!("{SHARES_FOLDER}/{code}.md")
}

/// The note the comments on share `code` are kept in, in its vault.
#[must_use]
pub fn comments_note_path(code: &str) -> String {
    format!("{SHARES_FOLDER}/{code}.comments.md")
}

/// How long a code is: 16 characters of base32, 80 random bits.
pub const CODE_LEN: usize = 16;

const CODE_ALPHABET: &[u8; 32] = b"abcdefghijkmnpqrstuvwxyz23456789";

/// A code from ten random bytes: [`CODE_LEN`] lower-case letters and
/// digits, with no `l`, `o`, `0` or `1` to misread.
#[must_use]
pub fn code_from(bytes: [u8; 10]) -> String {
    let mut bits: u128 = 0;
    for b in bytes {
        bits = bits.wrapping_shl(8) | u128::from(b);
    }
    (0..CODE_LEN)
        .rev()
        .map(|i| {
            let at = usize::try_from(
                bits.checked_shr(u32::try_from(i.saturating_mul(5)).unwrap_or(0))
                    .unwrap_or(0)
                    & 31,
            )
            .unwrap_or(0);
            char::from(CODE_ALPHABET.get(at).copied().unwrap_or(b'a'))
        })
        .collect()
}

/// The path of the link with this code: `/s/<code>/` ([`LINKS`]).
#[must_use]
pub fn link_path(code: &str) -> String {
    format!("{LINKS}/{code}/")
}

/// Whether `s` is shaped like a code (what a link's first segment must
/// be before the node looks it up).
#[must_use]
pub fn is_code(s: &str) -> bool {
    s.len() == CODE_LEN && s.bytes().all(|b| CODE_ALPHABET.contains(&b))
}

/// The part of a vault a share opens.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Facet)]
#[repr(C)]
pub enum ShareScope {
    /// Every note of the vault.
    Vault,
    /// A folder and everything under it; the link is rooted there.
    Folder(String),
    /// Every note with this tag (nested tags too).
    Tag(String),
    /// These notes, by vault path.
    Notes(Vec<String>),
    /// The notes a `.base` file selects.
    Base(String),
}

impl ShareScope {
    /// In a sentence: `the folder Album`.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Vault => "the whole vault".to_owned(),
            Self::Folder(f) => format!("the folder {f}"),
            Self::Tag(t) => format!("the notes tagged #{}", t.trim_start_matches('#')),
            Self::Notes(n) if n.len() == 1 => format!("the note {}", n.join("")),
            Self::Notes(n) => format!("{} notes", n.len()),
            Self::Base(b) => format!("the notes of {b}"),
        }
    }
}

/// Who may open a share.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Facet)]
#[repr(C)]
pub enum Audience {
    /// Anyone, and search engines may list it.
    Public,
    /// Anyone who has the link; it asks not to be listed (`noindex`).
    Link,
    /// People signed in at the issuer who are members of the org.
    Org,
    /// These people (by email, any case), signed in at the issuer.
    Invited(Vec<String>),
}

impl Audience {
    /// Parse `public`, `link`, `org` or `invited:<email>,<email>`.
    ///
    /// # Errors
    ///
    /// Anything else, or `invited` with nobody.
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        match s.to_ascii_lowercase().as_str() {
            "public" => Ok(Self::Public),
            "link" | "anyone" => Ok(Self::Link),
            "org" | "members" => Ok(Self::Org),
            _ => {
                let emails = s
                    .split_once(':')
                    .filter(|(k, _)| k.eq_ignore_ascii_case("invited"))
                    .map(|(_, v)| split_emails(v))
                    .ok_or_else(|| {
                        format!("no audience `{s}`: public, link, org or invited:<emails>")
                    })?;
                if emails.is_empty() {
                    return Err("invited: name somebody".to_owned());
                }
                Ok(Self::Invited(emails))
            }
        }
    }

    /// Whether a viewer must be signed in.
    #[must_use]
    pub const fn signed_in(&self) -> bool {
        matches!(self, Self::Org | Self::Invited(_))
    }

    /// The same audience, its emails trimmed, lower case, sorted, once each.
    #[must_use]
    pub fn normalised(self) -> Self {
        match self {
            Self::Invited(emails) => Self::Invited(split_emails(&emails.join(","))),
            other => other,
        }
    }

    /// As [`Self::parse`] reads it.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Public => "public".to_owned(),
            Self::Link => "link".to_owned(),
            Self::Org => "org".to_owned(),
            Self::Invited(e) => format!("invited:{}", e.join(",")),
        }
    }
}

fn split_emails(s: &str) -> Vec<String> {
    let mut out: Vec<String> = s
        .split([',', ' ', ';'])
        .map(|e| e.trim().to_lowercase())
        .filter(|e| e.contains('@'))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// How far a link plays (#528): the most its viewers may fetch.
///
/// Each layer holds the ones below it. The sharer sets it per link and
/// per audience; a viewer opts deeper within it; the node refuses a file
/// above it (an extension's `[[share_layers]]` rules say which layer each
/// file of a note type is in; a file no rule names is a page file).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Facet)]
#[repr(u8)]
pub enum Listen {
    /// The page alone: notes, charts, lyrics, files no rule places
    /// higher. No audio engine.
    Page,
    /// The page and the reference: what an engine needs to play the
    /// song's mix with a live click and guide, the project read-only
    /// (Studio's Reference Mode). The default for a new link.
    #[default]
    Reference,
    /// Everything: every stem, for the full multitrack engine.
    Full,
}

impl Listen {
    /// Every layer, lowest first.
    pub const ALL: [Self; 3] = [Self::Page, Self::Reference, Self::Full];

    /// `page`, `reference` or `full`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::Reference => "reference",
            Self::Full => "full",
        }
    }

    /// Parse `page`, `reference` or `full` (any case).
    ///
    /// # Errors
    ///
    /// Anything else.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "page" | "none" => Ok(Self::Page),
            "reference" | "ref" => Ok(Self::Reference),
            "full" | "multitrack" | "multitracks" => Ok(Self::Full),
            other => Err(format!("no layer `{other}`: page, reference or full")),
        }
    }
}

/// The longest name a [`ShareAudience`] may have.
pub const AUDIENCE_NAME_MAX: usize = 48;
/// The most named audiences one share carries.
pub const AUDIENCES_MAX: usize = 64;

/// A named slice of a share (#510): the notes of the share's scope that
/// `filter` holds for, on a link of its own.
///
/// Not who may open it (that is the share's [`Audience`]): which part of
/// the share its link shows. A note the filter does not hold for, and an
/// attachment only such notes link, is neither listed nor served on its
/// link.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Facet)]
pub struct ShareAudience {
    /// Its name in the share (`violin`): unique there, any case.
    pub name: String,
    /// A Bases filter expression over a note's properties and file
    /// (`instrument == "violin"`, `file.hasTag("rhythm")`); empty: every
    /// note of the scope.
    pub filter: String,
    /// Its own link's code: `<node>/s/<code>/` ([`is_code`]).
    pub code: String,
    pub created_at: i64,
    /// How far its link plays, never past the share's ([`Share::listen`]);
    /// None (and an audience written before layers): the share's.
    #[facet(default)]
    pub listen: Option<Listen>,
}

impl ShareAudience {
    /// Its path on the node: `/s/<code>/` ([`LINKS`]).
    #[must_use]
    pub fn path(&self) -> String {
        link_path(&self.code)
    }

    /// Its link on the node at `origin`.
    #[must_use]
    pub fn url(&self, origin: &str) -> String {
        format!("{}{}", origin.trim_end_matches('/'), self.path())
    }
}

/// A [`ShareAudience`] before it has a code
/// ([`crate::ShareLinks::add_audience`]).
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct NewAudience {
    pub name: String,
    pub filter: String,
    /// How far its link plays; None: the share's.
    pub listen: Option<Listen>,
}

/// Check a new audience's name against the ones `share` has (the filter
/// is the node's to parse: the Bases filter language is not in this
/// crate).
///
/// # Errors
///
/// An empty or over-long name, one with a control character or a slash,
/// one the share already has, or a share with [`AUDIENCES_MAX`].
pub fn validate_audience(share: &Share, new: &NewAudience) -> Result<(), ShareFault> {
    let bad = |why: String| Err(ShareFault::Invalid(why));
    let name = new.name.trim();
    if name.is_empty() {
        return bad("name the audience".to_owned());
    }
    if name.chars().count() > AUDIENCE_NAME_MAX {
        return bad(format!(
            "an audience's name is at most {AUDIENCE_NAME_MAX} characters"
        ));
    }
    if name
        .chars()
        .any(|c| c.is_control() || c == '/' || c == '\\')
    {
        return bad("an audience's name has no slash or control character".to_owned());
    }
    if share.audience_named(name).is_some() {
        return bad(format!("this share already has an audience `{name}`"));
    }
    if share.audiences.len() >= AUDIENCES_MAX {
        return bad(format!("a share has at most {AUDIENCES_MAX} audiences"));
    }
    Ok(())
}

/// What the people a share lets in may do there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Facet)]
#[repr(u8)]
pub enum ShareAccess {
    /// Read the notes.
    Read,
    /// Read them and leave comments.
    Comment,
}

/// What of a link's files is offered as a download.
///
/// A permission of the link, not another page: every viewer plays what
/// the link's layer serves; this says which of those files the app lists
/// to save and puts in "Download all". A file that is not offered is
/// still streamed to the player (its bytes must reach the browser), so
/// this is what the link offers, not a lock on what a browser can keep.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Facet)]
#[repr(u8)]
pub enum Download {
    /// Every file the link serves (as before).
    #[default]
    Everything,
    /// Everything but recordings and multitracks ([`is_media`]): they
    /// play, and are not listed to save or put in the archive.
    NotMedia,
    /// Nothing: the link is for looking and listening.
    Nothing,
}

impl Download {
    /// `everything`, `not-media` or `nothing`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Everything => "everything",
            Self::NotMedia => "not-media",
            Self::Nothing => "nothing",
        }
    }

    /// Parse [`Self::label`] (any case).
    ///
    /// # Errors
    ///
    /// Anything else.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "everything" | "all" => Ok(Self::Everything),
            "not-media" | "notmedia" | "no-media" => Ok(Self::NotMedia),
            "nothing" | "none" => Ok(Self::Nothing),
            other => Err(format!(
                "no download rule `{other}`: everything, not-media or nothing"
            )),
        }
    }

    /// Whether the file at `path` is offered as a download.
    #[must_use]
    pub fn offers(self, path: &str) -> bool {
        match self {
            Self::Everything => true,
            Self::NotMedia => !is_media(path),
            Self::Nothing => false,
        }
    }
}

/// Whether `path` is a recording by its name: sound or video.
#[must_use]
pub fn is_media(path: &str) -> bool {
    const MEDIA: [&str; 19] = [
        "wav", "wave", "flac", "aif", "aiff", "mp3", "m4a", "aac", "ogg", "oga", "opus", "wv",
        "caf", "w64", "mp4", "m4v", "mov", "webm", "mkv",
    ];
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| MEDIA.iter().any(|m| ext.eq_ignore_ascii_case(m)))
}

/// What a link's viewers may do with a project it carries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Facet)]
#[repr(u8)]
pub enum ProjectAccess {
    /// Open it read-only: its sessions and the media they reference.
    #[default]
    Read,
    /// That, and save its session documents back: a new version of the
    /// file each time, by a signed-in viewer the link lets in (never an
    /// anonymous one).
    Edit,
}

/// A project a link carries beside its notes: session documents of one
/// file root and the media they reference, and nothing else of the root.
///
/// The root is named by its place, `<org>/Projects/<project>`, as the
/// files engine does. The node resolves each session's references to the
/// root's files (a path beside the session, a path in the root, or a
/// file's sha-256, as a string anywhere in the document) and serves
/// exactly those, by hash.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Facet)]
pub struct ShareProject {
    /// `<org>/Projects/<project>`.
    pub root: String,
    /// The session documents shared, root-relative
    /// (`songs/joy/Joy.fts-session`). None named: every session document
    /// in the root.
    #[facet(default)]
    pub sessions: Vec<String>,
    #[facet(default)]
    pub access: ProjectAccess,
}

/// The most projects one link carries.
pub const PROJECTS_MAX: usize = 16;

/// Whether `path` names a session document: what a project link shares
/// and reads references from.
#[must_use]
pub fn is_session_document(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, ext)| {
        ext.eq_ignore_ascii_case("fts-session") || ext.eq_ignore_ascii_case("session")
    })
}

/// Whether `projects` may be a share's.
///
/// # Errors
///
/// A root that is not `<org>/Projects/<name>`, a session that is not a
/// session document's path inside it, the same root twice, or too many.
pub fn validate_projects(projects: &[ShareProject]) -> Result<(), ShareFault> {
    let bad = |why: String| Err(ShareFault::Invalid(why));
    if projects.len() > PROJECTS_MAX {
        return bad(format!("a link carries at most {PROJECTS_MAX} projects"));
    }
    let plain = |path: &str| {
        !path.is_empty()
            && path
                .split('/')
                .all(|s| !s.is_empty() && s != "." && s != ".." && !s.contains(['\\', '\0']))
    };
    for (i, project) in projects.iter().enumerate() {
        let parts: Vec<&str> = project.root.split('/').collect();
        if !(parts.len() == 3 && parts.get(1) == Some(&"Projects") && plain(&project.root)) {
            return bad(format!(
                "`{}` is not a project's root: <org>/Projects/<project>",
                project.root
            ));
        }
        if projects
            .iter()
            .take(i)
            .any(|earlier| earlier.root == project.root)
        {
            return bad(format!("`{}` is named twice", project.root));
        }
        if let Some(session) = project
            .sessions
            .iter()
            .find(|s| !plain(s) || !is_session_document(s))
        {
            return bad(format!(
                "`{session}` is not a session document's path in the project"
            ));
        }
    }
    Ok(())
}

/// A share link.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Share {
    /// The link's code: `<node>/s/<code>/` ([`is_code`]).
    pub code: String,
    /// The vault it shares from (the node's id for it).
    pub vault: String,
    pub scope: ShareScope,
    /// The site's name. None: its home note's title, else the folder's.
    pub title: Option<String>,
    pub audience: Audience,
    pub access: ShareAccess,
    /// Unix seconds after which it is refused. None: until revoked.
    pub expires_at: Option<i64>,
    /// When it was revoked (unix seconds); refused from then on.
    pub revoked_at: Option<i64>,
    pub created_at: i64,
    /// The issuer user id of who made it.
    pub created_by: String,
    /// A domain of its own (`album.example.com`): the node serves the
    /// share at that host's root too.
    pub domain: Option<String>,
    /// Its named slices, each on a link of its own (#510). A share
    /// written before them reads as having none.
    #[facet(default)]
    pub audiences: Vec<ShareAudience>,
    /// How far its own link plays (#528), and the most any of its
    /// audiences may. A share note without it (written before layers, or
    /// one that lost the field) reads as [`Listen::Reference`], the
    /// default: access fails closed.
    #[facet(default)]
    pub listen: Listen,
    /// What of its files is offered as a download; a share written
    /// before it offers everything.
    #[facet(default)]
    pub download: Download,
    /// The projects it carries beside its notes: their session documents
    /// and the media those reference.
    #[facet(default)]
    pub projects: Vec<ShareProject>,
}

/// Where a share stands at one moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum ShareState {
    Live,
    Expired,
    Revoked,
}

/// Who is asking to open a share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Viewer {
    /// Nobody signed in.
    Anonymous,
    /// A person signed in at the issuer.
    Person {
        user_id: String,
        email: Option<String>,
        /// Whether they are a member of the org the vault is the org's.
        member: bool,
    },
}

/// A share's answer to a viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// In.
    Open,
    /// The audience is signed-in people and nobody is signed in.
    SignIn,
    /// Signed in, but not somebody the share lets in.
    Denied,
}

impl Share {
    /// Where it stands at `now` (unix seconds).
    #[must_use]
    pub fn state(&self, now: i64) -> ShareState {
        if self.revoked_at.is_some() {
            ShareState::Revoked
        } else if self.expires_at.is_some_and(|at| now >= at) {
            ShareState::Expired
        } else {
            ShareState::Live
        }
    }

    /// Whether `viewer` may open it (its state is the caller's to check
    /// first: [`Self::state`]).
    #[must_use]
    pub fn admits(&self, viewer: &Viewer) -> Verdict {
        match (&self.audience, viewer) {
            (Audience::Public | Audience::Link, _) => Verdict::Open,
            (_, Viewer::Anonymous) => Verdict::SignIn,
            (Audience::Org, Viewer::Person { member, .. }) => {
                if *member {
                    Verdict::Open
                } else {
                    Verdict::Denied
                }
            }
            (Audience::Invited(emails), Viewer::Person { email, .. }) => {
                let invited = email
                    .as_deref()
                    .is_some_and(|e| emails.iter().any(|i| i.eq_ignore_ascii_case(e.trim())));
                if invited {
                    Verdict::Open
                } else {
                    Verdict::Denied
                }
            }
        }
    }

    /// Its audience called `name` (any case, trimmed).
    #[must_use]
    pub fn audience_named(&self, name: &str) -> Option<&ShareAudience> {
        let name = name.trim();
        self.audiences
            .iter()
            .find(|a| a.name.trim().eq_ignore_ascii_case(name))
    }

    /// Its audience whose link has `code`.
    #[must_use]
    pub fn audience_at(&self, code: &str) -> Option<&ShareAudience> {
        self.audiences.iter().find(|a| a.code == code)
    }

    /// How far the link of `audience` plays (the share's own link when
    /// None): the audience's cap, never past the share's.
    #[must_use]
    pub fn listen_for(&self, audience: Option<&ShareAudience>) -> Listen {
        audience
            .and_then(|a| a.listen)
            .map_or(self.listen, |a| a.min(self.listen))
    }

    /// Whether `new` asks for the link this share already is: the same
    /// vault, scope and audience, with the same access, layer, expiry
    /// and domain, and still open. Sharing the same thing with the same
    /// people again gives the link they already have; asking for another
    /// audience, layer or access is asking for another link.
    #[must_use]
    pub fn is_the_link_of(&self, new: &NewShare, now: i64) -> bool {
        self.state(now) == ShareState::Live
            && self.vault == new.vault
            && self.scope == new.scope
            && self.audience == new.audience.clone().normalised()
            && self.access == new.access
            && self.listen == new.listen
            && self.expires_at == new.expires_at
            && self.domain.as_deref() == new.domain.as_deref().map(str::trim)
    }

    /// Whether the people it lets in may comment.
    #[must_use]
    pub fn comments(&self) -> bool {
        self.access == ShareAccess::Comment
    }

    /// Its path on the node: `/s/<code>/` ([`LINKS`]).
    #[must_use]
    pub fn path(&self) -> String {
        link_path(&self.code)
    }

    /// Its link on the node at `origin` (`https://architect.example`):
    /// `<origin>/s/<code>/`.
    #[must_use]
    pub fn url(&self, origin: &str) -> String {
        format!("{}{}", origin.trim_end_matches('/'), self.path())
    }
}

/// What a new share is: a [`Share`] before it has a code.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct NewShare {
    pub vault: String,
    pub scope: ShareScope,
    pub title: Option<String>,
    pub audience: Audience,
    pub access: ShareAccess,
    pub expires_at: Option<i64>,
    pub domain: Option<String>,
    /// How far its link plays ([`Listen::default`]: Reference).
    pub listen: Listen,
}

/// The `[[share_layers]]` rules of an extension that is no longer
/// installed, kept by the node so that removing it never opens a link
/// (#528): until the owner forgets them
/// ([`crate::ShareLinks::forget_layers`]).
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct KeptLayers {
    /// The extension's id (`fts.studio`).
    pub extension: String,
    /// The folder it was installed in, under the node's extensions folder
    /// (rules are kept by folder: two folders never share them).
    pub folder: String,
    /// The note types its rules place (`song`).
    pub note_types: Vec<String>,
    /// When the node last read them from its manifest (unix seconds).
    pub since: i64,
}

/// What the node holds of its layer rules (#528).
///
/// The rules kept for extensions no longer installed, and what the owner
/// should know (a manifest or the kept rules' file that does not read,
/// rules a reinstall lowered), until acknowledged
/// ([`crate::ShareLinks::acknowledge_layers`]).
#[derive(Debug, Clone, PartialEq, Eq, Default, Facet)]
pub struct LayersReport {
    pub kept: Vec<KeptLayers>,
    /// One line each, newest last.
    pub notices: Vec<String>,
}

/// One change to a share ([`crate::ShareLinks::update`]).
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
#[repr(C)]
pub enum ShareChange {
    Audience(Audience),
    Access(ShareAccess),
    /// How far its own link plays, and the most its audiences may.
    Listen(Listen),
    /// A new expiry; None: until revoked.
    Expires(Option<i64>),
    Title(Option<String>),
    /// A domain of its own, or none.
    Domain(Option<String>),
    /// What of its files is offered as a download.
    Download(Download),
    /// The projects it carries, all of them (none: an empty list).
    Projects(Vec<ShareProject>),
}

/// A comment left on a shared page.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct ShareComment {
    /// The page's path below the link (`/track/`).
    pub page: String,
    /// Who: a signed-in person's name or email, else what they typed.
    pub by: String,
    pub text: String,
    pub at: i64,
}

/// Why a share call failed.
#[derive(Debug, Clone, PartialEq, Eq, Facet, thiserror::Error)]
#[repr(u8)]
pub enum ShareFault {
    #[error("no caller: sharing needs a person")]
    Unauthenticated,
    #[error("no share {0}")]
    NotFound(String),
    #[error("not permitted: {0}")]
    Denied(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("shares: {0}")]
    Internal(String),
}

/// Check a new share, at `now`.
///
/// # Errors
///
/// No vault, an empty scope, an expiry already past, a domain that is
/// not a host name.
pub fn validate(new: &NewShare, now: i64) -> Result<(), ShareFault> {
    let bad = |why: &str| Err(ShareFault::Invalid(why.to_owned()));
    if new.vault.trim().is_empty() {
        return bad("name the vault");
    }
    match &new.scope {
        ShareScope::Folder(f) | ShareScope::Tag(f) | ShareScope::Base(f) if f.trim().is_empty() => {
            return bad("the scope names nothing");
        }
        ShareScope::Notes(n) if n.iter().all(|p| p.trim().is_empty()) => {
            return bad("name a note");
        }
        _ => {}
    }
    if new.expires_at.is_some_and(|at| at <= now) {
        return bad("the expiry is already past");
    }
    if let Audience::Invited(e) = &new.audience {
        if e.is_empty() {
            return bad("invite somebody");
        }
    }
    if let Some(domain) = &new.domain {
        validate_domain(domain)?;
    }
    Ok(())
}

/// Check a domain of a share's own: a host name, lower case.
///
/// # Errors
///
/// Not a host name (`album.example.com`).
pub fn validate_domain(domain: &str) -> Result<(), ShareFault> {
    let ok = domain.contains('.')
        && domain.len() <= 253
        && domain.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && l.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        });
    if ok {
        Ok(())
    } else {
        Err(ShareFault::Invalid(format!(
            "`{domain}` is not a host name"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn share(audience: Audience) -> Share {
        Share {
            code: code_from([7; 10]),
            vault: "notes".into(),
            scope: ShareScope::Folder("Album".into()),
            title: None,
            audience,
            access: ShareAccess::Read,
            expires_at: Some(100),
            revoked_at: None,
            created_at: 0,
            created_by: "u1".into(),
            domain: None,
            audiences: Vec::new(),
            listen: Listen::Full,
            download: Download::default(),
            projects: Vec::new(),
        }
    }

    #[test]
    fn a_link_plays_what_it_does_not_offer_to_keep() {
        assert!(Download::Everything.offers("Songs/joy.wav"));
        assert!(!Download::NotMedia.offers("Songs/joy.WAV"));
        assert!(!Download::NotMedia.offers("Songs/Joy/Media/kick.flac"));
        assert!(Download::NotMedia.offers("Songs/joy.pdf"));
        assert!(Download::NotMedia.offers("Songs/Joy.fts-session"));
        assert!(!Download::Nothing.offers("Songs/joy.pdf"));
        for rule in [Download::Everything, Download::NotMedia, Download::Nothing] {
            assert_eq!(Download::parse(rule.label()), Ok(rule));
        }
        assert!(Download::parse("some").is_err());
        assert!(is_media("a.mp3") && !is_media("a.mp3.txt") && !is_media("wav"));
    }

    #[test]
    fn a_project_scope_names_a_root_and_session_documents_in_it() {
        let project = |root: &str, sessions: &[&str]| ShareProject {
            root: root.to_owned(),
            sessions: sessions.iter().map(|s| (*s).to_owned()).collect(),
            access: ProjectAccess::Read,
        };
        let good = project("indie/Projects/Album", &["songs/joy/Joy.fts-session"]);
        assert!(validate_projects(std::slice::from_ref(&good)).is_ok());
        assert!(validate_projects(&[project("indie/Projects/Album", &[])]).is_ok());
        for bad in [
            project("indie/Assets/Tracks", &[]),
            project("indie/Projects", &[]),
            project("indie/Projects/../x", &[]),
            project("indie/Projects/Album", &["songs/joy/kick.wav"]),
            project("indie/Projects/Album", &["../other/Joy.session"]),
            project("indie/Projects/Album", &["/abs/Joy.session"]),
        ] {
            assert!(
                validate_projects(std::slice::from_ref(&bad)).is_err(),
                "{bad:?}"
            );
        }
        assert!(validate_projects(&[good.clone(), good]).is_err(), "twice");
        assert!(is_session_document("a/b.SESSION") && !is_session_document("a/b.session-wal"));
    }

    #[test]
    fn sharing_the_same_thing_with_the_same_people_is_the_same_link() {
        let mut s = share(Audience::Invited(vec![
            "b@x.test".into(),
            "a@x.test".into(),
        ]));
        s.audience = s.audience.clone().normalised();
        let again = |audience: Audience, scope: ShareScope| NewShare {
            vault: s.vault.clone(),
            scope,
            title: Some("another name".into()),
            audience,
            access: s.access,
            expires_at: None,
            domain: None,
            listen: s.listen,
        };
        let same = Audience::Invited(vec![" A@x.test".into(), "b@X.test".into()]);
        assert!(s.is_the_link_of(&again(same.clone(), s.scope.clone()), 1));
        assert!(!s.is_the_link_of(&again(Audience::Link, s.scope.clone()), 1));
        assert!(!s.is_the_link_of(&again(same.clone(), ShareScope::Tag("x".into())), 1));
        // Another layer or access is another link.
        let mut deeper = again(same.clone(), s.scope.clone());
        deeper.listen = Listen::Page;
        assert!(!s.is_the_link_of(&deeper, 1));
        let mut comments = again(same.clone(), s.scope.clone());
        comments.access = ShareAccess::Comment;
        assert!(!s.is_the_link_of(&comments, 1));
        let mut gone = s.clone();
        gone.revoked_at = Some(1);
        assert!(
            !gone.is_the_link_of(&again(same, s.scope.clone()), 2),
            "revoked"
        );
    }

    fn person(email: &str, member: bool) -> Viewer {
        Viewer::Person {
            user_id: "u".into(),
            email: Some(email.into()),
            member,
        }
    }

    #[test]
    fn codes_are_sixteen_unambiguous_characters() {
        let code = code_from([0xff, 0, 1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(is_code(&code), "{code}");
        assert_ne!(code, code_from([0; 10]));
        assert!(!is_code("guide"));
        assert!(!is_code("ABCDEFGHJKMNPQRS"));
    }

    #[test]
    fn links_are_under_the_share_prefix() {
        let s = share(Audience::Link);
        assert_eq!(s.path(), format!("/s/{}/", s.code));
        assert_eq!(
            s.url("https://architect.example/"),
            format!("https://architect.example/s/{}/", s.code)
        );
    }

    #[test]
    fn each_audience_lets_in_its_people() {
        let anon = Viewer::Anonymous;
        let member = person("alan@x.test", true);
        let outsider = person("Ada@X.test", false);
        for open in [Audience::Public, Audience::Link] {
            assert_eq!(share(open.clone()).admits(&anon), Verdict::Open);
            assert_eq!(share(open).admits(&outsider), Verdict::Open);
        }
        let org = share(Audience::Org);
        assert_eq!(org.admits(&anon), Verdict::SignIn);
        assert_eq!(org.admits(&member), Verdict::Open);
        assert_eq!(org.admits(&outsider), Verdict::Denied);
        let invited = share(Audience::parse("invited: ada@x.test").unwrap());
        assert_eq!(invited.admits(&anon), Verdict::SignIn);
        assert_eq!(invited.admits(&outsider), Verdict::Open);
        assert_eq!(invited.admits(&member), Verdict::Denied);
    }

    #[test]
    fn audiences_are_named_once_and_found_by_name_or_code() {
        let mut s = share(Audience::Link);
        let new = |name: &str| NewAudience {
            name: name.into(),
            filter: "instrument == \"violin\"".into(),
            listen: None,
        };
        assert!(validate_audience(&s, &new("violin")).is_ok());
        assert!(validate_audience(&s, &new("  ")).is_err());
        assert!(validate_audience(&s, &new("a/b")).is_err());
        assert!(validate_audience(&s, &new(&"x".repeat(AUDIENCE_NAME_MAX + 1))).is_err());
        s.audiences.push(ShareAudience {
            name: "Violin".into(),
            filter: new("").filter,
            code: code_from([9; 10]),
            created_at: 1,
            listen: None,
        });
        assert!(validate_audience(&s, &new(" violin ")).is_err());
        let found = s.audience_named("VIOLIN").unwrap();
        assert_eq!(s.audience_at(&found.code), Some(found));
        assert_eq!(found.path(), format!("/s/{}/", found.code));
        assert_ne!(found.path(), s.path());
        assert!(s.audience_at(&s.code).is_none());
    }

    #[test]
    fn an_audiences_cap_only_narrows_the_links() {
        assert_eq!(Listen::default(), Listen::Reference, "a new link's default");
        assert!(Listen::Page < Listen::Reference && Listen::Reference < Listen::Full);
        for l in Listen::ALL {
            assert_eq!(Listen::parse(l.label()), Ok(l));
        }
        assert!(Listen::parse("stems").is_err());
        let audience = |listen| ShareAudience {
            name: "band".into(),
            filter: String::new(),
            code: code_from([9; 10]),
            created_at: 1,
            listen,
        };
        let mut s = share(Audience::Link);
        s.listen = Listen::Reference;
        assert_eq!(s.listen_for(None), Listen::Reference);
        assert_eq!(s.listen_for(Some(&audience(None))), Listen::Reference);
        assert_eq!(
            s.listen_for(Some(&audience(Some(Listen::Page)))),
            Listen::Page
        );
        // Asking for more than the link offers is the link's.
        assert_eq!(
            s.listen_for(Some(&audience(Some(Listen::Full)))),
            Listen::Reference
        );
        s.listen = Listen::Full;
        assert_eq!(
            s.listen_for(Some(&audience(Some(Listen::Full)))),
            Listen::Full
        );
    }

    #[test]
    fn expiry_and_revoke() {
        let mut s = share(Audience::Link);
        assert_eq!(s.state(99), ShareState::Live);
        assert_eq!(s.state(100), ShareState::Expired);
        s.revoked_at = Some(50);
        assert_eq!(s.state(10), ShareState::Revoked);
    }

    #[test]
    fn a_new_share_is_checked() {
        let new = NewShare {
            vault: "notes".into(),
            scope: ShareScope::Folder(" ".into()),
            title: None,
            audience: Audience::Link,
            access: ShareAccess::Read,
            expires_at: None,
            domain: None,
            listen: Listen::default(),
        };
        assert!(validate(&new, 0).is_err());
        let ok = NewShare {
            scope: ShareScope::Vault,
            expires_at: Some(10),
            domain: Some("album.example.com".into()),
            ..new
        };
        assert!(validate(&ok, 0).is_ok());
        assert!(validate(&ok, 10).is_err());
        assert!(validate_domain("Evil/x").is_err());
    }
}
