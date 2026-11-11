//! Language servers for the editor (feature `lsp`, native only).
//!
//! The editor's LSP client (`editor-lsp`) is re-exported whole. On top of
//! it, [`LanguageServers`] runs a language's server (the registry's
//! `server` command): one per language and workspace root, started when
//! the first document of the language opens ([`LanguageServers::open`]),
//! restarted with backoff when it dies ([`Restart`]), and stopped when
//! the last document closes or its owner goes ([`LanguageServers::stop_owner`]:
//! the extension host calls it when an extension is disabled).
//!
//! A [`Document`] is one open file: its edits go to the server as
//! `didChange`, its diagnostics come back ([`DocRef::watch_diagnostics`],
//! what `CodeEditor`'s `diagnostics` prop takes), and it asks for hover,
//! completion, formatting, rename and definition. Each server has one task
//! that sends everything for it in order, so a request never overtakes an
//! edit made before it.

pub use editor_lsp::*;

#[cfg(feature = "lsp-stub")]
pub mod stub;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use editor_lsp::lsp_types as types;
use editor_state::{Change, Changes, Doc};
use futures::channel::mpsc as watch;
use futures::future::BoxFuture;
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};

use crate::languages::{Registered, Server as Command};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How a server that died is started again: after `first`, doubling up
/// to `most`, at most `attempts` times in a row (a server that ran for
/// [`Restart::healthy`] starts counting again).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Restart {
    pub first: Duration,
    pub most: Duration,
    pub attempts: u32,
    pub healthy: Duration,
}

impl Default for Restart {
    fn default() -> Self {
        Self {
            first: Duration::from_millis(250),
            most: Duration::from_secs(10),
            attempts: 5,
            healthy: Duration::from_secs(30),
        }
    }
}

impl Restart {
    /// The wait before restart `attempt` (1, 2, …).
    #[must_use]
    pub fn delay(&self, attempt: u32) -> Duration {
        let doubled = self
            .first
            .saturating_mul(1_u32 << attempt.saturating_sub(1).min(16));
        doubled.min(self.most)
    }
}

/// Where a server is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum State {
    Starting,
    Running,
    /// It died; started again after a wait.
    Restarting {
        attempt: u32,
        error: String,
    },
    /// It died more times in a row than [`Restart::attempts`].
    Failed {
        error: String,
    },
    Stopped,
}

/// One server, as `EDITOR_SERVERS` and `APP_STATUS` show it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Status {
    pub language: String,
    pub root: PathBuf,
    /// Who registered the language: `app` or an extension's id.
    pub owner: String,
    pub command: String,
    #[serde(flatten)]
    pub state: State,
    pub restarts: u32,
    pub documents: usize,
}

/// Why a request was not answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LspError {
    /// The language has no server.
    NoServer(String),
    /// Its server is not running now (starting, restarting, failed).
    NotRunning(String),
    /// Its server was stopped (its owner went, or its last document
    /// closed).
    Stopped,
    /// Its server does not offer this (`hover`, `rename`, …).
    Unsupported(&'static str),
    /// The server answered with an error, or the protocol broke.
    Server(String),
}

impl std::fmt::Display for LspError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoServer(language) => write!(f, "{language} has no language server"),
            Self::NotRunning(why) => write!(f, "the language server is not running: {why}"),
            Self::Stopped => write!(f, "the language server was stopped"),
            Self::Unsupported(what) => write!(f, "the language server does not offer {what}"),
            Self::Server(why) => write!(f, "the language server: {why}"),
        }
    }
}

impl std::error::Error for LspError {}

impl From<editor_lsp::Error> for LspError {
    fn from(e: editor_lsp::Error) -> Self {
        Self::Server(e.to_string())
    }
}

/// What a server may be asked, past diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    Hover,
    Completion,
    Format,
    Rename,
    Definition,
}

impl Feature {
    const fn name(self) -> &'static str {
        match self {
            Self::Hover => "hover",
            Self::Completion => "completion",
            Self::Format => "formatting",
            Self::Rename => "rename",
            Self::Definition => "go to definition",
        }
    }

    const fn offered(self, caps: &types::ServerCapabilities) -> bool {
        use types::{HoverProviderCapability, OneOf};
        const fn one<T>(o: Option<&OneOf<bool, T>>) -> bool {
            !matches!(o, None | Some(OneOf::Left(false)))
        }
        match self {
            Self::Hover => !matches!(
                caps.hover_provider,
                None | Some(HoverProviderCapability::Simple(false))
            ),
            Self::Completion => caps.completion_provider.is_some(),
            Self::Format => one(caps.document_formatting_provider.as_ref()),
            Self::Rename => one(caps.rename_provider.as_ref()),
            Self::Definition => one(caps.definition_provider.as_ref()),
        }
    }
}

