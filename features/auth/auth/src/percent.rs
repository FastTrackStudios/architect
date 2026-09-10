//! Percent-encoding for the URLs an auth server builds.
//!
//! Four hand-rolled copies of the same byte loop lived across the
//! server — one per file that happened to need a query string — and one
//! of them decoded with `from_utf8(…).unwrap()` on a slice that a
//! `%`-escape near the end of the input can split mid-character. The
//! encoding half is not hard to get right, but writing it four times
//! means getting it right four times, and the copies had already
//! drifted: one of them left `/` alone and the others did not.
//!
//! `percent-encoding` is already in the tree through `url`, so this is
//! a thin naming of the two character sets an auth server actually
//! wants, not a new dependency.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

/// RFC 3986 *unreserved*: everything else is escaped.
///
/// The set to use for a value that must survive being embedded in
/// somebody else's query string — a `return_to` that itself carries a
/// query, most of all, where an unescaped `&` reads as a separator and
/// silently truncates the value.
const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// Unreserved, plus `/`.
///
/// For a value that is a *path* and is going into a query slot of its
/// own. `/` is legal in a query value, and leaving it alone is the
/// difference between a link a person can read and a wall of `%2F`.
const UNRESERVED_OR_SLASH: &AsciiSet = &UNRESERVED.remove(b'/');

/// Escape a value going inside a query-string parameter.
#[must_use]
pub fn encode_component(value: &str) -> String {
    utf8_percent_encode(value, UNRESERVED).to_string()
}

/// Escape a path-shaped value going inside a query-string parameter,
/// keeping `/` readable.
#[must_use]
pub fn encode_query_value(value: &str) -> String {
    utf8_percent_encode(value, UNRESERVED_OR_SLASH).to_string()
}

/// Undo either of the above.
///
/// Invalid UTF-8 in the escapes is replaced rather than rejected: this
/// decodes values that arrived from a browser, and a malformed one
/// should render as mojibake, not take the request down.
#[must_use]
pub fn decode(value: &str) -> String {
    percent_encoding::percent_decode_str(value)
        .decode_utf8_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::{decode, encode_component, encode_query_value};

    #[test]
    fn unreserved_characters_pass_through() {
        assert_eq!(encode_component("aZ09-_.~"), "aZ09-_.~");
    }

    #[test]
    fn a_nested_query_cannot_leak_its_separators() {
        // The regression: an unescaped `&` truncated `return_to`, and a
        // person finished sign-up stranded on a half-built redirect.
        let nested = "/oauth2/authorize?client_id=forum&redirect_uri=https://x/cb";
        let encoded = encode_component(nested);
        assert!(!encoded.contains('&'));
        assert!(!encoded.contains('?'));
        assert_eq!(decode(&encoded), nested);
    }

    #[test]
    fn a_path_keeps_its_slashes_but_loses_its_separators() {
        let encoded = encode_query_value("/a/b?x=1&y=2");
        assert_eq!(encoded, "/a/b%3Fx%3D1%26y%3D2");
        assert_eq!(decode(&encoded), "/a/b?x=1&y=2");
    }

    #[test]
    fn a_truncated_escape_does_not_panic() {
        // `from_utf8(&bytes[i + 1..i + 3]).unwrap()` is what this
        // replaces; a `%` in the last two bytes indexed past the end.
        assert_eq!(decode("abc%"), "abc%");
        assert_eq!(decode("abc%4"), "abc%4");
        assert_eq!(decode("%E2%9C"), "\u{fffd}");
    }

    #[test]
    fn a_round_trip_survives_non_ascii() {
        for raw in ["héllo wörld", "日本語", "a b&c=d?e#f"] {
            assert_eq!(decode(&encode_component(raw)), raw);
        }
    }
}
