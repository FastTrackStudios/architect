//! In-process extensions: a crate linked into the app contributes what a
//! manifest can, through one trait (`docs/content/architecture/extensions.md`,
//! "One trait, three ways to run it").
//!
//! ```ignore
//! pub struct Board;
//!
//! impl architect_extensions::linked::Extension for Board {
//!     fn manifest(&self) -> &'static str { include_str!("../extension.toml") }
//!
//!     fn build(&self, ext: &mut Builder) {
//!         ext.view("board", BoardView)
//!             .action_in("LAB_BOARD_PIN", |args, cx| async move { pin(args, cx) })
//!             .contribute(Contribution { tabs: vec![…], ..Default::default() })
//!             .context(Pins::default);
//!     }
//! }
//! ```
//!
//! The same value runs three ways:
//!
//! - **linked in** (feature `linked`): `Extensions::with_linked` registers
//!   it as a built-in (`Origin::Builtin`). It is turned on and off like any
//!   extension, and an installed or development copy of its id overrides
//!   it. Its actions run in the app's process; its views are Dioxus
//!   components mounted in the app's own document
//!   ([`Host::linked_view`](crate::host::Host::linked_view)); its root
//!   context is provided at the app's root.
//! - **as a process** (feature `guest`): its crate's `main` serves it
//!   with `Guest::extension(&Board).serve()`; its manifest's `[entry]`
//!   names that program, and the host runs it like any process extension
//!   (its views drawn remotely).
//! - **as WASM**, when the WASM runner (#167) lands, against the same
//!   trait.
//!
//! What it adds is the manifest's tables and the builder's
//! ([`Builder::contribute`]) together; a builder's actions and views must
//! be the manifest's (a process copy's hello is checked the same way).

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use dioxus::prelude::Element;
use serde_json::Value as Json;

use crate::contrib::{Contribution, StateWrite};

/// A host-service call's answer, to await.
pub type HostFuture =
    Pin<Box<dyn Future<Output = Result<Json, crate::hostcall::HostError>> + Send>>;

/// Where an extension's runtime additions go: the host's registry when it
/// is linked in, the host over the wire when it runs as a process.
pub trait Sink: Send + Sync {
    fn contribute(&self, contribution: Contribution);
    fn set_state(&self, write: StateWrite);
    fn clear_state(&self, key: Option<&str>);

    /// One call to the host service (`crate::hostcall`): the host's own
    /// session when linked in, the connection's when a process.
    fn host(&self, request: crate::hostcall::HostRequest) -> HostFuture {
        let _ = request;
        Box::pin(async {
            Err(crate::hostcall::HostError::Failed {
                message: "no host service here".to_owned(),
            })
        })
    }
}

/// An extension's way to add to the interface at run time, the same
/// linked in or as a process ([`Builder::handle`]).
///
/// Keep a clone (a `OnceLock` of the crate's) and call it from its actions
/// and views. Before the extension runs, it does nothing.
#[derive(Clone, Default)]
pub struct Handle(Arc<std::sync::Mutex<Option<Arc<dyn Sink>>>>);

impl std::fmt::Debug for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Handle")
            .field(&self.sink().is_some())
            .finish()
    }
}

impl Handle {
    fn sink(&self) -> Option<Arc<dyn Sink>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Where it goes from now on (the host's, or the guest's).
    pub fn connect(&self, sink: Arc<dyn Sink>) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(sink);
    }

    /// Asks the host service (`crate::hostcall`): the same call linked in
    /// or as a process.
    ///
    /// It runs an app action, reads the selection or a setting, keeps data,
    /// checked against what the extension may reach. Before the extension
    /// runs, it is refused.
    pub fn host(&self, request: crate::hostcall::HostRequest) -> HostFuture {
        match self.sink() {
            Some(s) => s.host(request),
            None => Box::pin(async {
                Err(crate::hostcall::HostError::Failed {
                    message: "the extension is not running yet".to_owned(),
                })
            }),
        }
    }

    /// What it adds at run time now, in place of what it added before.
    pub fn contribute(&self, contribution: Contribution) {
        if let Some(s) = self.sink() {
            s.contribute(contribution);
        }
    }

    /// One piece of its runtime state (a status item's text).
    pub fn set_state(&self, write: StateWrite) {
        if let Some(s) = self.sink() {
            s.set_state(write);
        }
    }

    /// Its status item `key`'s text, now.
    pub fn status(&self, key: &str, text: impl Into<String>) {
        self.set_state(StateWrite {
            key: key.to_owned(),
            text: text.into(),
            ..StateWrite::default()
        });
    }

    pub fn clear_state(&self, key: Option<&str>) {
        if let Some(s) = self.sink() {
            s.clear_state(key);
        }
    }
}

/// What an in-process action gives back.
pub type ActionFuture = Pin<Box<dyn Future<Output = Result<Json, String>> + Send>>;

/// An action: JSON arguments (`null` for none) and where it was invoked
/// from (`{source, target?, id?, pane?, url?, path?, event?, …}`), JSON
/// out or why it failed.
pub type Action = Arc<dyn Fn(Json, Json) -> ActionFuture + Send + Sync>;

/// Provides a root context value (run in the app's root scope).
pub type Provide = Arc<dyn Fn() + Send + Sync>;

