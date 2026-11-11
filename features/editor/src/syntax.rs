//! Highlighting through the registry (feature `syntax`), and the registry
//! made the editor's.
//!
//! The tree-sitter work is the editor's (`editor-syntax`). A registered
//! [`Language`] becomes an editor [`Syntax`]:
//!
//! - `grammar = "linked"`: a grammar compiled into the editor, found by
//!   the language's id or a fence name, with its own highlights query or
//!   the language's.
//! - `grammar = "linked:<name>"`: a grammar the app or a linked extension
//!   linked into the binary and handed over with [`link_grammar`], with
//!   the query given there or the language's.
//! - a grammar file: never. The tree-sitter the editor uses cannot load a
//!   grammar at run time, and [`Language::check`] refuses one.
//!
//! [`drive`] makes architect's registry the editor's: every language set
//! in it is registered in `editor_state::languages` (what the markdown
//! pass asks for a fence and what [`language_for_path`] answers for a
//! `CodeEditor`), and withdrawn when its owner withdraws it. Each change
//! moves [`languages_revision`], so an editor on screen draws again
//! (`ui::use_languages_revision`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

pub use editor_state::languages::{
    HostLanguage, Lang, LanguageEntry, LanguageError, LanguageFn, Syntax, Token, highlight_with,
    host_language, language_by_id, language_for_extension, language_for_fence, language_for_path,
    languages_revision, register_language, registered_languages, withdraw_language,
};
/// A tree-sitter grammar as the editor takes it (`editor_syntax::Grammar`,
/// tree-sitter's `Language`). A grammar crate's `LanguageFn` converts to
/// one with `.into()`.
pub use editor_syntax::Grammar as TreeSitterGrammar;

use crate::languages::{Grammar, Language, Languages, Registered};
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

/// Why a language is not highlighted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// No language of that id, fence name or file extension is
    /// registered.
    Unknown(String),
    /// Its grammar is a file. Grammars are linked into the binary; the
    /// tree-sitter the editor uses cannot load one at run time.
    GrammarFile { language: String, grammar: String },
    /// It says `linked`, but the editor has no grammar compiled in under
    /// its id or fences; or `linked:<name>`, and nothing linked a grammar
    /// under that name ([`link_grammar`]).
    NotLinked { language: String, grammar: String },
    /// `linked:<name>` with no highlights query, from the language or
    /// from [`link_grammar`].
    NoQuery { language: String },
    /// Tree-sitter refused the grammar or the query: the reason.
    Refused { language: String, reason: String },
}

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(name) => write!(f, "no language {name} is registered"),
            Self::GrammarFile { language, grammar } => write!(
                f,
                "{language}'s grammar is the file {grammar}; grammars are linked into the app, \
                 not loaded at run time"
            ),
            Self::NotLinked { language, grammar } => write!(
                f,
                "{language}'s grammar is {grammar}, and no grammar is linked in under that name"
            ),
            Self::NoQuery { language } => write!(
                f,
                "{language} has no highlights query: give `highlights`, or one to link_grammar"
            ),
            Self::Refused { language, reason } => {
                write!(f, "{language}'s grammar or query was refused: {reason}")
            }
        }
    }
}

impl std::error::Error for Unsupported {}

/// A grammar handed over with [`link_grammar`].
#[derive(Clone)]
struct Linked {
    grammar: TreeSitterGrammar,
    highlights: Option<String>,
    /// Moves when the name is linked again, so languages on it are built
    /// again.
    generation: u64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn grammars() -> &'static Mutex<(HashMap<String, Linked>, u64)> {
    static GRAMMARS: std::sync::OnceLock<Mutex<(HashMap<String, Linked>, u64)>> =
        std::sync::OnceLock::new();
    GRAMMARS.get_or_init(Mutex::default)
}

/// Hands the editor a grammar linked into this binary under `name`.
///
/// It is what a language with `grammar = "linked:<name>"` is highlighted
/// with. The app calls it for its own grammars, a linked extension for its (before
/// its `[[languages]]` load, or after: the registry [`drive`]n is brought
/// up to date). `highlights` is the query used when the language gives
/// none.
///
/// ```ignore
/// architect_editor::syntax::link_grammar("toml", tree_sitter_toml_ng::LANGUAGE, Some(QUERY));
/// ```
pub fn link_grammar(name: &str, grammar: impl Into<TreeSitterGrammar>, highlights: Option<&str>) {
    {
        let mut table = lock(grammars());
        table.1 = table.1.wrapping_add(1);
        let generation = table.1;
        table.0.insert(
            name.to_owned(),
            Linked {
                grammar: grammar.into(),
                highlights: highlights.map(str::to_owned),
                generation,
            },
        );
    }
    resync();
}

