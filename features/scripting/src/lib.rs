//! Scripts: small Luau programs over the app's Action List, run by a
//! person (from ⌘K, a key, the ribbon) and by a model over MCP
//! (`docs/content/architecture/extensions.md`, "Scripts and conditional
//! actions" and "An agent writes scripts").
//!
//! A script's API is the Action List: it lists and describes actions,
//! queries the read-only ones, runs the others, and loops and branches in
//! between. Its changes are one transaction ([`Transactions`]), so the
//! whole run is one named undo step ("Script: Arm selected tracks"), and a
//! run that fails is taken back. A script saved under a name is an action
//! itself (`SCRIPT_MY_ARM_SELECTED_TRACKS`): listed, in ⌘K, bindable,
//! pinnable, runnable by a model — the person's own, or the organization's
//! ([`Scope`]).
//!
//! This default set is the model, pure data: [`Script`], [`Scope`],
//! [`action_id`], [`ScriptError`] (a failure with its line), the
//! [`Transactions`] a run opens and the [`ScriptStore`]s a [`Library`]
//! keeps saved scripts in. The runtime is behind `luau`:
//! [`luau::Runtime`] runs one script, and [`actions::Scripting`] puts the
//! `SCRIPT_*` actions (run once, save, describe the API, list, delete) and
//! every saved script on an Action List. `crdt` makes a document's
//! `crdt::History` a script's transactions ([`history::HistoryTransactions`]);
//! `settings` keeps saved scripts in a settings file
//! ([`settings::SettingsStore`]).
//!
//! The runtime is the one the extensions runtime will host Luau
//! extensions on: [`luau::Runtime`] knows nothing of saved scripts or
//! MCP, only a list, a caller and a transaction.

use std::sync::{Mutex, PoisonError};

use facet::Facet;

#[cfg(feature = "luau")]
pub mod actions;
#[cfg(feature = "crdt")]
pub mod history;
#[cfg(feature = "luau")]
pub mod luau;
#[cfg(feature = "settings")]
pub mod settings;
#[cfg(feature = "luau")]
pub mod theme;
#[cfg(feature = "luau")]
pub mod vm;
#[cfg(feature = "luau")]
pub mod worker;

/// Whose a saved script is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Facet)]
#[repr(u8)]
pub enum Scope {
    /// The person's own: theirs on every device.
    Person,
    /// The organization's: everyone in it has it.
    Org,
}

impl Scope {
    /// The prefix of its scripts' action ids.
    const fn prefix(self) -> &'static str {
        match self {
            Self::Person => "SCRIPT_MY_",
            Self::Org => "SCRIPT_ORG_",
        }
    }

    /// Where its scripts sit in the action list (their `group`).
    #[must_use]
    pub const fn group(self) -> &'static str {
        match self {
            Self::Person => "Your scripts",
            Self::Org => "Organization scripts",
        }
    }
}

/// A saved script.
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
pub struct Script {
    /// What a person calls it, and what its action is labelled
    /// ("Arm selected tracks"). Unique in its scope, by [`action_id`].
    pub name: String,
    /// What it does, for the action list and a model.
    pub description: String,
    /// Its Luau source.
    pub source: String,
    pub scope: Scope,
}

impl Script {
    /// A script of the person's own.
    pub fn new(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            source: source.into(),
            scope: Scope::Person,
        }
    }

    #[must_use]
    pub fn described(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// This script, made the organization's.
    #[must_use]
    pub const fn for_org(mut self) -> Self {
        self.scope = Scope::Org;
        self
    }

    /// Its action id; `None` when its name has no letter or digit.
    #[must_use]
    pub fn id(&self) -> Option<String> {
        action_id(self.scope, &self.name)
    }
}

/// The action id a script named `name` runs as in `scope`.
///
/// Its letters and digits in upper case, every other run of characters
/// one `_`, after `SCRIPT_MY_` or `SCRIPT_ORG_` ("Arm selected tracks" →
/// `SCRIPT_MY_ARM_SELECTED_TRACKS`). `None` when `name` has no letter or
/// digit.
#[must_use]
pub fn action_id(scope: Scope, name: &str) -> Option<String> {
    let mut slug = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_uppercase());
        } else if !slug.is_empty() && !slug.ends_with('_') {
            slug.push('_');
        }
    }
    let slug = slug.trim_end_matches('_');
    (!slug.is_empty()).then(|| format!("{}{slug}", scope.prefix()))
}

