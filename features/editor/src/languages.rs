//! The language registry: what the editor asks by file type or fence
//! name, filled by the app and by extensions (`[[languages]]`).
//!
//! A [`Language`] is data: its id and name, the file extensions and fence
//! names it answers to, which tree-sitter grammar it is highlighted with
//! and its highlights query, its comment token, and the command of its
//! LSP server when it has one. [`Languages`] keeps them by owner (the app,
//! or an extension's id), so an owner's languages are replaced together
//! and go together when it is turned off.
//!
//! A grammar is always linked into the binary: one compiled into the
//! editor (`grammar = "linked"`), or one the app or a linked extension
//! hands over in Rust under a name (`grammar = "linked:<name>"`,
//! `syntax::link_grammar`). A grammar file (`grammar = "x.wasm"`) is
//! refused by [`Language::check`]: the tree-sitter the editor uses cannot
//! load a grammar at run time (`docs/content/features/editor.md`).
//!
//! The registry is data only. With feature `syntax`, `syntax::drive`
//! makes it the editor's: every language set here is registered in the
//! editor's own registry, so fences and `CodeEditor` highlight it. The
//! extension host runs a language's server (`server`).

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

/// The owner the app registers its own languages as.
pub const APP: &str = "app";

/// What `grammar = "linked:<name>"` starts with.
pub const LINKED_PREFIX: &str = "linked:";

/// Where a language's tree-sitter grammar comes from. Every grammar is
/// linked into the binary.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Grammar {
    /// Compiled into the editor (`grammar = "linked"`, the default): one
    /// of the grammars `editor-syntax` was built with, found by the
    /// language's id or a fence name.
    #[default]
    Linked,
    /// A grammar the app or a linked extension links in and hands over
    /// under this name (`grammar = "linked:toml"`; `syntax::link_grammar`).
    Host(String),
    /// A grammar file in the owner's package (`grammar = "x.wasm"`).
    /// Never loaded: [`Language::check`] refuses it.
    File(String),
}

impl From<String> for Grammar {
    fn from(text: String) -> Self {
        if text.is_empty() || text == "linked" {
            Self::Linked
        } else if let Some(name) = text.strip_prefix(LINKED_PREFIX) {
            Self::Host(name.to_owned())
        } else {
            Self::File(text)
        }
    }
}

impl From<Grammar> for String {
    fn from(grammar: Grammar) -> Self {
        match grammar {
            Grammar::Linked => "linked".to_owned(),
            Grammar::Host(name) => format!("{LINKED_PREFIX}{name}"),
            Grammar::File(path) => path,
        }
    }
}

/// A language's LSP server: the command the extension host runs, one per
/// language and workspace root, while an editor of the language is open.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Server {
    pub command: String,
    pub args: Vec<String>,
}

/// One language.
///
/// ```toml
/// [[languages]]
/// id = "keyflow"
/// name = "Keyflow chart"
/// extensions = ["kf"]
/// fences = ["kf"]
/// grammar = "linked:keyflow"         # one the app links; "linked" for the editor's own
/// highlights = "queries/highlights.scm"
/// comment = "//"
/// [languages.server]                  # optional
/// command = "kf-lsp"
/// args = ["--stdio"]
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Language {
    /// Lowercase letters, digits, `-` and `_`; what a file's language is
    /// written as. Unique in the registry: the first owner's wins.
    pub id: String,
    /// What a person reads (`Keyflow chart`). The id when empty.
    pub name: String,
    /// File extensions without the dot (`kf`), any case.
    pub extensions: Vec<String>,
    /// Fence names (```` ```kf ````), any case. Its id is always one.
    pub fences: Vec<String>,
    pub grammar: Grammar,
    /// Its highlights query, a file in the owner's package
    /// (`queries/highlights.scm`). None to use the one the grammar comes
    /// with (the editor's, or the one given to `syntax::link_grammar`).
    pub highlights: Option<String>,
    /// `highlights`' text, read from the package when its owner loaded.
    #[serde(skip_deserializing)]
    pub highlights_text: Option<String>,
    /// What starts a line comment (`//`, `#`).
    pub comment: Option<String>,
    /// Its LSP server, when it has one.
    pub server: Option<Server>,
}