/// A replacement of the byte range `from..to` of a document by `text`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Edit {
    pub from: usize,
    pub to: usize,
    pub text: String,
}

/// Edits to apply to an open document, made against `against` (an editor
/// applies them only while its text is still that).
#[derive(Clone, Debug)]
pub struct EditBatch {
    pub against: Doc,
    pub edits: Vec<Edit>,
}

impl EditBatch {
    /// The edits as one change set over `against`.
    #[must_use]
    pub fn changes(&self) -> Changes {
        Changes::from_sorted(
            self.edits
                .iter()
                .map(|e| Change::replace(e.from..e.to, e.text.clone()))
                .collect(),
        )
    }
}

/// A completion the server offered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Completion {
    pub label: String,
    pub detail: String,
    /// What is put in (its label when the server gives nothing else).
    pub insert: String,
}

/// Where a definition is: a file and a line and character (0-based), and
/// the byte offset when it is the document asked about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Target {
    pub uri: String,
    pub line: u32,
    pub character: u32,
    pub at: Option<usize>,
}

/// A rename's edits: those of the document asked about (applied to it),
/// and how many edits it makes in each other file (reported, not
/// applied).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Renamed {
    pub here: Vec<Edit>,
    pub elsewhere: Vec<(String, usize)>,
}

/// A file URI for `path` (absolute), percent-encoding what a URI path may
/// not hold.
///
/// # Errors
/// When the result does not parse as a URI.
pub fn file_uri(path: &Path) -> Result<Uri, LspError> {
    let mut out = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            out.push(char::from(b));
        } else {
            let _ = std::fmt::Write::write_fmt(&mut out, format_args!("%{b:02X}"));
        }
    }
    Uri::from_str(&out).map_err(|e| LspError::Server(format!("{}: {e}", path.display())))
}

/// One open document, as the server task and its handles share it.
struct DocState {
    uri: Uri,
    path: PathBuf,
    language: String,
    caret: AtomicUsize,
    diagnostics: Mutex<Vec<Diagnostic>>,
    diagnostic_watchers: Mutex<Vec<watch::UnboundedSender<Vec<Diagnostic>>>>,
    edit_watchers: Mutex<Vec<watch::UnboundedSender<EditBatch>>>,
}

impl DocState {
    fn publish(&self, diagnostics: &[Diagnostic]) {
        diagnostics.clone_into(&mut lock(&self.diagnostics));
        lock(&self.diagnostic_watchers).retain(|w| w.unbounded_send(diagnostics.to_vec()).is_ok());
    }
}

type Answer = Box<
    dyn FnOnce(Result<(Arc<LspClient>, Doc), LspError>) -> Option<BoxFuture<'static, ()>> + Send,
>;

enum Op {
    Open(Arc<DocState>, Doc),
    Change {
        uri: Uri,
        changes: Changes,
        after: Doc,
    },
    Close(Uri),
    Request(Uri, Answer),
    Stop,
}

/// One language's server for one root.
struct Server {
    language: String,
    root: PathBuf,
    owner: String,
    command: Command,
    ops: mpsc::UnboundedSender<Op>,
    /// Documents open on it; changed under the servers' lock.
    open: AtomicUsize,
    status: Mutex<(State, u32)>,
    caps: Mutex<Option<types::ServerCapabilities>>,
    docs: Mutex<Vec<Weak<DocState>>>,
}

impl Server {
    fn set(&self, state: State) {
        let mut status = lock(&self.status);
        if matches!(state, State::Restarting { .. }) {
            status.1 = status.1.saturating_add(1);
        }
        status.0 = state;
    }

    fn status(&self) -> Status {
        let (state, restarts) = lock(&self.status).clone();
        Status {
            language: self.language.clone(),
            root: self.root.clone(),
            owner: self.owner.clone(),
            command: self.command.command.clone(),
            state,
            restarts,
            documents: self.open.load(Ordering::SeqCst),
        }
    }

