//! The host service (#178): what an extension may ask the app for, each
//! call checked against what it may reach.
//!
//! [`Host::api`] is one extension's session: [`ExtensionApi::call`] takes
//! a [`HostRequest`] (`crate::hostcall`) and answers it — runs an app
//! action, reads the current organization or selection, opens and reads
//! documents and files, reads and writes its own settings and data, logs
//! a line — after checking it against the extension's capabilities as a
//! person granted them ([`Host::capabilities`]: its manifest's, narrowed
//! by `crate::consent` for an installed one). Every way an extension runs
//! comes here:
//!
//! - a **process** over its connection: the host serves
//!   `host_proto::ExtensionHost` on it, the guest calls `guest::host`;
//! - a **Luau** script through its `host` global (`host.run`,
//!   `host.selection`, `host.setting`, …), as whoever ran it;
//! - an **in-process** extension through its `linked::Handle::host`;
//! - the **WASM** runner (#167), when it lands, with one imported
//!   function taking and giving the same JSON.
//!
//! An action it runs is run as [`Principal::Extension`] — its id, and who
//! it acts for (the person or agent whose run it is part of; `None` for
//! the app itself) — so the list's authorizer, or a server's gate, can
//! tell its call from the person's. Each call is a span (`ext.host_call`)
//! carrying `perm.*` (the capability asked, the decision, the reason and
//! the principal), set with `architect_telemetry::wide::set` as well; a
//! refused or failed call is a line in the extension's log (`EXT_LOGS`).
//!
//! What the app knows that the host does not — the organization, the
//! selection, its documents — comes from an [`AppContext`] it gives
//! ([`Host::set_app_context`]), and the documents an extension lists,
//! creates, edits, renames, moves, deletes and watches from its
//! [`Documents`] ([`Host::set_documents`]; `crate::documents`); settings' values from its
//! [`SettingValues`] ([`Host::set_setting_values`]; in memory unless
//! given); data and files under [`Host::with_data_root`] (`<root>/<id>/`);
//! the interface it may use (notifications, the clipboard, pickers,
//! prompts, menus: `crate::hostui`) through its [`HostUi`]
//! ([`Host::set_ui`]; [`crate::hostui::NoUi`] unless given), the
//! decorations extensions set on rows and tabs in [`Host::decorations`],
//! the network (`fetch`, `crate::net`) with the secrets the host keeps
//! for extensions in its [`Secrets`] ([`Host::set_secrets`]; in memory
//! unless given), and MIDI ports, OSC endpoints and control surfaces
//! (`crate::ports`; [`Host::with_ports`], [`Host::with_targets`]).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use architect::action::{ActionCaller, ActionError, Invocation, InvocationSource, Principal};
use serde_json::Value;
use tracing::Instrument as _;

use crate::catalog::Origin;
use crate::consent::Grants;
use crate::documents::{DocInfo, Documents};
use crate::events::DocEvent;
use crate::host::{Host, Opening, WeakHost};
use crate::hostcall::{HostError, HostRequest, PickKind, Target};
use crate::hostui::{Asker, Decorated, Decorations, HostUi, NoUi, Notification};
use crate::logs::{Level, Logs};
use crate::manifest::{Capabilities, Manifest, Reach};
use crate::net::Secrets;
use architect_scripting::luau::Gate;

/// What the app knows that an extension may ask for (each only with the
/// capability for it): every method has a default that knows nothing.
pub trait AppContext: Send + Sync {
    /// The organization working in now (its id and name, as JSON).
    fn org(&self) -> Option<Value> {
        None
    }

    /// What is selected and focused now (the window, pane, view, the
    /// selected text or items).
    fn selection(&self) -> Option<Value> {
        None
    }

    /// Document `path`'s text, when it is one of the app's: `None` when it
    /// is not one it knows.
    fn document(&self, path: &str) -> Option<Result<String, String>> {
        let _ = path;
        None
    }
}

/// Where settings' values are read and written: the app's own store
/// (what its settings page shows), or [`MemoryValues`].
pub trait SettingValues: Send + Sync {
    /// The value set for `key`, when one is (its default otherwise).
    fn get(&self, key: &str) -> Option<Value>;

    /// Sets `key` to `value`.
    ///
    /// # Errors
    ///
    /// The store refused it.
    fn set(&self, key: &str, value: Value) -> Result<(), String>;
}

/// Settings' values in memory, for the run.
#[derive(Debug, Default)]
pub struct MemoryValues(Mutex<BTreeMap<String, Value>>);

impl SettingValues for MemoryValues {
    fn get(&self, key: &str) -> Option<Value> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    fn set(&self, key: &str, value: Value) -> Result<(), String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
        Ok(())
    }
}

/// One `perm.*` field on the wide event (`architect_telemetry::wide::set`,
/// feature `wide-events`); the span's own fields carry it regardless.
#[cfg(feature = "wide-events")]
fn wide(key: &'static str, value: impl Into<String>) {
    architect_telemetry::wide::set(key, value.into());
}