/// What kind of failure stopped a script.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Facet)]
#[repr(u8)]
pub enum ErrorKind {
    /// It is not Luau: it never ran.
    Syntax,
    /// It raised an error (`error(…)`, `nil + 1`).
    Runtime,
    /// An action it ran or asked about refused or failed.
    Action,
    /// It ran past a limit: its steps, its memory, scripts nested too deep.
    Limit,
}

/// Why a script stopped, and where.
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
pub struct ScriptError {
    pub kind: ErrorKind,
    /// The line of the script it stopped on, when it is known.
    pub line: Option<u32>,
    pub message: String,
}

impl ScriptError {
    pub fn new(kind: ErrorKind, line: Option<u32>, message: impl Into<String>) -> Self {
        Self {
            kind,
            line,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(line) => write!(f, "line {line}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for ScriptError {}

/// The app's undo, as a script run sees it.
///
/// One step is opened before the script runs, committed when it ends,
/// taken back when it fails. The actions it runs change the app as they always do (each perhaps a
/// transaction of its own); the steps they make fold into the open one.
///
/// `crdt` implements it over a document's `crdt::History`
/// ([`history::HistoryTransactions`]); an engine with its own undo
/// (Studio's) implements it over that.
pub trait Transactions: Send + Sync {
    /// Opens a step named `name`. False, and nothing opened, when one is
    /// already open — a script run by a script: it is part of that step.
    fn begin(&self, name: &str) -> bool;

    /// Closes the step: its name, when it changed something.
    fn commit(&self) -> Option<String>;

    /// Closes the step and takes back what it changed: its name, when it
    /// had changed something.
    fn abort(&self) -> Option<String>;
}

/// Where one scope's saved scripts are kept: a settings file
/// ([`settings::SettingsStore`]), the org's server, memory.
pub trait ScriptStore: Send + Sync {
    /// Every script it keeps.
    ///
    /// # Errors
    ///
    /// When they cannot be read.
    fn load(&self) -> Result<Vec<Script>, String>;

    /// Keeps `scripts`, all of them, in place of what it kept.
    ///
    /// # Errors
    ///
    /// When they cannot be written.
    fn save(&self, scripts: &[Script]) -> Result<(), String>;
}

/// Scripts kept in memory: for tests, and an app that keeps them elsewhere.
#[derive(Debug, Default)]
pub struct MemoryStore {
    scripts: Mutex<Vec<Script>>,
}

impl MemoryStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl ScriptStore for MemoryStore {
    fn load(&self) -> Result<Vec<Script>, String> {
        Ok(self
            .scripts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    fn save(&self, scripts: &[Script]) -> Result<(), String> {
        scripts.clone_into(&mut self.scripts.lock().unwrap_or_else(PoisonError::into_inner));
        Ok(())
    }
}

/// Why the [`Library`] did not keep or find a script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryError {
    /// Its name has no letter or digit to make an id of.
    NoName,
    /// There is nowhere to keep this scope's scripts (no organization).
    NoStore(Scope),
    /// The store failed.
    Store(String),
}

impl std::fmt::Display for LibraryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoName => f.write_str("a script's name needs a letter or a digit"),
            Self::NoStore(Scope::Org) => f.write_str("there is no organization to keep it in"),
            Self::NoStore(Scope::Person) => f.write_str("there is nowhere to keep it"),
            Self::Store(message) => write!(f, "the scripts could not be kept: {message}"),
        }
    }
}

impl std::error::Error for LibraryError {}

/// The saved scripts: the person's, and the organization's when there is
/// one, each scope in its own [`ScriptStore`].
pub struct Library {
    person: Box<dyn ScriptStore>,
    org: Option<Box<dyn ScriptStore>>,
}

impl std::fmt::Debug for Library {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Library")
            .field("org", &self.org.is_some())
            .finish_non_exhaustive()
    }
}

impl Library {
    /// The person's scripts, kept in `person`; no organization's.
    pub fn new(person: impl ScriptStore + 'static) -> Self {
        Self {
            person: Box::new(person),
            org: None,
        }
    }

    /// The organization's scripts too, kept in `org`.
    #[must_use]
    pub fn with_org(mut self, org: impl ScriptStore + 'static) -> Self {
        self.org = Some(Box::new(org));
        self
    }

    /// Whether it keeps `scope`'s scripts.
    #[must_use]
    pub const fn keeps(&self, scope: Scope) -> bool {
        match scope {
            Scope::Person => true,
            Scope::Org => self.org.is_some(),
        }
    }

