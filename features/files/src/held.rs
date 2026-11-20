//! The files of a project the org's node holds, for a signed-in member's
//! browser.
//!
//! A vault's folder or a held root's (`crate::sync::Engine`'s
//! `hold_at`), listed with size and sha256 and read by range, under the
//! member's rights on the org's lane. What a share link's guest gets from
//! `<link>_files.json`, for the org's own people, so an extension in the
//! web app (an engine that streams a project) can open the member's own
//! project, not only a link's.
//!
//! ```text
//! GET  /org/{slug}/held/list/{place}?folder=          a folder's files (bearer)
//! POST /org/{slug}/held/ticket/{place}/{*path}        a read ticket for one file (bearer)
//! GET  /org/{slug}/held/content/{place}/{*path}?ticket=   its bytes, ranged (bearer, or a ticket)
//! ```
//!
//! `{slug}` is the org's, as on its lane (`/org/{slug}/vox`). `{place}` is
//! `vault:<name>` (a vault of the org's) or `root:<id>` (a root the node
//! holds, its document id). A listing and a ticket act for the bearer the
//! web app already holds (the issuer's access token the org lane takes, in
//! `Authorization: Bearer`); the bytes for the same bearer, or for a
//! short-lived read ticket in `?ticket=`, for a media element or a worker
//! that cannot send a header. A session is never read from a URL. Each
//! listed file's `url` carries a ticket for the folder listed, and
//! [`HeldTicket::url`] one for its file: what a ticket reads is asked again
//! on every use, so a member removed from the org reads nothing more.
//! Outside a member's rights, or outside the place, a path is absent.
//!
//! The node's side is `architect-desktop`'s `node::held`; the URL builder
//! here is wasm-clean, and the JSON methods have a typed HTTP client
//! (`HeldFilesHttpClient`).

use architect::http::{ByteBody, RequestHead, encode_rest, encode_segment};
use facet::Facet;

use crate::error::FilesFault;

/// A vault's place: `vault:<name>`.
pub const VAULT: &str = "vault:";
/// A held root's place: `root:<document id>`.
pub const ROOT: &str = "root:";

/// One file of a listing.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct HeldFile {
    /// Its path in the place (`Songs/Joy/stems/bass.wav`).
    pub path: String,
    pub size: u64,
    /// Lowercase hex sha256 of its bytes.
    pub sha256: Option<String>,
    /// Where its bytes are, on the node's origin, with a ticket for the
    /// folder listed (until [`HeldListing::expires_us`]).
    pub url: String,
}

/// A folder's files, every one under it, in path order.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct HeldListing {
    pub place: String,
    /// The folder listed (`""`: the whole place).
    pub folder: String,
    pub files: Vec<HeldFile>,
    /// When the files' URLs stop reading, µs since the epoch.
    pub expires_us: i64,
}

/// A read ticket's URL for one file.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct HeldTicket {
    pub url: String,
    pub expires_us: i64,
}

/// The node's files of a project, for an org's signed-in members.
#[architect::service(path = "org/{slug}/held")]
pub trait HeldFiles {
    /// Every file under `folder` of `place` (all of it without one) that
    /// the bearer may read: path, size, sha256 and a URL.
    #[http(get, path = "list/{place}")]
    async fn list(
        &self,
        slug: String,
        place: String,
        folder: Option<String>,
        token: String,
    ) -> Result<HeldListing, FilesFault>;

    /// A short-lived read ticket for the file at `path`, for the bearer,
    /// as a URL that carries it.
    #[http(post, path = "ticket/{place}/{*path}")]
    async fn ticket(
        &self,
        slug: String,
        place: String,
        path: String,
        token: String,
    ) -> Result<HeldTicket, FilesFault>;

    /// The file's bytes, answering `Range` with `206`: for the request's
    /// bearer, or its `?ticket=`.
    #[http(get, path = "content/{place}/{*path}")]
    async fn content(
        &self,
        slug: String,
        place: String,
        path: String,
        head: RequestHead,
    ) -> Result<ByteBody, FilesFault>;
}

/// Where the bytes of `path` in `place` are, under `base` (the node's
/// origin, or `""` for the page's own), with `ticket` when given.
#[must_use]
pub fn content_url(
    base: &str,
    slug: &str,
    place: &str,
    path: &str,
    ticket: Option<&str>,
) -> String {
    let base = base.trim_end_matches('/');
    let mut url = format!(
        "{base}/org/{}/held/content/{}/{}",
        encode_segment(&slug),
        encode_segment(&place),
        encode_rest(&path.trim_start_matches('/')),
    );
    if let Some(ticket) = ticket {
        url.push_str("?ticket=");
        url.push_str(&encode_segment(&ticket));
    }
    url
}

/// A path inside a place, made plain: no leading or trailing `/`, no
/// empty, `.` or `..` segment, nothing hidden (a segment starting with
/// `.`, as `.architect/`), no backslash or NUL. `None` when it is not one.
#[must_use]
pub fn clean_path(path: &str) -> Option<String> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        return Some(String::new());
    }
    let ok = trimmed
        .split('/')
        .all(|s| !s.is_empty() && !s.starts_with('.') && !s.contains(['\\', '\0']));
    ok.then(|| trimmed.to_owned())
}

/// Whether `path` is `folder` or inside it (`""` holds everything).
#[must_use]
pub fn is_within(path: &str, folder: &str) -> bool {
    folder.is_empty()
        || path == folder
        || path
            .strip_prefix(folder)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The content type a held file is served as, by its extension: only
/// types a browser shows inertly (a page or an SVG would run its scripts
/// on the node's origin), anything else bytes.
#[must_use]
pub fn content_type(path: &str) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or_default();
    let ext = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "m4a" => "audio/mp4",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "aif" | "aiff" => "audio/aiff",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "pdf" => "application/pdf",
        "txt" | "md" | "lrc" => "text/plain; charset=utf-8",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_made_plain_or_refused() {
        assert_eq!(clean_path("/Songs/Joy/").as_deref(), Some("Songs/Joy"));
        assert_eq!(clean_path("").as_deref(), Some(""));
        for bad in ["a/../b", "a//b", ".architect/x", "a/.hidden", "a\\b", "./a"] {
            assert_eq!(clean_path(bad), None, "{bad}");
        }
    }

    #[test]
    fn within_is_by_segment() {
        assert!(is_within("Songs/Joy/a.wav", "Songs/Joy"));
        assert!(is_within("Songs/Joy", "Songs/Joy"));
        assert!(!is_within("Songs/Joyful/a.wav", "Songs/Joy"));
        assert!(is_within("x", ""));
    }

    #[test]
    fn a_content_url_encodes_each_segment_and_carries_the_ticket() {
        assert_eq!(
            content_url(
                "https://n.test/",
                "indie",
                "vault:notes",
                "Songs/Joy & Co/a b.wav",
                Some("t.1")
            ),
            "https://n.test/org/indie/held/content/vault%3Anotes/Songs/Joy%20%26%20Co/a%20b.wav?ticket=t.1"
        );
    }

    #[test]
    fn nothing_that_runs_is_served_as_itself() {
        for name in ["a.html", "a.svg", "a.js", "a"] {
            assert_eq!(content_type(name), "application/octet-stream", "{name}");
        }
        assert_eq!(content_type("x/A.WAV"), "audio/wav");
    }
}