    fn offers(&self, feature: Feature) -> bool {
        lock(&self.caps)
            .as_ref()
            .is_some_and(|caps| feature.offered(caps))
    }
}

struct Shared {
    runtime: tokio::runtime::Handle,
    restart: Restart,
    servers: Mutex<Vec<Arc<Server>>>,
    focused: Mutex<Option<(Weak<Server>, Weak<DocState>)>>,
}

/// The language servers the editor uses. Cloning shares them.
#[derive(Clone)]
pub struct LanguageServers {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for LanguageServers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LanguageServers")
            .field("servers", &self.status())
            .finish()
    }
}

impl LanguageServers {
    /// Servers run on `runtime`, restarted as [`Restart::default`] says.
    #[must_use]
    pub fn new(runtime: tokio::runtime::Handle) -> Self {
        Self::with_restart(runtime, Restart::default())
    }

    /// Servers run on `runtime`, restarted as `restart` says.
    #[must_use]
    pub fn with_restart(runtime: tokio::runtime::Handle, restart: Restart) -> Self {
        Self {
            shared: Arc::new(Shared {
                runtime,
                restart,
                servers: Mutex::new(Vec::new()),
                focused: Mutex::new(None),
            }),
        }
    }

    /// Opens the file at `path` (absolute), whose text is `text`, in the
    /// server of `language` for `root`: started if none runs for them.
    ///
    /// # Errors
    /// [`LspError::NoServer`] when the language has no server.
    #[expect(
        clippy::significant_drop_tightening,
        reason = "found or started and counted under one lock, so a close cannot stop it between"
    )]
    pub fn open(
        &self,
        language: &Registered,
        root: &Path,
        path: &Path,
        text: &str,
    ) -> Result<Document, LspError> {
        let command = language
            .language
            .server
            .clone()
            .ok_or_else(|| LspError::NoServer(language.language.id.clone()))?;
        let uri = file_uri(path)?;
        let state = Arc::new(DocState {
            uri,
            path: path.to_owned(),
            language: language.language.id.clone(),
            caret: AtomicUsize::new(0),
            diagnostics: Mutex::new(Vec::new()),
            diagnostic_watchers: Mutex::new(Vec::new()),
            edit_watchers: Mutex::new(Vec::new()),
        });
        let server = {
            let mut servers = lock(&self.shared.servers);
            let found = servers
                .iter()
                .find(|s| {
                    s.language == language.language.id
                        && s.root == root
                        && s.owner == language.owner
                        && s.command == command
                })
                .cloned();
            let server = found.unwrap_or_else(|| {
                let server = self.start(language, root, command);
                servers.push(server.clone());
                server
            });
            server.open.fetch_add(1, Ordering::SeqCst);
            server
        };
        lock(&server.docs).push(Arc::downgrade(&state));
        let _ = server.ops.send(Op::Open(state.clone(), Doc::new(text)));
        Ok(Document {
            doc: DocRef { server, doc: state },
            servers: Arc::downgrade(&self.shared),
        })
    }

    fn start(&self, language: &Registered, root: &Path, command: Command) -> Arc<Server> {
        let (ops, rx) = mpsc::unbounded_channel();
        let server = Arc::new(Server {
            language: language.language.id.clone(),
            root: root.to_owned(),
            owner: language.owner.clone(),
            command,
            ops,
            open: AtomicUsize::new(0),
            status: Mutex::new((State::Starting, 0)),
            caps: Mutex::new(None),
            docs: Mutex::new(Vec::new()),
        });
        self.shared
            .runtime
            .spawn(run(server.clone(), rx, self.shared.restart));
        server
    }

    /// Stops every server of the languages `owner` registered (an
    /// extension that was disabled or unloaded). Their documents stay
    /// open in their editors, with no diagnostics and no answers. How
    /// many it stopped.
    #[expect(
        clippy::must_use_candidate,
        reason = "how many it stopped is for a caller that cares"
    )]
    pub fn stop_owner(&self, owner: &str) -> usize {
        self.stop_where(|s| s.owner == owner)
    }

    /// Stops the server of `language` for every root.
    #[expect(
        clippy::must_use_candidate,
        reason = "how many it stopped is for a caller that cares"
    )]
    pub fn stop_language(&self, language: &str) -> usize {
        self.stop_where(|s| s.language == language)
    }

    fn stop_where(&self, which: impl Fn(&Server) -> bool) -> usize {
        let stopped: Vec<Arc<Server>> = {
            let mut servers = lock(&self.shared.servers);
            let (out, keep) = servers.drain(..).partition(|s| which(s));
            *servers = keep;
            out
        };
        for server in &stopped {
            let _ = server.ops.send(Op::Stop);
        }
        stopped.len()
    }

    /// Every server running or trying to.
    #[must_use]
    pub fn status(&self) -> Vec<Status> {
        lock(&self.shared.servers)
            .iter()
            .map(|s| s.status())
            .collect()
    }

    /// Makes `doc` the one the `EDITOR_*` actions act on by default.
    pub fn focus(&self, doc: &DocRef) {
        *lock(&self.shared.focused) = Some((Arc::downgrade(&doc.server), Arc::downgrade(&doc.doc)));
    }

    /// The document last focused, while it is open.
    #[must_use]
    pub fn focused(&self) -> Option<DocRef> {
        let (server, doc) = lock(&self.shared.focused).clone()?;
        Some(DocRef {
            server: server.upgrade()?,
            doc: doc.upgrade()?,
        })
    }

    /// The open document of the file at `path`.
    #[must_use]
    pub fn document(&self, path: &Path) -> Option<DocRef> {
        lock(&self.shared.servers).iter().find_map(|server| {
            lock(&server.docs)
                .iter()
                .filter_map(Weak::upgrade)
                .find(|d| d.path == path)
                .map(|doc| DocRef {
                    server: server.clone(),
                    doc,
                })
        })
    }
}