#[cfg(not(feature = "wide-events"))]
fn wide(_key: &'static str, _value: impl Into<String>) {}

/// The host's side of the service: what it was given.
pub(crate) struct Services {
    grants: Mutex<Option<Grants>>,
    logs: Logs,
    app: Mutex<Option<Arc<dyn AppContext>>>,
    /// The app's documents, for the document calls.
    documents: Mutex<Option<Arc<dyn Documents>>>,
    /// The app's editors, for the editor calls (`crate::editor`).
    editors: Mutex<Option<Arc<dyn crate::editor::Editors>>>,
    values: Mutex<Arc<dyn SettingValues>>,
    data_root: Mutex<Option<PathBuf>>,
    /// The app's own list, when its actions are not on the host's.
    app_list: Mutex<Option<architect::action::ActionList>>,
    /// Data kept in memory, when there is no data root.
    data: Mutex<BTreeMap<String, BTreeMap<String, Value>>>,
    /// The app's interface, for extensions (none given: [`NoUi`]).
    ui: Mutex<Option<Arc<dyn HostUi>>>,
    /// The window's own interface, kept for whoever draws it
    /// (`host_ui_view::WindowUi::lend`).
    #[cfg(feature = "ui")]
    window: Mutex<Option<Arc<dyn std::any::Any + Send + Sync>>>,
    /// What extensions set on rows and tabs.
    decorations: Decorations,
    /// The paths a person picked for each extension, for the run: each
    /// with whether it may write it (a save) and whether it is a folder.
    picked: Mutex<BTreeMap<String, Vec<Picked>>>,
    /// The secrets it keeps for extensions (`crate::net`).
    secrets: Mutex<Secrets>,
    /// The streamed responses extensions have open.
    #[cfg(feature = "net")]
    net: crate::net::Net,
    /// The ports extensions have open, and the control surfaces running
    /// (`crate::ports`).
    ports: crate::ports::PortsState,
}

/// A path a person picked for an extension.
#[derive(Clone, Debug)]
struct Picked {
    path: PathBuf,
    write: bool,
    folder: bool,
}

impl Services {
    /// What extension `id` was lent goes with it (it unloaded).
    pub(crate) fn forget(&self, id: &str) {
        self.decorations.withdraw(id);
        let editors = lock(&self.editors).clone();
        if let Some(editors) = editors {
            editors.withdraw(id);
        }
        lock(&self.picked).remove(id);
        #[cfg(feature = "net")]
        self.net.forget(id);
        self.ports.forget(id);
    }

    pub(crate) const fn ports(&self) -> &crate::ports::PortsState {
        &self.ports
    }
}

impl Default for Services {
    fn default() -> Self {
        Self {
            grants: Mutex::new(None),
            logs: Logs::default(),
            app: Mutex::new(None),
            documents: Mutex::new(None),
            editors: Mutex::new(None),
            values: Mutex::new(Arc::new(MemoryValues::default())),
            data_root: Mutex::new(None),
            app_list: Mutex::new(None),
            data: Mutex::new(BTreeMap::new()),
            ui: Mutex::new(None),
            #[cfg(feature = "ui")]
            window: Mutex::new(None),
            decorations: Decorations::default(),
            picked: Mutex::new(BTreeMap::new()),
            secrets: Mutex::new(Secrets::memory()),
            #[cfg(feature = "net")]
            net: crate::net::Net::default(),
            ports: crate::ports::PortsState::default(),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

const fn reach(effect: architect::action::Effect) -> Reach {
    match effect {
        architect::action::Effect::ReadOnly => Reach::ReadOnly,
        architect::action::Effect::Mutating => Reach::Mutating,
        architect::action::Effect::Destructive => Reach::Destructive,
    }
}

impl Host {
    /// Keeps consent in `grants`: an installed extension runs with what a
    /// person granted it (`crate::consent`). With none, every extension
    /// runs with what its manifest asks (an app that asks no one).
    #[must_use]
    pub fn with_grants(self, grants: Grants) -> Self {
        self.set_grants(Some(grants));
        self
    }

    /// [`Self::with_grants`] on a host already shared.
    pub fn set_grants(&self, grants: Option<Grants>) {
        *lock(&self.services().grants) = grants;
    }

    /// Where consent is kept, when the app asks for it.
    #[must_use]
    pub fn grants(&self) -> Option<Grants> {
        lock(&self.services().grants).clone()
    }

    /// Every extension's recent log lines (`EXT_LOGS`).
    #[must_use]
    pub fn logs(&self) -> &Logs {
        &self.services().logs
    }

    /// What the app tells extensions that may know it.
    #[must_use]
    pub fn with_app_context(self, context: impl AppContext + 'static) -> Self {
        self.set_app_context(Arc::new(context));
        self
    }

    /// [`Self::with_app_context`] on a host already shared.
    pub fn set_app_context(&self, context: Arc<dyn AppContext>) {
        *lock(&self.services().app) = Some(context);
    }

    /// The app's documents, for extensions that may read and change them
    /// (`crate::documents`): Task's vault, the lab's spaces.
    #[must_use]
    pub fn with_documents(self, documents: impl Documents + 'static) -> Self {
        self.set_documents(Arc::new(documents));
        self
    }

    /// [`Self::with_documents`] on a host already shared.
    pub fn set_documents(&self, documents: Arc<dyn Documents>) {
        *lock(&self.services().documents) = Some(documents);
    }

    /// The app's documents, when it gave them.
    #[must_use]
    pub fn documents(&self) -> Option<Arc<dyn Documents>> {
        lock(&self.services().documents).clone()
    }

    /// The app's editors, for extensions that may read and change the one
    /// being typed in (`crate::editor`): the lab's notes, Task's vault.
    #[must_use]
    pub fn with_editors(self, editors: impl crate::editor::Editors + 'static) -> Self {
        self.set_editors(Arc::new(editors));
        self
    }

    /// [`Self::with_editors`] on a host already shared.
    pub fn set_editors(&self, editors: Arc<dyn crate::editor::Editors>) {
        *lock(&self.services().editors) = Some(editors);
    }

    /// The app's editors, when it gave them.
    #[must_use]
    pub fn editors(&self) -> Option<Arc<dyn crate::editor::Editors>> {
        lock(&self.services().editors).clone()
    }

    /// Reads and writes settings' values in `values` (the app's store).
    #[must_use]
    pub fn with_setting_values(self, values: impl SettingValues + 'static) -> Self {
        self.set_setting_values(Arc::new(values));
        self
    }

    /// [`Self::with_setting_values`] on a host already shared.
    pub fn set_setting_values(&self, values: Arc<dyn SettingValues>) {
        *lock(&self.services().values) = values;
    }

    /// The app's Action List, when it is not the host's (an app that gives
    /// extensions a list of their own): what `run` runs the app's actions
    /// on. An extension's own actions run where they are.
    pub fn set_app_list(&self, list: architect::action::ActionList) {
        *lock(&self.services().app_list) = Some(list);
        self.show_background();
    }

    /// The app's own list, when it gave one ([`Self::set_app_list`]).
    pub(crate) fn app_list(&self) -> Option<architect::action::ActionList> {
        lock(&self.services().app_list).clone()
    }

    /// The list action `id` is on: the host's, or the app's.
    fn list_of(&self, id: &str) -> architect::action::ActionList {
        if self.list().info(id).is_some() {
            return self.list().clone();
        }
        lock(&self.services().app_list)
            .clone()
            .unwrap_or_else(|| self.list().clone())
    }

    /// Answers extensions' notifications, clipboard, pickers, prompts and
    /// menus with `ui` (the app's window's, or a `FakeUi`).
    #[must_use]
    pub fn with_ui(self, ui: impl HostUi + 'static) -> Self {
        self.set_ui(Arc::new(ui));
        self
    }

    /// [`Self::with_ui`] on a host already shared.
    pub fn set_ui(&self, ui: Arc<dyn HostUi>) {
        *lock(&self.services().ui) = Some(ui);
    }

    /// Gives `ui` unless the app gave one already (what the window's
    /// overlays do as they mount): whether it took it.
    pub fn offer_ui(&self, ui: Arc<dyn HostUi>) -> bool {
        let mut now = lock(&self.services().ui);
        if now.is_some() {
            return false;
        }
        *now = Some(ui);
        true
    }

    /// The window's interface, kept with the host for whoever draws it,
    /// made by `make` the first time it is asked for.
    #[cfg(feature = "ui")]
    pub(crate) fn window_slot<T: Clone + Send + Sync + 'static>(
        &self,
        make: impl FnOnce() -> T,
    ) -> T {
        let mut slot = lock(&self.services().window);
        if let Some(kept) = slot.as_ref().and_then(|k| k.downcast_ref::<T>()) {
            return kept.clone();
        }
        let fresh = make();
        *slot = Some(Arc::new(fresh.clone()));
        fresh
    }

    /// The interface extensions are lent.
    #[must_use]
    pub fn ui(&self) -> Arc<dyn HostUi> {
        lock(&self.services().ui)
            .clone()
            .unwrap_or_else(|| Arc::new(NoUi))
    }

    /// What extensions set on rows and tabs at run time (`decorate`).
    #[must_use]
    pub fn decorations(&self) -> &Decorations {
        &self.services().decorations
    }

    /// Every decoration on `target`: the ones manifests contribute
    /// (`app.decorations`) first, then the ones set at run time.
    #[must_use]
    pub fn decorations_of(&self, target: &Target) -> Vec<Decorated> {
        let mut out: Vec<Decorated> = self
            .points()
            .items(crate::hostui::DECORATIONS_POINT)
            .into_iter()
            .filter_map(|item| {
                let on: Target = serde_json::from_value(item.value.get("target")?.clone()).ok()?;
                (on == *target).then(|| Decorated {
                    from: item.from,
                    decoration: serde_json::from_value(item.value).unwrap_or_default(),
                })
            })
            .collect();
        out.extend(self.decorations().of(target));
        out
    }

    /// Keeps the secrets extensions' requests name in `secrets`
    /// (`crate::net`): a private file of the app's, never an extension's
    /// folder. Unless given, they are kept in memory for the run.
    #[must_use]
    pub fn with_secrets(self, secrets: Secrets) -> Self {
        self.set_secrets(secrets);
        self
    }

    /// [`Self::with_secrets`] on a host already shared.
    pub fn set_secrets(&self, secrets: Secrets) {
        *lock(&self.services().secrets) = secrets;
    }

    /// The secrets it keeps for extensions: which are set, never a value
    /// to an extension.
    #[must_use]
    pub fn secrets(&self) -> Secrets {
        lock(&self.services().secrets).clone()
    }

    /// Keeps each extension's data and files under `<root>/<id>/`.
    #[must_use]
    pub fn with_data_root(self, root: impl Into<PathBuf>) -> Self {
        *lock(&self.services().data_root) = Some(root.into());
        self
    }

    /// What `manifest`'s extension, loaded as `origin`, may reach now: what
    /// it asks, narrowed for an installed one to what a person granted.
    #[must_use]
    pub fn capabilities_of(&self, manifest: &Manifest, origin: Origin) -> Capabilities {
        match (origin, self.grants()) {
            (Origin::Installed, Some(grants)) => manifest
                .capabilities
                .within(&grants.granted(&manifest.id).unwrap_or_default()),
            _ => manifest.capabilities.clone(),
        }
    }

    /// What loaded extension `id` may reach now.
    #[must_use]
    pub fn capabilities(&self, id: &str) -> Option<Capabilities> {
        let loaded = self.get(id)?;
        Some(self.capabilities_of(&loaded.manifest, loaded.origin))
    }

    /// Extension `id`'s session with the host service, acting for the app
    /// itself ([`ExtensionApi::acting_for`] to say for whom).
    #[must_use]
    pub fn api(&self, id: &str) -> ExtensionApi {
        ExtensionApi {
            id: id.to_owned(),
            host: self.downgrade(),
            on_behalf_of: None,
        }
    }

    /// The gate a load runs `manifest`'s actions, hooks and scripts
    /// through: what it may reach at the moment it asks, so a grant taken
    /// back holds at once.
    pub(crate) fn live_gate(&self, manifest: &Manifest, origin: Origin) -> Gate {
        let host = self.downgrade();
        let manifest = manifest.clone();
        Arc::new(move |info: &architect::action::ActionInfo| {
            let Some(host) = host.upgrade() else {
                return Err("the app is closing".to_owned());
            };
            if info.id == crate::host::SERVICE_CALL || info.id == crate::host::PAGE_CALL {
                return Ok(());
            }
            host.capabilities_of(&manifest, origin)
                .may_run_app(&manifest.action_prefix(), &info.id, reach(info.effect))
                .map_err(|(_, why)| why)
        })
    }
}

/// One extension's session with the host service. Cheap to clone; holds
/// the host weakly (an extension does not keep the app alive).
#[derive(Clone)]
pub struct ExtensionApi {
    id: String,
    host: WeakHost,
    on_behalf_of: Option<Principal>,
}

impl std::fmt::Debug for ExtensionApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtensionApi")
            .field("id", &self.id)
            .field("on_behalf_of", &self.on_behalf_of)
            .finish_non_exhaustive()
    }
}

/// A capability check's answer: on, or the capability and why not.
type Check = Result<(), (String, String)>;

impl ExtensionApi {
    /// The extension's id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// This session, acting for `who` (the caller whose run this is part
    /// of; `None`: the app itself).
    #[must_use]
    pub fn acting_for(mut self, who: Option<Principal>) -> Self {
        // An extension acting for an extension acts for whoever that one
        // acted for.
        self.on_behalf_of = who.and_then(|w| match w {
            Principal::Extension { on_behalf_of, .. } => on_behalf_of.map(|b| *b),
            other => Some(other),
        });
        self
    }

    /// Who its calls are made as.
    #[must_use]
    pub fn principal(&self) -> Principal {
        Principal::extension(self.id.clone(), self.on_behalf_of.clone())
    }

    /// Answers `request`, checked against what the extension may reach.
    ///
    /// # Errors
    ///
    /// [`HostError::Refused`] outside its capabilities (or not granted),
    /// [`HostError::NotLoaded`] once it is unloaded, and what the call
    /// itself met.
    pub async fn call(&self, request: HostRequest) -> Result<Value, HostError> {
        let principal = self.principal();
        let span = tracing::info_span!(
            "ext.host_call",
            ext.id = %self.id,
            ext.op = request.op(),
            perm.principal = %principal.describe(),
            perm.capability = tracing::field::Empty,
            perm.decision = tracing::field::Empty,
            perm.reason = tracing::field::Empty,
            ext.outcome = tracing::field::Empty,
            http.request.method = tracing::field::Empty,
            server.address = tracing::field::Empty,
            http.response.status_code = tracing::field::Empty,
            ext.redirects = tracing::field::Empty,
            ext.bytes = tracing::field::Empty,
            ext.secrets = tracing::field::Empty,
        );
        let op = request.op();
        let answer = self.answer(request).instrument(span.clone()).await;
        let _entered = span.enter();
        let outcome = match &answer {
            Ok(_) => "ok",
            Err(HostError::Refused { .. }) => "refused",
            Err(HostError::ConfirmationRequired { .. }) => "confirmation_required",
            Err(_) => "failed",
        };
        span.record("ext.outcome", outcome);
        wide("ext.outcome", outcome);
        if let (Err(e), Some(host)) = (&answer, self.host.upgrade()) {
            let level = if matches!(e, HostError::Refused { .. }) {
                Level::Warn
            } else {
                Level::Error
            };
            host.logs()
                .push(&self.id, level, "host", format!("{op}: {e}"));
        }
        answer
    }

    /// [`Self::call`] from and to JSON text (what crosses a wire).
    /// The time on its host's clock (`Host::clock`), ms since the epoch:
    /// the system's once the host is gone.
    #[must_use]
    pub fn now_ms(&self) -> u64 {
        self.host
            .upgrade()
            .map_or_else(crate::host::now_ms, |host| host.clock().now_ms())
    }

    pub async fn call_json(&self, request: &str) -> String {
        let answer = match serde_json::from_str::<HostRequest>(request) {
            Ok(request) => self.call(request).await,
            Err(e) => Err(HostError::Failed {
                message: format!("not a host request: {e}"),
            }),
        };
        crate::hostcall::HostAnswer::from(answer).to_json()
    }

    fn asker(manifest: &Manifest) -> Asker {
        Asker {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
        }
    }

    /// Whether a person picked `file` for it: to read (any pick), or to
    /// write (a save, or inside a folder picked).
    fn was_picked(&self, host: &Host, file: &Path, write: bool) -> bool {
        lock(&host.services().picked)
            .get(&self.id)
            .is_some_and(|ps| {
                ps.iter().any(|p| {
                    (p.path == file || (p.folder && file.starts_with(&p.path)))
                        && (p.write || !write)
                })
            })
    }

    /// Records the permission decision on the call's span.
    fn decide(capability: &str, check: Check) -> Result<(), HostError> {
        let span = tracing::Span::current();
        span.record("perm.capability", capability);
        wide("perm.capability", capability.to_owned());
        match check {
            Ok(()) => {
                span.record("perm.decision", "allow");
                wide("perm.decision", "allow");
                Ok(())
            }
            Err((capability, why)) => {
                span.record("perm.capability", capability.as_str());
                span.record("perm.decision", "deny");
                span.record("perm.reason", why.as_str());
                wide("perm.capability", capability.clone());
                wide("perm.decision", "deny");
                wide("perm.reason", why.clone());
                Err(HostError::Refused { capability, why })
            }
        }
    }

    /// [`crate::hostcall::check`], recorded on the call's span.
    fn checked(
        manifest: &Manifest,
        may: &Capabilities,
        request: &HostRequest,
    ) -> Result<(), HostError> {
        let (capability, check) = crate::hostcall::check(manifest, may, request, Reach::ReadOnly);
        Self::decide(capability, check)
    }

    async fn answer(&self, request: HostRequest) -> Result<Value, HostError> {
        let host = self.host.upgrade().ok_or_else(|| HostError::NotLoaded {
            id: self.id.clone(),
        })?;
        let loaded = host.get(&self.id).ok_or_else(|| HostError::NotLoaded {
            id: self.id.clone(),
        })?;
        let may = host.capabilities_of(&loaded.manifest, loaded.origin);
        let manifest = &loaded.manifest;
        if !matches!(request, HostRequest::Run { .. }) {
            Self::checked(manifest, &may, &request)?;
        }
        match request {
            HostRequest::Run {
                ref action,
                ref args,
                ref ticket,
            } => {
                self.run(
                    &host,
                    manifest,
                    &may,
                    &request,
                    action,
                    args,
                    ticket.clone(),
                )
                .await
            }
            HostRequest::Org => Ok(host
                .app_context()
                .and_then(|c| c.org())
                .unwrap_or(Value::Null)),
            HostRequest::Selection => Ok(host
                .app_context()
                .and_then(|c| c.selection())
                .unwrap_or(Value::Null)),
            HostRequest::Open { path } => host
                .open(
                    Opening::Path(path),
                    Invocation::new(InvocationSource::Script),
                )
                .await
                .map_err(|message| HostError::Failed { message }),
            HostRequest::ReadDocument { path } => {
                if let Some(docs) = host.documents() {
                    match docs.read(&path) {
                        Ok(text) => return Ok(Value::String(text)),
                        Err(crate::documents::DocError::NotFound(_)) => {}
                        Err(e) => return Err(e.into()),
                    }
                }
                match host.app_context().and_then(|c| c.document(&path)) {
                    Some(Ok(text)) => Ok(Value::String(text)),
                    Some(Err(message)) => Err(HostError::Failed { message }),
                    None => Err(HostError::NotFound {
                        what: format!("document {path}"),
                    }),
                }
            }
            HostRequest::ListDocuments { .. }
            | HostRequest::CreateDocument { .. }
            | HostRequest::EditDocument { .. }
            | HostRequest::RenameDocument { .. }
            | HostRequest::MoveDocument { .. }
            | HostRequest::DeleteDocument { .. }
            | HostRequest::WatchDocuments { .. } => self.document(&host, manifest, request),
            HostRequest::EditorState
            | HostRequest::EditEditor { .. }
            | HostRequest::DecorateEditor { .. } => self.editor(&host, manifest, request),
            HostRequest::ReadFile { path } => {
                let file = self.file(&host, &loaded, &may, &path, false)?;
                std::fs::read_to_string(&file)
                    .map(Value::String)
                    .map_err(|e| match e.kind() {
                        std::io::ErrorKind::NotFound => HostError::NotFound {
                            what: format!("file {path}"),
                        },
                        _ => HostError::Failed {
                            message: format!("{}: {e}", file.display()),
                        },
                    })
            }
            HostRequest::WriteFile { path, text } => {
                let file = self.file(&host, &loaded, &may, &path, true)?;
                if let Some(dir) = file.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| HostError::Failed {
                        message: format!("{}: {e}", dir.display()),
                    })?;
                }
                std::fs::write(&file, text).map_err(|e| HostError::Failed {
                    message: format!("{}: {e}", file.display()),
                })?;
                Ok(Value::String(file.display().to_string()))
            }
            HostRequest::Setting { key } => {
                let set = lock(&host.services().values).clone().get(&key);
                Ok(set
                    .or_else(|| default_of(manifest, &key))
                    .unwrap_or(Value::Null))
            }
            HostRequest::SetSetting { key, value } => {
                if default_of(manifest, &key).is_none() {
                    return Err(HostError::NotFound {
                        what: format!("setting {key} in its manifest"),
                    });
                }
                let values = lock(&host.services().values).clone();
                values
                    .set(&key, value)
                    .map_err(|message| HostError::Failed { message })?;
                host.list().changed("settings", &key);
                Ok(Value::Null)
            }
            HostRequest::Data { key } => {
                let _held = self.data_lock(&host);
                Ok(self.data(&host)?.remove(&key).unwrap_or(Value::Null))
            }
            HostRequest::SetData { key, value } => {
                // Read, change and write as one: two hooks setting keys at
                // once both keep theirs.
                let _held = self.data_lock(&host);
                let mut data = self.data(&host)?;
                match value {
                    Some(v) => {
                        data.insert(key, v);
                    }
                    None => {
                        data.remove(&key);
                    }
                }
                self.keep_data(&host, data)?;
                Ok(Value::Null)
            }
            HostRequest::Service {
                service,
                method,
                args,
            } => {
                self.service(&host, manifest, &may, &service, &method, &args)
                    .await
            }
            HostRequest::Point { point } => {
                let owns = host
                    .points()
                    .declared(&point)
                    .is_some_and(|d| d.owner == self.id);
                let check = if owns {
                    Ok(())
                } else {
                    Err((
                        "points".to_owned(),
                        format!("{point} is not one of its points ([[points]])"),
                    ))
                };
                Self::decide("points", check)?;
                Ok(serde_json::to_value(host.points().items(&point)).unwrap_or(Value::Null))
            }
            HostRequest::Window { id, open, state } => {
                Self::decide("windows", Ok(()))?;
                if open {
                    host.open_window(&self.id, &id, state)
                        .map_err(|what| HostError::NotFound { what })?;
                    Ok(Value::Bool(true))
                } else {
                    Ok(Value::Bool(host.close_window(&self.id, &id)))
                }
            }
            HostRequest::Notify { text, kind, title } => {
                let kind = kind.unwrap_or_else(|| "info".to_owned());
                let level = match kind.as_str() {
                    "error" => Level::Error,
                    "warning" | "warn" => Level::Warn,
                    _ => Level::Info,
                };
                host.logs().push(&self.id, level, "notify", text.clone());
                let kept = serde_json::json!({
                    "from": self.id, "text": text, "kind": kind, "title": title,
                });
                host.ui().notify(Notification {
                    from: Self::asker(manifest),
                    text,
                    kind,
                    title,
                })?;
                keep_toast(&host, &self.id, &kept).await;
                Ok(Value::Null)
            }
            HostRequest::ClipboardRead => Ok(host
                .ui()
                .clipboard_read()?
                .map_or(Value::Null, Value::String)),
            HostRequest::ClipboardWrite { text } => {
                host.ui().clipboard_write(&text)?;
                Ok(Value::Null)
            }
            HostRequest::Pick(pick) => {
                let kind = pick.kind;
                let paths = host.ui().pick(&Self::asker(manifest), pick).await?;
                if let Some(paths) = &paths {
                    let now: Vec<Picked> = paths
                        .iter()
                        .map(|p| Picked {
                            path: expand(p),
                            write: matches!(kind, PickKind::Save | PickKind::Folder),
                            folder: kind == PickKind::Folder,
                        })
                        .collect();
                    lock(&host.services().picked)
                        .entry(self.id.clone())
                        .or_default()
                        .extend(now);
                }
                Ok(crate::hostui::pick_answer(kind, paths))
            }
            HostRequest::Prompt(prompt) => Ok(host
                .ui()
                .prompt(&Self::asker(manifest), prompt)
                .await?
                .unwrap_or(Value::Null)),
            HostRequest::Menu(menu) => {
                if menu.items.iter().all(|i| i.separator) {
                    return Err(HostError::Failed {
                        message: "a menu with no items".to_owned(),
                    });
                }
                Ok(host
                    .ui()
                    .menu(&Self::asker(manifest), menu)
                    .await?
                    .map_or(Value::Null, Value::String))
            }
            HostRequest::Decorate { target, decoration } => {
                host.decorations().set(&self.id, &target, decoration);
                Ok(Value::Null)
            }
            HostRequest::Log { text, level } => {
                host.logs().push(
                    &self.id,
                    level.as_deref().map_or(Level::Info, Level::parse),
                    "log",
                    text,
                );
                Ok(Value::Null)
            }
            HostRequest::Subscribe(hook) => host
                .subscribe(&self.id, hook)
                .map(Value::String)
                .map_err(|message| HostError::Failed { message }),
            HostRequest::Unsubscribe { key } => Ok(Value::Bool(host.unsubscribe(&self.id, &key))),
            HostRequest::Events => Ok(host.events_for(&self.id)),
            HostRequest::Raise { name, detail } => host
                .raise_own(&self.id, &name, &detail)
                .map(Value::from)
                .map_err(|message| HostError::Failed { message }),
            #[cfg(feature = "net")]
            HostRequest::Fetch(fetch) => self.fetch(&host, manifest, &may, fetch).await,
            #[cfg(feature = "net")]
            HostRequest::ReadBody { stream, max } => {
                host.services().net.read(&self.id, &stream, max).await
            }
            #[cfg(feature = "net")]
            HostRequest::CloseBody { stream } => {
                Ok(Value::Bool(host.services().net.close(&self.id, &stream)))
            }
            #[cfg(not(feature = "net"))]
            HostRequest::Fetch(_)
            | HostRequest::ReadBody { .. }
            | HostRequest::CloseBody { .. } => Err(HostError::Unsupported {
                what: "the network: this build has none (architect-extensions' feature net)"
                    .to_owned(),
            }),
            HostRequest::Secret { name } => Ok(serde_json::json!({
                "set": host.secrets().is_set(&self.id, &name),
                "title": manifest.secret(&name).map(|s| s.title.clone()),
            })),
            HostRequest::AskSecret { name } => self
                .ask_secret(&host, manifest, &name)
                .await
                .map(Value::Bool),
            HostRequest::Ports { kind } => host.list_ports(kind.as_deref()),
            HostRequest::OpenPort { kind, name, listen } => {
                host.open_port(&self.id, &kind, &name, listen)
            }
            HostRequest::PortSend { port, message } => host.port_send(&self.id, &port, &message),
            HostRequest::PortReceive { port, max } => host.port_receive(&self.id, &port, max),
            HostRequest::ClosePort { port } => Ok(Value::Bool(host.close_port(&self.id, &port))),
            HostRequest::Targets => Ok(host.target_kinds()),
            HostRequest::Surfaces => Ok(host.surfaces_of(&self.id)),
            HostRequest::SurfaceBank { surface, bank } => host
                .set_surface_bank(&self.id, &surface, bank)
                .map(Value::from)
                .map_err(|what| HostError::NotFound { what }),
        }
    }

    /// An HTTP request (`crate::net`), its URL already checked: the
    /// secrets it names put in (asked for, the first time), sent, each
    /// redirect checked; its answer, or its stream kept.
    #[cfg(feature = "net")]
    async fn fetch(
        &self,
        host: &Host,
        manifest: &Manifest,
        may: &Capabilities,
        fetch: crate::hostcall::Fetch,
    ) -> Result<Value, HostError> {
        use crate::net::{Got, Plan};
        let span = tracing::Span::current();
        let url = crate::hostcall::may_reach_url(may, &fetch.url)
            .map_err(|(capability, why)| HostError::Refused { capability, why })?;
        let address = url.host_str().unwrap_or_default().to_owned();
        let method = fetch
            .method
            .as_deref()
            .unwrap_or("GET")
            .to_ascii_uppercase();
        span.record("server.address", address.as_str());
        span.record("http.request.method", method.as_str());
        wide("server.address", address);
        wide("http.request.method", method);
        let names: std::collections::BTreeSet<String> = fetch
            .headers
            .values()
            .flat_map(|v| crate::hostcall::secret_refs(v))
            .map(str::to_owned)
            .collect();
        if !names.is_empty() {
            let listed = names.iter().cloned().collect::<Vec<_>>().join(",");
            span.record("ext.secrets", listed.as_str());
            wide("ext.secrets", listed);
        }
        let secrets = host.secrets();
        for name in &names {
            if !secrets.is_set(&self.id, name) && !self.ask_secret(host, manifest, name).await? {
                return Err(HostError::Failed {
                    message: format!("secret {name} is not set: the person did not enter it"),
                });
            }
        }
        let reach = may.clone();
        let owner = manifest.clone();
        let plan = Plan::of(
            &fetch,
            url,
            |name| {
                secrets
                    .get(&self.id, name)
                    .ok_or_else(|| HostError::Failed {
                        message: format!("secret {name} is not set"),
                    })
            },
            Arc::new(move |u| crate::hostcall::may_reach_url(&reach, u.as_str()).map(|_| ())),
            Arc::new(move |name, u| {
                owner.secret(name).is_some_and(|d| {
                    owner.secret_reaches(
                        d,
                        u.host_str().unwrap_or_default(),
                        u.port_or_known_default(),
                    )
                })
            }),
        )?;
        let (binary, cap, timeout) = (plan.binary, plan.max, plan.timeout);
        let (head, body) = match crate::net::send(plan).await {
            Ok(fetched) => fetched.split(),
            // A redirect to a host it may not reach.
            Err(HostError::Refused { capability, why }) => {
                Self::decide(crate::net::REDIRECT, Err((capability, why)))?;
                return Err(HostError::Failed {
                    message: "refused".to_owned(),
                });
            }
            Err(e) => return Err(e),
        };
        span.record("http.response.status_code", head.status);
        span.record("ext.redirects", head.redirects);
        wide("http.response.status_code", head.status.to_string());
        wide("ext.redirects", head.redirects.to_string());
        match body {
            Got::Whole(bytes) => {
                span.record("ext.bytes", bytes.len());
                wide("ext.bytes", bytes.len().to_string());
                Ok(head.answer(Some(crate::net::whole(&bytes, binary)), None))
            }
            Got::Stream(response) => {
                let id = host
                    .services()
                    .net
                    .keep(&self.id, response, binary, cap, timeout)?;
                Ok(head.answer(None, Some(&id)))
            }
        }
    }

    /// Asks the person for secret `name` of `manifest`'s (`[[secrets]]`),
    /// through the app's prompt, and keeps what they enter: whether they
    /// did. The extension sees only that.
    async fn ask_secret(
        &self,
        host: &Host,
        manifest: &Manifest,
        name: &str,
    ) -> Result<bool, HostError> {
        let decl = manifest.secret(name).ok_or_else(|| HostError::NotFound {
            what: format!("secret {name} in its manifest"),
        })?;
        let hosts = if decl.hosts.is_empty() {
            &manifest.capabilities.hosts
        } else {
            &decl.hosts
        };
        let mut message = decl.description.clone().unwrap_or_default();
        if !message.is_empty() {
            message.push(' ');
        }
        let _ = write!(
            message,
            "The app keeps it and sends it only to {}; {} never sees it.",
            hosts.join(", "),
            manifest.name
        );
        let prompt = crate::hostcall::Prompt {
            kind: crate::hostcall::PromptKind::Secret,
            title: if decl.title.is_empty() {
                name.to_owned()
            } else {
                decl.title.clone()
            },
            message: Some(message),
            ..crate::hostcall::Prompt::default()
        };
        match host.ui().prompt(&Self::asker(manifest), prompt).await? {
            Some(Value::String(v)) if !v.trim().is_empty() => {
                host.secrets()
                    .set(&self.id, name, v.trim())
                    .map_err(|message| HostError::Failed { message })?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// An editor call (`crate::editor`), its capability checked: on the
    /// app's focused editor, as the extension.
    fn editor(
        &self,
        host: &Host,
        manifest: &Manifest,
        request: HostRequest,
    ) -> Result<Value, HostError> {
        use crate::editor::{EditCommand, EditorError};
        let editors = host.editors().ok_or_else(|| HostError::Unsupported {
            what: "the editor: this app gives extensions none".to_owned(),
        })?;
        let view = |v: crate::editor::EditorView| serde_json::to_value(v).unwrap_or(Value::Null);
        match request {
            HostRequest::EditorState => editors
                .focused()
                .map(view)
                .ok_or_else(|| EditorError::NoEditor.into()),
            HostRequest::EditEditor {
                changes,
                selection,
                name,
                base,
            } => {
                let step = name
                    .clone()
                    .unwrap_or_else(|| format!("{}: edit", manifest.name));
                let edit = EditCommand {
                    changes,
                    selection,
                    name,
                    base,
                };
                Ok(view(editors.apply(&edit, &step, &self.principal())?))
            }
            HostRequest::DecorateEditor { decorations } => {
                let embeds: Vec<String> = manifest
                    .contributes
                    .embeds
                    .iter()
                    .map(|e| e.id.clone())
                    .chain(
                        host.contributions()
                            .of(&self.id)
                            .into_iter()
                            .flat_map(|c| c.embeds.into_iter().map(|e| e.id)),
                    )
                    .collect();
                crate::editor::check_widgets(&decorations, &embeds, &manifest.view_ids())
                    .map_err(|message| HostError::Failed { message })?;
                editors.decorate(&self.id, decorations)?;
                Ok(Value::Null)
            }
            other => Err(HostError::Failed {
                message: format!("{} is not an editor call", other.op()),
            }),
        }
    }

    /// A document call (`crate::documents`), its capability checked: done
    /// on the app's [`Documents`] as the extension, and the document event
    /// it is raised for the hooks that watch.
    fn document(
        &self,
        host: &Host,
        manifest: &Manifest,
        request: HostRequest,
    ) -> Result<Value, HostError> {
        use crate::documents::{checked_path, moved_to};
        let path_ok = |p: &str| {
            checked_path(p)
                .map(str::to_owned)
                .map_err(|message| HostError::Failed { message })
        };
        if let HostRequest::WatchDocuments {
            action,
            script,
            function,
        } = request
        {
            let keys = crate::events::DOC_EVENTS
                .iter()
                .map(|on| {
                    host.subscribe(
                        &self.id,
                        crate::contrib::HookDecl {
                            on: (*on).to_owned(),
                            action: action.clone(),
                            script: script.clone(),
                            function: function.clone(),
                            ..crate::contrib::HookDecl::default()
                        },
                    )
                    .map(Value::String)
                    .map_err(|message| HostError::Failed { message })
                })
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(Value::Array(keys));
        }
        let docs = host.documents().ok_or_else(|| HostError::Unsupported {
            what: "documents: this app gives extensions none".to_owned(),
        })?;
        let by = self.principal();
        let info = |i: DocInfo| serde_json::to_value(i).unwrap_or(Value::Null);
        let (answer, event) = match request {
            HostRequest::ListDocuments { under } => {
                let list = docs.list(under.as_deref().filter(|u| !u.is_empty()))?;
                (serde_json::to_value(list).unwrap_or(Value::Null), None)
            }
            HostRequest::CreateDocument { path, text } => {
                let path = path_ok(&path)?;
                let made = docs.create(&path, &text, &by)?;
                (info(made), Some(DocEvent::Created { path }))
            }
            HostRequest::EditDocument { path, edits, name } => {
                let path = path_ok(&path)?;
                let step = name.unwrap_or_else(|| format!("{}: edit", manifest.name));
                let edited = docs.edit(&path, &edits, &step, &by)?;
                // An edit that changed nothing is no save: a format-on-save
                // hook that finds nothing to do is not heard again.
                let event = edited.changed.then_some(DocEvent::Saved { path });
                (Value::String(edited.text), event)
            }
            HostRequest::RenameDocument { path, to } => {
                let path = path_ok(&path)?;
                let to = path_ok(&to)?;
                let moved = docs.rename(&path, &to, &by)?;
                (info(moved), Some(DocEvent::Renamed { from: path, to }))
            }
            HostRequest::MoveDocument { path, folder } => {
                let path = path_ok(&path)?;
                let to =
                    moved_to(&path, &folder).map_err(|message| HostError::Failed { message })?;
                let moved = docs.rename(&path, &to, &by)?;
                (info(moved), Some(DocEvent::Renamed { from: path, to }))
            }
            HostRequest::DeleteDocument { path } => {
                let path = path_ok(&path)?;
                docs.delete(&path, &by)?;
                (Value::Null, Some(DocEvent::Deleted { path }))
            }
            other => {
                return Err(HostError::Failed {
                    message: format!("{} is not a document call", other.op()),
                });
            }
        };
        if let Some(event) = event {
            host.raise_doc(&event);
        }
        Ok(answer)
    }

    async fn run(
        &self,
        host: &Host,
        manifest: &Manifest,
        may: &Capabilities,
        request: &HostRequest,
        action: &str,
        args: &Value,
        ticket: Option<String>,
    ) -> Result<Value, HostError> {
        let info = host
            .list_of(action)
            .info(action)
            .ok_or_else(|| HostError::NotFound {
                what: format!("action {action}"),
            })?;
        let (capability, check) =
            crate::hostcall::check(manifest, may, request, reach(info.effect));
        Self::decide(capability, check)?;
        let mut caller = ActionCaller::new(self.principal())
            .with_invocation(Invocation::new(InvocationSource::Script));
        if let Some(t) = ticket {
            caller = caller.with_ticket(t);
        }
        let bytes = if args.is_null() {
            Vec::new()
        } else {
            args.to_string().into_bytes()
        };
        match host
            .list_of(action)
            .run_json_async_with(&caller, action, &bytes)
            .await
        {
            Ok(out) => Ok(serde_json::from_slice(&out).unwrap_or(Value::Null)),
            Err(ActionError::ConfirmationRequired { id, ticket, .. }) => {
                Err(HostError::ConfirmationRequired { action: id, ticket })
            }
            Err(ActionError::Denied { reason, .. }) => Err(HostError::Refused {
                capability: "authorizer".to_owned(),
                why: reason,
            }),
            Err(ActionError::Unknown(id)) => Err(HostError::NotFound {
                what: format!("action {id}"),
            }),
            Err(e) => Err(HostError::Failed {
                message: e.to_string(),
            }),
        }
    }

    /// A service call: the provider of `service` that meets this
    /// extension's `[requires]`, its action for `method`, run as this
    /// extension (for whoever it acts for), within its `effect`.
    async fn service(
        &self,
        host: &Host,
        manifest: &Manifest,
        may: &Capabilities,
        service: &str,
        method: &str,
        args: &Value,
    ) -> Result<Value, HostError> {
        // A service it requires, or one it uses when it is there.
        let req = manifest
            .requires
            .get(service)
            .or_else(|| manifest.uses.get(service))
            .cloned();
        let check = req.as_ref().map(|_| ()).ok_or_else(|| {
            (
                "requires".to_owned(),
                format!(
                    "{} neither requires nor uses {service} ([requires], [uses])",
                    manifest.id
                ),
            )
        });
        Self::decide("requires", check)?;
        let req = req.unwrap_or_default();
        let parsed = semver::VersionReq::parse(&req).map_err(|e| HostError::Failed {
            message: format!("{service} {req}: {e}"),
        })?;
        let (provider, version) =
            host.provider(service, &parsed)
                .ok_or_else(|| HostError::NotFound {
                    what: format!("provider of {service} {req}"),
                })?;
        let loaded = host.get(&provider).ok_or_else(|| HostError::NotLoaded {
            id: provider.clone(),
        })?;
        let action = crate::interop::method_action(&loaded.manifest.action_prefix(), method);
        if !loaded.actions.contains(&action) {
            return Err(HostError::NotFound {
                what: format!(
                    "method {method} of {service} ({provider} {version} has no {action})"
                ),
            });
        }
        let info = host
            .list_of(&action)
            .info(&action)
            .ok_or_else(|| HostError::NotFound {
                what: format!("action {action}"),
            })?;
        let effect = reach(info.effect);
        let check = if effect > may.effect {
            Err((
                "effect".to_owned(),
                format!(
                    "{service}.{method} is {}: this extension may run {} actions at most",
                    effect.name(),
                    may.effect.name()
                ),
            ))
        } else {
            Ok(())
        };
        Self::decide("effect", check)?;
        let caller = ActionCaller::new(self.principal())
            .with_invocation(Invocation::new(InvocationSource::Script));
        let bytes = if args.is_null() {
            Vec::new()
        } else {
            args.to_string().into_bytes()
        };
        match host
            .list_of(&action)
            .run_json_async_with(&caller, &action, &bytes)
            .await
        {
            Ok(out) => Ok(serde_json::from_slice(&out).unwrap_or(Value::Null)),
            Err(ActionError::ConfirmationRequired { id, ticket, .. }) => {
                Err(HostError::ConfirmationRequired { action: id, ticket })
            }
            Err(ActionError::Denied { reason, .. }) => Err(HostError::Refused {
                capability: "authorizer".to_owned(),
                why: reason,
            }),
            Err(e) => Err(HostError::Failed {
                message: e.to_string(),
            }),
        }
    }

    /// Its own folder: `<data root>/<id>`.
    fn own_dir(&self, host: &Host) -> Option<PathBuf> {
        lock(&host.services().data_root)
            .as_ref()
            .map(|r| r.join(crate::install::slug(&self.id)))
    }

    /// Where `path` is, when it may reach it (`write`: to change it).
    fn file(
        &self,
        host: &Host,
        loaded: &crate::host::Loaded,
        may: &Capabilities,
        path: &str,
        write: bool,
    ) -> Result<PathBuf, HostError> {
        let deny = |why: String| Self::decide("folders", Err(("folders".to_owned(), why)));
        let raw = Path::new(path);
        if raw.components().any(|c| matches!(c, Component::ParentDir)) {
            deny(format!("{path}: a path with `..` in it"))?;
        }
        let own = self.own_dir(host);
        if !(raw.is_absolute() || path.starts_with('~')) {
            let Some(own) = own else {
                return Err(HostError::Failed {
                    message: "this app keeps no files for extensions".to_owned(),
                });
            };
            Self::decide("folders", Ok(()))?;
            return Ok(own.join("files").join(raw));
        }
        let file = expand(path);
        if host.secrets().file().is_some_and(|s| s == file) {
            deny(format!("{path} is where the app keeps secrets"))?;
        }
        let text = file.display().to_string();
        let in_own = own.as_ref().is_some_and(|o| file.starts_with(o));
        let in_package = !write && loaded.dir.as_ref().is_some_and(|d| file.starts_with(d));
        let folders = Capabilities {
            folders: may
                .folders
                .iter()
                .map(|f| expand(f).display().to_string())
                .collect(),
            ..Capabilities::default()
        };
        if in_own
            || in_package
            || folders.may_reach_folder(&text)
            || self.was_picked(host, &file, write)
        {
            Self::decide("folders", Ok(()))?;
            Ok(file)
        } else {
            deny(format!(
                "{text} is outside its own folder and the folders it may reach (capabilities.folders)"
            ))?;
            Ok(file)
        }
    }

    fn data_file(&self, host: &Host) -> Option<PathBuf> {
        self.own_dir(host).map(|d| d.join("data.json"))
    }

    /// The lock on this extension's data: one read-change-write at a
    /// time, whichever of its runtimes asks.
    fn data_lock(&self, host: &Host) -> std::sync::MutexGuard<'static, ()> {
        static LOCKS: std::sync::LazyLock<Mutex<BTreeMap<String, &'static Mutex<()>>>> =
            std::sync::LazyLock::new(Mutex::default);
        let key = self
            .data_file(host)
            .map_or_else(|| format!("mem:{}", self.id), |f| f.display().to_string());
        let one: &'static Mutex<()> = lock(&LOCKS)
            .entry(key)
            .or_insert_with(|| Box::leak(Box::new(Mutex::new(()))));
        lock(one)
    }

    /// Its data: none yet is empty. A file that does not parse is an
    /// error, never read as empty, so the next write cannot wipe it.
    fn data(&self, host: &Host) -> Result<BTreeMap<String, Value>, HostError> {
        let Some(file) = self.data_file(host) else {
            return Ok(lock(&host.services().data)
                .get(&self.id)
                .cloned()
                .unwrap_or_default());
        };
        let text = match std::fs::read_to_string(&file) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(e) => {
                return Err(HostError::Failed {
                    message: format!("{}: {e}", file.display()),
                });
            }
        };
        serde_json::from_str(&text).map_err(|e| HostError::Failed {
            message: format!(
                "{}: not its data ({e}); left as it is, and nothing is written over it",
                file.display()
            ),
        })
    }

    /// Writes its data: a file only its owner reads (0600), written
    /// whole or not at all (a temporary file, synced, renamed over it).
    fn keep_data(&self, host: &Host, data: BTreeMap<String, Value>) -> Result<(), HostError> {
        let Some(file) = self.data_file(host) else {
            lock(&host.services().data).insert(self.id.clone(), data);
            return Ok(());
        };
        let failed = |e: &dyn std::fmt::Display| HostError::Failed {
            message: format!("{}: {e}", file.display()),
        };
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| failed(&e))?;
        }
        let text = serde_json::to_string_pretty(&data).map_err(|e| failed(&e))?;
        write_private(&file, text.as_bytes()).map_err(|e| failed(&e))
    }
}