/// Takes the grammar linked under `name` back. True when there was one.
#[expect(
    clippy::must_use_candidate,
    reason = "whether it was there is for a caller that cares"
)]
pub fn unlink_grammar(name: &str) -> bool {
    let had = lock(grammars()).0.remove(name).is_some();
    if had {
        resync();
    }
    had
}

/// The names grammars are linked under, sorted.
#[must_use]
pub fn linked_grammars() -> Vec<String> {
    let mut names: Vec<String> = lock(grammars()).0.keys().cloned().collect();
    names.sort();
    names
}

/// The grammar compiled into the editor for `language`, by its id then
/// its fences.
fn builtin_lang(language: &Language) -> Option<Lang> {
    std::iter::once(&language.id)
        .chain(&language.fences)
        .find_map(|name| Lang::from_fence_tag(name))
}

/// What a [`Syntax`] is built from: when it is the same, the built one is
/// reused (a query compiles once).
#[derive(Clone, PartialEq, Eq)]
struct Key {
    language: Language,
    generation: u64,
}

fn key_of(language: &Language) -> Key {
    let generation = match &language.grammar {
        Grammar::Host(name) => lock(grammars()).0.get(name).map_or(0, |l| l.generation),
        _ => 0,
    };
    Key {
        language: language.clone(),
        generation,
    }
}

/// `language` as the editor's [`Syntax`].
///
/// # Errors
/// Why it cannot be highlighted.
pub fn syntax_of(language: &Language) -> Result<Syntax, Unsupported> {
    let id = || language.id.clone();
    let refused = |e: LanguageError| Unsupported::Refused {
        language: id(),
        reason: e.to_string(),
    };
    match &language.grammar {
        Grammar::File(path) => Err(Unsupported::GrammarFile {
            language: id(),
            grammar: path.clone(),
        }),
        Grammar::Linked => {
            let lang = builtin_lang(language).ok_or_else(|| Unsupported::NotLinked {
                language: id(),
                grammar: "linked".to_owned(),
            })?;
            language.highlights_text.as_ref().map_or_else(
                || Ok(Syntax::Builtin(lang)),
                |query| {
                    HostLanguage::new(&language.id, lang.grammar(), query)
                        .map(Syntax::from)
                        .map_err(refused)
                },
            )
        }
        Grammar::Host(name) => {
            let linked =
                lock(grammars())
                    .0
                    .get(name)
                    .cloned()
                    .ok_or_else(|| Unsupported::NotLinked {
                        language: id(),
                        grammar: format!("linked:{name}"),
                    })?;
            let query = language
                .highlights_text
                .as_deref()
                .or(linked.highlights.as_deref())
                .ok_or_else(|| Unsupported::NoQuery { language: id() })?;
            HostLanguage::new(&language.id, linked.grammar, query)
                .map(Syntax::from)
                .map_err(refused)
        }
    }
}

/// What [`drive`] keeps: the registry it follows, what it registered in
/// the editor, and what it could not.
struct Driven {
    languages: Languages,
    registered: HashMap<String, (Key, Syntax)>,
    problems: Vec<(String, Unsupported)>,
}

fn driven() -> &'static Mutex<Option<Driven>> {
    static DRIVEN: std::sync::OnceLock<Mutex<Option<Driven>>> = std::sync::OnceLock::new();
    DRIVEN.get_or_init(Mutex::default)
}

/// Makes `languages` the editor's.
///
/// One registry per process: the app's, the one its extension host
/// keeps. Now and after each change, every
/// language in it is registered in `editor_state::languages` under its
/// id, fences and extensions, and what went is withdrawn. A language that
/// cannot be highlighted is left out and kept in [`problems`].
///
/// Driving another registry later stops following this one: what this
/// one registered is withdrawn first.
pub fn drive(languages: &Languages) {
    {
        let mut slot = lock(driven());
        if slot.as_ref().is_some_and(|d| d.languages.same(languages)) {
            drop(slot);
            resync();
            return;
        }
        if let Some(old) = slot.take() {
            for id in old.registered.keys() {
                withdraw_language(id);
            }
        }
        *slot = Some(Driven {
            languages: languages.clone(),
            registered: HashMap::new(),
            problems: Vec::new(),
        });
    }
    let follows = languages.clone();
    languages.on_change(Arc::new(move |changed: &Languages| {
        if changed.same(&follows) {
            resync();
        }
    }));
    resync();
}

