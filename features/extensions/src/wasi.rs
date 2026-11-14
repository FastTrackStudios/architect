//! WASI component extensions (feature `wasi`: desktop, Android and iOS;
//! `docs/content/architecture/extensions.md`, "WASI components").
//!
//! [`WasiRunner`] is the [`Runner`] for `kind = "wasm"`: it compiles the
//! package's component (`[wasm] component`) with Cranelift — to native
//! code, or on iOS (no JIT) to bytecode Wasmtime's Pulley interpreter
//! runs ([`PULLEY`]) — once — the
//! compiled code is kept on disk by the component's hash
//! ([`WasiRunner::with_cache`]) — and instantiates it against the
//! `architect:extension` world (`features/extensions/wit/extension.wit`):
//!
//! - **imports**: the host service (`run-action`, `setting`,
//!   `set-setting`, `theme`, `log`, `call-service`, the interface's
//!   `notify`, `clipboard-read`, `clipboard-write`, `pick`, `prompt`,
//!   `menu` and `decorate`, the host's clock (`now`), hooks made at
//!   run time (`subscribe`, `unsubscribe`), the app's documents
//!   (`list-documents`, `read-document`, `create-document`,
//!   `edit-document`, `rename-document`, `move-document`,
//!   `delete-document`, `watch-documents`), the editor being typed in
//!   (`editor-state`, `edit-editor`, `decorate-editor`), the network through the host
//!   (`fetch`, `read-body`, `close-body`, `secret-set`, `ask-secret`:
//!   `crate::net`, every host and redirect checked against
//!   `capabilities.hosts`), MIDI ports and OSC endpoints (`ports`,
//!   `open-port`, `port-send`, `port-receive`, `close-port`:
//!   `crate::ports`, each port checked against `capabilities.ports`), and
//!   `call` for every
//!   `hostcall::HostRequest`), each call answered by the extension's own
//!   [`ExtensionApi`] — so checked against its capabilities as a person
//!   granted them, as a process's, a script's and a linked extension's
//!   are — and WASI 0.2 with nothing to reach: no files, no network, no
//!   environment; its stdout and stderr are lines in its log
//!   (`EXT_LOGS`). A component importing the network (`wasi:sockets`,
//!   `wasi:http`) is refused before it is linked: no capability grants it.
//! - **exports**: `init`, `actions`, `views`, `invoke`, `render`,
//!   `on-event`, `pending`, `close`, `save-state`, `tick`. A view renders as the
//!   same edits a process's does (`wire::Edit`), so the host's
//!   `RemoteDocument` draws it ([`WasiRunner::with_processes`]: a component
//!   is a local [`Process`], and `RemoteView` shows it).
//!
//! There are no threads in a component. Each instance is a `Store` of its
//! own on a thread of its own, an actor its calls queue for: one call at a
//! time, so an action, an event and a render never meet. Its limits
//! ([`Limits`]):
//!
//! - **memory**: a `ResourceLimiter` refuses growth past
//!   [`Limits::memory`]; the call traps;
//! - **time**: one engine ticks an epoch every 10 ms; each tick a call
//!   yields to its thread's executor, and one past [`Limits::call`] of its
//!   own time (time it spends waiting for the host does not count) traps.
//!
//! A trap ends the instance (a component cannot be entered again after
//! one): the call fails saying why, its views say so, and it is started
//! again, fresh, as a process that died is. Unloading it ends its thread
//! and frees its store.
//!
//! Its heavy work goes to workers ([`workers`]): more instances of the
//! component, each on a thread of the runner's pool with limits of its
//! own, spoken to by messages only — parallelism from more instances, not
//! threads in one.
//!
//! In a browser the same components run transpiled by jco (`crate::web`,
//! feature `web`). Not yet: WASI 0.3's async.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::hash::{Hash as _, Hasher as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use architect::action::{ActionCaller, ActionError, ActionFuture, CallerAsyncHandler, Principal};
use futures::channel::{mpsc, oneshot};
use serde_json::Value as Json;
use sha2::{Digest as _, Sha256};
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, ResourceLimiter, Store, UpdateDeadline};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::host::{Context, Instance, Runner};
use crate::hostapi::ExtensionApi;
use crate::hostcall::{HostError, HostRequest};
use crate::logs::{Level, Logs};
use crate::manifest::{ActionDecl, Kind, Manifest};
use crate::package::Package;
use crate::wire;

mod workers;

pub use workers::{MAX_WORKERS, NEWS_EVERY, Pool, QUEUE, WorkerInfo, WorkerStatus};

mod bindings {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "extension",
        imports: { default: async },
        exports: { default: async },
    });
}

use bindings::Extension as World;
use bindings::architect::extension::{host as wit_host, types as wit};

/// How often the engine's epoch moves: how soon a running call yields, and
/// how finely its time is measured.
const TICK: Duration = Duration::from_millis(10);

/// What one instance may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Its linear memories and tables, in bytes, at most.
    pub memory: usize,
    /// One call's own time, at most (its waits on the host aside).
    pub call: Duration,
}

impl Default for Limits {
    /// 64 MiB, and 5 s a call.
    fn default() -> Self {
        Self {
            memory: 64 << 20,
            call: Duration::from_secs(5),
        }
    }
}

impl Limits {
    /// A worker's: 64 MiB, and 60 s of its own time for its whole run.
    #[must_use]
    pub const fn worker() -> Self {
        Self {
            memory: 64 << 20,
            call: Duration::from_secs(60),
        }
    }
}

/// Whether components are interpreted by Pulley, Wasmtime's portable
/// interpreter, rather than compiled to native code by Cranelift: on iOS,
/// which allows no JIT, and wherever the `pulley` feature is on.
pub const PULLEY: bool = cfg!(any(target_os = "ios", feature = "pulley"));

/// Each linear memory's address space under [`PULLEY`]: the default
/// [`Limits::memory`].
///
/// Cranelift's native code reserves 4 GiB and a guard so its loads need
/// no bounds check. An iOS app has a few GiB of address space in all;
/// Pulley checks each access's bounds itself, and a memory that grows
/// past this moves.
pub const PULLEY_RESERVATION: u64 = 64 << 20;

/// The engine's settings: the component model and epoch interruption;
/// under [`PULLEY`], Cranelift compiling to Pulley bytecode (`pulley64`),
/// traps found by the interpreter rather than by signals, and a small
/// memory reservation with no guard.
fn config() -> Result<Config, String> {
    let mut config = Config::new();
    config.wasm_component_model(true).epoch_interruption(true);
    if PULLEY {
        config.target("pulley64").map_err(|e| e.to_string())?;
        config
            .signals_based_traps(false)
            .memory_reservation(PULLEY_RESERVATION)
            .memory_guard_size(0)
            .memory_reservation_for_growth(0);
    }
    Ok(config)
}

/// The one engine ([`config`]) and its epoch's ticker.
fn engine() -> Result<&'static Engine, String> {
    static ENGINE: OnceLock<Result<Engine, String>> = OnceLock::new();
    ENGINE
        .get_or_init(|| {
            let engine = Engine::new(&config()?).map_err(|e| e.to_string())?;
            let weak = engine.weak();
            std::thread::Builder::new()
                .name("ext-wasi-epoch".to_owned())
                .spawn(move || {
                    while let Some(engine) = weak.upgrade() {
                        engine.increment_epoch();
                        drop(engine);
                        std::thread::sleep(TICK);
                    }
                })
                .map_err(|e| e.to_string())?;
            Ok(engine)
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// The compile cache's name for `bytes`: their SHA-256, with the engine's
/// compatibility (Wasmtime's version and settings), in hex.
#[must_use]
pub fn cache_key(engine_hash: u64, bytes: &[u8]) -> String {
    let mut sha = Sha256::new();
    sha.update(bytes);
    sha.update(engine_hash.to_le_bytes());
    sha.finalize()
        .iter()
        .fold(String::with_capacity(64), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        })
}

fn engine_hash(engine: &Engine) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    engine.precompile_compatibility_hash().hash(&mut h);
    h.finish()
}

