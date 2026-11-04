//! The network, for extensions (#310): the host service's `fetch`, its
//! streamed bodies, and the secrets the host keeps for them.
//!
//! An extension never opens a socket: a WASI component's world has no
//! `wasi:sockets` or `wasi:http` (`crate::wasi` refuses one that imports
//! them), a Luau script and a page have no network of their own. They ask
//! the host to `fetch` (`crate::hostcall::Fetch`), and the host checks the
//! URL's host against `capabilities.hosts` as a person granted them
//! (`crate::hostcall::may_reach_url`), sends it, and follows each redirect
//! itself, checking every one the same way: a redirect to a host it may
//! not reach is refused, not followed. Each request has a time limit
//! ([`TIMEOUT`], at most [`MAX_TIMEOUT`]) and a size limit on what comes
//! back ([`MAX_BODY`] read whole; [`MAX_STREAM`] streamed); what it sends
//! is at most [`MAX_UPLOAD`].
//!
//! A response fetched with `stream` is kept by the host ([`Net`]) and read
//! a part at a time (`read_body`), by the extension that fetched it only;
//! at most [`MAX_STREAMS`] are open for one extension, and they go when it
//! unloads.
//!
//! **Secrets** ([`Secrets`]): what a manifest's `[[secrets]]` declare — an
//! API key the person enters once, asked for through the app's prompt
//! (`crate::hostui::HostUi::prompt`, a `secret` prompt) — kept by the host
//! in a file of its own (`<app dir>/extension-secrets.json` in the lab,
//! `0600`), never in the extension's folder, and never handed to it. A
//! request names one in a header's value (`"Bearer {{secret:api_key}}"`);
//! the host puts it in on a request to one of that secret's hosts, and
//! takes it off again on a redirect anywhere else.
//!
//! The requests run on a runtime of their own (two threads, `ext-net`),
//! so a fetch works from whichever executor its caller is on: a WASI
//! instance's thread, a Luau script's, a page's.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{Value, json};

use crate::hostcall::{Fetch, HostError};

/// How long a request waits for its response when it says nothing
/// (`timeout_ms`), and a streamed body for each part.
pub const TIMEOUT: Duration = Duration::from_secs(30);
/// The longest a request may ask to wait.
pub const MAX_TIMEOUT: Duration = Duration::from_secs(300);
/// The most of a body read whole (`max_bytes` may ask for less, not
/// more).
pub const MAX_BODY: u64 = 16 << 20;
/// The most of a streamed body, by default and at most.
pub const MAX_STREAM: u64 = 1 << 30;
/// The most a request may send.
pub const MAX_UPLOAD: usize = 16 << 20;
/// The most redirects one request follows.
pub const MAX_REDIRECTS: u32 = 10;
/// How much of a streamed body one `read_body` gives, when it says
/// nothing.
pub const CHUNK: u64 = 64 << 10;
/// The most streamed responses one extension has open.
pub const MAX_STREAMS: usize = 16;

/// The headers an extension may not set: the host's to say.
const HOST_HEADERS: [&str; 5] = [
    "host",
    "content-length",
    "connection",
    "transfer-encoding",
    "upgrade",
];

/// The headers that go only to the host they were meant for: on a
/// redirect to another host they are left off (as a browser does).
const CREDENTIALS: [&str; 3] = ["authorization", "cookie", "proxy-authorization"];

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ── Secrets ─────────────────────────────────────────────────────────

/// The secrets the host keeps for extensions, by extension id and the
/// secret's id: in memory ([`Self::memory`]) or in one private file
/// ([`Self::open`]). Cheap to clone (the clones share them).
///
/// Its `Debug` shows which secrets are set, never a value.
#[derive(Clone, Default)]
pub struct Secrets {
    inner: Arc<SecretsInner>,
}

