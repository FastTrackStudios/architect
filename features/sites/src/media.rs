//! Audio and video attachments: which files a page plays (#511).
//!
//! An embed of one (`![[take.mp3]]`) is drawn as a native `<audio>` or
//! `<video>` element marked `data-site-media`, which the page's player
//! (`assets/player.js`) follows: it is what the playhead reads and what a
//! share's queue plays. Its bytes are served with ranges by whoever hosts
//! the site (the node's share links answer `Range` on every file).

/// What a media attachment is drawn as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Audio,
    Video,
}

impl MediaKind {
    /// Its element's name (`audio`, `video`).
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }
}

/// The lower-case extension of `path` (a vault path or a URL), without
/// any query or fragment.
fn extension(path: &str) -> Option<String> {
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let name = path.rsplit('/').next().unwrap_or_default();
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty()).then(|| ext.to_ascii_lowercase())
}

/// Whether `path` is audio or video a browser plays, by its extension.
#[must_use]
pub fn media_kind(path: &str) -> Option<MediaKind> {
    match extension(path)?.as_str() {
        "mp3" | "m4a" | "aac" | "wav" | "ogg" | "oga" | "opus" | "flac" => Some(MediaKind::Audio),
        "mp4" | "m4v" | "webm" | "mov" | "ogv" => Some(MediaKind::Video),
        _ => None,
    }
}

/// The `Content-Type` of a media file, by its extension.
#[must_use]
pub fn media_type(path: &str) -> Option<&'static str> {
    Some(match extension(path)?.as_str() {
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "flac" => "audio/flac",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "ogv" => "video/ogg",
        _ => return None,
    })
}

/// Whether `path` is a synced-lyrics file (`.lrc`).
#[must_use]
pub fn is_lyrics(path: &str) -> bool {
    extension(path).as_deref() == Some("lrc")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_extension() {
        assert_eq!(media_kind("Set/Joy.MP3"), Some(MediaKind::Audio));
        assert_eq!(media_kind("/s/x/clip.webm?v=1"), Some(MediaKind::Video));
        assert_eq!(media_kind("chart.pdf"), None);
        assert_eq!(media_kind(".mp3"), None);
        assert_eq!(media_type("a.m4a"), Some("audio/mp4"));
        assert!(is_lyrics("Joy.lrc"));
        assert!(!is_lyrics("lrc"));
    }
}