/// Where a component's code came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Compiled {
    Fresh,
    Cached,
}

/// `bytes` compiled: from `cache` (`<key>.cwasm`) when it is there, else
/// compiled and written there.
fn compile(
    engine: &Engine,
    bytes: &[u8],
    cache: Option<&Path>,
) -> Result<(Component, Compiled), String> {
    let path =
        cache.map(|dir| dir.join(format!("{}.cwasm", cache_key(engine_hash(engine), bytes))));
    if let Some(path) = path.as_deref().filter(|p| p.is_file()) {
        // SAFETY: the file is this function's own output
        // (`Component::serialize`), in the app's own cache folder, named
        // by the hash of the component it was compiled from and of this
        // engine; one that does not deserialize is compiled again.
        if let Ok(component) = unsafe { Component::deserialize_file(engine, path) } {
            return Ok((component, Compiled::Cached));
        }
    }
    let component = Component::new(engine, bytes).map_err(|e| format!("{e:#}"))?;
    if let (Some(path), Ok(code)) = (path, component.serialize()) {
        let tmp = path.with_extension("tmp");
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&tmp, code))
            .and_then(|()| std::fs::rename(&tmp, &path));
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
    Ok((component, Compiled::Fresh))
}

/// The interfaces a component may import: the world's, and WASI 0.2's
/// with nothing to reach. The network is no capability's.
fn import_allowed(name: &str) -> bool {
    [
        "architect:extension/",
        "wasi:cli/",
        "wasi:io/",
        "wasi:clocks/",
        "wasi:random/",
        "wasi:filesystem/",
    ]
    .iter()
    .any(|p| name.starts_with(p))
}

/// The first import of `component` outside what it may import.
fn refused_import(engine: &Engine, component: &Component) -> Option<String> {
    component
        .component_type()
        .imports(engine)
        .map(|(name, _)| name.to_owned())
        .find(|name| !import_allowed(name))
}

/// What one instance's memory limit saw.
struct Limiter {
    max: usize,
    /// Its memories' size now, in bytes.
    now: Arc<AtomicUsize>,
    /// Why it refused, when it did in this call.
    hit: Option<String>,
}

impl ResourceLimiter for Limiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > self.max {
            let why = format!(
                "out of memory: it asked for {} MiB, past its limit of {} MiB",
                desired >> 20,
                self.max >> 20
            );
            self.hit = Some(why.clone());
            return Err(wasmtime::format_err!("{why}"));
        }
        self.now
            .fetch_add(desired.saturating_sub(current), Ordering::Relaxed);
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= 1 << 20)
    }
}

/// One call's time.
#[derive(Default)]
struct Clock {
    started: Option<Instant>,
    /// Spent waiting for the host in this call.
    host: Duration,
    budget: Duration,
    over: bool,
}

impl Clock {
    fn own(&self) -> Duration {
        self.started
            .map_or(Duration::ZERO, |s| s.elapsed().saturating_sub(self.host))
    }
}

/// A store's data: WASI's, the limits, and the extension's session with
/// the host service.
struct State {
    wasi: WasiCtx,
    table: ResourceTable,
    limiter: Limiter,
    clock: Clock,
    /// Its session, acting for whoever ran the call now.
    api: ExtensionApi,
    /// Its own actions: run by itself, not through the host (that call
    /// would wait for the one making it).
    own: BTreeSet<String>,
    theme: Option<architect_theme::ThemeStore>,
    /// An owner, with its workers (when it declares any), or a worker.
    side: Side,
}

/// Which side of the `worker` interface an instance is.
enum Side {
    Owner(Option<workers::Crew>),
    Worker(Arc<workers::Link>),
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl State {
    /// A fresh store's data: WASI with nothing to reach (its stdout and
    /// stderr into its log), within `limits`.
    #[allow(clippy::too_many_arguments)]
    fn new(
        id: &str,
        logs: &Logs,
        limits: Limits,
        memory: Arc<AtomicUsize>,
        api: ExtensionApi,
        own: BTreeSet<String>,
        theme: Option<architect_theme::ThemeStore>,
        side: Side,
    ) -> Self {
        let worker = matches!(side, Side::Worker(_));
        let pipe = |source: &'static str, level| LogPipe {
            id: id.to_owned(),
            logs: logs.clone(),
            source: if worker { "worker" } else { source },
            level,
            partial: Arc::default(),
        };
        let wasi = WasiCtxBuilder::new()
            .stdout(pipe("stdout", Level::Info))
            .stderr(pipe("stderr", Level::Warn))
            .allow_ip_name_lookup(false)
            .build();
        memory.store(0, Ordering::Relaxed);
        Self {
            wasi,
            table: ResourceTable::new(),
            limiter: Limiter {
                max: limits.memory,
                now: memory,
                hit: None,
            },
            clock: Clock {
                budget: limits.call,
                ..Clock::default()
            },
            api,
            own,
            theme,
            side,
        }
    }

    /// A worker reaches its owner only, by messages: never the app.
    fn worker_refusal(&self) -> Option<String> {
        matches!(self.side, Side::Worker(_)).then(|| {
            error_json(&HostError::Failed {
                message: "a worker reaches its owner only, by messages, not the app".to_owned(),
            })
        })
    }

    async fn host(&mut self, request: HostRequest) -> Result<Json, HostError> {
        let started = Instant::now();
        let api = self.api.clone();
        let answer = api.call(request).await;
        self.clock.host = self.clock.host.saturating_add(started.elapsed());
        answer
    }
}

impl State {
    /// `request`, refused in a worker: its answer, or the error as JSON.
    async fn ask(&mut self, request: HostRequest) -> Result<Json, String> {
        if let Some(no) = self.worker_refusal() {
            return Err(no);
        }
        self.host(request).await.map_err(|e| error_json(&e))
    }

    /// Request `op` from its fields as JSON text: its answer as JSON text.
    async fn ask_json(&mut self, op: &str, fields: &str) -> Result<String, String> {
        let mut v = json_arg(fields);
        if v.is_null() {
            v = Json::Object(serde_json::Map::new());
        }
        let request = if let Json::Object(map) = &mut v {
            map.insert("op".to_owned(), Json::String(op.to_owned()));
            serde_json::from_value::<HostRequest>(v).map_err(|e| e.to_string())
        } else {
            Err("its fields are a JSON object".to_owned())
        };
        let request = request.map_err(|message| {
            error_json(&HostError::Failed {
                message: format!("not a {op} request: {message}"),
            })
        })?;
        self.ask(request).await.map(|v| v.to_string())
    }
}

fn json_arg(text: &str) -> Json {
    if text.trim().is_empty() {
        return Json::Null;
    }
    serde_json::from_str(text).unwrap_or_else(|_| Json::String(text.to_owned()))
}

fn error_json(e: &HostError) -> String {
    serde_json::to_string(e).unwrap_or_else(|_| e.to_string())
}

impl wit_host::Host for State {
    async fn run_action(&mut self, action: String, args: String) -> Result<String, String> {
        if let Some(no) = self.worker_refusal() {
            return Err(no);
        }
        if self.own.contains(&action) {
            return Err(error_json(&HostError::Failed {
                message: format!("{action} is its own: it runs it itself, not through the host"),
            }));
        }
        let request = HostRequest::Run {
            action,
            args: json_arg(&args),
            ticket: None,
        };
        self.host(request)
            .await
            .map(|v| v.to_string())
            .map_err(|e| error_json(&e))
    }

    async fn setting(&mut self, key: String) -> Result<String, String> {
        if let Some(no) = self.worker_refusal() {
            return Err(no);
        }
        self.host(HostRequest::Setting { key })
            .await
            .map(|v| v.to_string())
            .map_err(|e| error_json(&e))
    }

    async fn set_setting(&mut self, key: String, value: String) -> Result<(), String> {
        if let Some(no) = self.worker_refusal() {
            return Err(no);
        }
        let value = json_arg(&value);
        self.host(HostRequest::SetSetting { key, value })
            .await
            .map(|_| ())
            .map_err(|e| error_json(&e))
    }