#[derive(Default)]
struct SecretsInner {
    file: Option<PathBuf>,
    kept: Mutex<BTreeMap<String, BTreeMap<String, String>>>,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kept = lock(&self.inner.kept);
        let set: BTreeMap<&String, Vec<&String>> =
            kept.iter().map(|(id, s)| (id, s.keys().collect())).collect();
        f.debug_struct("Secrets")
            .field("file", &self.inner.file)
            .field("set", &set)
            .finish()
    }
}

impl Secrets {
    /// Kept in memory, for the run.
    #[must_use]
    pub fn memory() -> Self {
        Self::default()
    }

    /// Kept in `file` (JSON, written `0600` on unix), read now when it is
    /// there. A file that does not read is taken as empty.
    #[must_use]
    pub fn open(file: impl Into<PathBuf>) -> Self {
        let file = file.into();
        let kept = std::fs::read_to_string(&file)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Self {
            inner: Arc::new(SecretsInner {
                file: Some(file),
                kept: Mutex::new(kept),
            }),
        }
    }

    /// The file they are kept in, when they are.
    #[must_use]
    pub fn file(&self) -> Option<&Path> {
        self.inner.file.as_deref()
    }

    /// Whether extension `ext`'s secret `name` is set.
    #[must_use]
    pub fn is_set(&self, ext: &str, name: &str) -> bool {
        lock(&self.inner.kept)
            .get(ext)
            .is_some_and(|s| s.contains_key(name))
    }

    /// The secrets set for extension `ext`, by id.
    #[must_use]
    pub fn names(&self, ext: &str) -> Vec<String> {
        lock(&self.inner.kept)
            .get(ext)
            .map(|s| s.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Sets extension `ext`'s secret `name` to `value`.
    ///
    /// # Errors
    ///
    /// Its file could not be written.
    pub fn set(&self, ext: &str, name: &str, value: &str) -> Result<(), String> {
        let mut kept = lock(&self.inner.kept);
        kept.entry(ext.to_owned())
            .or_default()
            .insert(name.to_owned(), value.to_owned());
        self.keep(&kept)
    }

    /// Forgets extension `ext`'s secret `name`: whether it was set.
    ///
    /// # Errors
    ///
    /// Its file could not be written.
    pub fn forget(&self, ext: &str, name: &str) -> Result<bool, String> {
        let mut kept = lock(&self.inner.kept);
        let had = kept.get_mut(ext).is_some_and(|s| s.remove(name).is_some());
        if kept.get(ext).is_some_and(BTreeMap::is_empty) {
            kept.remove(ext);
        }
        self.keep(&kept)?;
        Ok(had)
    }

    /// Extension `ext`'s secret `name`'s value: the host's, to put into a
    /// request. Never an extension's to read.
    pub(crate) fn get(&self, ext: &str, name: &str) -> Option<String> {
        lock(&self.inner.kept)
            .get(ext)
            .and_then(|s| s.get(name))
            .cloned()
    }

    fn keep(&self, kept: &BTreeMap<String, BTreeMap<String, String>>) -> Result<(), String> {
        let Some(file) = &self.inner.file else {
            return Ok(());
        };
        let text = serde_json::to_string_pretty(kept).map_err(|e| e.to_string())?;
        architect::settings::write_atomic(file, text.as_bytes(), true)
            .map(|_| ())
            .map_err(|e| format!("{}: {e}", file.display()))
    }
}

// ── The runtime and the client ──────────────────────────────────────

/// The network's own runtime: what a fetch runs on, whoever asks.
fn runtime() -> Result<&'static tokio::runtime::Runtime, HostError> {
    static RT: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ext-net")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(|e| HostError::Unsupported {
        what: format!("the network: {e}"),
    })
}

/// The one client: no redirects of its own (each is the host's to check).
fn client() -> Result<reqwest::Client, String> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .user_agent(concat!("architect-extensions/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|e| e.to_string())
        })
        .clone()
}

