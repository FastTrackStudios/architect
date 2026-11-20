//! A linked extension whose code arrives when it is first used.
//!
//! A distribution that links many extensions into one browser build pays
//! for all of their code on the first visit, though a page uses a few. A
//! [`Defer`] extension is linked by its manifest alone: its views, tabs,
//! favourites, claims and actions are known from the start (the shell
//! lists and opens them like any other), and its code, its
//! [`Extension::build`], runs the first time one of its views is drawn or
//! one of its actions is run. A distribution makes that first call fetch
//! a wasm chunk of the extension's own (Dioxus's `#[wasm_split]` on
//! [`Defer::load`]), so a browser downloads an extension's code when a
//! page first opens one of its views, and nothing of it before; natively
//! [`Defer::load`] builds it at once.
//!
//! ```ignore
//! #[derive(Default)]
//! pub struct Mixer;
//!
//! impl Defer for Mixer {
//!     fn manifest() -> &'static str {
//!         studio_mixer::Mixer.manifest()
//!     }
//!     fn load() -> Loading {
//!         Box::pin(mixer())
//!     }
//! }
//!
//! #[wasm_split::wasm_split(mixer)]
//! async fn mixer() -> Builder {
//!     Builder::of(&studio_mixer::Mixer)
//! }
//!
//! // In the distribution:
//! bundle.member(&Deferred::<Mixer>::NEW)
//! ```
//!
//! Until it is built, what its code would add past its manifest's tables
//! ([`Builder::contribute`]) is not there, and its root context
//! ([`Builder::context`]) is provided at the app's root when it is built,
//! before its first view is drawn.

use std::collections::BTreeMap;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use dioxus::prelude::*;
use serde_json::Value as Json;

use super::{Builder, Extension, Handle};

/// What [`Defer::load`] gives back: the extension, built.
#[cfg(not(target_family = "wasm"))]
pub type Loading = Pin<Box<dyn Future<Output = Builder> + Send>>;
/// What [`Defer::load`] gives back: the extension, built.
#[cfg(target_family = "wasm")]
pub type Loading = Pin<Box<dyn Future<Output = Builder>>>;

/// An extension linked by its manifest, its code built on first use.
pub trait Defer: Send + Sync + 'static {
    /// Its manifest's text: what the app knows of it before it is built.
    fn manifest() -> &'static str;

    /// Its package's other files (its themes, its scripts).
    #[must_use]
    fn files() -> &'static [(&'static str, &'static str)] {
        &[]
    }

    /// The extension, built ([`Builder::of`]): in a browser, the call
    /// that fetches its chunk.
    fn load() -> Loading;
}

/// [`Defer`] `D` as a linked [`Extension`]: link `&Deferred::<D>::NEW`.
pub struct Deferred<D>(PhantomData<fn() -> D>);

impl<D: Defer> Deferred<D> {
    pub const NEW: Self = Self(PhantomData);
}

/// The most views a deferred extension may declare.
const MAX_VIEWS: usize = 24;

impl<D: Defer> Extension for Deferred<D> {
    fn manifest(&self) -> &'static str {
        D::manifest()
    }

    fn files(&self) -> &'static [(&'static str, &'static str)] {
        D::files()
    }

    fn build(&self, ext: &mut Builder) {
        let Ok(manifest) = crate::Manifest::parse(D::manifest()) else {
            return;
        };
        // Its own handle, kept: the host connects it as it links this, and
        // the built extension's is pointed where it goes.
        state(&manifest.id).set_handle(ext.handle());
        for (index, view) in manifest.views.iter().enumerate().take(MAX_VIEWS) {
            if let Some(root) = view_root::<D>(index) {
                ext.view(view.id.clone(), root);
            }
        }
        for action in &manifest.actions {
            let id = action.id.clone();
            let extension = manifest.id.clone();
            ext.action_in(action.id.clone(), move |args: Json, cx: Json| {
                let id = id.clone();
                let extension = extension.clone();
                async move {
                    let built = load::<D>(&extension).await;
                    match built.action_fn(&id) {
                        Some(run) => run(args, cx).await,
                        None => Err(format!("{extension} has no action {id}")),
                    }
                }
            });
        }
    }
}

/// One deferred extension's state: its handle, and itself once built.
#[derive(Default)]
struct State {
    handle: Mutex<Option<Handle>>,
    built: Mutex<Option<Arc<Builder>>>,
}

impl State {
    fn set_handle(&self, handle: Handle) {
        *self.handle.lock().unwrap_or_else(PoisonError::into_inner) = Some(handle);
    }

    fn built(&self) -> Option<Arc<Builder>> {
        self.built
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

static STATES: Mutex<BTreeMap<String, Arc<State>>> = Mutex::new(BTreeMap::new());

fn state(id: &str) -> Arc<State> {
    let mut states = STATES.lock().unwrap_or_else(PoisonError::into_inner);
    Arc::clone(states.entry(id.to_owned()).or_default())
}

fn id_of<D: Defer>() -> Option<String> {
    crate::Manifest::parse(D::manifest()).ok().map(|m| m.id)
}

/// `D`, built: once, the first time anything of it is used. Its handle
/// goes where the linked one's goes, and what its code contributes is
/// contributed then.
async fn load<D: Defer>(id: &str) -> Arc<Builder> {
    let state = state(id);
    if let Some(built) = state.built() {
        return built;
    }
    let built = D::load().await;
    let mut slot = state.built.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(earlier) = slot.as_ref() {
        // Two first uses at once: the one built first stays.
        return Arc::clone(earlier);
    }
    if let Some(linked) = state
        .handle
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        && let Some(sink) = linked.sink()
    {
        if !built.contribution.is_empty() {
            sink.contribute(built.contribution.clone());
        }
        built.handle.connect(sink);
    }
    let built = Arc::new(built);
    *slot = Some(Arc::clone(&built));
    built
}

/// The component for view number `index` of `D`'s manifest.
fn view_root<D: Defer>(index: usize) -> Option<fn() -> Element> {
    macro_rules! roots {
        ($($n:literal)*) => {
            match index {
                $($n => {
                    let root: fn() -> Element = deferred_view::<D, $n>;
                    Some(root)
                })*
                _ => None,
            }
        };
    }
    roots!(0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23)
}

/// View number `V` of `D`: nothing while `D` is built (its chunk
/// fetched), then the built view, its root context provided first.
fn deferred_view<D: Defer, const V: usize>() -> Element {
    let built = use_resource(|| async {
        let id = id_of::<D>()?;
        let built = load::<D>(&id).await;
        provide_once(&id, &built);
        let view = crate::Manifest::parse(D::manifest())
            .ok()?
            .views
            .get(V)
            .map(|v| v.id.clone())?;
        built.view_root(&view)
    });
    let root = *built.read();
    match root {
        Some(Some(root)) => rsx! { Built { root: Root(root) } },
        _ => rsx! {},
    }
}

/// A built view's component.
#[derive(Clone, Copy)]
struct Root(fn() -> Element);

impl PartialEq for Root {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::fn_addr_eq(self.0, other.0)
    }
}

/// A built view's component, drawn as one of its own.
#[component]
fn Built(root: Root) -> Element {
    (root.0)()
}

/// Provides `built`'s root context at the app's root, once.
fn provide_once(id: &str, built: &Builder) {
    static PROVIDED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    {
        let mut provided = PROVIDED.lock().unwrap_or_else(PoisonError::into_inner);
        if provided.iter().any(|p| p == id) {
            return;
        }
        provided.push(id.to_owned());
    }
    dioxus::core::Runtime::current().in_scope(ScopeId::ROOT, || built.provide_context());
}
