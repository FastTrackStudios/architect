//! Language servers through the host (feature `lsp`, native; #512).
//!
//! The app gives the host its [`LanguageServers`]
//! ([`Host::with_language_servers`]). A code editor opens its file with
//! [`Host::open_code`]: the language is found in the registry by the
//! file's extension, and its server (`[languages.server]`) is run, one
//! per language and workspace root, only when the language is the app's
//! own or its extension may (`capabilities.languages = ["servers"]`,
//! granted). An extension that is disabled or unloaded takes its
//! servers with it.
//!
//! The `EDITOR_*` actions reach the focused code document, or the one
//! `path` names, at its caret, or at `at` (a byte offset):
//!
//! | Id | What |
//! |---|---|
//! | `EDITOR_SERVERS` | every language server: its language, root, owner, command, state (`starting`, `running`, `restarting`, `failed`, `stopped`), restarts and open documents |
//! | `EDITOR_DIAGNOSTICS` | a document's diagnostics now |
//! | `EDITOR_HOVER` | the server's hover text at a point |
//! | `EDITOR_COMPLETE` | the server's completions at a point |
//! | `EDITOR_DEFINITION` | where the symbol at a point is defined |
//! | `EDITOR_FORMAT` | formats the document: the server's edits, applied in its editor as one step |
//! | `EDITOR_RENAME` | renames the symbol at a point to `name`: this document's edits applied, other files' counted |

use std::path::{Path, PathBuf};
use std::sync::PoisonError;

use architect::action::{ActionCaller, ActionError, ActionMeta, Effect, RawJson};
use architect_editor::languages::APP;
use architect_editor::lsp::{DocRef, Document, LanguageServers};
use facet::Facet;
use serde_json::json;

use crate::host::Host;