/// Runs `work` on the network's runtime: its answer, from any executor.
async fn on_runtime<T: Send + 'static>(
    work: impl std::future::Future<Output = Result<T, HostError>> + Send + 'static,
) -> Result<T, HostError> {
    runtime()?
        .spawn(work)
        .await
        .map_err(|e| HostError::Failed {
            message: format!("the request stopped: {e}"),
        })?
}

// ── A request ───────────────────────────────────────────────────────

/// A header to send: its value, and, when a secret is in it, the hosts it
/// may go to.
#[derive(Clone)]
pub(crate) struct Header {
    pub name: String,
    pub value: String,
    /// The secrets it carries: none, a plain header.
    pub secrets: Vec<String>,
}

/// A fetch the host checked, its secrets put in: what [`send`] sends.
pub(crate) struct Plan {
    pub url: url::Url,
    pub method: reqwest::Method,
    pub headers: Vec<Header>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
    pub max: u64,
    pub stream: bool,
    pub binary: bool,
    /// Whether the extension may reach a redirect's URL.
    pub may_reach: Arc<dyn Fn(&url::Url) -> Result<(), (String, String)> + Send + Sync>,
    /// Whether a secret may go to a URL: `(secret, url)`.
    pub secret_reaches: Arc<dyn Fn(&str, &url::Url) -> bool + Send + Sync>,
}

/// What came back.
pub(crate) struct Fetched {
    pub status: u16,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub redirects: u32,
    pub body: Got,
}

/// A response's body: read whole, or to be read a part at a time.
pub(crate) enum Got {
    Whole(Vec<u8>),
    Stream(reqwest::Response),
}

/// Why a redirect was refused: the capability and why, for the span.
pub(crate) const REDIRECT: &str = "hosts";

impl Plan {
    /// `fetch`'s fields, checked: its method, headers and body within the
    /// limits. `secret(name)` is each secret's value.
    ///
    /// # Errors
    ///
    /// A method or header that is not one, a header that is the host's, a
    /// body past [`MAX_UPLOAD`], or what `secret` refused.
    pub(crate) fn of(
        fetch: &Fetch,
        url: url::Url,
        mut secret: impl FnMut(&str) -> Result<String, HostError>,
        may_reach: Arc<dyn Fn(&url::Url) -> Result<(), (String, String)> + Send + Sync>,
        secret_reaches: Arc<dyn Fn(&str, &url::Url) -> bool + Send + Sync>,
    ) -> Result<Self, HostError> {
        let failed = |message: String| HostError::Failed { message };
        let method = fetch.method.as_deref().unwrap_or("GET").to_ascii_uppercase();
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| failed(format!("{method} is not an HTTP method")))?;
        let mut headers = Vec::new();
        for (name, value) in &fetch.headers {
            let lower = name.to_ascii_lowercase();
            if HOST_HEADERS.contains(&lower.as_str()) {
                return Err(failed(format!("{name} is the host's header to set")));
            }
            reqwest::header::HeaderName::from_bytes(lower.as_bytes())
                .map_err(|_| failed(format!("{name} is not a header's name")))?;
            let secrets: Vec<String> = crate::hostcall::secret_refs(value)
                .into_iter()
                .map(str::to_owned)
                .collect();
            let value = crate::hostcall::with_secrets(value, &mut secret)?;
            reqwest::header::HeaderValue::from_str(&value).map_err(|_| {
                failed(format!(
                    "{name}'s value is not a header's (a line break, a control character)"
                ))
            })?;
            headers.push(Header {
                name: lower,
                value,
                secrets,
            });
        }
        let body = fetch.body.as_ref().map(|b| b.clone().into_bytes());
        if body.as_ref().is_some_and(|b| b.len() > MAX_UPLOAD) {
            return Err(failed(format!(
                "its body is past the most a request sends ({MAX_UPLOAD} bytes)"
            )));
        }
        let timeout = fetch
            .timeout_ms
            .map_or(TIMEOUT, Duration::from_millis)
            .min(MAX_TIMEOUT);
        let cap = if fetch.stream { MAX_STREAM } else { MAX_BODY };
        Ok(Self {
            url,
            method,
            headers,
            body,
            timeout,
            max: fetch.max_bytes.unwrap_or(cap).min(cap),
            stream: fetch.stream,
            binary: fetch.binary,
            may_reach,
            secret_reaches,
        })
    }
}

