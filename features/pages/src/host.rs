//! The page host in process (feature `runner`): [`AppHost`], the
//! [`PageHost`] service over the app's Action List, theme, settings,
//! shared state and notifications, as every in-process runner binds it.
//!
//! A page's action runs go to the app's Tokio runtime (`Dispatch`), never
//! the page's thread, so an action that needs one works; they run as the
//! page's caller, through its extension's gate (the namespaces it named),
//! as a Luau script's do. The full host service, with consent, is #178's;
//! this is the subset that exists today.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use architect::action::{ActionCaller, ActionList, Effect};
use architect_scripting::luau::Gate;
use architect_scripting::theme::ThemeSource;
use architect_scripting::vm::Dispatch;
use serde_json::Value;

use crate::api::{
    ActionSummary, DataEvent, Notice, PageError, PageHost, ThemeSnapshot, ThemeToken,
};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

type Listener = Arc<dyn Fn(&DataEvent) + Send + Sync>;

/// State every page of an app shares, by key: JSON values, each change
/// told to every page that listens ([`Self::listen`]). Cheap to clone;
/// clones share it.
#[derive(Clone, Default)]
pub struct SharedState(Arc<SharedInner>);

#[derive(Default)]
struct SharedInner {
    values: Mutex<BTreeMap<String, Value>>,
    listeners: Mutex<Vec<(u64, Listener)>>,
    next: AtomicU64,
}

impl std::fmt::Debug for SharedState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedState")
            .field("keys", &lock(&self.0.values).len())
            .finish_non_exhaustive()
    }
}

impl SharedState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<Value> {
        lock(&self.0.values).get(key).cloned()
    }

    /// Sets `key` (`null` clears it), telling every listener.
    pub fn set(&self, key: &str, value: Value) {
        let event = DataEvent {
            topic: key.to_owned(),
            value: value.to_string(),
        };
        {
            let mut values = lock(&self.0.values);
            if value.is_null() {
                values.remove(key);
            } else {
                values.insert(key.to_owned(), value);
            }
        }
        let listeners: Vec<Listener> = lock(&self.0.listeners)
            .iter()
            .map(|(_, l)| Arc::clone(l))
            .collect();
        for l in listeners {
            l(&event);
        }
    }

    /// `f` for every change from now, until the [`Listening`] goes.
    #[must_use]
    pub fn listen(&self, f: impl Fn(&DataEvent) + Send + Sync + 'static) -> Listening {
        let id = self.0.next.fetch_add(1, Ordering::Relaxed);
        lock(&self.0.listeners).push((id, Arc::new(f)));
        Listening {
            state: self.clone(),
            id,
        }
    }
}

/// A listener on [`SharedState`]; dropped, it stops.
pub struct Listening {
    state: SharedState,
    id: u64,
}

impl std::fmt::Debug for Listening {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Listening").field(&self.id).finish()
    }
}

impl Drop for Listening {
    fn drop(&mut self) {
        lock(&self.state.0.listeners).retain(|(id, _)| *id != self.id);
    }
}

/// Where a page's settings are kept: the app's settings, or (by default)
/// memory.
pub trait Settings: Send + Sync {
    fn get(&self, key: &str) -> Option<Value>;

    /// # Errors
    ///
    /// The value is not one the setting takes.
    fn set(&self, key: &str, value: Value) -> Result<(), String>;
}

/// Settings in memory.
#[derive(Debug, Default)]
pub struct MemorySettings(Mutex<BTreeMap<String, Value>>);

impl Settings for MemorySettings {
    fn get(&self, key: &str) -> Option<Value> {
        lock(&self.0).get(key).cloned()
    }

    fn set(&self, key: &str, value: Value) -> Result<(), String> {
        lock(&self.0).insert(key.to_owned(), value);
        Ok(())
    }
}

/// Where a page's notifications go: the app's toasts, or (by default)
/// the log and the last few kept ([`AppHost::notices`]).
pub type Notify = Arc<dyn Fn(&Notice) + Send + Sync>;

/// The action a page calls another extension's service with (it is the
/// extensions host's: `architect_extensions::interop_actions`).
const SERVICE_CALL: &str = "EXT_SERVICE_CALL";

/// The action a page makes any host-service call with, as its extension
/// (the extensions host's: `architect_extensions::host::PAGE_CALL`).
const PAGE_CALL: &str = "EXT_PAGE_CALL";