/// An extension written in Rust: linked in, or run as a process, from
/// one value.
pub trait Extension: Send + Sync + 'static {
    /// Its manifest's text (`include_str!("../extension.toml")`): who it
    /// is and what it declares, the same file its process copy ships.
    fn manifest(&self) -> &'static str;

    /// Its package's other files, each `(path, text)`: its themes, its
    /// scripts.
    fn files(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }

    /// What its code adds: its actions, its views, what it adds to the
    /// interface past its manifest's tables, its root context.
    fn build(&self, ext: &mut Builder);
}

/// What an [`Extension`] builds.
#[derive(Clone, Default)]
pub struct Builder {
    pub(crate) actions: BTreeMap<String, Action>,
    pub(crate) views: BTreeMap<String, fn() -> Element>,
    pub(crate) contribution: Contribution,
    pub(crate) context: Vec<Provide>,
    pub(crate) handle: Handle,
}

impl std::fmt::Debug for Builder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Builder")
            .field("actions", &self.actions.keys().collect::<Vec<_>>())
            .field("views", &self.views.keys().collect::<Vec<_>>())
            .field("contribution", &self.contribution)
            .field("context", &self.context.len())
            .field("handle", &self.handle)
            .finish()
    }
}

impl Builder {
    /// What `extension` builds.
    #[must_use]
    pub fn of(extension: &dyn Extension) -> Self {
        let mut b = Self::default();
        extension.build(&mut b);
        b
    }

    /// Its action `id` (one of its manifest's `[[actions]]`): JSON in,
    /// JSON out.
    pub fn action<F, Fut>(&mut self, id: impl Into<String>, run: F) -> &mut Self
    where
        F: Fn(Json) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Json, String>> + Send + 'static,
    {
        self.actions
            .insert(id.into(), Arc::new(move |args, _| Box::pin(run(args))));
        self
    }

    /// Its action `id`, told where it was invoked from.
    pub fn action_in<F, Fut>(&mut self, id: impl Into<String>, run: F) -> &mut Self
    where
        F: Fn(Json, Json) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Json, String>> + Send + 'static,
    {
        self.actions
            .insert(id.into(), Arc::new(move |args, cx| Box::pin(run(args, cx))));
        self
    }

    /// Its view `id` (one of its manifest's `[[views]]`), drawn by `root`.
    pub fn view(&mut self, id: impl Into<String>, root: fn() -> Element) -> &mut Self {
        self.views.insert(id.into(), root);
        self
    }

    /// What it adds to the interface past its manifest's tables (an id in
    /// both is the builder's).
    // By value, so a literal reads as `contribute(Contribution { … })`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn contribute(&mut self, contribution: Contribution) -> &mut Self {
        self.contribution = self.contribution.merged(&contribution);
        self
    }

    /// A value its views read with `use_context`, provided at the app's
    /// root when it is linked in. It stays until the app ends.
    pub fn context<T: Clone + 'static>(&mut self, make: fn() -> T) -> &mut Self {
        self.context.push(Arc::new(move || {
            dioxus::prelude::provide_context(make());
        }));
        self
    }

    /// Its way to add to the interface at run time: keep a clone.
    #[must_use]
    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }

    /// Its actions' ids.
    #[must_use]
    pub fn action_ids(&self) -> Vec<String> {
        self.actions.keys().cloned().collect()
    }

    /// Its views' ids.
    #[must_use]
    pub fn view_ids(&self) -> Vec<String> {
        self.views.keys().cloned().collect()
    }

    /// What it adds past its manifest.
    #[must_use]
    pub const fn contribution(&self) -> &Contribution {
        &self.contribution
    }

    /// Its view `id`'s component.
    #[must_use]
    pub fn view_root(&self, id: &str) -> Option<fn() -> Element> {
        self.views.get(id).copied()
    }

    /// Its action `id`.
    #[must_use]
    pub fn action_fn(&self, id: &str) -> Option<Action> {
        self.actions.get(id).cloned()
    }

    /// Provides its root context in the scope this is called in: call it
    /// in the app's root component, so every view below reads it.
    pub fn provide_context(&self) {
        for provide in &self.context {
            provide();
        }
    }
}

#[cfg(feature = "guest")]
impl crate::guest::Guest {
    /// `extension` served as a process: its views, its actions (told
    /// where they were invoked from) and what it adds past its manifest,
    /// sent as the host connects.
    #[must_use]
    pub fn extension(extension: &dyn Extension) -> Self {
        struct Wire;
        impl Sink for Wire {
            fn contribute(&self, contribution: Contribution) {
                crate::guest::contribute(&contribution);
            }
            fn set_state(&self, write: StateWrite) {
                crate::guest::set_state(&write);
            }
            fn clear_state(&self, key: Option<&str>) {
                crate::guest::clear_state(key);
            }
            fn host(&self, request: crate::hostcall::HostRequest) -> HostFuture {
                Box::pin(crate::guest::host(request))
            }
        }
        let built = Builder::of(extension);
        built.handle.connect(Arc::new(Wire));
        let mut guest = Self::new();
        for (id, root) in &built.views {
            guest = guest.view(id.clone(), *root);
        }
        for (id, run) in built.actions {
            guest = guest.action_in(id, move |args, cx| run(args, cx));
        }
        if !built.contribution.is_empty() {
            guest = guest.contribution(&built.contribution);
        }
        guest
    }
}