    async fn theme(&mut self) -> wit::ThemeData {
        self.theme.as_ref().map_or_else(
            || wit::ThemeData {
                id: String::new(),
                scheme: String::new(),
                revision: 0,
                tokens: Vec::new(),
            },
            |store| theme_to(wire::ThemeData::from_resolved(&store.current())),
        )
    }

    async fn log(&mut self, level: wit_host::Level, text: String) {
        let level = match level {
            wit_host::Level::Debug => "debug",
            wit_host::Level::Info => "info",
            wit_host::Level::Warn => "warn",
            wit_host::Level::Error => "error",
        };
        let _ = self
            .host(HostRequest::Log {
                text,
                level: Some(level.to_owned()),
            })
            .await;
    }

    async fn call_service(
        &mut self,
        service: String,
        method: String,
        args: String,
    ) -> Result<String, String> {
        if let Some(no) = self.worker_refusal() {
            return Err(no);
        }
        self.host(HostRequest::Service {
            service,
            method,
            args: json_arg(&args),
        })
        .await
        .map(|v| v.to_string())
        .map_err(|e| error_json(&e))
    }

    async fn notify(&mut self, text: String, kind: String) -> Result<(), String> {
        self.ask(HostRequest::Notify {
            text,
            kind: (!kind.is_empty()).then_some(kind),
            title: None,
        })
        .await
        .map(|_| ())
    }

    async fn clipboard_read(&mut self) -> Result<Option<String>, String> {
        self.ask(HostRequest::ClipboardRead)
            .await
            .map(|v| v.as_str().map(str::to_owned))
    }

    async fn clipboard_write(&mut self, text: String) -> Result<(), String> {
        self.ask(HostRequest::ClipboardWrite { text })
            .await
            .map(|_| ())
    }

    async fn pick(&mut self, request: String) -> Result<String, String> {
        self.ask_json("pick", &request).await
    }

    async fn prompt(&mut self, request: String) -> Result<String, String> {
        self.ask_json("prompt", &request).await
    }

    async fn menu(&mut self, request: String) -> Result<String, String> {
        self.ask_json("menu", &request).await
    }

    async fn decorate(
        &mut self,
        kind: String,
        id: String,
        decoration: Option<String>,
    ) -> Result<(), String> {
        let decoration = match decoration.map(|d| serde_json::from_str(&d)).transpose() {
            Ok(d) => d,
            Err(e) => {
                return Err(error_json(&HostError::Failed {
                    message: format!("not a decoration: {e}"),
                }));
            }
        };
        self.ask(HostRequest::Decorate {
            target: crate::hostcall::Target { kind, id },
            decoration,
        })
        .await
        .map(|_| ())
    }

    async fn now(&mut self) -> u64 {
        self.api.now_ms()
    }

    async fn subscribe(&mut self, hook: String) -> Result<String, String> {
        self.ask_json("subscribe", &hook)
            .await
            .map(|key| serde_json::from_str::<String>(&key).unwrap_or(key))
    }

    async fn unsubscribe(&mut self, key: String) -> Result<bool, String> {
        self.ask(HostRequest::Unsubscribe { key })
            .await
            .map(|v| v.as_bool().unwrap_or(false))
    }

    async fn list_documents(&mut self, under: String) -> Result<String, String> {
        self.ask(HostRequest::ListDocuments {
            under: (!under.is_empty()).then_some(under),
        })
        .await
        .map(|v| v.to_string())
    }

    async fn read_document(&mut self, path: String) -> Result<String, String> {
        self.ask(HostRequest::ReadDocument { path })
            .await
            .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
    }

    async fn create_document(&mut self, path: String, text: String) -> Result<String, String> {
        self.ask(HostRequest::CreateDocument { path, text })
            .await
            .map(|v| v.to_string())
    }

    async fn edit_document(
        &mut self,
        path: String,
        edits: String,
        name: Option<String>,
    ) -> Result<String, String> {
        let edits = match serde_json::from_str(&edits) {
            Ok(edits) => edits,
            Err(e) => {
                return Err(error_json(&HostError::Failed {
                    message: format!("not a document's edits: {e}"),
                }));
            }
        };
        self.ask(HostRequest::EditDocument { path, edits, name })
            .await
            .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
    }

    async fn rename_document(&mut self, path: String, to: String) -> Result<String, String> {
        self.ask(HostRequest::RenameDocument { path, to })
            .await
            .map(|v| v.to_string())
    }

    async fn move_document(&mut self, path: String, folder: String) -> Result<String, String> {
        self.ask(HostRequest::MoveDocument { path, folder })
            .await
            .map(|v| v.to_string())
    }

    async fn delete_document(&mut self, path: String) -> Result<(), String> {
        self.ask(HostRequest::DeleteDocument { path })
            .await
            .map(|_| ())
    }

    async fn watch_documents(&mut self, hook: String) -> Result<String, String> {
        self.ask_json("watch_documents", &hook).await
    }

    async fn editor_state(&mut self) -> Result<String, String> {
        self.ask(HostRequest::EditorState)
            .await
            .map(|v| v.to_string())
    }

    async fn edit_editor(&mut self, command: String) -> Result<String, String> {
        self.ask_json("edit_editor", &command).await
    }

    async fn decorate_editor(&mut self, decorations: String) -> Result<(), String> {
        self.ask_json(
            "decorate_editor",
            &format!("{{\"decorations\": {decorations}}}"),
        )
        .await
        .map(|_| ())
    }

    async fn fetch(&mut self, request: String) -> Result<String, String> {
        self.ask_json("fetch", &request).await
    }

    async fn read_body(&mut self, stream: String, max: u32) -> Result<String, String> {
        self.ask(HostRequest::ReadBody {
            stream,
            max: (max > 0).then_some(u64::from(max)),
        })
        .await
        .map(|v| v.to_string())
    }

    async fn close_body(&mut self, stream: String) -> bool {
        self.ask(HostRequest::CloseBody { stream })
            .await
            .is_ok_and(|v| v.as_bool().unwrap_or(false))
    }

    async fn secret_set(&mut self, name: String) -> Result<bool, String> {
        self.ask(HostRequest::Secret { name })
            .await
            .map(|v| v.get("set").and_then(Json::as_bool).unwrap_or(false))
    }

    async fn ask_secret(&mut self, name: String) -> Result<bool, String> {
        self.ask(HostRequest::AskSecret { name })
            .await
            .map(|v| v.as_bool().unwrap_or(false))
    }

    async fn ports(&mut self, kind: String) -> Result<String, String> {
        self.ask(HostRequest::Ports {
            kind: (!kind.is_empty()).then_some(kind),
        })
        .await
        .map(|v| v.to_string())
    }

    async fn open_port(&mut self, request: String) -> Result<String, String> {
        self.ask_json("open_port", &request).await
    }

    async fn port_send(&mut self, port: String, message: String) -> Result<(), String> {
        let message: Json = serde_json::from_str(&message).map_err(|e| e.to_string())?;
        self.ask(HostRequest::PortSend { port, message })
            .await
            .map(|_| ())
    }

    async fn port_receive(&mut self, port: String, max: u32) -> Result<String, String> {
        self.ask(HostRequest::PortReceive {
            port,
            max: (max > 0).then_some(max),
        })
        .await
        .map(|v| v.to_string())
    }

    async fn close_port(&mut self, port: String) -> bool {
        self.ask(HostRequest::ClosePort { port })
            .await
            .is_ok_and(|v| v.as_bool().unwrap_or(false))
    }

    async fn call(&mut self, request: String) -> String {
        if let Some(no) = self.worker_refusal() {
            return format!("{{\"error\": {no}}}");
        }
        let started = Instant::now();
        let api = self.api.clone();
        let answer = api.call_json(&request).await;
        self.clock.host = self.clock.host.saturating_add(started.elapsed());
        answer
    }
}

impl wit::Host for State {}

/// A component's stdout or stderr: each line into its log.
#[derive(Clone)]
struct LogPipe {
    id: String,
    logs: Logs,
    source: &'static str,
    level: Level,
    partial: Arc<Mutex<Vec<u8>>>,
}