    fn store(&self, scope: Scope) -> Result<&dyn ScriptStore, LibraryError> {
        match scope {
            Scope::Person => Ok(self.person.as_ref()),
            Scope::Org => self.org.as_deref().ok_or(LibraryError::NoStore(scope)),
        }
    }

    /// Every saved script, the person's first.
    ///
    /// # Errors
    ///
    /// When a store cannot be read.
    pub fn scripts(&self) -> Result<Vec<Script>, LibraryError> {
        let mut all = self.person.load().map_err(LibraryError::Store)?;
        if let Some(org) = &self.org {
            all.extend(org.load().map_err(LibraryError::Store)?);
        }
        Ok(all)
    }

    /// The saved script whose action is `id`.
    ///
    /// # Errors
    ///
    /// When a store cannot be read.
    pub fn find(&self, id: &str) -> Result<Option<Script>, LibraryError> {
        Ok(self
            .scripts()?
            .into_iter()
            .find(|s| s.id().as_deref() == Some(id)))
    }

    /// Keeps `script` in its scope, in place of one of the same id: its
    /// action id.
    ///
    /// # Errors
    ///
    /// Its name makes no id, its scope has no store, or the store fails.
    pub fn save(&self, script: Script) -> Result<String, LibraryError> {
        let id = script.id().ok_or(LibraryError::NoName)?;
        let store = self.store(script.scope)?;
        let mut scripts = store.load().map_err(LibraryError::Store)?;
        scripts.retain(|s| s.id().as_deref() != Some(id.as_str()));
        scripts.push(script);
        store.save(&scripts).map_err(LibraryError::Store)?;
        Ok(id)
    }

    /// Forgets `scope`'s script named `name`: its action id, when there
    /// was one.
    ///
    /// # Errors
    ///
    /// Its scope has no store, or the store fails.
    pub fn delete(&self, scope: Scope, name: &str) -> Result<Option<String>, LibraryError> {
        let Some(id) = action_id(scope, name) else {
            return Ok(None);
        };
        let store = self.store(scope)?;
        let mut scripts = store.load().map_err(LibraryError::Store)?;
        let before = scripts.len();
        scripts.retain(|s| s.id().as_deref() != Some(id.as_str()));
        if scripts.len() == before {
            return Ok(None);
        }
        store.save(&scripts).map_err(LibraryError::Store)?;
        Ok(Some(id))
    }
}

#[cfg(test)]
mod tests {
    use super::{Library, LibraryError, MemoryStore, Scope, Script, action_id};

    #[test]
    fn a_name_becomes_an_action_id() {
        assert_eq!(
            action_id(Scope::Person, "Arm selected tracks").as_deref(),
            Some("SCRIPT_MY_ARM_SELECTED_TRACKS")
        );
        assert_eq!(
            action_id(Scope::Org, "  mix: -3 dB!  ").as_deref(),
            Some("SCRIPT_ORG_MIX_3_DB")
        );
        assert_eq!(action_id(Scope::Person, "→ ←"), None);
    }

    #[test]
    fn a_library_keeps_each_scope_apart_and_replaces_by_name() {
        let library = Library::new(MemoryStore::new());
        assert_eq!(
            library.save(Script::new("Arm", "return 1").for_org()),
            Err(LibraryError::NoStore(Scope::Org))
        );
        let library = library.with_org(MemoryStore::new());
        library.save(Script::new("Arm", "return 1")).ok();
        library.save(Script::new("ARM", "return 2")).ok();
        library.save(Script::new("Arm", "return 3").for_org()).ok();
        let all = library.scripts().unwrap_or_default();
        assert_eq!(all.len(), 2, "one per scope: {all:?}");
        assert_eq!(all.first().map(|s| s.source.as_str()), Some("return 2"));
        assert_eq!(
            library
                .find("SCRIPT_ORG_ARM")
                .ok()
                .flatten()
                .map(|s| s.source),
            Some("return 3".to_owned())
        );
        assert_eq!(
            library.delete(Scope::Person, "arm"),
            Ok(Some("SCRIPT_MY_ARM".to_owned()))
        );
        assert_eq!(library.delete(Scope::Person, "arm"), Ok(None));
        assert_eq!(library.scripts().map(|s| s.len()), Ok(1));
        assert_eq!(
            library.save(Script::new("!!", "")),
            Err(LibraryError::NoName)
        );
    }
}
