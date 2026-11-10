//! Highlighting through the registry (feature `syntax`): the spans of a
//! source in a registered language, each named by the theme token it is
//! drawn in.
//!
//! The tree-sitter work is the editor's (`editor-syntax`). At the editor
//! revision architect pins it highlights only the grammars it was
//! compiled with, with the queries they ship: `editor_syntax::highlight`
//! takes its closed `Lang` enum, and `grammar_for` (private) is the only
//! source of a grammar and a query. So a registered language is
//! highlighted when its grammar is [`Grammar::Linked`], its id or one of
//! its fences names one of those grammars, and it brings no query of its
//! own; anything else is [`Unsupported`], which says what is missing.

use crate::languages::{Grammar, Languages, Registered};
use crate::theme::token_for_tag;

/// A highlighted byte range of the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    /// The theme token it is drawn in (`syntax.keyword`).
    pub token: &'static str,
    /// The editor's scope tag (`k`): its CSS class is `md-tok-<tag>`.
    pub tag: &'static str,
}

/// Why a source was not highlighted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// No language of that id, fence name or file extension is
    /// registered.
    Unknown(String),
    /// Its grammar is a file: the pinned `editor-syntax` cannot load one.
    GrammarFile { language: String, grammar: String },
    /// It brings its own highlights query: the pinned `editor-syntax`
    /// cannot take one.
    OwnQuery { language: String },
    /// It says `linked`, but the editor has no grammar compiled in under
    /// its id or fences.
    NotLinked { language: String },
}

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(name) => write!(f, "no language {name} is registered"),
            Self::GrammarFile { language, grammar } => write!(
                f,
                "{language}'s grammar is the file {grammar}, and the editor cannot load a grammar file yet"
            ),
            Self::OwnQuery { language } => write!(
                f,
                "{language} brings its own highlights query, and the editor cannot take one yet"
            ),
            Self::NotLinked { language } => write!(
                f,
                "{language} says its grammar is linked, but the editor has none compiled in for it"
            ),
        }
    }
}

impl std::error::Error for Unsupported {}

/// The grammar compiled into the editor for `registered`, by its id then
/// its fences.
fn linked(registered: &Registered) -> Result<editor_syntax::Lang, Unsupported> {
    let language = &registered.language;
    match &language.grammar {
        Grammar::File(grammar) => {
            return Err(Unsupported::GrammarFile {
                language: language.id.clone(),
                grammar: grammar.clone(),
            });
        }
        Grammar::Linked => {}
    }
    if language.highlights.is_some() || language.highlights_text.is_some() {
        return Err(Unsupported::OwnQuery {
            language: language.id.clone(),
        });
    }
    std::iter::once(&language.id)
        .chain(&language.fences)
        .find_map(|name| editor_syntax::Lang::from_fence_tag(name))
        .ok_or_else(|| Unsupported::NotLinked {
            language: language.id.clone(),
        })
}

/// Whether the editor can highlight the registered language `name` (an
/// id, a fence name or a file extension) as it stands.
///
/// # Errors
/// What is missing.
pub fn supported(languages: &Languages, name: &str) -> Result<(), Unsupported> {
    let registered = languages
        .resolve(name)
        .ok_or_else(|| Unsupported::Unknown(name.to_owned()))?;
    linked(&registered).map(|_| ())
}

/// `source` highlighted as the registered language `name` (an id, a fence
/// name or a file extension): non-overlapping spans in order, each named
/// by its theme token. Ranges no role colours are left out.
///
/// # Errors
/// [`Unsupported`] when no such language is registered, or the editor
/// cannot highlight it as it stands.
pub fn highlight(
    languages: &Languages,
    name: &str,
    source: &str,
) -> Result<Vec<Span>, Unsupported> {
    let registered = languages
        .resolve(name)
        .ok_or_else(|| Unsupported::Unknown(name.to_owned()))?;
    let lang = linked(&registered)?;
    Ok(editor_syntax::highlight(lang, source)
        .into_iter()
        .filter_map(|t| {
            token_for_tag(t.tag).map(|token| Span {
                start: t.start,
                end: t.end,
                token,
                tag: t.tag,
            })
        })
        .collect())
}