/// An open document in a language server. Dropped, it is closed; the
/// server stops when its last document closes.
pub struct Document {
    doc: DocRef,
    servers: Weak<Shared>,
}

impl Document {
    /// Makes it the one the `EDITOR_*` actions act on by default
    /// ([`LanguageServers::focused`]).
    pub fn focus(&self) {
        if let Some(shared) = self.servers.upgrade() {
            *lock(&shared.focused) = Some((
                Arc::downgrade(&self.doc.server),
                Arc::downgrade(&self.doc.doc),
            ));
        }
    }

    /// A handle that does not keep it open.
    #[must_use]
    pub fn handle(&self) -> DocRef {
        self.doc.clone()
    }
}

impl std::ops::Deref for Document {
    type Target = DocRef;
    fn deref(&self) -> &DocRef {
        &self.doc
    }
}

impl Drop for Document {
    fn drop(&mut self) {
        let server = &self.doc.server;
        let _ = server.ops.send(Op::Close(self.doc.doc.uri.clone()));
        lock(&server.docs).retain(|d| d.upgrade().is_some_and(|d| !Arc::ptr_eq(&d, &self.doc.doc)));
        let Some(shared) = self.servers.upgrade() else {
            return;
        };
        // Counted under the servers' lock, so an open of the same
        // language and root either finds it still counted or starts anew.
        let last = {
            let mut servers = lock(&shared.servers);
            let last = server.open.fetch_sub(1, Ordering::SeqCst) == 1;
            if last {
                servers.retain(|s| !Arc::ptr_eq(s, server));
            }
            last
        };
        if last {
            let _ = server.ops.send(Op::Stop);
        }
    }
}

/// An open document, as the `EDITOR_*` actions reach it: what a
/// [`Document`] is, without holding it open.
#[derive(Clone)]
pub struct DocRef {
    server: Arc<Server>,
    doc: Arc<DocState>,
}

