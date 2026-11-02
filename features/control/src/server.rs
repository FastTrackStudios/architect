//! The per-app local control endpoint (feature `server`).
//!
//! [`Control::serve`] binds the app's socket ([`crate::endpoint`]) and
//! answers on a thread of its own, with a runtime of its own, so an app
//! that has none (a Blitz window, a REAPER extension) serves it as easily
//! as one that has. Each connection is a conversation: one JSON request a
//! line (`{"id", "method", "params"}`), one answer a line, in order. A
//! request is answered on a blocking thread, as MCP's HTTP transport
//! answers one: a `watch` that waits holds only its own connection.
//!
//! **Who may connect.** Only this user: the socket is `0600` in a `0700`
//! directory, and a connection whose peer is another user (checked against
//! the directory's owner) is closed unanswered. On Windows the pipe takes
//! no remote clients. Who a connection *is*, to the list's authorizer, is
//! [`Control::with_caller`]'s to say — the surface's principal unless it
//! does — exactly as MCP's server says it for a model.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;

use architect::action::ActionCaller;
use serde_json::Value;
use tokio::io::{
    AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _,
};

use crate::Surface;
use crate::protocol::{Answer, Code, Failure, Request};

/// The longest request line read; a longer one ends the connection.
pub const MAX_LINE: u64 = 16 * 1024 * 1024;

/// Who is on the other end of a connection, as the operating system
/// says: what [`Control::with_caller`] decides the caller from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Peer {
    /// Its user id (Unix).
    pub uid: Option<u32>,
    /// Its process id, where the platform tells it.
    pub pid: Option<i32>,
}

type Resolve = Arc<dyn Fn(&Peer) -> ActionCaller + Send + Sync>;

/// An app's control endpoint, not yet serving.
pub struct Control {
    surface: Arc<Surface>,
    path: PathBuf,
    resolve: Resolve,
}

impl std::fmt::Debug for Control {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Control")
            .field("app", &self.surface.name())
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Control {
    /// `surface` at app `name`'s socket ([`crate::endpoint::socket_path`]).
    ///
    /// # Errors
    ///
    /// `name` is not a plain app name.
    pub fn new(name: &str, surface: Surface) -> io::Result<Self> {
        Ok(Self::at(crate::endpoint::socket_path(name)?, surface))
    }

    /// `surface` at `path` (a socket's path; a pipe's name on Windows).
    #[must_use]
    pub fn at(path: impl Into<PathBuf>, surface: Surface) -> Self {
        Self {
            surface: Arc::new(surface),
            path: path.into(),
            resolve: Arc::new(|_| ActionCaller::app()),
        }
    }

    /// Who a connection runs as: its principal, checked by the list's
    /// authorizer, and the context its actions' handlers read. A caller
    /// with no principal runs as the surface's (never as the trusted app).
    #[must_use]
    pub fn with_caller(
        mut self,
        resolve: impl Fn(&Peer) -> ActionCaller + Send + Sync + 'static,
    ) -> Self {
        self.resolve = Arc::new(resolve);
        self
    }

    /// Where it serves.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Serve when [`crate::endpoint::wanted`] says so — `FTS_CONTROL`, else
    /// on in a debug build only.
    ///
    /// # Errors
    ///
    /// As [`Self::serve`].
    pub fn serve_if_wanted(self) -> io::Result<Option<Serving>> {
        if crate::endpoint::wanted(crate::endpoint::dev_default()) {
            self.serve().map(Some)
        } else {
            Ok(None)
        }
    }

    /// Bind the socket and answer on a thread of its own until the
    /// [`Serving`] is dropped. A socket left by an app that is gone is
    /// replaced; one an app still answers on is not.
    ///
    /// # Errors
    ///
    /// Another app serves this socket ([`io::ErrorKind::AddrInUse`]), or
    /// it could not be bound.
    pub fn serve(self) -> io::Result<Serving> {
        let bound = bind(&self.path)?;
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let path = self.path.clone();
        let thread = std::thread::Builder::new()
            .name(format!("{}-control", self.surface.name()))
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                runtime.block_on(accept(bound, self.surface, self.resolve, stopped));
                // A request still waiting (a `watch`) is not waited for.
                runtime.shutdown_timeout(std::time::Duration::from_millis(100));
            })?;
        Ok(Serving {
            path,
            stop: Some(stop),
            thread: Some(thread),
        })
    }
}

/// A control endpoint serving. Dropping it stops it and removes its
/// socket.
#[derive(Debug)]
pub struct Serving {
    path: PathBuf,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Serving {
    /// Where it serves.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Serving {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        #[cfg(unix)]
        let _ = std::fs::remove_file(&self.path);
    }
}

