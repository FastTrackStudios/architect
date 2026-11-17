//! How an extension renders a note type's text in the editor
//! (`[[renderers]]`, [`crate::contrib::RendererDecl`]).
//!
//! The text stays what is edited: a renderer only says how ranges of it
//! read. Its action answers [`Rendered`], a list of [`RenderItem`]s over
//! the text's bytes, which the app draws as the editor's live preview
//! draws markdown: a range hidden, a range shown as other words, a range
//! or a line in a [`Look`]. The line the caret is on shows its source.
//!
//! The looks are the app's, named, not CSS an extension sends: what a
//! renderer marks reads like the rest of the app in every theme.

use serde::{Deserialize, Serialize};

/// What a renderer's action answers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Rendered {
    pub items: Vec<RenderItem>,
}

/// One thing a renderer says about the text: byte offsets into what it
/// was given.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RenderItem {
    /// `from..to` is not drawn (a markup's marks: Cooklang's `@`, `{…}`).
    Conceal { from: usize, to: usize },
    /// `from..to` reads as `text`, in `look` (`@flour{250%g}` as "250 g
    /// flour").
    Replace {
        from: usize,
        to: usize,
        text: String,
        #[serde(default)]
        look: Look,
    },
    /// `from..to` in `look`, its text as it is.
    Mark { from: usize, to: usize, look: Look },
    /// The line holding `at` in `look` (a section's heading).
    Line { at: usize, look: Look },
}

impl RenderItem {
    /// Its range (a line's: `at..at`).
    #[must_use]
    pub const fn range(&self) -> (usize, usize) {
        match self {
            Self::Conceal { from, to }
            | Self::Replace { from, to, .. }
            | Self::Mark { from, to, .. } => (*from, *to),
            Self::Line { at, .. } => (*at, *at),
        }
    }

    /// Whether it changes what the text reads as (hidden or replaced): the
    /// caret's line shows its source instead.
    #[must_use]
    pub const fn hides(&self) -> bool {
        matches!(self, Self::Conceal { .. } | Self::Replace { .. })
    }

    /// Whether it fits `len` bytes of text on char boundaries of `text`.
    #[must_use]
    pub const fn fits(&self, text: &str) -> bool {
        let (from, to) = self.range();
        from <= to && to <= text.len() && text.is_char_boundary(from) && text.is_char_boundary(to)
    }
}

/// How a rendered range or line reads: the app's looks, which its theme
/// draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Look {
    /// As the text around it.
    #[default]
    Plain,
    /// A section's title.
    Heading,
    /// Bold.
    Strong,
    /// Quieter than the text (a comment, a unit).
    Muted,
    /// In the accent colour.
    Accent,
    /// A small rounded chip.
    Chip,
    /// A chip in the accent colour (what a note is about: an ingredient).
    ChipAccent,
    /// Monospaced.
    Code,
}

impl Look {
    /// Its class in the editor: `look-<name>` (the app styles it).
    #[must_use]
    pub const fn class(self) -> &'static str {
        match self {
            Self::Plain => "look-plain",
            Self::Heading => "look-heading",
            Self::Strong => "look-strong",
            Self::Muted => "look-muted",
            Self::Accent => "look-accent",
            Self::Chip => "look-chip",
            Self::ChipAccent => "look-chip-accent",
            Self::Code => "look-code",
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn items_read_from_an_actions_answer() {
        let r: Rendered = serde_json::from_value(serde_json::json!({ "items": [
            { "kind": "replace", "from": 0, "to": 13, "text": "250 g flour", "look": "chip-accent" },
            { "kind": "conceal", "from": 14, "to": 16 },
            { "kind": "line", "at": 20, "look": "heading" },
        ]}))
        .unwrap();
        assert_eq!(r.items.len(), 3);
        assert!(r.items[0].hides() && r.items[1].hides() && !r.items[2].hides());
        assert_eq!(Look::ChipAccent.class(), "look-chip-accent");
    }

    #[test]
    fn an_item_past_the_text_or_inside_a_char_does_not_fit() {
        let text = "é1";
        assert!(RenderItem::Conceal { from: 0, to: 2 }.fits(text));
        assert!(!RenderItem::Conceal { from: 1, to: 2 }.fits(text));
        assert!(!RenderItem::Conceal { from: 0, to: 9 }.fits(text));
    }
}