impl DocRef {
    /// The file it is.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.doc.path
    }

    /// Its language's id.
    #[must_use]
    pub fn language(&self) -> &str {
        &self.doc.language
    }

    /// Its server, as [`LanguageServers::status`] shows it.
    #[must_use]
    pub fn server(&self) -> Status {
        self.server.status()
    }

    /// Its diagnostics now.
    #[must_use]
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        lock(&self.doc.diagnostics).clone()
    }

    /// Its diagnostics now and each time they change.
    #[must_use]
    pub fn watch_diagnostics(&self) -> watch::UnboundedReceiver<Vec<Diagnostic>> {
        let (tx, rx) = watch::unbounded();
        let _ = tx.unbounded_send(self.diagnostics());
        lock(&self.doc.diagnostic_watchers).push(tx);
        rx
    }

    /// The edits a format or a rename makes to it, for its editor to
    /// apply.
    #[must_use]
    pub fn watch_edits(&self) -> watch::UnboundedReceiver<EditBatch> {
        let (tx, rx) = watch::unbounded();
        lock(&self.doc.edit_watchers).push(tx);
        rx
    }

    /// Where its caret is (a byte offset): what the actions ask at when
    /// they are not told.
    pub fn set_caret(&self, at: usize) {
        self.doc.caret.store(at, Ordering::SeqCst);
    }

    /// Where its caret is.
    #[must_use]
    pub fn caret(&self) -> usize {
        self.doc.caret.load(Ordering::SeqCst)
    }

    /// Whether its server offers `feature` (false until it started).
    #[must_use]
    pub fn offers(&self, feature: Feature) -> bool {
        self.server.offers(feature)
    }

    /// An edit to it: `changes` made it `after`. Sent as `didChange`.
    pub fn change(&self, changes: Changes, after: Doc) {
        let _ = self.server.ops.send(Op::Change {
            uri: self.doc.uri.clone(),
            changes,
            after,
        });
    }

    async fn ask<T: Send + 'static>(
        &self,
        feature: Feature,
        ask: impl FnOnce(Arc<LspClient>, Doc, Uri) -> BoxFuture<'static, Result<T, LspError>>
        + Send
        + 'static,
    ) -> Result<T, LspError> {
        if lock(&self.server.caps).is_some() && !self.offers(feature) {
            return Err(LspError::Unsupported(feature.name()));
        }
        let (tx, rx) = oneshot::channel();
        let uri = self.doc.uri.clone();
        let answer: Answer = Box::new(move |got| match got {
            Ok((client, doc)) => Some(Box::pin(async move {
                let _ = tx.send(ask(client, doc, uri).await);
            })),
            Err(e) => {
                let _ = tx.send(Err(e));
                None
            }
        });
        self.server
            .ops
            .send(Op::Request(self.doc.uri.clone(), answer))
            .map_err(|_| LspError::Stopped)?;
        rx.await.map_err(|_| LspError::Stopped)?
    }

    /// The server's hover text at byte `at`, if it has any.
    ///
    /// # Errors
    /// [`LspError`]: no server running, or it does not offer hover.
    pub async fn hover(&self, at: usize) -> Result<Option<String>, LspError> {
        self.ask(Feature::Hover, move |client, doc, uri| {
            Box::pin(async move {
                let hover = client
                    .request::<types::request::HoverRequest>(types::HoverParams {
                        text_document_position_params: position(uri, &doc, at),
                        work_done_progress_params: types::WorkDoneProgressParams::default(),
                    })
                    .await?;
                Ok(hover.map(|h| hover_text(h.contents)))
            })
        })
        .await
    }

    /// The server's completions at byte `at`.
    ///
    /// # Errors
    /// [`LspError`]: no server running, or it does not offer completion.
    pub async fn completion(&self, at: usize) -> Result<Vec<Completion>, LspError> {
        self.ask(Feature::Completion, move |client, doc, uri| {
            Box::pin(async move {
                let got = client
                    .request::<types::request::Completion>(types::CompletionParams {
                        text_document_position: position(uri, &doc, at),
                        work_done_progress_params: types::WorkDoneProgressParams::default(),
                        partial_result_params: types::PartialResultParams::default(),
                        context: None,
                    })
                    .await?;
                let items = match got {
                    None => Vec::new(),
                    Some(types::CompletionResponse::Array(items)) => items,
                    Some(types::CompletionResponse::List(list)) => list.items,
                };
                Ok(items
                    .into_iter()
                    .map(|item| Completion {
                        insert: item
                            .insert_text
                            .clone()
                            .unwrap_or_else(|| item.label.clone()),
                        detail: item.detail.unwrap_or_default(),
                        label: item.label,
                    })
                    .collect())
            })
        })
        .await
    }

    /// The server's formatting of it, as edits (not applied).
    ///
    /// # Errors
    /// [`LspError`]: no server running, or it does not offer formatting.
    pub async fn format_edits(&self) -> Result<EditBatch, LspError> {
        self.ask(Feature::Format, move |client, doc, uri| {
            Box::pin(async move {
                let edits = client
                    .request::<types::request::Formatting>(types::DocumentFormattingParams {
                        text_document: types::TextDocumentIdentifier { uri },
                        options: types::FormattingOptions {
                            tab_size: 4,
                            insert_spaces: true,
                            ..types::FormattingOptions::default()
                        },
                        work_done_progress_params: types::WorkDoneProgressParams::default(),
                    })
                    .await?
                    .unwrap_or_default();
                Ok(EditBatch {
                    edits: byte_edits(&doc, &edits),
                    against: doc,
                })
            })
        })
        .await
    }

    /// Formats it: the server's edits, handed to its editor
    /// ([`DocRef::watch_edits`]) and returned.
    ///
    /// # Errors
    /// As [`DocRef::format_edits`].
    pub async fn format(&self) -> Result<Vec<Edit>, LspError> {
        let batch = self.format_edits().await?;
        self.hand_over(&batch);
        Ok(batch.edits)
    }

    /// Renames the symbol at byte `at` to `name`: the edits to this
    /// document are handed to its editor; the other files' are counted.
    ///
    /// # Errors
    /// [`LspError`]: no server running, or it does not offer rename.
    pub async fn rename(&self, at: usize, name: &str) -> Result<Renamed, LspError> {
        let name = name.to_owned();
        let me = self.doc.uri.to_string();
        let (batch, elsewhere) = self
            .ask(Feature::Rename, move |client, doc, uri| {
                Box::pin(async move {
                    let edit = client
                        .request::<types::request::Rename>(types::RenameParams {
                            text_document_position: position(uri, &doc, at),
                            new_name: name,
                            work_done_progress_params: types::WorkDoneProgressParams::default(),
                        })
                        .await?
                        .unwrap_or_default();
                    let mut here = Vec::new();
                    let mut elsewhere = Vec::new();
                    for (uri, edits) in workspace_edits(edit) {
                        if uri == me {
                            here.extend(edits);
                        } else {
                            elsewhere.push((uri, edits.len()));
                        }
                    }
                    Ok((
                        EditBatch {
                            edits: byte_edits(&doc, &here),
                            against: doc,
                        },
                        elsewhere,
                    ))
                })
            })
            .await?;
        self.hand_over(&batch);
        Ok(Renamed {
            here: batch.edits,
            elsewhere,
        })
    }

    /// Where the symbol at byte `at` is defined.
    ///
    /// # Errors
    /// [`LspError`]: no server running, or it does not offer definitions.
    pub async fn definition(&self, at: usize) -> Result<Vec<Target>, LspError> {
        let me = self.doc.uri.to_string();
        self.ask(Feature::Definition, move |client, doc, uri| {
            Box::pin(async move {
                let got = client
                    .request::<types::request::GotoDefinition>(types::GotoDefinitionParams {
                        text_document_position_params: position(uri, &doc, at),
                        work_done_progress_params: types::WorkDoneProgressParams::default(),
                        partial_result_params: types::PartialResultParams::default(),
                    })
                    .await?;
                let locations: Vec<(Uri, types::Position)> = match got {
                    None => Vec::new(),
                    Some(types::GotoDefinitionResponse::Scalar(l)) => {
                        vec![(l.uri, l.range.start)]
                    }
                    Some(types::GotoDefinitionResponse::Array(ls)) => {
                        ls.into_iter().map(|l| (l.uri, l.range.start)).collect()
                    }
                    Some(types::GotoDefinitionResponse::Link(ls)) => ls
                        .into_iter()
                        .map(|l| (l.target_uri, l.target_selection_range.start))
                        .collect(),
                };
                Ok(locations
                    .into_iter()
                    .map(|(uri, pos)| {
                        let uri = uri.to_string();
                        Target {
                            at: (uri == me).then(|| position_to_byte(&doc, pos)),
                            uri,
                            line: pos.line,
                            character: pos.character,
                        }
                    })
                    .collect())
            })
        })
        .await
    }

    fn hand_over(&self, batch: &EditBatch) {
        if batch.edits.is_empty() {
            return;
        }
        lock(&self.doc.edit_watchers).retain(|w| w.unbounded_send(batch.clone()).is_ok());
    }
}