impl Language {
    /// A language of a grammar compiled into the editor.
    #[must_use]
    pub fn linked(id: &str, name: &str) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            ..Self::default()
        }
    }

    /// With these file extensions.
    #[must_use]
    pub fn extensions(mut self, extensions: &[&str]) -> Self {
        self.extensions = extensions.iter().map(|e| (*e).to_owned()).collect();
        self
    }

    /// With these fence names.
    #[must_use]
    pub fn fences(mut self, fences: &[&str]) -> Self {
        self.fences = fences.iter().map(|f| (*f).to_owned()).collect();
        self
    }

    /// With this line-comment token.
    #[must_use]
    pub fn comment(mut self, token: &str) -> Self {
        self.comment = Some(token.to_owned());
        self
    }

    /// With a grammar the host links under `name` (`linked:<name>`).
    #[must_use]
    pub fn host_grammar(mut self, name: &str) -> Self {
        self.grammar = Grammar::Host(name.to_owned());
        self
    }

    /// With this highlights query, as text.
    #[must_use]
    pub fn highlights_query(mut self, query: &str) -> Self {
        self.highlights_text = Some(query.to_owned());
        self
    }

    /// With this LSP server command.
    #[must_use]
    pub fn server(mut self, command: &str, args: &[&str]) -> Self {
        self.server = Some(Server {
            command: command.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
        });
        self
    }

    /// What a person reads: its name, or its id without one.
    #[must_use]
    pub fn title(&self) -> &str {
        if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        }
    }

    /// Whether a fence named `name` (any case, trimmed) is this language.
    #[must_use]
    pub fn answers_fence(&self, name: &str) -> bool {
        let name = name.trim();
        self.id.eq_ignore_ascii_case(name)
            || self.fences.iter().any(|f| f.eq_ignore_ascii_case(name))
    }

    /// Whether a file with `extension` (no dot, any case) is this
    /// language.
    #[must_use]
    pub fn answers_extension(&self, extension: &str) -> bool {
        let extension = extension.trim_start_matches('.');
        self.extensions
            .iter()
            .any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(extension))
    }

    /// What is wrong with it as written, if anything: an id that is not
    /// lowercase letters, digits, `-` and `_`; an empty extension or
    /// fence; a grammar file (grammars are linked, never loaded); a
    /// `linked:` grammar with no name; a query path that leaves its
    /// package; a server with no command.
    ///
    /// # Errors
    /// The reason, as a sentence fragment (`its id is …`).
    pub fn check(&self) -> Result<(), String> {
        let plain = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_';
        if self.id.is_empty() || !self.id.chars().all(plain) {
            return Err("its id is lowercase letters, digits, - and _".to_owned());
        }
        let name_like = |s: &String| !s.trim().is_empty() && !s.chars().any(char::is_whitespace);
        if !self.extensions.iter().all(name_like) {
            return Err("an extension is empty or has a space".to_owned());
        }
        if !self.fences.iter().all(name_like) {
            return Err("a fence name is empty or has a space".to_owned());
        }
        match &self.grammar {
            Grammar::Linked => {}
            Grammar::Host(name) if !name.is_empty() && name.chars().all(plain) => {}
            Grammar::Host(name) => {
                return Err(format!(
                    "its grammar linked:{name} is not a name (lowercase letters, digits, - and _)"
                ));
            }
            Grammar::File(path) => {
                return Err(format!(
                    "its grammar is the file {path}, and grammars are linked into the app, \
                     not loaded at run time: write grammar = \"linked:<name>\" for one the app \
                     links, or \"linked\" for one the editor has"
                ));
            }
        }
        if let Some(path) = &self.highlights
            && !inside(path)
        {
            return Err(format!("{path} is outside the package"));
        }
        if self
            .server
            .as_ref()
            .is_some_and(|s| s.command.trim().is_empty())
        {
            return Err("its server has no command".to_owned());
        }
        Ok(())
    }
}

/// Whether `path` stays in the package it is written in: relative, no
/// `..`.
fn inside(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with(['/', '\\'])
        && !path.contains(':')
        && path.split(['/', '\\']).all(|part| part != "..")
}

/// A language in the registry and who registered it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registered {
    /// [`APP`], or an extension's id.
    pub owner: String,
    pub language: Language,
}

/// What [`Languages::on_change`] calls.
pub type Listener = Arc<dyn Fn(&Languages) + Send + Sync>;

#[derive(Default)]
struct Inner {
    /// In the order each owner first registered.
    owners: Vec<(String, Vec<Language>)>,
    revision: u64,
    listeners: Vec<Listener>,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner")
            .field("owners", &self.owners)
            .field("revision", &self.revision)
            .field("listeners", &self.listeners.len())
            .finish()
    }
}

/// The registry. Cloning shares it.
#[derive(Clone, Debug, Default)]
pub struct Languages {
    inner: Arc<Mutex<Inner>>,
}

