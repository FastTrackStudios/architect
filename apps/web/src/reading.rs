//! A note's body, read (#668).
//!
//! Its headings, paragraphs and fences as blocks, without the fences that repeat a collection's list
//! (`architect_media::is_listing_fence`). What the web app's reading of a
//! collection note draws (`app::collection::ReadingBody`); no browser
//! here, so tests check it natively.

/// A block of a body, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    Heading(u8, String),
    Paragraph(String),
    Fence(String),
}

/// `body`'s headings, paragraphs and fences, the listing fences left out.
#[must_use]
pub fn blocks(body: &str) -> Vec<Reading> {
    let mut out = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let mut fence: Option<(bool, Vec<&str>)> = None;
    let flush = |para: &mut Vec<&str>, out: &mut Vec<Reading>| {
        if !para.is_empty() {
            out.push(Reading::Paragraph(para.join(" ")));
            para.clear();
        }
    };
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some((listing, lines)) = fence.as_mut() {
            if trimmed.starts_with("```") {
                if !*listing {
                    out.push(Reading::Fence(lines.join("\n")));
                }
                fence = None;
            } else {
                lines.push(line);
            }
            continue;
        }
        if let Some(info) = trimmed.strip_prefix("```") {
            flush(&mut para, &mut out);
            fence = Some((architect_media::is_listing_fence(info), Vec::new()));
        } else if trimmed.is_empty() {
            flush(&mut para, &mut out);
        } else if let Some(level) = heading_level(trimmed) {
            flush(&mut para, &mut out);
            let text = trimmed.get(usize::from(level)..).unwrap_or_default().trim();
            out.push(Reading::Heading(level, text.to_owned()));
        } else {
            para.push(trimmed);
        }
    }
    flush(&mut para, &mut out);
    if let Some((false, lines)) = fence {
        out.push(Reading::Fence(lines.join("\n")));
    }
    out
}

fn heading_level(line: &str) -> Option<u8> {
    let n = line.bytes().take_while(|b| *b == b'#').count();
    ((1..=6).contains(&n) && line.as_bytes().get(n) == Some(&b' '))
        .then(|| u8::try_from(n).ok())
        .flatten()
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_fence_is_folded_out_of_the_reading() {
        let body = "# Merry & Bright\n\nA set in two parts.\nCovers drawn.\n\n```studio-setlist\n## Opening\n1 | Sleigh Ride\n```\n\n```kf\n| C |\n```\n";
        assert_eq!(
            blocks(body),
            vec![
                Reading::Heading(1, "Merry & Bright".into()),
                Reading::Paragraph("A set in two parts. Covers drawn.".into()),
                Reading::Fence("| C |".into()),
            ]
        );
    }
}