fn position(uri: Uri, doc: &Doc, at: usize) -> types::TextDocumentPositionParams {
    types::TextDocumentPositionParams {
        text_document: types::TextDocumentIdentifier { uri },
        position: byte_to_position(doc, at),
    }
}

fn hover_text(contents: types::HoverContents) -> String {
    let marked = |m: types::MarkedString| match m {
        types::MarkedString::String(s) => s,
        types::MarkedString::LanguageString(l) => l.value,
    };
    match contents {
        types::HoverContents::Scalar(m) => marked(m),
        types::HoverContents::Array(ms) => {
            ms.into_iter().map(marked).collect::<Vec<_>>().join("\n\n")
        }
        types::HoverContents::Markup(m) => m.value,
    }
}

/// A workspace edit's text edits by file URI.
fn workspace_edits(edit: types::WorkspaceEdit) -> Vec<(String, Vec<types::TextEdit>)> {
    let mut out: Vec<(String, Vec<types::TextEdit>)> = edit
        .changes
        .unwrap_or_default()
        .into_iter()
        .map(|(uri, edits)| (uri.to_string(), edits))
        .collect();
    let documents = match edit.document_changes {
        None => Vec::new(),
        Some(types::DocumentChanges::Edits(edits)) => edits,
        Some(types::DocumentChanges::Operations(ops)) => ops
            .into_iter()
            .filter_map(|op| match op {
                types::DocumentChangeOperation::Edit(e) => Some(e),
                types::DocumentChangeOperation::Op(_) => None,
            })
            .collect(),
    };
    for document in documents {
        let edits = document
            .edits
            .into_iter()
            .map(|e| match e {
                types::OneOf::Left(e) => e,
                types::OneOf::Right(annotated) => annotated.text_edit,
            })
            .collect();
        out.push((document.text_document.uri.to_string(), edits));
    }
    out
}