/// Brings the editor's registry up to the driven one.
fn resync() {
    let mut slot = lock(driven());
    let Some(driven) = slot.as_mut() else {
        return;
    };
    let mut kept: HashMap<String, (Key, Syntax)> = HashMap::new();
    let mut problems = Vec::new();
    for Registered { language, .. } in driven.languages.all() {
        let key = key_of(&language);
        if let Some((was, syntax)) = driven.registered.get(&language.id)
            && *was == key
        {
            kept.insert(language.id.clone(), (key, syntax.clone()));
            continue;
        }
        match syntax_of(&language) {
            Ok(syntax) => {
                register_language(
                    LanguageEntry::new(language.id.clone(), syntax.clone())
                        .fences(language.fences.iter().cloned())
                        .extensions(language.extensions.iter().cloned()),
                );
                kept.insert(language.id.clone(), (key, syntax));
            }
            Err(why) => problems.push((language.id.clone(), why)),
        }
    }
    for id in driven.registered.keys() {
        if !kept.contains_key(id) {
            withdraw_language(id);
        }
    }
    driven.registered = kept;
    driven.problems = problems;
    drop(slot);
    notify();
}

/// The languages in the driven registry the editor cannot highlight, by
/// id, with why.
#[must_use]
pub fn problems() -> Vec<(String, Unsupported)> {
    lock(driven())
        .as_ref()
        .map(|d| d.problems.clone())
        .unwrap_or_default()
}

type Waker = Box<dyn Fn(u64) -> bool + Send>;

fn watchers() -> &'static Mutex<Vec<Waker>> {
    static WATCHERS: std::sync::OnceLock<Mutex<Vec<Waker>>> = std::sync::OnceLock::new();
    WATCHERS.get_or_init(Mutex::default)
}

/// Calls `waker` with [`languages_revision`] after each change [`drive`]
/// makes to the editor's registry, until it returns false. It is how a
/// view hears that a language came or went (`ui::use_languages_revision`).
pub fn watch(waker: impl Fn(u64) -> bool + Send + 'static) {
    lock(watchers()).push(Box::new(waker));
}

fn notify() {
    let revision = languages_revision();
    lock(watchers()).retain(|waker| waker(revision));
}

/// The registered language `name` means (an id, a fence name or a file
/// extension) as the editor's [`Syntax`].
///
/// # Errors
/// [`Unsupported`] when no such language is registered, or it cannot be
/// highlighted.
pub fn resolve(languages: &Languages, name: &str) -> Result<Syntax, Unsupported> {
    let registered = languages
        .resolve(name)
        .ok_or_else(|| Unsupported::Unknown(name.to_owned()))?;
    syntax_of(&registered.language)
}

/// Whether the editor can highlight the registered language `name` (an
/// id, a fence name or a file extension) as it stands.
///
/// # Errors
/// What is missing.
pub fn supported(languages: &Languages, name: &str) -> Result<(), Unsupported> {
    resolve(languages, name).map(|_| ())
}

/// `source` highlighted as the registered language `name` (an id, a fence
/// name or a file extension): non-overlapping spans in order, each named
/// by its theme token. Ranges no role colours are left out.
///
/// # Errors
/// [`Unsupported`] when no such language is registered, or the editor
/// cannot highlight it.
pub fn highlight(
    languages: &Languages,
    name: &str,
    source: &str,
) -> Result<Vec<Span>, Unsupported> {
    let syntax = resolve(languages, name)?;
    Ok(spans(&syntax, source))
}

/// `source` highlighted by `syntax`, each span named by its theme token.
#[must_use]
pub fn spans(syntax: &Syntax, source: &str) -> Vec<Span> {
    syntax
        .highlight(source)
        .into_iter()
        .filter_map(|t| {
            token_for_tag(t.tag).map(|token| Span {
                start: t.start,
                end: t.end,
                token,
                tag: t.tag,
            })
        })
        .collect()
}