impl LogPipe {
    fn write(&self, bytes: &[u8]) {
        let mut lines = Vec::new();
        {
            let mut partial = self.partial.lock().unwrap_or_else(PoisonError::into_inner);
            partial.extend_from_slice(bytes);
            while let Some(at) = partial.iter().position(|b| *b == b'\n') {
                let mut line: Vec<u8> = partial.drain(..=at).collect();
                line.pop();
                lines.push(String::from_utf8_lossy(&line).into_owned());
            }
            // A line that never ends is cut, not kept growing.
            if partial.len() > 4096 {
                lines.push(String::from_utf8_lossy(&partial).into_owned());
                partial.clear();
            }
        }
        for text in lines {
            self.logs.push(&self.id, self.level, self.source, text);
        }
    }
}

impl wasmtime_wasi::cli::IsTerminal for LogPipe {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl wasmtime_wasi::cli::StdoutStream for LogPipe {
    fn async_stream(&self) -> Box<dyn tokio::io::AsyncWrite + Send + Sync> {
        Box::new(self.clone())
    }
}

impl tokio::io::AsyncWrite for LogPipe {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.write(buf);
        std::task::Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

/// What a running component is doing, for the manager (`component`) and
/// tests. Cheap to clone.
#[derive(Debug)]
pub struct Stats {
    pub id: String,
    pub name: String,
    pub limits: Limits,
    memory: Arc<AtomicUsize>,
    calls: AtomicU64,
    traps: AtomicU32,
    /// Its thread runs and its store is held.
    alive: AtomicBool,
    last_trap: Mutex<Option<String>>,
    restart: Mutex<Option<mpsc::UnboundedSender<Job>>>,
    crew: Option<workers::Crew>,
}

impl Stats {
    /// Its workers that have not ended, or whose end it has not heard.
    #[must_use]
    pub fn workers(&self) -> Vec<WorkerInfo> {
        self.crew
            .as_ref()
            .map(workers::Crew::infos)
            .unwrap_or_default()
    }

    /// Its workers' instances held now (running, or waiting for the
    /// pool): 0 once they are all gone.
    #[must_use]
    pub fn workers_alive(&self) -> usize {
        self.crew.as_ref().map_or(0, workers::Crew::alive)
    }

    /// Its linear memory now, in bytes.
    #[must_use]
    pub fn memory(&self) -> usize {
        self.memory.load(Ordering::Relaxed)
    }

    /// The calls it has answered.
    #[must_use]
    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::Relaxed)
    }

    /// The calls it trapped in (each started it again).
    #[must_use]
    pub fn traps(&self) -> u32 {
        self.traps.load(Ordering::Relaxed)
    }

    /// Why it last trapped.
    #[must_use]
    pub fn last_trap(&self) -> Option<String> {
        self.last_trap
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether its store is still held: `false` once it is unloaded and
    /// its thread has ended.
    #[must_use]
    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    /// Starts it again, fresh.
    pub fn restart(&self) {
        if let Some(jobs) = &*self.restart.lock().unwrap_or_else(PoisonError::into_inner) {
            let _ = jobs.unbounded_send(Job::Restart);
        }
    }

    /// Runs its views' timers that are due on the host's clock now,
    /// without waiting for the next one's time (a test that moved its
    /// clock).
    pub fn tick(&self) {
        if let Some(jobs) = &*self.restart.lock().unwrap_or_else(PoisonError::into_inner) {
            let _ = jobs.unbounded_send(Job::Tick);
        }
    }
}

/// The components running now, by extension id. Cheap to clone.
#[derive(Clone, Debug, Default)]
pub struct Components(Arc<Mutex<BTreeMap<String, Arc<Stats>>>>);

impl Components {
    #[must_use]
    pub fn get(&self, id: &str) -> Option<Arc<Stats>> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned()
    }

    #[must_use]
    pub fn all(&self) -> Vec<Arc<Stats>> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }

    fn put(&self, stats: Arc<Stats>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(stats.id.clone(), stats);
    }

    fn take(&self, stats: &Arc<Stats>) {
        let mut all = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if all.get(&stats.id).is_some_and(|s| Arc::ptr_eq(s, stats)) {
            all.remove(&stats.id);
        }
    }
}

/// What an instance is asked, in its queue.
enum Job {
    Invoke {
        action: String,
        args: String,
        invocation: String,
        who: Option<Principal>,
        reply: oneshot::Sender<Result<String, String>>,
    },
    Render {
        pane: u64,
        view: String,
        frames: mpsc::UnboundedSender<wire::Frame>,
    },
    Event {
        pane: u64,
        event: wire::ViewEvent,
    },
    Close {
        pane: u64,
    },
    SaveState {
        reply: std::sync::mpsc::Sender<Option<Vec<u8>>>,
    },
    /// Its place among the app's processes, once its thread runs.
    Attach(crate_process::Handle),
    Restart,
    /// Runs its timers that are due now ([`Stats::tick`]).
    Tick,
    Stop,
}

/// What every instance of one extension is made from.
struct Recipe {
    manifest: Manifest,
    component: Component,
    linker: Arc<Linker<State>>,
    limits: Limits,
    api: ExtensionApi,
    logs: Logs,
    theme: Option<architect_theme::ThemeStore>,
    memory: Arc<AtomicUsize>,
    /// Its workers, when its manifest declares any.
    crew: Option<workers::Crew>,
}

/// A store for `data`: its memory limit, and its epoch deadline — each
/// tick it yields; past its call's budget, or (a worker) once cancelled,
/// it traps.
fn store(data: State) -> Result<Store<State>, String> {
    let engine = engine()?;
    let mut store = Store::new(engine, data);
    store.limiter(|s| &mut s.limiter);
    store.set_epoch_deadline(1);
    store.epoch_deadline_callback(|mut cx| {
        let data = cx.data_mut();
        if matches!(&data.side, Side::Worker(link) if link.cancelled()) {
            return Err(wasmtime::format_err!("cancelled"));
        }
        let clock = &mut data.clock;
        if clock.started.is_some() && clock.own() > clock.budget {
            clock.over = true;
            return Err(wasmtime::format_err!("interrupted"));
        }
        Ok(UpdateDeadline::Yield(1))
    });
    Ok(store)
}

impl Recipe {
    fn log(&self, level: Level, text: impl Into<String>) {
        self.logs.push(&self.manifest.id, level, "wasi", text);
    }

    /// A fresh instance, started with `state`, checked against its
    /// manifest.
    async fn instantiate(&self, state: Option<Vec<u8>>) -> Result<(Store<State>, World), String> {
        let data = State::new(
            &self.manifest.id,
            &self.logs,
            self.limits,
            Arc::clone(&self.memory),
            self.api.clone(),
            self.manifest.actions.iter().map(|a| a.id.clone()).collect(),
            self.theme.clone(),
            Side::Owner(self.crew.clone()),
        );
        let mut store = store(data)?;
        begin(&mut store);
        let made = World::instantiate_async(&mut store, &self.component, &self.linker).await;
        let world =
            end(&mut store, made).map_err(|why| format!("it does not instantiate: {why}"))?;
        begin(&mut store);
        let guest = world.architect_extension_guest();
        let init = guest.call_init(&mut store, state.as_deref()).await;
        end(&mut store, init)?.map_err(|why| format!("its init failed: {why}"))?;
        begin(&mut store);
        let actions = guest.call_actions(&mut store).await;
        let actions = end(&mut store, actions)?;
        begin(&mut store);
        let views = guest.call_views(&mut store).await;
        let views = end(&mut store, views)?;
        let m = &self.manifest;
        if let Some(missing) = m
            .actions
            .iter()
            .find(|a| !m.host_runs(a) && !actions.contains(&a.id))
        {
            return Err(format!(
                "its manifest declares action {} it does not run",
                missing.id
            ));
        }
        if let Some(missing) = m
            .views
            .iter()
            .find(|v| v.kind == architect_workspace::DIOXUS && !views.contains(&v.id))
        {
            return Err(format!(
                "its manifest declares view {} it does not render",
                missing.id
            ));
        }
        Ok((store, world))
    }
}