/// The page host in process: one per page (its caller and gate are the
/// page's), the app's parts shared.
#[derive(Clone)]
pub struct AppHost {
    list: ActionList,
    caller: ActionCaller,
    /// The extension whose page it is ([`Self::with_extension`]).
    extension: Option<String>,
    gate: Option<Gate>,
    theme: Option<Arc<dyn ThemeSource>>,
    shared: SharedState,
    settings: Arc<dyn Settings>,
    notify: Option<Notify>,
    notices: Arc<Mutex<Vec<Notice>>>,
    dispatch: Dispatch,
}

impl std::fmt::Debug for AppHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppHost")
            .field("gate", &self.gate.is_some())
            .field("theme", &self.theme.is_some())
            .field("dispatch", &self.dispatch)
            .finish_non_exhaustive()
    }
}

impl AppHost {
    /// Over `list`, as the app; no theme, settings in memory. Its action
    /// runs go to the runtime of the thread that makes it, if any.
    #[must_use]
    pub fn new(list: ActionList) -> Self {
        Self {
            list,
            caller: ActionCaller::app(),
            extension: None,
            gate: None,
            theme: None,
            shared: SharedState::new(),
            settings: Arc::new(MemorySettings::default()),
            notify: None,
            notices: Arc::default(),
            dispatch: Dispatch::here(),
        }
    }

    #[must_use]
    pub fn with_caller(mut self, caller: ActionCaller) -> Self {
        self.caller = caller;
        self
    }

    /// The extension whose page this is: its service calls
    /// (`EXT_SERVICE_CALL`, `services.call`) are made as it.
    #[must_use]
    pub fn with_extension(mut self, id: impl Into<String>) -> Self {
        self.extension = Some(id.into());
        self
    }

    /// Its runs see and run only what `gate` lets through (an
    /// extension's capabilities).
    #[must_use]
    pub fn with_gate(mut self, gate: Gate) -> Self {
        self.gate = Some(gate);
        self
    }

    #[must_use]
    pub fn with_theme(mut self, theme: Arc<dyn ThemeSource>) -> Self {
        self.theme = Some(theme);
        self
    }

    #[must_use]
    pub fn with_shared(mut self, shared: SharedState) -> Self {
        self.shared = shared;
        self
    }

    #[must_use]
    pub fn with_settings(mut self, settings: Arc<dyn Settings>) -> Self {
        self.settings = settings;
        self
    }

    #[must_use]
    pub fn with_notify(mut self, notify: Notify) -> Self {
        self.notify = Some(notify);
        self
    }

    /// Its action runs go to `dispatch`.
    #[must_use]
    pub fn with_dispatch(mut self, dispatch: Dispatch) -> Self {
        self.dispatch = dispatch;
        self
    }

    #[must_use]
    pub const fn shared_state(&self) -> &SharedState {
        &self.shared
    }

    /// The notifications shown, the last 50.
    #[must_use]
    pub fn notices(&self) -> Vec<Notice> {
        lock(&self.notices).clone()
    }

    /// The theme's revision now (0 with none): cheap, read often.
    #[must_use]
    pub fn theme_revision(&self) -> u64 {
        self.theme.as_ref().map_or(0, |t| t.revision())
    }

    /// The theme now, as a page reads it.
    #[must_use]
    pub fn theme_snapshot(&self) -> ThemeSnapshot {
        let Some(source) = &self.theme else {
            return ThemeSnapshot {
                scheme: "dark".to_owned(),
                ..ThemeSnapshot::default()
            };
        };
        let theme = source.current();
        ThemeSnapshot {
            id: theme.id().to_owned(),
            scheme: theme.scheme().as_str().to_owned(),
            // A key that moves, compared, never ordered: it may wrap.
            revision: u32::try_from(source.revision() & u64::from(u32::MAX)).unwrap_or_default(),
            tokens: theme
                .tokens()
                .iter()
                .map(|(name, value)| ThemeToken {
                    name: name.clone(),
                    value: value.clone(),
                })
                .collect(),
        }
    }

    /// A shared key, as JSON (`null` when unset).
    #[must_use]
    pub fn shared_json(&self, key: &str) -> String {
        self.shared.get(key).unwrap_or(Value::Null).to_string()
    }

    /// Sets a shared key from JSON.
    ///
    /// # Errors
    ///
    /// `value` is not JSON.
    pub fn set_shared_json(&self, key: &str, value: &str) -> Result<(), PageError> {
        let value: Value =
            serde_json::from_str(value).map_err(|e| PageError::Failed(format!("not JSON: {e}")))?;
        self.shared.set(key, value);
        Ok(())
    }

