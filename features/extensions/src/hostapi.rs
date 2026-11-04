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
//! ([`Host::set_app_context`]); settings' values from its
//! [`SettingValues`] ([`Host::set_setting_values`]; in memory unless
//! given); data and files under [`Host::with_data_root`] (`<root>/<id>/`).

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use architect::action::{ActionCaller, ActionError, Invocation, InvocationSource, Principal};
use serde_json::Value;
use tracing::Instrument as _;

use crate::capabilities::{DOCUMENTS, ORG, SELECTION};
use crate::catalog::Origin;
use crate::consent::Grants;
use crate::host::{Host, Opening, WeakHost};
use crate::hostcall::{HostError, HostRequest};
use crate::logs::{Level, Logs};
use crate::manifest::{Capabilities, Manifest, Reach};
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

/// The host's side of the service: what it was given.
pub(crate) struct Services {
    grants: Mutex<Option<Grants>>,
    logs: Logs,
    app: Mutex<Option<Arc<dyn AppContext>>>,
    values: Mutex<Arc<dyn SettingValues>>,
    data_root: Mutex<Option<PathBuf>>,
    /// The app's own list, when its actions are not on the host's.
    app_list: Mutex<Option<architect::action::ActionList>>,
    /// Data kept in memory, when there is no data root.
    data: Mutex<BTreeMap<String, BTreeMap<String, Value>>>,
}

impl Default for Services {
    fn default() -> Self {
        Self {
            grants: Mutex::new(None),
            logs: Logs::default(),
            app: Mutex::new(None),
            values: Mutex::new(Arc::new(MemoryValues::default())),
            data_root: Mutex::new(None),
            app_list: Mutex::new(None),
            data: Mutex::new(BTreeMap::new()),
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
            host.capabilities_of(&manifest, origin)
                .may_run(&manifest.action_prefix(), &info.id, reach(info.effect))
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
        architect_telemetry::wide::set("ext.outcome", outcome);
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
    pub async fn call_json(&self, request: &str) -> String {
        let answer = match serde_json::from_str::<HostRequest>(request) {
            Ok(request) => self.call(request).await,
            Err(e) => Err(HostError::Failed {
                message: format!("not a host request: {e}"),
            }),
        };
        crate::hostcall::HostAnswer::from(answer).to_json()
    }

    /// Records the permission decision on the call's span.
    fn decide(capability: &str, check: Check) -> Result<(), HostError> {
        let span = tracing::Span::current();
        span.record("perm.capability", capability);
        architect_telemetry::wide::set("perm.capability", capability.to_owned());
        match check {
            Ok(()) => {
                span.record("perm.decision", "allow");
                architect_telemetry::wide::set("perm.decision", "allow");
                Ok(())
            }
            Err((capability, why)) => {
                span.record("perm.capability", capability.as_str());
                span.record("perm.decision", "deny");
                span.record("perm.reason", why.as_str());
                architect_telemetry::wide::set("perm.capability", capability.clone());
                architect_telemetry::wide::set("perm.decision", "deny");
                architect_telemetry::wide::set("perm.reason", why.clone());
                Err(HostError::Refused { capability, why })
            }
        }
    }

    fn reads(may: &Capabilities, what: &str) -> Result<(), HostError> {
        let capability = format!("reads.{what}");
        let check = if may.may_read(what) {
            Ok(())
        } else {
            Err((
                capability.clone(),
                format!("this extension may not read the app's {what} (capabilities.reads)"),
            ))
        };
        Self::decide(&capability, check)
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
        match request {
            HostRequest::Run {
                action,
                args,
                ticket,
            } => {
                self.run(&host, manifest, &may, &action, &args, ticket)
                    .await
            }
            HostRequest::Org => {
                Self::reads(&may, ORG)?;
                Ok(host
                    .app_context()
                    .and_then(|c| c.org())
                    .unwrap_or(Value::Null))
            }
            HostRequest::Selection => {
                Self::reads(&may, SELECTION)?;
                Ok(host
                    .app_context()
                    .and_then(|c| c.selection())
                    .unwrap_or(Value::Null))
            }
            HostRequest::Open { path } => {
                Self::reads(&may, DOCUMENTS)?;
                host.open(
                    Opening::Path(path),
                    Invocation::new(InvocationSource::Script),
                )
                .await
                .map_err(|message| HostError::Failed { message })
            }
            HostRequest::ReadDocument { path } => {
                Self::reads(&may, DOCUMENTS)?;
                match host.app_context().and_then(|c| c.document(&path)) {
                    Some(Ok(text)) => Ok(Value::String(text)),
                    Some(Err(message)) => Err(HostError::Failed { message }),
                    None => Err(HostError::NotFound {
                        what: format!("document {path}"),
                    }),
                }
            }
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
                Self::own_setting(manifest, &key)?;
                let set = lock(&host.services().values).clone().get(&key);
                Ok(set
                    .or_else(|| default_of(manifest, &key))
                    .unwrap_or(Value::Null))
            }
            HostRequest::SetSetting { key, value } => {
                Self::own_setting(manifest, &key)?;
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
                Self::decide("data", Ok(()))?;
                Ok(self.data(&host).remove(&key).unwrap_or(Value::Null))
            }
            HostRequest::SetData { key, value } => {
                Self::decide("data", Ok(()))?;
                let mut data = self.data(&host);
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
            HostRequest::Log { text, level } => {
                host.logs().push(
                    &self.id,
                    level.as_deref().map_or(Level::Info, Level::parse),
                    "log",
                    text,
                );
                Ok(Value::Null)
            }
        }
    }

    async fn run(
        &self,
        host: &Host,
        manifest: &Manifest,
        may: &Capabilities,
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
        let check = may
            .may_run(&manifest.action_prefix(), action, reach(info.effect))
            .map_err(|(c, why)| (c.to_owned(), why));
        Self::decide("actions", check)?;
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

    fn own_setting(manifest: &Manifest, key: &str) -> Result<(), HostError> {
        let check = if key.starts_with(&format!("{}.", manifest.id)) {
            Ok(())
        } else {
            Err((
                "settings".to_owned(),
                format!(
                    "{key} is not one of its settings: an extension reads and writes only its own ({}.…)",
                    manifest.id
                ),
            ))
        };
        Self::decide("settings", check)
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
        if in_own || in_package || folders.may_reach_folder(&text) {
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

    fn data(&self, host: &Host) -> BTreeMap<String, Value> {
        self.data_file(host).map_or_else(
            || {
                lock(&host.services().data)
                    .get(&self.id)
                    .cloned()
                    .unwrap_or_default()
            },
            |file| {
                std::fs::read_to_string(file)
                    .ok()
                    .and_then(|t| serde_json::from_str(&t).ok())
                    .unwrap_or_default()
            },
        )
    }

    fn keep_data(&self, host: &Host, data: BTreeMap<String, Value>) -> Result<(), HostError> {
        let Some(file) = self.data_file(host) else {
            lock(&host.services().data).insert(self.id.clone(), data);
            return Ok(());
        };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let text = serde_json::to_string_pretty(&data).map_err(|e| HostError::Failed {
            message: e.to_string(),
        })?;
        std::fs::write(&file, text).map_err(|e| HostError::Failed {
            message: format!("{}: {e}", file.display()),
        })
    }
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
