//! A QR code as inline SVG.
//!
//! Drawn rather than rasterised: an SVG is a few hundred bytes of path
//! data in the document that already had to be sent, where a PNG would
//! be a base64 data URI several times the size, blurry when scaled, and
//! wrong in a printed page. It also needs no image crate.
//!
//! The only thing here that ever needs to be a QR is the `otpauth://`
//! URL on the two-factor enrolment page — which is exactly the case
//! where a camera is the only reasonable input method, because the
//! alternative is typing thirty-two base32 characters correctly.

use std::fmt::Write as _;

use qrcodegen::{QrCode, QrCodeEcc};

/// Render `text` as an SVG `<svg>` element.
///
/// # Errors
///
/// If `text` is too long to encode — around 2,900 bytes at this error
/// correction level, which no `otpauth://` URL approaches.
pub fn svg(text: &str, label: &str) -> Result<String, QrError> {
    // Medium correction: an `otpauth://` URL is read once, on a screen,
    // at arm's length. High correction would only make the modules
    // smaller for no benefit here.
    let code = QrCode::encode_text(text, QrCodeEcc::Medium).map_err(|_| QrError::TooLong)?;
    let size = code.size();
    // A one-module quiet zone would be out of spec; four is what the
    // standard asks for and what scanners assume.
    let quiet = 4_i32;
    let dimension = size.saturating_add(quiet.saturating_mul(2));

    let mut path = String::new();
    for y in 0..size {
        for x in 0..size {
            if code.get_module(x, y) {
                // One `M x y h1 v1 h-1 z` per dark module. Verbose, and
                // it gzips to almost nothing.
                let (px, py) = (x.saturating_add(quiet), y.saturating_add(quiet));
                // Writing to a `String` is infallible.
                let _ = write!(path, "M{px} {py}h1v1h-1z");
            }
        }
    }

    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {dimension} {dimension}\" \
         width=\"200\" height=\"200\" role=\"img\" aria-label=\"{label}\" \
         style=\"background:#fff;border-radius:6px;padding:0\" \
         shape-rendering=\"crispEdges\">\
         <rect width=\"{dimension}\" height=\"{dimension}\" fill=\"#fff\"/>\
         <path d=\"{path}\" fill=\"#000\"/></svg>"
    ))
}

#[derive(Debug, thiserror::Error)]
pub enum QrError {
    #[error("the value is too long to encode as a QR code")]
    TooLong,
}

#[cfg(test)]
mod tests {
    use super::svg;

    #[test]
    fn an_otpauth_url_encodes_and_carries_its_own_white_ground() {
        let url = "otpauth://totp/FastTrackStudio:ada@example.com\
                   ?secret=JBSWY3DPEHPK3PXP&issuer=FastTrackStudio";
        let rendered = svg(url, "Scan this").expect("encode");
        assert!(rendered.starts_with("<svg"));
        assert!(rendered.ends_with("</svg>"));
        // A QR is only readable dark-on-light, so it paints its own
        // ground rather than inheriting the page's — these pages are
        // dark.
        assert!(rendered.contains("fill=\"#fff\""));
        assert!(rendered.contains("fill=\"#000\""));
        assert!(rendered.contains("aria-label=\"Scan this\""));
    }

    #[test]
    fn something_far_too_long_is_an_error_not_a_panic() {
        let huge = "a".repeat(10_000);
        assert!(svg(&huge, "x").is_err());
    }
}