/// Sends `plan` and follows its redirects, each checked: its response,
/// its body read whole within its cap (or, `stream`, to be read).
///
/// # Errors
///
/// [`HostError::Refused`] for a redirect to a host it may not reach;
/// [`HostError::Failed`] for a time or size limit passed, a network error,
/// too many redirects.
pub(crate) async fn send(plan: Plan) -> Result<Fetched, HostError> {
    on_runtime(follow(plan)).await
}

async fn follow(plan: Plan) -> Result<Fetched, HostError> {
    let failed = |message: String| HostError::Failed { message };
    let client = client().map_err(|e| failed(format!("the network: {e}")))?;
    let started = Instant::now();
    let first_host = plan.url.host_str().unwrap_or_default().to_owned();
    let mut url = plan.url.clone();
    let mut method = plan.method.clone();
    let mut body = plan.body.clone();
    let mut redirects = 0_u32;
    loop {
        let host = url.host_str().unwrap_or_default().to_owned();
        let mut request = client.request(method.clone(), url.clone());
        for h in &plan.headers {
            let goes = if h.secrets.is_empty() {
                host == first_host || !CREDENTIALS.contains(&h.name.as_str())
            } else {
                h.secrets.iter().all(|s| (plan.secret_reaches)(s, &url))
            };
            if goes {
                request = request.header(&h.name, &h.value);
            }
        }
        if let Some(b) = &body {
            request = request.body(b.clone());
        }
        let left = plan.timeout.saturating_sub(started.elapsed());
        let response = match tokio::time::timeout(left, request.send()).await {
            Err(_) => return Err(timed_out(plan.timeout)),
            Ok(Err(e)) => return Err(failed(network_error(&url, &e))),
            Ok(Ok(r)) => r,
        };
        let status = response.status();
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|l| l.to_str().ok())
            .map(str::to_owned);
        if status.is_redirection()
            && let Some(location) = location
        {
            redirects += 1;
            if redirects > MAX_REDIRECTS {
                return Err(failed(format!(
                    "more than {MAX_REDIRECTS} redirects from {}",
                    plan.url
                )));
            }
            let next = url
                .join(&location)
                .map_err(|e| failed(format!("a redirect to {location}, not a URL: {e}")))?;
            (plan.may_reach)(&next).map_err(|(capability, why)| HostError::Refused {
                capability,
                why: format!("redirected to {next}: {why}"),
            })?;
            let code = status.as_u16();
            if code == 303 || (matches!(code, 301 | 302) && method == reqwest::Method::POST) {
                method = reqwest::Method::GET;
                body = None;
            }
            url = next;
            continue;
        }
        let headers = headers_of(response.headers());
        let final_url = response.url().to_string();
        if response.content_length().is_some_and(|n| n > plan.max) {
            return Err(too_big(plan.max));
        }
        let got = if plan.stream {
            Got::Stream(response)
        } else {
            let left = plan.timeout.saturating_sub(started.elapsed());
            match tokio::time::timeout(left, read_whole(response, plan.max)).await {
                Err(_) => return Err(timed_out(plan.timeout)),
                Ok(r) => Got::Whole(r?),
            }
        };
        return Ok(Fetched {
            status: status.as_u16(),
            url: final_url,
            headers,
            redirects,
            body: got,
        });
    }
}

async fn read_whole(mut response: reqwest::Response, max: u64) -> Result<Vec<u8>, HostError> {
    let mut out = Vec::new();
    while let Some(part) = response.chunk().await.map_err(|e| HostError::Failed {
        message: format!("its body: {e}"),
    })? {
        out.extend_from_slice(&part);
        if out.len() as u64 > max {
            return Err(too_big(max));
        }
    }
    Ok(out)
}