/// One request line answered, as the line to send back.
///
/// Not JSON, or no method: a [`Code::BadRequest`] error for id `null` (or
/// the id it had). A method or parameter there is none of: that refusal.
/// Otherwise what [`Surface::answer`] says for `caller`.
pub async fn answer_line(surface: &Surface, caller: &ActionCaller, line: &str) -> Value {
    let message: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            let failure = Failure::new(Code::BadRequest, format!("not JSON: {e}"));
            return Answer::Failed(failure).to_line(&Value::Null);
        }
    };
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        let failure = Failure::new(Code::BadRequest, "no method");
        return Answer::Failed(failure).to_line(&id);
    };
    let params = message
        .get("params")
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    match Request::parse(method, &params) {
        Ok(request) => surface.answer(caller, &request).await.to_line(&id),
        Err(failure) => Answer::Failed(failure).to_line(&id),
    }
}

/// One connection: request lines in, answer lines out, until it closes.
async fn converse<S>(stream: S, surface: Arc<Surface>, caller: ActionCaller)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (read, mut write) = tokio::io::split(stream);
    let mut read = tokio::io::BufReader::new(read);
    let mut line = Vec::new();
    loop {
        line.clear();
        let Ok(n) = (&mut read)
            .take(MAX_LINE)
            .read_until(b'\n', &mut line)
            .await
        else {
            return;
        };
        if n == 0 {
            return;
        }
        let too_long = !line.ends_with(b"\n") && u64::try_from(n).is_ok_and(|n| n >= MAX_LINE);
        let text = String::from_utf8_lossy(&line).trim().to_owned();
        if text.is_empty() && !too_long {
            continue;
        }
        let reply = if too_long {
            let failure = Failure::new(
                Code::BadRequest,
                format!("a request is {MAX_LINE} bytes at most"),
            );
            Answer::Failed(failure).to_line(&Value::Null)
        } else {
            let surface = surface.clone();
            let caller = caller.clone();
            // A request may block (a `watch` that waits, a sync action):
            // on a blocking thread, as MCP's HTTP transport answers one.
            tokio::task::spawn_blocking(move || {
                futures::executor::block_on(answer_line(&surface, &caller, &text))
            })
            .await
            .unwrap_or_else(|_| {
                Answer::Failed(Failure::new(Code::Failed, "the request's thread failed"))
                    .to_line(&Value::Null)
            })
        };
        let mut bytes = reply.to_string().into_bytes();
        bytes.push(b'\n');
        if write.write_all(&bytes).await.is_err() || write.flush().await.is_err() {
            return;
        }
        if too_long {
            return;
        }
    }
}

#[cfg(unix)]
type Bound = std::os::unix::net::UnixListener;

#[cfg(windows)]
type Bound = PathBuf;

/// The socket bound: its directory made (`0700`), a dead app's socket
/// replaced, the socket `0600`.
#[cfg(unix)]
fn bind(path: &Path) -> io::Result<Bound> {
    use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    if path.exists() {
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("an app already answers on {}", path.display()),
            ));
        }
        std::fs::remove_file(path)?;
    }
    let listener = std::os::unix::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

#[cfg(windows)]
#[allow(clippy::unnecessary_wraps)]
fn bind(path: &Path) -> io::Result<Bound> {
    Ok(path.to_path_buf())
}

/// The user a socket's directory belongs to: the only one let in.
#[cfg(unix)]
fn owner_of(path: &Path) -> Option<u32> {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::metadata(path.parent()?).ok().map(|m| m.uid())
}

#[cfg(unix)]
async fn accept(
    bound: Bound,
    surface: Arc<Surface>,
    resolve: Resolve,
    mut stopped: tokio::sync::oneshot::Receiver<()>,
) {
    let owner = bound
        .local_addr()
        .ok()
        .and_then(|a| a.as_pathname().and_then(owner_of));
    let Ok(listener) = tokio::net::UnixListener::from_std(bound) else {
        return;
    };
    loop {
        let stream = tokio::select! {
            _ = &mut stopped => return,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => stream,
                Err(_) => continue,
            },
        };
        let cred = stream.peer_cred().ok();
        let peer = Peer {
            uid: cred.map(|c| c.uid()),
            pid: cred.and_then(|c| c.pid()),
        };
        if owner.is_some() && peer.uid != owner {
            continue;
        }
        let caller = resolve(&peer);
        tokio::spawn(converse(stream, surface.clone(), caller));
    }
}

#[cfg(windows)]
async fn accept(
    bound: Bound,
    surface: Arc<Surface>,
    resolve: Resolve,
    mut stopped: tokio::sync::oneshot::Receiver<()>,
) {
    use tokio::net::windows::named_pipe::ServerOptions;
    let name = bound.as_os_str().to_owned();
    let Ok(mut server) = ServerOptions::new()
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .create(&name)
    else {
        return;
    };
    loop {
        tokio::select! {
            _ = &mut stopped => return,
            connected = server.connect() => if connected.is_err() { continue },
        }
        let Ok(next) = ServerOptions::new()
            .reject_remote_clients(true)
            .create(&name)
        else {
            return;
        };
        let connected = std::mem::replace(&mut server, next);
        let caller = resolve(&Peer::default());
        tokio::spawn(converse(connected, surface.clone(), caller));
    }
}