/// A call starts: its clock and its memory's refusals.
fn begin(store: &mut Store<State>) {
    let data = store.data_mut();
    data.clock.started = Some(Instant::now());
    data.clock.host = Duration::ZERO;
    data.clock.over = false;
    data.limiter.hit = None;
}

/// A call ended: its answer, or why it trapped.
fn end<R>(store: &mut Store<State>, result: wasmtime::Result<R>) -> Result<R, String> {
    let data = store.data_mut();
    let own = data.clock.own();
    data.clock.started = None;
    result.map_err(|e| {
        if data.clock.over {
            format!(
                "interrupted: it ran for {} ms, past its limit of {} ms",
                own.as_millis(),
                data.clock.budget.as_millis()
            )
        } else if let Some(why) = data.limiter.hit.take() {
            why
        } else {
            format!("it trapped: {e:#}")
        }
    })
}

/// One extension's instance, on its own thread: its calls, one at a time.
struct Actor {
    recipe: Recipe,
    live: Option<(Store<State>, World)>,
    panes: HashMap<u64, mpsc::UnboundedSender<wire::Frame>>,
    stats: Arc<Stats>,
    process: Option<crate_process::Handle>,
    generation: u32,
    /// Its workers' news waits to be heard.
    news_due: bool,
    /// When it last heard their news.
    heard_at: Instant,
    /// When its views' next timer is due.
    timer_at: Option<Instant>,
}

/// What its actor does next.
enum Step {
    Job(Option<Job>),
    Nudged,
    Hear,
    Timers,
}

/// The soonest a component's timers run again: a 0 ms interval does not
/// spin its thread.
const TIMER_FLOOR: Duration = Duration::from_millis(4);

impl Drop for Actor {
    fn drop(&mut self) {
        // Its workers go with it.
        if let Some(crew) = &self.recipe.crew {
            crew.cancel_all();
        }
        self.live = None;
        self.stats.alive.store(false, Ordering::Release);
    }
}

impl Actor {
    async fn run(
        mut self,
        mut jobs: mpsc::UnboundedReceiver<Job>,
        news: mpsc::UnboundedReceiver<()>,
    ) {
        use futures::{FutureExt as _, StreamExt as _};
        let mut news = news.fuse();
        loop {
            // Its workers' news, at most once each NEWS_EVERY, between
            // its calls.
            let wait = self
                .news_due
                .then(|| NEWS_EVERY.saturating_sub(self.heard_at.elapsed()));
            let timer = async move {
                match wait {
                    Some(d) => tokio::time::sleep(d).await,
                    None => std::future::pending::<()>().await,
                }
            };
            let timers_at = self.timer_at;
            let timers = async move {
                match timers_at {
                    Some(at) => tokio::time::sleep_until(at.into()).await,
                    None => std::future::pending::<()>().await,
                }
            };
            let step = futures::select_biased! {
                job = jobs.next() => Step::Job(job),
                () = news.select_next_some() => Step::Nudged,
                () = Box::pin(timer).fuse() => Step::Hear,
                () = Box::pin(timers).fuse() => Step::Timers,
            };
            match step {
                Step::Job(None) => break,
                Step::Job(Some(job)) => {
                    if !self.handle(job).await {
                        break;
                    }
                }
                Step::Nudged => self.news_due = true,
                Step::Hear => self.hear().await,
                Step::Timers => self.timers().await,
            }
        }
        if let Some(p) = &self.process {
            p.stopped();
        }
    }

    /// Does `job`; `false` when it was told to stop.
    async fn handle(&mut self, job: Job) -> bool {
        match job {
            Job::Invoke {
                action,
                args,
                invocation,
                who,
                reply,
            } => {
                let ran = self.invoke(&action, &args, &invocation, who).await;
                let _ = reply.send(ran);
                self.timers().await;
            }
            Job::Render { pane, view, frames } => {
                self.render(pane, &view, frames).await;
                // What it rendered may have set a timer.
                self.timers().await;
            }
            Job::Event { pane, event } => {
                self.event(pane, event).await;
                self.timers().await;
            }
            Job::Tick => self.timers().await,
            Job::Close { pane } => {
                self.panes.remove(&pane);
                if let Some((store, world)) = &mut self.live {
                    begin(store);
                    let closed = world
                        .architect_extension_guest()
                        .call_close(&mut *store, pane)
                        .await;
                    if let Err(why) = end(store, closed) {
                        self.trapped(why).await;
                    }
                }
            }
            Job::SaveState { reply } => {
                let mut saved = None;
                if let Some((store, world)) = &mut self.live {
                    begin(store);
                    let got = world
                        .architect_extension_guest()
                        .call_save_state(&mut *store)
                        .await;
                    match end(store, got) {
                        Ok(state) => saved = state,
                        Err(why) => self.trapped(why).await,
                    }
                }
                let _ = reply.send(saved);
            }
            Job::Attach(handle) => {
                self.process = Some(handle);
                return true;
            }
            Job::Restart => self.restart("asked to start again".to_owned()).await,
            Job::Stop => return false,
        }
        self.stats.calls.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// Its workers' news, into it: what they posted, how far they are,
    /// how they ended; then what its views changed.
    async fn hear(&mut self) {
        self.news_due = false;
        self.heard_at = Instant::now();
        let Some(crew) = self.recipe.crew.clone() else {
            return;
        };
        let news = crew.news();
        if news.is_empty() {
            return;
        }
        let Some((store, world)) = &mut self.live else {
            return;
        };
        if let Err(why) = workers::deliver(store, world, news).await {
            self.trapped(why).await;
            return;
        }
        self.flush().await;
    }

    /// Its timers that are due, then what its views changed; when the
    /// next is due.
    async fn timers(&mut self) {
        let Some((store, world)) = &mut self.live else {
            self.timer_at = None;
            return;
        };
        begin(store);
        let ticked = world
            .architect_extension_guest()
            .call_tick(&mut *store)
            .await;
        match end(store, ticked) {
            Ok(next) => {
                self.timer_at = next.and_then(|ms| {
                    Instant::now()
                        .checked_add(Duration::from_millis(u64::from(ms)).max(TIMER_FLOOR))
                });
            }
            Err(why) => {
                self.timer_at = None;
                self.trapped(why).await;
                return;
            }
        }
        self.flush().await;
    }

    fn not_running(&self) -> String {
        self.stats.last_trap().map_or_else(
            || format!("{} is not running", self.stats.name),
            |why| format!("{} is not running: {why}", self.stats.name),
        )
    }

    async fn invoke(
        &mut self,
        action: &str,
        args: &str,
        invocation: &str,
        who: Option<Principal>,
    ) -> Result<String, String> {
        let not_running = self.not_running();
        let Some((store, world)) = &mut self.live else {
            return Err(not_running);
        };
        store.data_mut().api = self.recipe.api.clone().acting_for(who);
        begin(store);
        let ran = world
            .architect_extension_guest()
            .call_invoke(&mut *store, action, args, invocation)
            .await;
        match end(store, ran) {
            Ok(answer) => answer,
            Err(why) => {
                self.trapped(why.clone()).await;
                Err(why)
            }
        }
    }

    async fn render(&mut self, pane: u64, view: &str, frames: mpsc::UnboundedSender<wire::Frame>) {
        let not_running = self.not_running();
        let Some((store, world)) = &mut self.live else {
            let _ = frames.unbounded_send(wire::Frame::Failed(not_running));
            return;
        };
        begin(store);
        let rendered = world
            .architect_extension_guest()
            .call_render(&mut *store, pane, view)
            .await;
        match end(store, rendered) {
            Ok(Ok(edits)) => {
                let first = wire::Frame::Edits {
                    edits: edits.into_iter().map(edit_from).collect(),
                };
                if frames.unbounded_send(first).is_ok() {
                    self.panes.insert(pane, frames);
                }
            }
            Ok(Err(why)) => {
                let _ = frames.unbounded_send(wire::Frame::Failed(why));
            }
            Err(why) => {
                let _ = frames.unbounded_send(wire::Frame::Failed(why.clone()));
                self.trapped(why).await;
            }
        }
    }

    async fn event(&mut self, pane: u64, event: wire::ViewEvent) {
        let Some((store, world)) = &mut self.live else {
            return;
        };
        if !self.panes.contains_key(&pane) {
            return;
        }
        begin(store);
        let handled = world
            .architect_extension_guest()
            .call_on_event(&mut *store, pane, &event_to(event))
            .await;
        match end(store, handled) {
            Ok(Ok(edits)) => self.send(pane, edits),
            Ok(Err(why)) => {
                if let Some(frames) = self.panes.remove(&pane) {
                    let _ = frames.unbounded_send(wire::Frame::Failed(why));
                }
            }
            Err(why) => self.trapped(why).await,
        }
    }

    /// What its views changed by themselves, to their panes.
    async fn flush(&mut self) {
        if self.panes.is_empty() {
            return;
        }
        let Some((store, world)) = &mut self.live else {
            return;
        };
        begin(store);
        let pending = world
            .architect_extension_guest()
            .call_pending(&mut *store)
            .await;
        match end(store, pending) {
            Ok(pending) => {
                for p in pending {
                    self.send(p.pane, p.edits);
                }
            }
            Err(why) => self.trapped(why).await,
        }
    }

    fn send(&mut self, pane: u64, edits: Vec<wit::Edit>) {
        if edits.is_empty() {
            return;
        }
        let frame = wire::Frame::Edits {
            edits: edits.into_iter().map(edit_from).collect(),
        };
        if let Some(frames) = self.panes.get(&pane)
            && frames.unbounded_send(frame).is_err()
        {
            self.panes.remove(&pane);
        }
    }

    /// It trapped in a call: it cannot be entered again, so it is started
    /// again, fresh.
    async fn trapped(&mut self, why: String) {
        self.stats.traps.fetch_add(1, Ordering::Relaxed);
        *self
            .stats
            .last_trap
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(why.clone());
        self.recipe
            .log(Level::Error, format!("{why}; starting it again"));
        self.restart(why).await;
    }

    async fn restart(&mut self, why: String) {
        self.live = None;
        self.timer_at = None;
        // Its workers were the old instance's.
        if let Some(line) = self
            .recipe
            .crew
            .as_ref()
            .and_then(|c| workers::cancelled_line(c.cancel_all(), "it is starting again"))
        {
            self.recipe.log(Level::Info, line);
        }
        // Its views' streams end: their panes say why, and mount again
        // once it runs.
        self.panes.clear();
        if let Some(p) = &self.process {
            p.exited(why.clone(), true);
        }
        match self.recipe.instantiate(None).await {
            Ok(live) => {
                self.live = Some(live);
                self.generation = self.generation.saturating_add(1);
                if let Some(p) = &self.process {
                    p.running(self.generation);
                }
            }
            Err(again) => {
                self.recipe
                    .log(Level::Error, format!("it did not start again: {again}"));
                if let Some(p) = &self.process {
                    p.exited(again, false);
                }
            }
        }
    }
}

/// A component as a local [`Process`](crate::process::Process): what lets
/// `RemoteView` and `EXT_CHECK` show its views.
#[cfg(all(feature = "process", unix))]
mod crate_process {
    use std::sync::Arc;

