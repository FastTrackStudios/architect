//! Process extensions, for the host (feature `process`, desk only;
//! `docs/content/architecture/extensions.md`, "Processes over vox").
//!
//! [`ProcessRunner`] is the [`Runner`] for `kind = "process"`: it starts
//! the package's program with [`SOCKET`] in its environment, accepts its
//! connection, and asks it hello (its API, its wire, its views and
//! actions), which must cover what its manifest declares. From then on
//! the program is supervised: when it dies the host keeps running, its
//! [`Process`] says why ([`State::Exited`]), and it is started again with
//! backoff ([`Restart`]); a crash loop stops after a few attempts and
//! waits for [`Process::restart`]. Its actions run over vox; its views
//! open as streams of edits ([`Process::open`]), which `remote` mounts in
//! a pane.
//!
//! With the app's theme store ([`ProcessRunner::with_theme`]), a process
//! that speaks wire 3 or later is sent the theme as data
//! ([`ThemeData`]) as it connects and on every change, for a process that
//! paints itself (`guest::Guest::theme`). Its views' documents get the
//! theme as CSS either way (`remote`).
//!
//! Not yet: stdio as a transport, lazy start and stopping idle ones,
//! handing state over on a reload (`save-state`), the host service an
//! extension calls back into, Windows.

use std::collections::BTreeMap;
use std::os::unix::process::ExitStatusExt as _;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use architect::action::{ActionCaller, ActionError, ActionFuture, CallerAsyncHandler};
use futures::channel::mpsc;
use tokio::io::{AsyncBufReadExt as _, BufReader};
use tokio::sync::{Notify, watch};

use crate::host::{Context, Instance, Runner};
use crate::manifest::{API, ActionDecl, Kind, Manifest};
use crate::package::Package;
use crate::proto::{ProcessExtensionClient, ProcessExtensionStreamClient, SOCKET};
use crate::wire::{Frame, Hello, Open, THEMED_WIRE, ThemeData, ViewEvent, WIRE, WIRE_MIN};

/// How a process that died is started again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Restart {
    /// How many times in a row; then it waits for [`Process::restart`].
    pub attempts: u32,
    /// The first wait, doubled each attempt up to `max_backoff`.
    pub backoff: Duration,
    pub max_backoff: Duration,
    /// Running this long counts as having started well: the attempts
    /// start over.
    pub stable: Duration,
}

impl Default for Restart {
    fn default() -> Self {
        Self {
            attempts: 3,
            backoff: Duration::from_millis(250),
            max_backoff: Duration::from_secs(10),
            stable: Duration::from_secs(30),
        }
    }
}

impl Restart {
    /// Never on its own: only [`Process::restart`] starts it again.
    #[must_use]
    pub fn never() -> Self {
        Self {
            attempts: 0,
            ..Self::default()
        }
    }

    fn wait(&self, attempt: u32) -> Duration {
        let doubled = self
            .backoff
            .saturating_mul(1_u32 << attempt.saturating_sub(1).min(16));
        doubled.min(self.max_backoff)
    }
}

/// Where a process extension is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Being started (or started again).
    Starting,
    /// Connected and said hello.
    Running {
        /// Counts its starts: a view opened in an earlier one is gone.
        generation: u32,
        pid: u32,
    },
    /// It died, and why ("killed by signal 9"); whether it will be
    /// started again by itself.
    Exited { why: String, restarting: bool },
    /// Unloaded: it will not run again.
    Stopped,
}

/// The process extensions running now, by extension id. Cheap to clone.
#[derive(Clone)]
pub struct Processes {
    by_id: Arc<Mutex<BTreeMap<String, Process>>>,
    changes: watch::Sender<u64>,
}

impl Default for Processes {
    fn default() -> Self {
        Self {
            by_id: Arc::default(),
            changes: watch::Sender::new(0),
        }
    }
}

impl std::fmt::Debug for Processes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Processes").field(&self.ids()).finish()
    }
}

impl PartialEq for Processes {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.by_id, &other.by_id)
    }
}