impl Languages {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry with the app's own `languages` in it (owner [`APP`]).
    #[must_use]
    pub fn with_app(languages: Vec<Language>) -> Self {
        let registry = Self::new();
        registry.set(APP, languages);
        registry
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Makes `languages` everything `owner` registers (in place of what
    /// it registered before; none withdraws it). True when the registry
    /// changed.
    #[expect(
        clippy::must_use_candidate,
        reason = "it changes the registry; whether it did is for a caller that cares"
    )]
    pub fn set(&self, owner: &str, languages: Vec<Language>) -> bool {
        let mut inner = self.lock();
        let at = inner.owners.iter().position(|(o, _)| o == owner);
        let changed = match (at, languages.is_empty()) {
            (None, true) => false,
            (None, false) => {
                inner.owners.push((owner.to_owned(), languages));
                true
            }
            (Some(at), true) => {
                inner.owners.remove(at);
                true
            }
            (Some(at), false) => inner.owners.get_mut(at).is_some_and(|(_, kept)| {
                let differs = *kept != languages;
                *kept = languages;
                differs
            }),
        };
        let listeners = if changed {
            inner.revision = inner.revision.wrapping_add(1);
            inner.listeners.clone()
        } else {
            Vec::new()
        };
        drop(inner);
        for listener in &listeners {
            listener(self);
        }
        changed
    }

    /// Calls `listener` after every change, outside the registry's lock
    /// (it may read the registry). On the thread that made the change.
    pub fn on_change(&self, listener: Listener) {
        self.lock().listeners.push(listener);
    }

    /// Whether `other` is this registry (a clone of it), not just equal.
    #[must_use]
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// Takes everything `owner` registered out. True when it had any.
    #[expect(
        clippy::must_use_candidate,
        reason = "it changes the registry; whether it did is for a caller that cares"
    )]
    pub fn withdraw(&self, owner: &str) -> bool {
        self.set(owner, Vec::new())
    }

    /// It moves on every change: what a view compares to know to ask
    /// again.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.lock().revision
    }

    /// Every language, in the order its owner came. A language two owners
    /// register is the first's.
    #[must_use]
    pub fn all(&self) -> Vec<Registered> {
        let owners = self.lock().owners.clone();
        let mut out: Vec<Registered> = Vec::new();
        for (owner, languages) in &owners {
            for language in languages {
                if out.iter().all(|r| r.language.id != language.id) {
                    out.push(Registered {
                        owner: owner.clone(),
                        language: language.clone(),
                    });
                }
            }
        }
        out
    }

    /// What `owner` registered, shadowed or not.
    #[must_use]
    pub fn of(&self, owner: &str) -> Vec<Language> {
        self.lock()
            .owners
            .iter()
            .find(|(o, _)| o == owner)
            .map(|(_, languages)| languages.clone())
            .unwrap_or_default()
    }

    /// The language `id`.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<Registered> {
        self.all().into_iter().find(|r| r.language.id == id)
    }

    /// The language a fence named `name` is in (```` ```rs ````).
    #[must_use]
    pub fn for_fence(&self, name: &str) -> Option<Registered> {
        self.all()
            .into_iter()
            .find(|r| r.language.answers_fence(name))
    }

    /// The language of files with `extension` (no dot).
    #[must_use]
    pub fn for_extension(&self, extension: &str) -> Option<Registered> {
        self.all()
            .into_iter()
            .find(|r| r.language.answers_extension(extension))
    }

    /// The language of the file at `path`, by its extension.
    #[must_use]
    pub fn for_path(&self, path: &str) -> Option<Registered> {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        let (stem, extension) = name.rsplit_once('.')?;
        if stem.is_empty() {
            return None;
        }
        self.for_extension(extension)
    }

    /// The language `name` means: an id, else a fence name, else a file
    /// extension.
    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<Registered> {
        self.get(name)
            .or_else(|| self.for_fence(name))
            .or_else(|| self.for_extension(name))
    }
}

/// The languages whose grammars the editor has compiled in
/// (`editor-syntax`'s default set), as registry entries: what an app
/// registers as its own ([`Languages::with_app`]).
#[must_use]
pub fn builtin() -> Vec<Language> {
    vec![
        Language::linked("rust", "Rust")
            .extensions(&["rs"])
            .fences(&["rs"])
            .comment("//"),
        Language::linked("typescript", "TypeScript")
            .extensions(&["ts", "mts", "cts"])
            .fences(&["ts", "mts", "cts"])
            .comment("//"),
        Language::linked("javascript", "JavaScript")
            .extensions(&["js", "mjs", "cjs"])
            .fences(&["js", "mjs", "cjs", "node"])
            .comment("//"),
        Language::linked("python", "Python")
            .extensions(&["py"])
            .fences(&["py", "python3"])
            .comment("#"),
        Language::linked("json", "JSON")
            .extensions(&["json", "jsonc"])
            .fences(&["jsonc"]),
        Language::linked("bash", "Shell")
            .extensions(&["sh", "bash", "zsh"])
            .fences(&["sh", "shell", "zsh"])
            .comment("#"),
        Language::linked("markdown", "Markdown")
            .extensions(&["md", "markdown"])
            .fences(&["md"]),
    ]
}