/// LSP text edits as byte edits over `doc`, sorted, overlaps dropped.
fn byte_edits(doc: &Doc, edits: &[types::TextEdit]) -> Vec<Edit> {
    let mut out: Vec<Edit> = edits
        .iter()
        .map(|e| {
            let range = pos::range_to_byte_range(doc, e.range);
            Edit {
                from: range.start,
                to: range.end,
                text: e.new_text.clone(),
            }
        })
        .collect();
    out.sort_by_key(|e| (e.from, e.to));
    let mut end = 0;
    out.retain(|e| {
        let keep = e.from >= end;
        if keep {
            end = e.to;
        }
        keep
    });
    out
}

/// How a server's run ended.
enum Ended {
    Stopped,
    Died(String),
}

type Docs = HashMap<String, (Arc<DocState>, Doc)>;

/// A server's task: starts it, keeps it fed, restarts it, stops it.
async fn run(server: Arc<Server>, mut ops: mpsc::UnboundedReceiver<Op>, restart: Restart) {
    let mut docs: Docs = HashMap::new();
    let mut attempt = 0_u32;
    loop {
        let error = match start(&server).await {
            Ok((client, events, caps)) => {
                let client = Arc::new(client);
                *lock(&server.caps) = Some(caps);
                let started = Instant::now();
                match serve(&server, &client, events, &mut ops, &mut docs).await {
                    Ended::Stopped => {
                        let _ =
                            tokio::time::timeout(Duration::from_secs(2), client.shutdown()).await;
                        stopped(&server, &docs, &mut ops);
                        return;
                    }
                    Ended::Died(error) => {
                        if started.elapsed() >= restart.healthy {
                            attempt = 0;
                        }
                        error
                    }
                }
            }
            Err(error) => error,
        };
        for (state, _) in docs.values() {
            state.publish(&[]);
        }
        attempt = attempt.saturating_add(1);
        if attempt > restart.attempts {
            server.set(State::Failed { error });
            // Answer what still comes, until it is stopped.
            while let Some(op) = ops.recv().await {
                if matches!(op, Op::Stop) {
                    break;
                }
                offline(op, &mut docs, "it failed");
            }
            stopped(&server, &docs, &mut ops);
            return;
        }
        server.set(State::Restarting { attempt, error });
        let wait = tokio::time::sleep(restart.delay(attempt));
        tokio::pin!(wait);
        loop {
            tokio::select! {
                () = &mut wait => break,
                op = ops.recv() => match op {
                    None | Some(Op::Stop) => {
                        stopped(&server, &docs, &mut ops);
                        return;
                    }
                    Some(op) => offline(op, &mut docs, "it is restarting"),
                },
            }
        }
    }
}

type Started = (
    LspClient,
    mpsc::Receiver<ServerMessage>,
    types::ServerCapabilities,
);