    /// Shows `notice`.
    pub fn show(&self, notice: &Notice) {
        {
            let mut kept = lock(&self.notices);
            kept.push(notice.clone());
            let over = kept.len().saturating_sub(50);
            kept.drain(..over);
        }
        if let Some(f) = &self.notify {
            f(notice);
        } else if let Some(ext) = self
            .extension
            .clone()
            .filter(|_| self.list.info(PAGE_CALL).is_some())
        {
            // An extension's page: the app's interface shows it (its
            // `capabilities.ui` holding), off the page's thread.
            let (list, dispatch) = (self.list.clone(), self.dispatch.clone());
            let caller = ActionCaller::new(architect::action::Principal::extension(ext, None));
            let body =
                serde_json::json!({ "op": "notify", "text": notice.text, "kind": notice.kind })
                    .to_string();
            architect_scripting::vm::spawn(move || {
                let _ = futures::executor::block_on(dispatch.call(async move {
                    list.run_json_async_with(&caller, PAGE_CALL, body.as_bytes())
                        .await
                }));
            });
        } else {
            tracing::info!(kind = %notice.kind, text = %notice.text, "a page's notice");
        }
    }

    fn refused(&self, id: &str) -> Option<PageError> {
        let Some(info) = self.list.info(id) else {
            return Some(PageError::Unknown(format!("no action {id}")));
        };
        match self.gate.as_ref().map(|g| g(&info)) {
            Some(Err(why)) => Some(PageError::Denied(format!("{id}: {why}"))),
            _ => None,
        }
    }
}

const fn effect_name(effect: Effect) -> &'static str {
    match effect {
        Effect::ReadOnly => "read_only",
        Effect::Mutating => "mutating",
        Effect::Destructive => "destructive",
    }
}

impl PageHost for AppHost {
    async fn run(&self, id: String, args: String) -> Result<String, PageError> {
        if let Some(why) = self.refused(&id) {
            return Err(why);
        }
        let (list, mut caller) = (self.list.clone(), self.caller.clone());
        // A service call and a host call are the page's extension's own.
        if (id == SERVICE_CALL || id == PAGE_CALL)
            && let Some(ext) = &self.extension
        {
            caller = ActionCaller::new(architect::action::Principal::extension(ext.clone(), None));
        }
        let call = {
            let id = id.clone();
            async move {
                list.run_json_async_with(&caller, &id, args.as_bytes())
                    .await
            }
        };
        let out = self
            .dispatch
            .call(call)
            .await
            .map_err(|e| PageError::Failed(format!("{id}: {e}")))?
            .map_err(|e| PageError::Failed(e.to_string()))?;
        if out.is_empty() {
            return Ok("null".to_owned());
        }
        String::from_utf8(out).map_err(|e| PageError::Failed(e.to_string()))
    }

    async fn actions(&self, filter: String) -> Result<Vec<ActionSummary>, PageError> {
        let filter = filter.to_lowercase();
        let infos = self
            .caller
            .principal()
            .map_or_else(|| self.list.infos(), |p| self.list.infos_for(p));
        Ok(infos
            .into_iter()
            .filter(|i| self.gate.as_ref().is_none_or(|g| g(i).is_ok()))
            .filter(|i| {
                filter.is_empty()
                    || [&i.id, &i.display_name, &i.description, &i.group]
                        .iter()
                        .any(|f| f.to_lowercase().contains(&filter))
            })
            .map(|i| ActionSummary {
                effect: effect_name(i.effect).to_owned(),
                id: i.id,
                name: i.display_name,
                description: i.description,
                group: i.group,
                enabled: i.enabled,
            })
            .collect())
    }

    async fn setting(&self, key: String) -> Result<String, PageError> {
        Ok(self.settings.get(&key).unwrap_or(Value::Null).to_string())
    }

    async fn set_setting(&self, key: String, value: String) -> Result<(), PageError> {
        let value: Value = serde_json::from_str(&value)
            .map_err(|e| PageError::Failed(format!("not JSON: {e}")))?;
        self.settings.set(&key, value).map_err(PageError::Failed)
    }

    async fn theme(&self) -> Result<ThemeSnapshot, PageError> {
        Ok(self.theme_snapshot())
    }

    async fn shared(&self, key: String) -> Result<String, PageError> {
        Ok(self.shared_json(&key))
    }

    async fn set_shared(&self, key: String, value: String) -> Result<(), PageError> {
        self.set_shared_json(&key, &value)
    }

    async fn notify(&self, notice: Notice) -> Result<(), PageError> {
        self.show(&notice);
        Ok(())
    }

    async fn host_call(&self, request: String) -> Result<String, PageError> {
        if self.extension.is_none() {
            return Err(PageError::Denied(
                "only an extension's page makes host-service calls".to_owned(),
            ));
        }
        self.run(PAGE_CALL.to_owned(), request).await
    }
}