impl Processes {
    /// The process of extension `id`, when it is loaded.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<Process> {
        self.by_id
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned()
    }

    #[must_use]
    pub fn ids(&self) -> Vec<String> {
        self.by_id
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect()
    }

    /// Resolves when one is loaded or unloaded after this was called.
    pub fn changed(&self) -> impl Future<Output = ()> + use<> {
        let mut seen = self.changes.subscribe();
        async move {
            let _ = seen.changed().await;
        }
    }

    fn put(&self, process: Process) {
        self.by_id
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(process.id().to_owned(), process);
        self.changes.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// Takes `process` out, unless another has taken its place (a reload).
    fn take(&self, process: &Process) {
        let mut by_id = self.by_id.lock().unwrap_or_else(PoisonError::into_inner);
        if by_id
            .get(process.id())
            .is_some_and(|p| Arc::ptr_eq(&p.inner, &process.inner))
        {
            by_id.remove(process.id());
            drop(by_id);
            self.changes.send_modify(|n| *n = n.wrapping_add(1));
        }
    }
}

/// The connection to a running process.
#[derive(Clone)]
struct Link {
    client: ProcessExtensionClient,
    stream: ProcessExtensionStreamClient,
    /// Kept so the connection lives as long as the link.
    _connection: vox_core::ConnectionHandle,
}

struct Inner {
    id: String,
    name: String,
    rt: tokio::runtime::Handle,
    state: watch::Sender<State>,
    link: Mutex<Option<Link>>,
    hello: Mutex<Option<Hello>>,
    pid: Mutex<Option<u32>>,
    stop: watch::Sender<bool>,
    restart: Notify,
}

/// One supervised process extension. Cheap to clone.
#[derive(Clone)]
pub struct Process {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Process {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Process")
            .field("id", &self.inner.id)
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

impl PartialEq for Process {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

/// The host's number for each view it mounts.
static PANES: AtomicU64 = AtomicU64::new(1);

impl Process {
    /// Its extension's id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.inner.id
    }

    /// Its extension's name, for a placeholder.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    #[must_use]
    pub fn state(&self) -> State {
        self.inner.state.borrow().clone()
    }

    /// Its state from now on: `changed().await` on it when it moves.
    #[must_use]
    pub fn watch(&self) -> watch::Receiver<State> {
        self.inner.state.subscribe()
    }