fn headers_of(map: &reqwest::header::HeaderMap) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in map {
        let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
        out.entry(name.as_str().to_owned())
            .and_modify(|v| {
                v.push_str(", ");
                v.push_str(&value);
            })
            .or_insert(value);
    }
    out
}

fn timed_out(after: Duration) -> HostError {
    HostError::Failed {
        message: format!("timed out after {} ms", after.as_millis()),
    }
}

fn too_big(max: u64) -> HostError {
    HostError::Failed {
        message: format!("the response is past the most it takes ({max} bytes)"),
    }
}

/// A network error in words: what reqwest says, past the URL (which it
/// repeats).
fn network_error(url: &url::Url, e: &reqwest::Error) -> String {
    let mut why = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        why = format!("{why}: {s}");
        source = s.source();
    }
    format!("{}: {why}", url.host_str().unwrap_or_default())
}

/// A body as an answer carries it: text (`binary`: base64).
fn encode(bytes: &[u8], binary: bool) -> Value {
    if binary {
        Value::String(base64::engine::general_purpose::STANDARD.encode(bytes))
    } else {
        Value::String(String::from_utf8_lossy(bytes).into_owned())
    }
}

impl Fetched {
    /// The answer to the extension: `{status, url, headers, body}`, or with
    /// `stream` for `body`.
    pub(crate) fn answer(&self, body: Option<Value>, stream: Option<&str>) -> Value {
        let mut out = json!({
            "status": self.status,
            "url": self.url,
            "headers": self.headers,
        });
        if let Some(body) = body {
            out["body"] = body;
        }
        if let Some(stream) = stream {
            out["stream"] = Value::String(stream.to_owned());
        }
        out
    }
}

// ── Streams ─────────────────────────────────────────────────────────

/// The streamed responses extensions have open.
#[derive(Default)]
pub(crate) struct Net {
    streams: Mutex<BTreeMap<String, Arc<Open>>>,
    next: AtomicU64,
}

struct Open {
    owner: String,
    binary: bool,
    cap: u64,
    timeout: Duration,
    body: tokio::sync::Mutex<Reading>,
}

struct Reading {
    response: Option<reqwest::Response>,
    /// Read and not yet given (past a part's `max`; an unfinished
    /// character of text).
    pending: Vec<u8>,
    read: u64,
}

impl Net {
    /// Keeps `response` for `owner`: its stream's id.
    ///
    /// # Errors
    ///
    /// It has [`MAX_STREAMS`] open already.
    pub(crate) fn keep(
        &self,
        owner: &str,
        response: reqwest::Response,
        plan_binary: bool,
        cap: u64,
        timeout: Duration,
    ) -> Result<String, HostError> {
        let mut streams = lock(&self.streams);
        if streams.values().filter(|o| o.owner == owner).count() >= MAX_STREAMS {
            return Err(HostError::Failed {
                message: format!(
                    "it has {MAX_STREAMS} streamed responses open: close one (close_body) first"
                ),
            });
        }
        let id = format!("s{}", self.next.fetch_add(1, Ordering::Relaxed) + 1);
        streams.insert(
            id.clone(),
            Arc::new(Open {
                owner: owner.to_owned(),
                binary: plan_binary,
                cap,
                timeout,
                body: tokio::sync::Mutex::new(Reading {
                    response: Some(response),
                    pending: Vec::new(),
                    read: 0,
                }),
            }),
        );
        Ok(id)
    }