    use futures::channel::{mpsc, oneshot};

    use super::Job;
    use crate::process::{Local, Process, Processes, State};
    use crate::wire::{Frame, ViewEvent};

    /// The [`Local`] a component's process is served by: its queue.
    pub(super) struct Door {
        pub(super) jobs: mpsc::UnboundedSender<Job>,
        pub(super) rt: tokio::runtime::Handle,
        pub(super) views: Vec<String>,
    }

    impl Local for Door {
        fn run(
            &self,
            action: String,
            args: String,
            context: Option<String>,
        ) -> futures::future::BoxFuture<'static, Result<String, String>> {
            let (reply, answer) = oneshot::channel();
            let sent = self.jobs.unbounded_send(Job::Invoke {
                action,
                args,
                invocation: context.unwrap_or_else(|| "null".to_owned()),
                who: None,
                reply,
            });
            Box::pin(async move {
                sent.map_err(|_| "it was unloaded".to_owned())?;
                answer.await.map_err(|_| "it was unloaded".to_owned())?
            })
        }

        fn open(
            &self,
            view: &str,
            pane: u64,
            frames: mpsc::UnboundedSender<Frame>,
        ) -> Result<mpsc::UnboundedSender<ViewEvent>, String> {
            if !self.views.iter().any(|v| v == view) {
                return Err(format!("it has no view {view}"));
            }
            self.jobs
                .unbounded_send(Job::Render {
                    pane,
                    view: view.to_owned(),
                    frames,
                })
                .map_err(|_| "it was unloaded".to_owned())?;
            let (events, mut from) = mpsc::unbounded::<ViewEvent>();
            let jobs = self.jobs.clone();
            self.rt.spawn(async move {
                use futures::StreamExt as _;
                while let Some(event) = from.next().await {
                    if jobs.unbounded_send(Job::Event { pane, event }).is_err() {
                        break;
                    }
                }
            });
            Ok(events)
        }

        fn close(&self, pane: u64) {
            let _ = self.jobs.unbounded_send(Job::Close { pane });
        }
    }

    /// A component's place in the app's [`Processes`].
    pub(super) struct Handle {
        process: Process,
        processes: Processes,
        rt: tokio::runtime::Handle,
    }

    impl Handle {
        pub(super) fn running(&self, generation: u32) {
            self.process.set_state(State::Running {
                generation,
                pid: std::process::id(),
            });
        }

        pub(super) fn exited(&self, why: String, restarting: bool) {
            self.process.set_state(State::Exited { why, restarting });
        }

        pub(super) fn stopped(&self) {
            self.process.set_state(State::Stopped);
            self.processes.take(&self.process);
        }

        /// Its "Start again" (a pane's button, the manager's restart) asks
        /// the component.
        pub(super) fn follow_restarts(&self, jobs: mpsc::UnboundedSender<Job>) {
            let process = self.process.clone();
            self.rt.spawn(async move {
                loop {
                    process.restart_asked().await;
                    if process.stop_asked() || jobs.unbounded_send(Job::Restart).is_err() {
                        break;
                    }
                }
            });
        }

        pub(super) fn make(
            id: &str,
            name: &str,
            door: Door,
            processes: Processes,
        ) -> (Self, Arc<dyn Local>) {
            let rt = door.rt.clone();
            let local: Arc<dyn Local> = Arc::new(door);
            let process = Process::local(id, name, rt.clone(), Arc::clone(&local));
            processes.put(process.clone());
            (
                Self {
                    process,
                    processes,
                    rt,
                },
                local,
            )
        }
    }
}

/// Without `process`, a component has no place among processes.
#[cfg(not(all(feature = "process", unix)))]
mod crate_process {
    pub(super) struct Handle;

    impl Handle {
        pub(super) fn running(&self, _: u32) {}
        pub(super) fn exited(&self, _: String, _: bool) {}
        pub(super) fn stopped(&self) {}
    }
}

/// Runs WASI component extensions (`kind = "wasm"`).
pub struct WasiRunner {
    limits: Limits,
    worker_limits: Limits,
    pool: Pool,
    cache: Option<PathBuf>,
    theme: Option<architect_theme::ThemeStore>,
    components: Components,
    #[cfg(all(feature = "process", unix))]
    processes: Option<crate::process::Processes>,
    linker: Mutex<Option<Arc<Linker<State>>>>,
}

impl std::fmt::Debug for WasiRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasiRunner")
            .field("limits", &self.limits)
            .field("worker_limits", &self.worker_limits)
            .field("pool", &self.pool)
            .field("cache", &self.cache)
            .finish_non_exhaustive()
    }
}

