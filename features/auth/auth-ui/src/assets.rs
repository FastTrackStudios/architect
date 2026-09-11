//! The stylesheet these pages are drawn with.
//!
//! # Why it is a route and not an inline `<style>`
//!
//! Every page used to carry its CSS in the document. That was tolerable
//! while the CSS was one hand-written sheet; the design system's is
//! nearly seventy kilobytes, and re-sending it on every navigation
//! within a settings app is the difference between a page that feels
//! instant and one that does not.
//!
//! The URL carries a hash of the content, so the response can be cached
//! forever and a deployment that changes the CSS changes the URL. That
//! is the only cache strategy that is both fast and never stale: an
//! `ETag` still costs a round trip per page, and a plain `max-age` is a
//! promise to serve something out of date.
//!
//! An old URL is still served the current sheet rather than a 404. The
//! alternative — keeping every sheet a running server has ever built —
//! buys nothing, because the only client that can hold a stale URL is
//! one that already has the page it came from.

use std::sync::OnceLock;

use axum::extract::Path;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::chrome::STYLE;

/// The design system's tokens and every utility these pages use.
///
/// Compiled from `tailwind.css` in this crate, which scans both this
/// crate's sources and `architect-ui`'s — see the note there for why it
/// cannot simply inject `architect_ui::UTILITIES_CSS`.
const UTILITIES: &str = include_str!("../assets/utilities.css");

/// Everything the pages need, in cascade order.
///
/// The compiled utilities come first and this crate's own hand-written
/// sheet last, because during the conversion to the design system it is
/// the one that must keep winning.
fn sheet() -> &'static str {
    static SHEET: OnceLock<String> = OnceLock::new();
    SHEET.get_or_init(|| {
        let mut css = String::with_capacity(
            UTILITIES
                .len()
                .saturating_add(STYLE.len())
                .saturating_add(1),
        );
        css.push_str(UTILITIES);
        css.push('\n');
        css.push_str(STYLE);
        css
    })
}

/// The URL to link, with the content's fingerprint in it.
///
/// Stable for a given build and different for any other, which is what
/// makes the immutable cache header above safe to send.
#[must_use]
pub fn stylesheet_path() -> &'static str {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| format!("{PREFIX}/ui-{:016x}.css", fingerprint(sheet())))
}

const PREFIX: &str = "/auth/assets";

/// A hash of the sheet, for the URL.
///
/// Not a checksum anybody verifies — it only has to change when the
/// content does, so FNV-1a is enough and avoids a dependency.
fn fingerprint(css: &str) -> u64 {
    css.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x1000_0000_01b3)
    })
}

/// `GET /auth/assets/{file}`
pub async fn serve(Path(file): Path<String>) -> Response {
    // One file is served here, under many names. Anything else is a
    // 404 rather than the stylesheet, so a typo in a template shows up
    // as a missing file instead of a page that mysteriously works.
    // A URL segment this crate generates, not a filename off a disk —
    // so the comparison is deliberately case-sensitive, and written
    // with `strip_*` to say that rather than reading as a file
    // extension check.
    if file
        .strip_prefix("ui-")
        .and_then(|rest| rest.strip_suffix(".css"))
        .is_none()
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/css; charset=utf-8"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            ),
        ],
        sheet(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::{fingerprint, sheet, stylesheet_path};

    #[test]
    fn the_sheet_carries_the_tokens_the_utilities_and_our_own_rules() {
        let css = sheet();
        assert!(css.contains("--background"), "design tokens");
        assert!(css.contains("@layer utilities"), "compiled utilities");
        assert!(css.contains(".panel"), "this crate's own sheet");
    }

    #[test]
    fn our_own_rules_come_last_so_they_win() {
        let css = sheet();
        let tokens = css.find("--background").expect("tokens");
        let ours = css.find(".panel").expect("ours");
        assert!(tokens < ours, "the design system must not override us");
    }

    #[test]
    fn the_path_names_the_content() {
        let path = stylesheet_path();
        assert!(path.starts_with("/auth/assets/ui-"), "{path}");
        assert!(path.strip_suffix(".css").is_some(), "{path}");
        // Same content, same URL — a page rendered twice must not send
        // a browser back for a sheet it already holds.
        assert_eq!(path, stylesheet_path());
    }

    #[test]
    fn different_content_is_a_different_url() {
        assert_ne!(fingerprint("a{}"), fingerprint("b{}"));
        assert_ne!(fingerprint(""), fingerprint(" "));
    }
}