    /// The next part of `owner`'s stream `id`, at most `max` bytes:
    /// `{chunk, done}`. A stream that is done, failed or past its cap is
    /// closed.
    ///
    /// # Errors
    ///
    /// No such stream of its own; its time limit or cap passed; the
    /// network failed.
    pub(crate) async fn read(&self, owner: &str, id: &str, max: Option<u64>) -> Result<Value, HostError> {
        let open = lock(&self.streams)
            .get(id)
            .filter(|o| o.owner == owner)
            .cloned()
            .ok_or_else(|| HostError::NotFound {
                what: format!("stream {id}"),
            })?;
        let max = usize::try_from(max.unwrap_or(CHUNK).clamp(1, MAX_BODY)).unwrap_or(usize::MAX);
        let reading = Arc::clone(&open);
        let got = on_runtime(async move { next_part(&reading, max).await }).await;
        match got {
            Ok((bytes, done)) => {
                if done {
                    lock(&self.streams).remove(id);
                }
                Ok(json!({ "chunk": encode(&bytes, open.binary), "done": done }))
            }
            Err(e) => {
                lock(&self.streams).remove(id);
                Err(e)
            }
        }
    }

    /// Closes `owner`'s stream `id`: whether it had it open.
    pub(crate) fn close(&self, owner: &str, id: &str) -> bool {
        let mut streams = lock(&self.streams);
        if streams.get(id).is_some_and(|o| o.owner == owner) {
            streams.remove(id);
            true
        } else {
            false
        }
    }

    /// Closes every stream `owner` has open (it unloaded).
    pub(crate) fn forget(&self, owner: &str) {
        lock(&self.streams).retain(|_, o| o.owner != owner);
    }
}

/// Reads `open`'s next part: at most `max` bytes, and whether that was
/// all. Text is given whole characters at a time.
async fn next_part(open: &Open, max: usize) -> Result<(Vec<u8>, bool), HostError> {
    let mut r = open.body.lock().await;
    while r.pending.len() < max {
        // Enough for a part already: give it, read more next time.
        if !r.pending.is_empty() && (open.binary || std::str::from_utf8(&r.pending).is_ok()) {
            break;
        }
        let Some(response) = r.response.as_mut() else {
            break;
        };
        match tokio::time::timeout(open.timeout, response.chunk()).await {
            Err(_) => return Err(timed_out(open.timeout)),
            Ok(Err(e)) => {
                return Err(HostError::Failed {
                    message: format!("its body: {e}"),
                });
            }
            Ok(Ok(None)) => r.response = None,
            Ok(Ok(Some(part))) => {
                r.read += part.len() as u64;
                if r.read > open.cap {
                    return Err(too_big(open.cap));
                }
                r.pending.extend_from_slice(&part);
            }
        }
    }
    let mut n = r.pending.len().min(max);
    if !open.binary {
        // Never half a character: what is left of one waits for the rest.
        match std::str::from_utf8(&r.pending[..n]) {
            Ok(_) => {}
            Err(e) if e.error_len().is_none() && r.response.is_some() => n = e.valid_up_to(),
            Err(e) if e.error_len().is_none() && n < r.pending.len() => n = e.valid_up_to(),
            Err(_) => {}
        }
    }
    let part: Vec<u8> = r.pending.drain(..n).collect();
    let done = r.pending.is_empty() && r.response.is_none();
    Ok((part, done))
}

/// A body read whole, as its answer carries it.
pub(crate) fn whole(bytes: &[u8], binary: bool) -> Value {
    encode(bytes, binary)
}

#[cfg(test)]
mod tests {
    use super::Secrets;

    #[test]
    fn secrets_are_kept_privately_and_never_shown() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("secrets.json");
        let s = Secrets::open(&file);
        s.set("lab.ai", "api_key", "sk-very-secret").unwrap();
        assert!(s.is_set("lab.ai", "api_key"));
        assert!(!format!("{s:?}").contains("sk-very-secret"), "{s:?}");
        let again = Secrets::open(&file);
        assert_eq!(again.get("lab.ai", "api_key").as_deref(), Some("sk-very-secret"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(again.forget("lab.ai", "api_key").unwrap());
        assert!(!Secrets::open(&file).is_set("lab.ai", "api_key"));
    }
}