impl Default for WasiRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl WasiRunner {
    /// A runner with the default [`Limits`] and no compile cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            limits: Limits::default(),
            worker_limits: Limits::worker(),
            pool: Pool::default(),
            cache: None,
            theme: None,
            components: Components::default(),
            #[cfg(all(feature = "process", unix))]
            processes: None,
            linker: Mutex::new(None),
        }
    }

    /// Each instance's limits.
    #[must_use]
    pub const fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Each worker's limits ([`Limits::worker`] when not given): its
    /// memory, and its whole run's own time.
    #[must_use]
    pub const fn with_worker_limits(mut self, limits: Limits) -> Self {
        self.worker_limits = limits;
        self
    }

    /// The threads its workers run on ([`Pool::default`] when not given).
    #[must_use]
    pub fn with_pool(mut self, pool: Pool) -> Self {
        self.pool = pool;
        self
    }

    /// The threads its workers run on.
    #[must_use]
    pub fn pool(&self) -> Pool {
        self.pool.clone()
    }

    /// Keeps compiled components in `dir` (`<hash>.cwasm`), so one is
    /// compiled once, not at every start.
    #[must_use]
    pub fn with_cache(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cache = Some(dir.into());
        self
    }

    /// The theme its components' `theme` import reads.
    #[must_use]
    pub fn with_theme(mut self, store: architect_theme::ThemeStore) -> Self {
        self.theme = Some(store);
        self
    }

    /// Puts each component among `processes` as a local process, so
    /// `RemoteView` shows its views and `EXT_CHECK` renders them.
    #[cfg(all(feature = "process", unix))]
    #[must_use]
    pub fn with_processes(mut self, processes: crate::process::Processes) -> Self {
        self.processes = Some(processes);
        self
    }

    /// The components it runs.
    #[must_use]
    pub fn components(&self) -> Components {
        self.components.clone()
    }

    fn linker(&self, engine: &Engine) -> Result<Arc<Linker<State>>, String> {
        let mut slot = self.linker.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(linker) = &*slot {
            return Ok(Arc::clone(linker));
        }
        let mut linker = Linker::<State>::new(engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker).map_err(|e| format!("{e:#}"))?;
        World::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s)
            .map_err(|e| format!("{e:#}"))?;
        let linker = Arc::new(linker);
        *slot = Some(Arc::clone(&linker));
        drop(slot);
        Ok(linker)
    }
}

/// A loaded component.
struct Loaded {
    jobs: mpsc::UnboundedSender<Job>,
    stats: Arc<Stats>,
    components: Components,
    limits: Limits,
}

impl Instance for Loaded {
    fn action(&self, decl: &ActionDecl) -> Result<CallerAsyncHandler, String> {
        let jobs = self.jobs.clone();
        let id = decl.id.clone();
        Ok(Arc::new(move |caller: &ActionCaller, args: &[u8]| {
            let args = String::from_utf8(args.to_vec()).map_err(|e| ActionError::BadArgs {
                id: id.clone(),
                message: e.to_string(),
                expected: None,
            })?;
            let (reply, answer) = oneshot::channel();
            let sent = jobs.unbounded_send(Job::Invoke {
                action: id.clone(),
                args,
                invocation: crate::invocation::caller_json(caller).to_string(),
                who: caller.principal().cloned(),
                reply,
            });
            let id = id.clone();
            let run: ActionFuture = Box::pin(async move {
                let failed = |message: String| ActionError::Failed {
                    id: id.clone(),
                    message,
                };
                sent.map_err(|_| failed("it was unloaded".to_owned()))?;
                answer
                    .await
                    .map_err(|_| failed("it was unloaded".to_owned()))?
                    .map(String::into_bytes)
                    .map_err(failed)
            });
            Ok(run)
        }))
    }

    fn save_state(&self) -> Option<Vec<u8>> {
        let (reply, saved) = std::sync::mpsc::channel();
        self.jobs.unbounded_send(Job::SaveState { reply }).ok()?;
        saved
            .recv_timeout(self.limits.call.saturating_add(Duration::from_secs(1)))
            .ok()
            .flatten()
    }

    fn stop(&self) {
        let _ = self.jobs.unbounded_send(Job::Stop);
        self.components.take(&self.stats);
    }
}

impl Runner for WasiRunner {
    fn kind(&self) -> Kind {
        Kind::Wasm
    }

    fn start(&self, package: &Package, cx: Context<'_>) -> Result<Box<dyn Instance>, String> {
        let manifest = package.manifest.clone();
        let path = manifest
            .component()
            .ok_or("its manifest names no component ([wasm] component)")?;
        let dir = package.dir().ok_or("a WASM extension runs from a folder")?;
        let bytes = std::fs::read(dir.join(path)).map_err(|e| format!("{path}: {e}"))?;
        let engine = engine()?;
        let started = Instant::now();
        let (component, compiled) = compile(engine, &bytes, self.cache.as_deref())?;
        if let Some(import) = refused_import(engine, &component) {
            return Err(format!(
                "it imports {import}: a component reaches the app through the host service only"
            ));
        }
        cx.logs.push(
            &manifest.id,
            Level::Info,
            "wasi",
            format!(
                "{} in {} ms ({})",
                match compiled {
                    Compiled::Fresh => "compiled",
                    Compiled::Cached => "loaded from the compile cache",
                },
                started.elapsed().as_millis(),
                if PULLEY {
                    "Pulley, interpreted"
                } else {
                    "Cranelift, native"
                }
            ),
        );
        let linker = self.linker(engine)?;
        let (nudge, news) = workers::Nudge::new();
        let crew = self.crew(&manifest, dir, &component, &linker, &cx, nudge)?;
        let memory = Arc::new(AtomicUsize::new(0));
        let recipe = Recipe {
            manifest: manifest.clone(),
            component,
            linker,
            limits: self.limits,
            api: cx.api.clone(),
            logs: cx.logs.clone(),
            theme: self.theme.clone(),
            memory: Arc::clone(&memory),
            crew: crew.clone(),
        };
        let (jobs, queue) = mpsc::unbounded::<Job>();
        let stats = Arc::new(Stats {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            limits: self.limits,
            memory,
            calls: AtomicU64::new(0),
            traps: AtomicU32::new(0),
            alive: AtomicBool::new(true),
            last_trap: Mutex::new(None),
            restart: Mutex::new(Some(jobs.clone())),
            crew,
        });
        let (say_up, heard) = std::sync::mpsc::channel::<Result<tokio::runtime::Handle, String>>();
        let saved = cx.state;
        let thread_stats = Arc::clone(&stats);
        let spawned = std::thread::Builder::new()
            .name(format!("ext-wasi {}", manifest.id))
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        thread_stats.alive.store(false, Ordering::Release);
                        let _ = say_up.send(Err(e.to_string()));
                        return;
                    }
                };
                rt.block_on(async move {
                    let live = match recipe.instantiate(saved).await {
                        Ok(live) => live,
                        Err(why) => {
                            thread_stats.alive.store(false, Ordering::Release);
                            let _ = say_up.send(Err(why));
                            return;
                        }
                    };
                    let actor = Actor {
                        recipe,
                        live: Some(live),
                        panes: HashMap::new(),
                        stats: thread_stats,
                        process: None,
                        generation: 1,
                        news_due: false,
                        heard_at: Instant::now(),
                        timer_at: None,
                    };
                    let _ = say_up.send(Ok(tokio::runtime::Handle::current()));
                    actor.run(queue, news).await;
                });
            })
            .map_err(|e| e.to_string())?;
        let waited = self
            .limits
            .call
            .saturating_mul(4)
            .saturating_add(Duration::from_secs(2));
        let rt = match heard.recv_timeout(waited) {
            Ok(Ok(rt)) => rt,
            Ok(Err(why)) => {
                let _ = spawned.join();
                return Err(why);
            }
            Err(_) => {
                let _ = jobs.unbounded_send(Job::Stop);
                return Err("it did not start in time".to_owned());
            }
        };
        self.attach(&manifest, &jobs, rt);
        self.components.put(Arc::clone(&stats));
        Ok(Box::new(Loaded {
            jobs,
            stats,
            components: self.components.clone(),
            limits: self.limits,
        }))
    }
}