/// `bytes` as the file at `path`, atomically, readable by its owner only.
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut f = options.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

impl Host {
    fn app_context(&self) -> Option<Arc<dyn AppContext>> {
        lock(&self.services().app).clone()
    }
}

/// `~` and `~/…` under the home folder.
fn expand(path: &str) -> PathBuf {
    path.strip_prefix('~').map_or_else(
        || PathBuf::from(path),
        |rest| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            home.join(rest.trim_start_matches('/'))
        },
    )
}

/// Setting `key`'s default, from the manifest that declares it.
fn default_of(manifest: &Manifest, key: &str) -> Option<Value> {
    manifest
        .sections
        .iter()
        .flat_map(|s| &s.settings)
        .find(|s| s.key == key)
        .map(|s| serde_json::to_value(&s.default).unwrap_or(Value::Null))
}

/// The service a notification centre provides: a toast shown is kept
/// there too (its `toasted` method), so the centre lists the session's
/// toasts beside the persistent notifications.
pub const NOTIFY_SERVICE: &str = "architect.notify";

/// Hands a toast extension `from` showed to the provider of
/// [`NOTIFY_SERVICE`] (its `<PREFIX>_TOASTED` action), unless it is the
/// one that showed it. Nothing provides it: nothing to do.
async fn keep_toast(host: &Host, from: &str, toast: &Value) {
    let Some((provider, _)) = host.provider(NOTIFY_SERVICE, &semver::VersionReq::STAR) else {
        return;
    };
    if provider == from {
        return;
    }
    let Some(loaded) = host.get(&provider) else {
        return;
    };
    let action = crate::interop::method_action(&loaded.manifest.action_prefix(), "toasted");
    if !loaded.actions.contains(&action) {
        return;
    }
    let caller = ActionCaller::app().with_invocation(Invocation::new(InvocationSource::App));
    let _ = host
        .list_of(&action)
        .run_json_async_with(&caller, &action, toast.to_string().as_bytes())
        .await;
}