    /// Its program's process id, while it runs.
    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        *self
            .inner
            .pid
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// What it said at its last hello.
    #[must_use]
    pub fn hello(&self) -> Option<Hello> {
        self.inner
            .hello
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Starts it again now, when it died and stopped trying.
    pub fn restart(&self) {
        self.inner.restart.notify_one();
    }

    /// Stops it for good (it is being unloaded).
    pub fn stop(&self) {
        self.inner.stop.send_replace(true);
    }

    fn link(&self) -> Result<Link, String> {
        self.inner
            .link
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or_else(|| match self.state() {
                State::Exited { why, .. } => format!("{} is not running: {why}", self.inner.name),
                _ => format!("{} is not running", self.inner.name),
            })
    }

    /// Runs its action `action` with JSON `args`.
    ///
    /// # Errors
    ///
    /// It is not running, has no such action, or the action failed.
    pub async fn run(&self, action: &str, args: String) -> Result<String, String> {
        let link = self.link()?;
        let action = action.to_owned();
        self.inner
            .rt
            .spawn(async move {
                link.client
                    .run(action, args)
                    .await
                    .map_err(architect::vox_into_app::<crate::proto::ProcessError>)
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| e.to_string())?
    }

    /// Mounts its view `view`: the frames it renders, and where to send
    /// the pane's events. The frames end when the view or the process
    /// does; dropping the [`ViewSession`] unmounts it.
    ///
    /// # Errors
    ///
    /// It is not running.
    pub fn open(&self, view: &str) -> Result<ViewSession, String> {
        let link = self.link()?;
        let pane = PANES.fetch_add(1, Ordering::Relaxed);
        let (frames_to, frames) = mpsc::unbounded();
        let (events, mut events_from) = mpsc::unbounded::<ViewEvent>();
        let open = Open {
            view: view.to_owned(),
            pane,
        };
        let stream = link.stream.clone();
        let streaming = self.inner.rt.spawn(async move {
            let (tx, mut rx) = vox::channel::<Frame>();
            let mut call = std::pin::pin!(stream.view(open, tx));
            loop {
                tokio::select! {
                    biased;
                    _ = &mut call => break,
                    frame = rx.recv() => match frame {
                        Ok(Some(frame)) => {
                            let mut got = None;
                            let _ = frame.map(|f| got = Some(f));
                            if let Some(frame) = got
                                && frames_to.unbounded_send(frame).is_err()
                            {
                                break;
                            }
                        }
                        _ => break,
                    },
                }
            }
        });
        let client = link.client.clone();
        let forwarding = self.inner.rt.spawn(async move {
            use futures::StreamExt as _;
            while let Some(event) = events_from.next().await {
                let _ = client.event(pane, event).await;
            }
        });
        Ok(ViewSession {
            pane,
            frames,
            events,
            tasks: vec![streaming.abort_handle(), forwarding.abort_handle()],
            close: Some((self.inner.rt.clone(), link.client)),
        })
    }
}

/// A view mounted from a process. Dropping it unmounts the view.
pub struct ViewSession {
    /// The host's number for this mount.
    pub pane: u64,
    /// What it renders; ends when it stops.
    pub frames: mpsc::UnboundedReceiver<Frame>,
    /// The pane's events, for its handlers.
    pub events: mpsc::UnboundedSender<ViewEvent>,
    tasks: Vec<tokio::task::AbortHandle>,
    close: Option<(tokio::runtime::Handle, ProcessExtensionClient)>,
}

impl std::fmt::Debug for ViewSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ViewSession")
            .field("pane", &self.pane)
            .finish_non_exhaustive()
    }
}

impl Drop for ViewSession {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        if let Some((rt, client)) = self.close.take() {
            let pane = self.pane;
            rt.spawn(async move {
                let _ = client.close(pane).await;
            });
        }
    }
}

/// What starts the program.
#[derive(Clone, Debug)]
struct Launch {
    program: PathBuf,
    args: Vec<String>,
    socket_dir: PathBuf,
    hello_within: Duration,
}

/// The socket a start listens on.
static SOCKETS: AtomicU64 = AtomicU64::new(0);

/// A started program, connected and introduced.
struct Started {
    child: tokio::process::Child,
    link: Link,
    hello: Hello,
}

/// Starts the program, accepts its connection, asks its hello.
async fn launch(id: &str, launch: &Launch) -> Result<Started, String> {
    let socket = launch.socket_dir.join(format!(
        "ax-{}-{}.sock",
        std::process::id(),
        SOCKETS.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&socket);
    let listener = vox_stream::LocalListener::bind(&socket)
        .map_err(|e| format!("listening at {}: {e}", socket.display()))?;
    let spawned = tokio::process::Command::new(&launch.program)
        .args(&launch.args)
        .env(SOCKET, &socket)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(e) => {
            let _ = std::fs::remove_file(&socket);
            return Err(format!("{}: {e}", launch.program.display()));
        }
    };
    // Its output is its log.
    if let Some(out) = child.stdout.take() {
        tokio::spawn(log_lines(id.to_owned(), out));
    }
    if let Some(err) = child.stderr.take() {
        tokio::spawn(log_lines(id.to_owned(), err));
    }
    let introduced = tokio::time::timeout(launch.hello_within, async {
        let stream = tokio::select! {
            accepted = listener.accept() => accepted.map_err(|e| e.to_string())?,
            status = child.wait() => {
                return Err(status.map_or_else(|e| e.to_string(), |s| exited(s)));
            }
        };
        let connection = vox_core::initiator_on(vox_stream::StreamLink::unix(stream))
            .establish_connection()
            .await
            .map_err(|e| format!("connecting: {e:?}"))?;
        let client = connection
            .open_lane::<ProcessExtensionClient>()
            .await
            .map_err(|e| format!("connecting: {e:?}"))?;
        let stream = connection
            .open_lane::<ProcessExtensionStreamClient>()
            .await
            .map_err(|e| format!("connecting: {e:?}"))?;
        let hello = client
            .hello()
            .await
            .map_err(architect::vox_into_app::<crate::proto::ProcessError>)
            .map_err(|e| format!("hello: {e}"))?;
        Ok((
            Link {
                client,
                stream,
                _connection: connection,
            },
            hello,
        ))
    })
    .await;
    let _ = std::fs::remove_file(&socket);
    match introduced {
        Ok(Ok((link, hello))) => Ok(Started { child, link, hello }),
        Ok(Err(why)) => {
            let _ = child.kill().await;
            Err(why)
        }
        Err(_) => {
            let _ = child.kill().await;
            Err(format!(
                "it did not say hello within {} ms",
                launch.hello_within.as_millis()
            ))
        }
    }
}

async fn log_lines(id: String, from: impl tokio::io::AsyncRead + Unpin) {
    let mut lines = BufReader::new(from).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::info!(ext.id = %id, ext.output = %line);
    }
}