impl WasiRunner {
    /// What its `[[workers]]` run, each compiled (a separate component
    /// through the cache): `None` with none.
    fn crew(
        &self,
        manifest: &Manifest,
        dir: &Path,
        component: &Component,
        linker: &Arc<Linker<State>>,
        cx: &Context<'_>,
        nudge: workers::Nudge,
    ) -> Result<Option<workers::Crew>, String> {
        if manifest.workers.is_empty() {
            return Ok(None);
        }
        let engine = engine()?;
        let mut entries = BTreeMap::new();
        for w in &manifest.workers {
            let Some(entry) = &w.entry else {
                continue;
            };
            let made = if Path::new(entry)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("wasm"))
            {
                let bytes = std::fs::read(dir.join(entry))
                    .map_err(|e| format!("worker {}: {entry}: {e}", w.id))?;
                let (separate, _) = compile(engine, &bytes, self.cache.as_deref())
                    .map_err(|e| format!("worker {}: {e}", w.id))?;
                if let Some(import) = refused_import(engine, &separate) {
                    return Err(format!(
                        "worker {}: it imports {import}: a component reaches the app through the host service only",
                        w.id
                    ));
                }
                workers::Entry {
                    component: separate,
                    entry: w.id.clone(),
                }
            } else {
                workers::Entry {
                    component: component.clone(),
                    entry: entry.clone(),
                }
            };
            entries.insert(w.id.clone(), made);
        }
        let recipe = workers::WorkerRecipe {
            id: manifest.id.clone(),
            entries,
            linker: Arc::clone(linker),
            limits: self.worker_limits,
            api: cx.api.clone(),
            logs: cx.logs.clone(),
            pool: self.pool.clone(),
        };
        Ok(Some(workers::Crew::new(recipe, nudge)))
    }

    /// Puts a started component among the app's processes, when it has
    /// them.
    #[cfg(all(feature = "process", unix))]
    fn attach(
        &self,
        manifest: &Manifest,
        jobs: &mpsc::UnboundedSender<Job>,
        rt: tokio::runtime::Handle,
    ) {
        let Some(processes) = &self.processes else {
            return;
        };
        let door = crate_process::Door {
            jobs: jobs.clone(),
            rt,
            views: manifest.views.iter().map(|v| v.id.clone()).collect(),
        };
        let (handle, _) =
            crate_process::Handle::make(&manifest.id, &manifest.name, door, processes.clone());
        handle.follow_restarts(jobs.clone());
        let _ = jobs.unbounded_send(Job::Attach(handle));
    }

    #[cfg(not(all(feature = "process", unix)))]
    fn attach(&self, _: &Manifest, _: &mpsc::UnboundedSender<Job>, _: tokio::runtime::Handle) {}
}

// The wire's types and the world's, one way and the other.

fn value_from(v: wit::Value) -> wire::Value {
    match v {
        wit::Value::None => wire::Value::None,
        wit::Value::Text(t) => wire::Value::Text(t),
        wit::Value::Float(f) => wire::Value::Float(f),
        wit::Value::Int(i) => wire::Value::Int(i),
        wit::Value::Bool(b) => wire::Value::Bool(b),
    }
}

fn edit_from(e: wit::Edit) -> wire::Edit {
    use wire::Edit as W;
    use wit::Edit as E;
    match e {
        E::PushId(id) => W::PushId { id },
        E::SetId(id) => W::SetId { id },
        E::Child(index) => W::Child { index },
        E::Pop => W::Pop,
        E::CreateElement(c) => W::CreateElement {
            tag: c.tag,
            namespace: c.namespace,
        },
        E::CreateText(value) => W::CreateText { value },
        E::Clone => W::Clone,
        E::AppendChildren(m) => W::AppendChildren { m },
        E::ReplaceWith(m) => W::ReplaceWith { m },
        E::InsertAfter(m) => W::InsertAfter { m },
        E::InsertBefore(m) => W::InsertBefore { m },
        E::SetAttribute(a) => W::SetAttribute {
            name: a.name,
            namespace: a.namespace,
            value: value_from(a.value),
        },
        E::SetText(value) => W::SetText { value },
        E::Listen(name) => W::Listen { name },
        E::Unlisten(name) => W::Unlisten { name },
        E::Remove => W::Remove,
    }
}

const fn mods_to(m: wire::Mods) -> wit::Mods {
    wit::Mods {
        alt: m.alt,
        ctrl: m.ctrl,
        meta: m.meta,
        shift: m.shift,
    }
}

fn pointer_to(p: wire::Pointer) -> wit::Pointer {
    wit::Pointer {
        client_x: p.client_x,
        client_y: p.client_y,
        page_x: p.page_x,
        page_y: p.page_y,
        screen_x: p.screen_x,
        screen_y: p.screen_y,
        element_x: p.element_x,
        element_y: p.element_y,
        button: p.button,
        buttons: p.buttons,
        mods: mods_to(p.mods),
    }
}

fn touch_point_to(t: wire::TouchPoint) -> wit::TouchPoint {
    wit::TouchPoint {
        id: t.id,
        client_x: t.client_x,
        client_y: t.client_y,
        page_x: t.page_x,
        page_y: t.page_y,
        screen_x: t.screen_x,
        screen_y: t.screen_y,
        force: t.force,
    }
}

fn event_to(e: wire::ViewEvent) -> wit::ViewEvent {
    let data = match e.data {
        wire::EventData::Mouse(p) => wit::EventData::Mouse(pointer_to(p)),
        wire::EventData::Key(k) => wit::EventData::Key(wit::KeyData {
            key: k.key,
            code: k.code,
            location: k.location,
            repeat: k.repeat,
            composing: k.composing,
            mods: mods_to(k.mods),
        }),
        wire::EventData::Form(value) => wit::EventData::Form(value),
        wire::EventData::Focus => wit::EventData::Focus,
        wire::EventData::Pointer(p) => wit::EventData::Pointer(wit::PointerData {
            at: pointer_to(p.at),
            pointer_id: p.pointer_id,
            pointer_type: p.pointer_type,
            is_primary: p.is_primary,
            pressure: p.pressure,
            tilt_x: p.tilt_x,
            tilt_y: p.tilt_y,
            twist: p.twist,
        }),
        wire::EventData::Touch(t) => wit::EventData::Touch(wit::TouchData {
            touches: t.touches.into_iter().map(touch_point_to).collect(),
            changed: t.changed.into_iter().map(touch_point_to).collect(),
            mods: mods_to(t.mods),
        }),
        wire::EventData::Wheel(w) => wit::EventData::Wheel(wit::WheelData {
            at: pointer_to(w.at),
            delta_x: w.delta_x,
            delta_y: w.delta_y,
            delta_mode: w.delta_mode,
        }),
        wire::EventData::Scroll(s) => wit::EventData::Scroll(wit::ScrollData {
            top: s.top,
            left: s.left,
            width: s.width,
            height: s.height,
            client_width: s.client_width,
            client_height: s.client_height,
        }),
        wire::EventData::Composition(text) => wit::EventData::Composition(text),
    };
    wit::ViewEvent {
        element: e.element,
        name: e.name,
        bubbles: e.bubbles,
        data,
    }
}

fn theme_to(t: wire::ThemeData) -> wit::ThemeData {
    wit::ThemeData {
        id: t.id,
        scheme: t.scheme,
        revision: t.revision,
        tokens: t
            .tokens
            .into_iter()
            .map(|t| wit::Token {
                name: t.name,
                value: t.value,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::{cache_key, import_allowed};

    #[test]
    fn the_network_is_no_components() {
        assert!(import_allowed("architect:extension/host@0.1.0"));
        assert!(import_allowed("wasi:cli/stdout@0.2.6"));
        assert!(!import_allowed("wasi:sockets/tcp@0.2.6"));
        assert!(!import_allowed("wasi:http/outgoing-handler@0.2.6"));
        assert!(!import_allowed("acme:anything/else"));
    }

    #[test]
    fn the_cache_is_keyed_by_the_component_and_the_engine() {
        let a = cache_key(1, b"component");
        assert_eq!(a, cache_key(1, b"component"));
        assert_ne!(a, cache_key(1, b"another"));
        assert_ne!(a, cache_key(2, b"component"));
        assert_eq!(a.len(), 64);
    }
}
