//! The vault's query service in the browser (#495, #710's data model):
//! queries (Bases, a project's tasks, the calendar's events) run here,
//! over the vault index, with the one implementation every client runs
//! (`vault_query::QueryEngine`), served in this page
//! (`architect::serve_local`). The extension host holds only its client
//! (`Host::set_vault`, with the documents, for the person).
//!
//! - **Signed in**, the index is the node's, synced: a snapshot of the
//!   org's vault as this person may read it, then the deltas since its
//!   revision (`VaultIndexSync` on the org lane), applied to a
//!   [`Replica`]. The browser never holds the vault: an index entry is a
//!   note's title, properties, tags, links, headings and task lines, no
//!   body. Note bodies come only for the notes opened (live) or read.
//! - **In the demo**, the index is built here from the demo vault in the
//!   tab's memory, with the same builder a desk keeps its own with
//!   (`vault_query::Built` over [`DemoNotes`]).
//!
//! Kept in memory only; sign-out (a reload) drops it.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use architect_extensions::documents::Documents;
use architect_extensions::host::Host;
use architect_extensions::hostapi::VaultScope;
use architect_navigator::{MemorySource, NavigatorSource as _, Subscription};
use vault_query::{Built, IndexFeed, Note, NoteSource, QueryEngine, Replica};

/// How often a signed-in page asks the node what changed, ms.
const POLL_MS: u32 = 2_000;

/// The demo vault as the index builder reads it: its notes and `.base`
/// files, in path order, their text from the source. Its revision moves
/// with every change the source says.
pub struct DemoNotes {
    source: MemorySource,
    revision: Arc<AtomicU64>,
    _changes: Subscription,
}

impl DemoNotes {
    /// Over `source`.
    #[must_use]
    pub fn new(source: MemorySource) -> Self {
        let revision = Arc::new(AtomicU64::new(1));
        let moved = Arc::clone(&revision);
        let changes = source.subscribe(Box::new(move |_| {
            moved.fetch_add(1, Ordering::Relaxed);
        }));
        Self {
            source,
            revision,
            _changes: changes,
        }
    }
}

fn indexed(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".base")
}

impl NoteSource for DemoNotes {
    fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    fn visit(&self, from: usize, n: usize, each: &mut dyn FnMut(Note<'_>)) -> (u64, usize) {
        let revision = self.revision();
        let mut files: Vec<_> = self
            .source
            .files()
            .into_iter()
            .filter(|f| indexed(&f.path))
            .collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let total = files.len();
        for f in files.iter().skip(from).take(n) {
            let Some(text) = self.source.read_text(&f.path) else {
                continue;
            };
            let mtime = u64::try_from(f.modified).unwrap_or(0);
            let len = u64::try_from(text.len()).unwrap_or(u64::MAX);
            each(Note {
                path: &f.path,
                // Its time and its length: a change moves one or both.
                stamp: mtime.rotate_left(20) ^ len,
                text: vault_query::Text::Here(&text),
                parsed: None,
                mtime,
                is_base: f.path.to_ascii_lowercase().ends_with(".base"),
            });
        }
        (revision, total)
    }
}

/// Serves `feed`'s query engine in this page and gives `host` its client,
/// with `documents`, for the person whose vault it is
/// (`Host::set_vault`). Kept for the page's life.
fn serve(host: &Host, documents: Arc<dyn Documents>, feed: Arc<dyn IndexFeed>) {
    let host = host.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let scope = architect::resource::Scope::new();
        let engine = QueryEngine::new(feed);
        match architect::serve_local(engine, &scope).establish().await {
            Ok(client) => {
                host.set_vault(documents, client, VaultScope::Person);
                // The service lives as long as its scope.
                std::mem::forget(scope);
            }
            Err(e) => web_sys::console::warn_1(
                &format!("the vault's query service did not start: {e}").into(),
            ),
        }
    });
}

/// The demo's index, built here from `source`, and its query service.
pub fn serve_demo(host: &Host, documents: Arc<dyn Documents>, source: MemorySource) {
    serve(
        host,
        documents,
        Arc::new(Built::new(DemoNotes::new(source))),
    );
}