/// Why a program ended, in words.
fn exited(status: ExitStatus) -> String {
    match (status.code(), status.signal()) {
        (Some(code), _) => format!("exited with code {code}"),
        (None, Some(signal)) => format!("killed by signal {signal}"),
        (None, None) => "exited".to_owned(),
    }
}

/// What a manifest declares that its hello does not cover.
fn uncovered(manifest: &Manifest, hello: &Hello) -> Option<String> {
    if hello.api != API {
        return Some(format!("it speaks API {}, this app {API}", hello.api));
    }
    if !(WIRE_MIN..=WIRE).contains(&hello.wire) {
        return Some(format!(
            "it speaks wire {}, this app {WIRE_MIN} to {WIRE}",
            hello.wire
        ));
    }
    if let Some(view) = manifest
        .views
        .iter()
        .find(|v| v.document.is_none() && !hello.views.contains(&v.id))
    {
        return Some(format!("it does not draw its view {}", view.id));
    }
    manifest
        .actions
        .iter()
        .find(|a| a.preset.is_none() && !hello.actions.contains(&a.id))
        .map(|a| format!("it does not run its action {}", a.id))
}

/// Resolves once it is told to stop.
async fn stopped(stop: &mut watch::Receiver<bool>) {
    let _ = stop.wait_for(|s| *s).await;
}

/// Runs a process extension for as long as it is loaded.
/// The app's theme, to one process: a revision that moves on every change.
struct ThemeFeed {
    store: architect_theme::ThemeStore,
    changes: watch::Receiver<u64>,
    _subscription: architect_theme::Subscription,
}

impl ThemeFeed {
    fn new(store: architect_theme::ThemeStore) -> Self {
        let (tx, changes) = watch::channel(store.revision());
        let subscription = store.subscribe(move |r| {
            tx.send_replace(r.revision());
        });
        Self {
            store,
            changes,
            _subscription: subscription,
        }
    }

    /// Sends the theme now to `link`'s process.
    async fn send(&mut self, link: &Link, id: &str) {
        self.changes.borrow_and_update();
        let theme = ThemeData::from_resolved(&self.store.current());
        let revision = theme.revision;
        if let Err(e) = link.client.theme(theme).await {
            tracing::warn!(ext.id = %id, ext.theme_revision = revision, error = ?e, "the theme did not reach it");
        }
    }
}

