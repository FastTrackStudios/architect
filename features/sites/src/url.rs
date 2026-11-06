//! Where a note is published, and the ids its headings get.

use std::fmt::Write as _;

/// One path segment of a URL, from a file or folder name: lower case,
/// whitespace to `-`, and nothing but letters, digits, `-`, `_` and `.`.
/// `Getting Started` is `getting-started`.
#[must_use]
pub fn slug_segment(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.trim().chars() {
        if ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.extend(ch.to_lowercase());
        } else if (ch.is_whitespace() || ch == '+') && !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// A heading's text as its `id`.
///
/// Lower case, every run of anything but letters and digits one `-`: the
/// rule Zola uses, so an anchor written
/// against a docs folder authored for it (`@/guide/ui.md#the-status-line`)
/// still lands.
#[must_use]
pub fn fragment_id(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.trim().chars() {
        if ch.is_alphanumeric() || ch == '_' {
            out.extend(ch.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        // `#` alone addresses the top of the page, not the heading.
        out.push_str("section");
    }
    out
}

/// Whether `path` names a markdown note (`.md`, any case).
#[must_use]
pub fn is_md(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// Whether a note's file name makes it its folder's page: Zola's
/// `_index.md`, an `index.md`, or a `README.md`.
#[must_use]
pub fn is_index_name(basename: &str) -> bool {
    matches!(
        basename.to_ascii_lowercase().as_str(),
        "_index" | "index" | "readme"
    )
}

/// A note's folders, from its vault path (`guide/ui.md` → `["guide"]`).
#[must_use]
pub fn folders(rel_path: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = rel_path.split('/').filter(|s| !s.is_empty()).collect();
    parts.pop();
    parts
}

/// The URL of a folder path under `base` (`/`, `/docs/`), ending in `/`.
#[must_use]
pub fn folder_url(base: &str, folders: &[&str]) -> String {
    let mut url = base.to_owned();
    for folder in folders {
        url.push_str(&slug_segment(folder));
        url.push('/');
    }
    url
}

/// `base` as a site keeps it: starting and ending in `/`.
#[must_use]
pub fn normalise_base(base: &str) -> String {
    let inner = base.trim().trim_matches('/');
    if inner.is_empty() {
        "/".to_owned()
    } else {
        format!("/{inner}/")
    }
}

/// A vault path as a URL path: each segment percent-encoded where a URL
/// needs it (a space in an attachment's name).
#[must_use]
pub fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(char::from(byte));
            }
            other => {
                let _ = write!(out, "%{other:02X}");
            }
        }
    }
    out
}

/// `%20` and friends back to text, for a link written URL-encoded
/// (`[x](My%20Note.md)`). Leaves a malformed escape as written.
#[must_use]
pub fn decode_percent(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0_usize;
    while let Some(&byte) = bytes.get(at) {
        let escaped = (byte == b'%')
            .then(|| bytes.get(at.saturating_add(1)..at.saturating_add(3)))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(decoded) = escaped {
            out.push(decoded);
            at = at.saturating_add(3);
        } else {
            out.push(byte);
            at = at.saturating_add(1);
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_and_fragments() {
        assert_eq!(slug_segment("Getting Started"), "getting-started");
        assert_eq!(slug_segment("key+meter"), "key-meter");
        assert_eq!(fragment_id("The status line"), "the-status-line");
        assert_eq!(
            fragment_id("`EngineHost::site`: a mount"),
            "enginehost-site-a-mount"
        );
        assert_eq!(fragment_id("!!"), "section");
    }

    #[test]
    fn urls() {
        assert_eq!(normalise_base(""), "/");
        assert_eq!(normalise_base("docs"), "/docs/");
        assert_eq!(
            folder_url("/", &["Guide", "Deep Dive"]),
            "/guide/deep-dive/"
        );
        assert_eq!(folders("guide/ui.md"), vec!["guide"]);
        assert!(is_index_name("_index") && is_index_name("README"));
        assert_eq!(encode_path("img/My Pic.png"), "img/My%20Pic.png");
        assert_eq!(decode_percent("My%20Note.md"), "My Note.md");
        assert_eq!(decode_percent("100%"), "100%");
    }
}