/// The org's vault `vault`'s index, synced from the node, and its query
/// service: once its first snapshot is in. Taking a snapshot again is
/// the normal way back after a miss (the node started again, this
/// person's access moved, deltas no longer kept, a delta out of step).
/// What fails is the vault's problems.
pub fn serve_org(
    host: &Host,
    documents: Arc<dyn Documents>,
    org: super::org::OrgVaults,
    vault: String,
) {
    let host = host.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let mut at = match org.index_snapshot(&vault).await {
            Ok(got) => got,
            Err(e) => {
                org.report(format!("The vault's index is not here: {e}"));
                return;
            }
        };
        let replica = match Replica::from_bytes(&at.bytes) {
            Ok(r) => Arc::new(r),
            Err(e) => {
                org.report(format!("The vault's index did not read: {e}"));
                return;
            }
        };
        at.bytes = Vec::new();
        serve(&host, documents, Arc::clone(&replica) as Arc<dyn IndexFeed>);
        loop {
            gloo_timers::future::TimeoutFuture::new(POLL_MS).await;
            let next = match org.index_since(&vault, &at).await {
                Ok(Some(delta)) if replica.apply(&delta.bytes).is_ok() => Some(delta),
                // A miss, or a delta out of step: take it whole.
                Ok(_) => match org.index_snapshot(&vault).await {
                    Ok(whole) if replica.reset(&whole.bytes).is_ok() => Some(whole),
                    _ => None,
                },
                // A dropped lane: the next beat asks again.
                Err(_) => None,
            };
            if let Some(mut next) = next {
                next.bytes = Vec::new();
                at = next;
            }
        }
    });
}

/// `window.architectProbeIndex(url, yaml)`, in the demo's probe mode only
/// (`?demo&probe`): fetches a snapshot's bytes from `url` (this origin),
/// reads it into a [`Replica`] and runs `yaml` as a base over it with the
/// one engine, twice: each step's milliseconds and the rows, as JSON. What
/// `apps/web/test/app-index.test.mjs` times a large vault's first load
/// with.
pub fn install_probe() {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::prelude::Closure;
    let Some(window) = web_sys::window() else {
        return;
    };
    let probe = Closure::<dyn Fn(String, String) -> js_sys::Promise>::new(
        move |url: String, yaml: String| {
            wasm_bindgen_futures::future_to_promise(async move {
                let now = || {
                    web_sys::window()
                        .and_then(|w| w.performance())
                        .map_or(0.0, |p| p.now())
                };
                let err = |e: &str| wasm_bindgen::JsValue::from_str(e);
                let window = web_sys::window().ok_or_else(|| err("no window"))?;
                let t0 = now();
                let response: web_sys::Response =
                    wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(&url))
                        .await?
                        .dyn_into()?;
                let buffer = wasm_bindgen_futures::JsFuture::from(response.array_buffer()?).await?;
                let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
                let t1 = now();
                let replica = Replica::from_bytes(&bytes).map_err(|e| err(&e))?;
                let t2 = now();
                let engine = QueryEngine::new(Arc::new(replica));
                let request = vault_proto::query::QueryRequest {
                    source: Some(yaml),
                    ..Default::default()
                };
                let first = engine.run(&request).map_err(|e| err(&e))?;
                let t3 = now();
                let _ = engine.run(&request).map_err(|e| err(&e))?;
                let t4 = now();
                let rows = first.get("total").cloned().unwrap_or_default();
                Ok(wasm_bindgen::JsValue::from_str(
                    &serde_json::json!({
                        "bytes": bytes.len(),
                        "fetch_ms": t1 - t0,
                        "parse_ms": t2 - t1,
                        "first_query_ms": t3 - t2,
                        "second_query_ms": t4 - t3,
                        "total": rows,
                    })
                    .to_string(),
                ))
            })
        },
    );
    let _ = js_sys::Reflect::set(
        &window,
        &"architectProbeIndex".into(),
        probe.as_ref().unchecked_ref(),
    );
    probe.forget();
}