/// Resolves when the theme changed (never, with none).
async fn theme_changed(feed: Option<&mut ThemeFeed>) {
    match feed {
        Some(feed) => {
            if feed.changes.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
        None => std::future::pending().await,
    }
}

async fn supervise(
    process: Process,
    launch_with: Launch,
    manifest: Manifest,
    restart: Restart,
    theme: Option<architect_theme::ThemeStore>,
    first: std::sync::mpsc::Sender<Result<(), String>>,
) {
    let inner = &process.inner;
    let mut stop = inner.stop.subscribe();
    let mut first = Some(first);
    let mut generation = 0_u32;
    let mut attempts = 0_u32;
    let span = tracing::info_span!("ext.process", ext.id = %inner.id);
    loop {
        if *stop.borrow() {
            break;
        }
        inner.state.send_replace(State::Starting);
        let started = launch(&inner.id, &launch_with)
            .await
            .and_then(|s| uncovered(&manifest, &s.hello).map_or(Ok(s), Err));
        let why = match started {
            Ok(Started {
                mut child,
                link,
                hello,
            }) => {
                generation = generation.wrapping_add(1);
                let pid = child.id().unwrap_or_default();
                // The theme, before any view opens; then on every change.
                let mut feed = theme
                    .clone()
                    .filter(|_| hello.wire >= THEMED_WIRE)
                    .map(ThemeFeed::new);
                if let Some(feed) = feed.as_mut() {
                    feed.send(&link, &inner.id).await;
                }
                let themed = link.clone();
                *inner.link.lock().unwrap_or_else(PoisonError::into_inner) = Some(link);
                *inner.hello.lock().unwrap_or_else(PoisonError::into_inner) = Some(hello);
                *inner.pid.lock().unwrap_or_else(PoisonError::into_inner) = Some(pid);
                inner.state.send_replace(State::Running { generation, pid });
                span.in_scope(|| {
                    tracing::info!(ext.pid = pid, ext.generation = generation, "started");
                });
                if let Some(first) = first.take() {
                    let _ = first.send(Ok(()));
                }
                let up = Instant::now();
                let ended = loop {
                    tokio::select! {
                        status = child.wait() => {
                            break Some(status.map_or_else(|e| e.to_string(), exited));
                        }
                        () = stopped(&mut stop) => {
                            let _ = child.kill().await;
                            break None;
                        }
                        () = theme_changed(feed.as_mut()) => {
                            if let Some(feed) = feed.as_mut() {
                                feed.send(&themed, &inner.id).await;
                            }
                        }
                    }
                };
                drop((feed, themed));
                inner
                    .link
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                inner
                    .pid
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                let Some(why) = ended else { break };
                if up.elapsed() >= restart.stable {
                    attempts = 0;
                }
                why
            }
            Err(why) => {
                if let Some(first) = first.take() {
                    // It never ran: the load fails, the last good copy stays.
                    let _ = first.send(Err(why));
                    inner.state.send_replace(State::Stopped);
                    return;
                }
                why
            }
        };
        attempts = attempts.saturating_add(1);
        let restarting = attempts <= restart.attempts;
        span.in_scope(|| tracing::warn!(ext.why = %why, ext.restarting = restarting, "exited"));
        inner.state.send_replace(State::Exited { why, restarting });
        if restarting {
            tokio::select! {
                () = tokio::time::sleep(restart.wait(attempts)) => {}
                () = inner.restart.notified() => {}
                () = stopped(&mut stop) => break,
            }
        } else {
            tokio::select! {
                () = inner.restart.notified() => attempts = 0,
                () = stopped(&mut stop) => break,
            }
        }
    }
    inner.state.send_replace(State::Stopped);
    span.in_scope(|| tracing::info!("stopped"));
}

/// Runs process extensions on a runtime of its own.
pub struct ProcessRunner {
    rt: Option<tokio::runtime::Runtime>,
    processes: Processes,
    restart: Restart,
    socket_dir: PathBuf,
    hello_within: Duration,
    theme: Option<architect_theme::ThemeStore>,
}

impl Drop for ProcessRunner {
    fn drop(&mut self) {
        // It may be dropped on an async thread, where a blocking shutdown
        // is not allowed.
        if let Some(rt) = self.rt.take() {
            rt.shutdown_background();
        }
    }
}

impl std::fmt::Debug for ProcessRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessRunner")
            .field("processes", &self.processes)
            .field("restart", &self.restart)
            .finish_non_exhaustive()
    }
}