const fn meta(id: &'static str, name: &'static str, description: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "Editor",
        method_name: id,
        display_name: name,
        description,
        category: "Editor",
        group: "Editor",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

static SERVERS: ActionMeta = meta(
    "EDITOR_SERVERS",
    "Language servers",
    "Every language server the editor runs: its language, workspace root, owner (`app` or the \
     extension), command, `state` (`starting`, `running`, `restarting` with its attempt and \
     error, `failed` with its error, `stopped`), how often it restarted and how many documents \
     are open on it.",
);
static DIAGNOSTICS: ActionMeta = meta(
    "EDITOR_DIAGNOSTICS",
    "Diagnostics",
    "A code document's diagnostics from its language server (byte ranges, severity, message, \
     source, code): the focused one, or the one at `path`.",
);
static HOVER: ActionMeta = meta(
    "EDITOR_HOVER",
    "Hover",
    "The language server's hover text in a code document at `at` (a byte offset; the caret \
     without it): the focused document, or the one at `path`. `null` when it has none.",
);
static COMPLETE: ActionMeta = meta(
    "EDITOR_COMPLETE",
    "Complete",
    "The language server's completions in a code document at `at` (the caret without it): \
     each one's label, detail and the text it puts in.",
);
static DEFINITION: ActionMeta = meta(
    "EDITOR_DEFINITION",
    "Go to definition",
    "Where the symbol at `at` (the caret without it) in a code document is defined: each \
     target's file URI, line and character (0-based), and its byte offset when it is in the \
     same document.",
);
static FORMAT: ActionMeta = meta(
    "EDITOR_FORMAT",
    "Format document",
    "Formats a code document with its language server: the focused one, or the one at `path`. \
     The server's edits are applied in its editor as one undoable step, and returned.",
);
static RENAME: ActionMeta = meta(
    "EDITOR_RENAME",
    "Rename symbol",
    "Renames the symbol at `at` (the caret without it) in a code document to `name`, with its \
     language server. This document's edits are applied in its editor as one step; how many \
     edits it makes in each other file is returned (not applied).",
);

/// A code document and a point in it: `path` (the focused document
/// without it) and `at` (a byte offset; its caret without it).
#[derive(Debug, Default, Facet)]
pub struct EditorAt {
    pub path: Option<String>,
    pub at: Option<u64>,
}

/// `EDITOR_RENAME`'s arguments.
#[derive(Debug, Default, Facet)]
pub struct RenameArgs {
    /// The new name.
    pub name: String,
    pub path: Option<String>,
    pub at: Option<u64>,
}

/// `EDITOR_SERVERS`' arguments: none.
#[derive(Debug, Default, Facet)]
pub struct NoArgs {}

impl Host {
    /// Runs languages' servers on `servers`: the ones a code editor opens
    /// with [`Host::open_code`], stopped with their extension.
    #[must_use]
    pub fn with_language_servers(self, servers: LanguageServers) -> Self {
        *self
            .servers_slot()
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(servers);
        self
    }

    /// The language servers it runs, when the app gave them.
    #[must_use]
    pub fn language_servers(&self) -> Option<LanguageServers> {
        self.servers_slot()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether languages `owner` registered may run their servers: the
    /// app's always; an extension's when it may
    /// (`capabilities.languages = ["servers"]`, granted).
    #[must_use]
    pub fn may_run_servers(&self, owner: &str) -> bool {
        owner == APP
            || self.capabilities(owner).is_some_and(|c| {
                c.languages
                    .iter()
                    .any(|l| l == crate::capabilities::SERVERS)
            })
    }

    /// Opens the file at `path` (absolute; its text `text`) in its
    /// language's server for `root`: the language by the file's
    /// extension, its server started if none runs.
    ///
    /// # Errors
    /// The host has no language servers; no language is registered for
    /// the file, or it has no server; its extension may not run one.
    pub fn open_code(&self, path: &Path, root: &Path, text: &str) -> Result<Document, String> {
        let servers = self
            .language_servers()
            .ok_or("the app runs no language servers")?;
        let name = path.to_string_lossy();
        let language = self
            .languages()
            .for_path(&name)
            .ok_or_else(|| format!("no language is registered for {name}"))?;
        if !self.may_run_servers(&language.owner) {
            return Err(format!(
                "{} may not run its languages' servers: it needs capabilities.languages = \
                 [\"servers\"], granted",
                language.owner
            ));
        }
        servers
            .open(&language, root, path, text)
            .map_err(|e| e.to_string())
    }

    /// Stops the servers of the languages `owner` registered (it was
    /// disabled or unloaded).
    pub(crate) fn stop_language_servers(&self, owner: &str) {
        if let Some(servers) = self.language_servers() {
            let _ = servers.stop_owner(owner);
        }
    }

    /// Puts the `EDITOR_*` actions on its list (the module's table).
    ///
    /// # Errors
    /// An id already registered.
    pub fn install_editor_actions(&self) -> Result<(), ActionError> {
        let list = self.list();
        let me = self.clone();
        list.register_with(
            &SERVERS,
            Effect::ReadOnly,
            move |_: &ActionCaller, _: NoArgs| {
                let status = me
                    .language_servers()
                    .map(|s| s.status())
                    .unwrap_or_default();
                Ok::<_, String>(RawJson::from(json!(status).to_string()))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &DIAGNOSTICS,
            Effect::ReadOnly,
            move |_: &ActionCaller, a: EditorAt| {
                let (doc, _) = me.code_document(&a)?;
                let diagnostics: Vec<_> = doc
                    .diagnostics()
                    .into_iter()
                    .map(|d| {
                        json!({
                            "from": d.from,
                            "to": d.to,
                            "severity": format!("{:?}", d.severity).to_lowercase(),
                            "message": d.message,
                            "source": d.source,
                            "code": d.code,
                        })
                    })
                    .collect();
                Ok::<_, String>(RawJson::from(
                    json!({"path": doc.path(), "diagnostics": diagnostics}).to_string(),
                ))
            },
        )?;
        let me = self.clone();
        list.register_async_with(
            &HOVER,
            Effect::ReadOnly,
            move |_: &ActionCaller, a: EditorAt| {
                let found = me.code_document(&a);
                async move {
                    let (doc, at) = found?;
                    let hover = doc.hover(at).await.map_err(|e| e.to_string())?;
                    Ok(RawJson::from(json!({"at": at, "hover": hover}).to_string()))
                }
            },
        )?;
        let me = self.clone();
        list.register_async_with(
            &COMPLETE,
            Effect::ReadOnly,
            move |_: &ActionCaller, a: EditorAt| {
                let found = me.code_document(&a);
                async move {
                    let (doc, at) = found?;
                    let items = doc.completion(at).await.map_err(|e| e.to_string())?;
                    Ok(RawJson::from(json!({"at": at, "items": items}).to_string()))
                }
            },
        )?;
        let me = self.clone();
        list.register_async_with(
            &DEFINITION,
            Effect::ReadOnly,
            move |_: &ActionCaller, a: EditorAt| {
                let found = me.code_document(&a);
                async move {
                    let (doc, at) = found?;
                    let targets = doc.definition(at).await.map_err(|e| e.to_string())?;
                    Ok(RawJson::from(
                        json!({"at": at, "targets": targets}).to_string(),
                    ))
                }
            },
        )?;
        let me = self.clone();
        list.register_async_with(
            &FORMAT,
            Effect::Mutating,
            move |_: &ActionCaller, a: EditorAt| {
                let found = me.code_document(&a);
                async move {
                    let (doc, _) = found?;
                    let edits = doc.format().await.map_err(|e| e.to_string())?;
                    Ok(RawJson::from(
                        json!({"path": doc.path(), "edits": edits}).to_string(),
                    ))
                }
            },
        )?;
        let me = self.clone();
        list.register_async_with(
            &RENAME,
            Effect::Mutating,
            move |_: &ActionCaller, a: RenameArgs| {
                let found = me.code_document(&EditorAt {
                    path: a.path,
                    at: a.at,
                });
                let name = a.name;
                async move {
                    if name.trim().is_empty() {
                        return Err("give the new `name`".to_owned());
                    }
                    let (doc, at) = found?;
                    let renamed = doc.rename(at, &name).await.map_err(|e| e.to_string())?;
                    Ok(RawJson::from(
                        json!({"path": doc.path(), "at": at, "renamed": renamed}).to_string(),
                    ))
                }
            },
        )?;
        Ok(())
    }

    /// The code document `a` names (its `path`, else the focused one) and
    /// the point in it (`at`, else its caret).
    fn code_document(&self, a: &EditorAt) -> Result<(DocRef, usize), String> {
        let servers = self
            .language_servers()
            .ok_or("the app runs no language servers")?;
        let doc = match &a.path {
            Some(path) => servers
                .document(&PathBuf::from(path))
                .ok_or_else(|| format!("{path} is not open in a code editor"))?,
            None => servers
                .focused()
                .ok_or("no code editor is focused: give `path`")?,
        };
        let at =
            a.at.and_then(|at| usize::try_from(at).ok())
                .unwrap_or_else(|| doc.caret());
        Ok((doc, at))
    }
}