/// Spawns the server and initializes it.
async fn start(server: &Server) -> Result<Started, String> {
    let args: Vec<&str> = server.command.args.iter().map(String::as_str).collect();
    let transport = Transport::stdio(&server.command.command, &args, Some(&server.root))
        .map_err(|e| format!("{} could not start: {e}", server.command.command))?;
    let (client, events) = LspClient::new(transport);
    let root = file_uri(&server.root).map_err(|e| e.to_string())?;
    let init = tokio::time::timeout(Duration::from_secs(20), client.initialize(Some(root)))
        .await
        .map_err(|_| "it did not answer initialize".to_owned())?
        .map_err(|e| format!("initialize: {e}"))?;
    Ok((client, events, init.capabilities))
}

/// Feeds a running server until it dies or is stopped.
async fn serve(
    server: &Server,
    client: &Arc<LspClient>,
    mut events: mpsc::Receiver<ServerMessage>,
    ops: &mut mpsc::UnboundedReceiver<Op>,
    docs: &mut Docs,
) -> Ended {
    let mut store = DiagnosticsStore::new();
    for (state, doc) in docs.values() {
        if let Err(e) = client
            .did_open(state.uri.clone(), &state.language, doc)
            .await
        {
            return Ended::Died(e.to_string());
        }
    }
    server.set(State::Running);
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Some(ServerMessage::Diagnostics(published)) => {
                    let key = published.uri.to_string();
                    if let Some((state, doc)) = docs.get(&key)
                        && store.apply(&published, client.version_of(&published.uri), doc)
                    {
                        state.publish(store.get(&published.uri));
                    }
                }
                Some(ServerMessage::Notification { .. }) => {}
                None => return Ended::Died("it exited".to_owned()),
            },
            op = ops.recv() => match op {
                None | Some(Op::Stop) => return Ended::Stopped,
                Some(op) => {
                    if let Err(e) = online(op, &client, docs, &mut store).await {
                        return Ended::Died(e);
                    }
                }
            },
        }
    }
}

/// One op with the server running.
async fn online(
    op: Op,
    client: &Arc<LspClient>,
    docs: &mut Docs,
    store: &mut DiagnosticsStore,
) -> Result<(), String> {
    match op {
        Op::Open(state, doc) => {
            client
                .did_open(state.uri.clone(), &state.language, &doc)
                .await
                .map_err(|e| e.to_string())?;
            docs.insert(state.uri.to_string(), (state, doc));
        }
        Op::Change {
            uri,
            changes,
            after,
        } => {
            if let Some((state, doc)) = docs.get_mut(&uri.to_string()) {
                client
                    .did_change(&uri, &changes, doc)
                    .await
                    .map_err(|e| e.to_string())?;
                *doc = after;
                store.map_through(&uri, &changes);
                state.publish(store.get(&uri));
            }
        }
        Op::Close(uri) => {
            if docs.remove(&uri.to_string()).is_some() {
                store.remove(&uri);
                client.did_close(&uri).await.map_err(|e| e.to_string())?;
            }
        }
        Op::Request(uri, answer) => match docs.get(&uri.to_string()) {
            Some((_, doc)) => {
                if let Some(asked) = answer(Ok((client.clone(), doc.clone()))) {
                    tokio::spawn(asked);
                }
            }
            None => {
                let _ = answer(Err(LspError::Stopped));
            }
        },
        Op::Stop => {}
    }
    Ok(())
}

/// An op while no server runs: the text is kept, a request is answered
/// with why.
fn offline(op: Op, docs: &mut Docs, why: &str) {
    match op {
        Op::Open(state, doc) => {
            docs.insert(state.uri.to_string(), (state, doc));
        }
        Op::Change { uri, after, .. } => {
            if let Some((_, doc)) = docs.get_mut(&uri.to_string()) {
                *doc = after;
            }
        }
        Op::Close(uri) => {
            docs.remove(&uri.to_string());
        }
        Op::Request(_, answer) => {
            let _ = answer(Err(LspError::NotRunning(why.to_owned())));
        }
        Op::Stop => {}
    }
}

/// Stopped: its documents lose their diagnostics, what still comes is
/// answered so.
fn stopped(server: &Server, docs: &Docs, ops: &mut mpsc::UnboundedReceiver<Op>) {
    server.set(State::Stopped);
    for (state, _) in docs.values() {
        state.publish(&[]);
    }
    ops.close();
    while let Ok(op) = ops.try_recv() {
        if let Op::Request(_, answer) = op {
            let _ = answer(Err(LspError::Stopped));
        }
    }
}