impl ProcessRunner {
    /// A runner with its own two threads, listening in the system's
    /// temporary folder.
    ///
    /// # Errors
    ///
    /// Its runtime could not be made.
    pub fn new() -> std::io::Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ext-process")
            .enable_all()
            .build()?;
        Ok(Self {
            rt: Some(rt),
            processes: Processes::default(),
            restart: Restart::default(),
            socket_dir: std::env::temp_dir(),
            hello_within: Duration::from_secs(10),
            theme: None,
        })
    }

    /// Sends its processes the app's theme as data ([`ThemeData`]): as
    /// each connects, and on every change of `store`.
    #[must_use]
    pub fn with_theme(mut self, store: architect_theme::ThemeStore) -> Self {
        self.theme = Some(store);
        self
    }

    /// How a process that died is started again.
    #[must_use]
    pub const fn with_restart(mut self, restart: Restart) -> Self {
        self.restart = restart;
        self
    }

    /// Where its sockets go (a Unix socket's path is short: about 100
    /// bytes).
    #[must_use]
    pub fn with_socket_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.socket_dir = dir.into();
        self
    }

    /// How long a program has to connect and say hello.
    #[must_use]
    pub const fn with_hello_within(mut self, within: Duration) -> Self {
        self.hello_within = within;
        self
    }

    /// Keeps its processes in `processes`: one the app made first, for
    /// views drawn before the runner exists.
    #[must_use]
    pub fn with_processes(mut self, processes: Processes) -> Self {
        self.processes = processes;
        self
    }

    /// The processes it runs, for the views that show them.
    #[must_use]
    pub fn processes(&self) -> Processes {
        self.processes.clone()
    }
}

/// A loaded process extension.
struct Loaded {
    process: Process,
    processes: Processes,
}

impl Instance for Loaded {
    fn action(&self, decl: &ActionDecl) -> Result<CallerAsyncHandler, String> {
        let process = self.process.clone();
        let id = decl.id.clone();
        Ok(Arc::new(move |_: &ActionCaller, args: &[u8]| {
            let args = String::from_utf8(args.to_vec()).map_err(|e| ActionError::BadArgs {
                id: id.clone(),
                message: e.to_string(),
                expected: None,
            })?;
            let (process, id) = (process.clone(), id.clone());
            let run: ActionFuture = Box::pin(async move {
                process
                    .run(&id, args)
                    .await
                    .map(String::into_bytes)
                    .map_err(|message| ActionError::Failed { id, message })
            });
            Ok(run)
        }))
    }

    fn stop(&self) {
        self.process.stop();
        self.processes.take(&self.process);
    }
}

impl Runner for ProcessRunner {
    fn kind(&self) -> Kind {
        Kind::Process
    }

    fn start(&self, package: &Package, _: Context<'_>) -> Result<Box<dyn Instance>, String> {
        let manifest = &package.manifest;
        let dir = package
            .dir()
            .ok_or("a process extension runs from a folder")?;
        if manifest.entry.path.is_empty() {
            return Err("its manifest names no program ([entry] path)".to_owned());
        }
        let rt = self.rt.as_ref().ok_or("this runner has no runtime")?;
        let launch = Launch {
            program: dir.join(&manifest.entry.path),
            args: manifest.entry.args.clone(),
            socket_dir: self.socket_dir.clone(),
            hello_within: self.hello_within,
        };
        let process = Process {
            inner: Arc::new(Inner {
                id: manifest.id.clone(),
                name: manifest.name.clone(),
                rt: rt.handle().clone(),
                state: watch::Sender::new(State::Starting),
                link: Mutex::new(None),
                hello: Mutex::new(None),
                pid: Mutex::new(None),
                stop: watch::Sender::new(false),
                restart: Notify::new(),
            }),
        };
        let (first, started) = std::sync::mpsc::channel();
        rt.spawn(supervise(
            process.clone(),
            launch,
            manifest.clone(),
            self.restart,
            self.theme.clone(),
            first,
        ));
        let waited = self.hello_within.saturating_add(Duration::from_secs(1));
        match started.recv_timeout(waited) {
            Ok(Ok(())) => {}
            Ok(Err(why)) => return Err(why),
            Err(_) => {
                process.stop();
                return Err("it did not start in time".to_owned());
            }
        }
        self.processes.put(process.clone());
        Ok(Box::new(Loaded {
            process,
            processes: self.processes.clone(),
        }))
    }
}
