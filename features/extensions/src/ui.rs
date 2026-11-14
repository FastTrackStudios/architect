//! Loaded extensions' settings on the settings page (feature `ui`).
//!
//! A [`Host`](crate::host::Host) is `Send`; the page's registry is a
//! signal of the window's. [`use_extension_sections`] is the bridge: a
//! [`SectionSink`] the host writes to from any thread, whose changes the
//! window applies to `architect-settings`' registry — an extension's
//! sections come when it loads, change when it reloads (values kept), and
//! go when it unloads.

use std::cell::RefCell;
use std::rc::Rc;

use architect_settings::{Section, try_use_settings};
use dioxus::prelude::*;
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

use crate::host::{SectionSink, with_sections};

/// A change to the page, from the host.
#[derive(Debug)]
enum Change {
    Put(String, Vec<Section>),
    Withdraw(String),
}

/// The settings page, as a host sees it.
#[derive(Clone, Debug)]
pub struct UiSections(UnboundedSender<Change>);

impl SectionSink for UiSections {
    fn put(&self, owner: &str, sections: Vec<Section>) -> Result<(), String> {
        self.0
            .unbounded_send(Change::Put(owner.to_owned(), sections))
            .map_err(|_| "the settings page is gone".to_owned())
    }

    fn withdraw(&self, owner: &str) {
        let _ = self.0.unbounded_send(Change::Withdraw(owner.to_owned()));
    }
}

/// The settings page above (`use_settings_provider`), as a host's
/// [`SectionSink`]: give it to `Host::new`.
#[must_use = "give it to `Host::new`"]
pub fn use_extension_sections() -> UiSections {
    let handle = try_use_settings();
    let (sink, receiver) = use_hook(|| {
        let (tx, rx) = unbounded::<Change>();
        (UiSections(tx), Rc::new(RefCell::new(Some(rx))))
    });
    use_future(move || {
        let receiver: Option<UnboundedReceiver<Change>> = receiver.borrow_mut().take();
        async move {
            let (Some(mut rx), Some(h)) = (receiver, handle) else {
                return;
            };
            while let Some(change) = rx.next().await {
                match change {
                    Change::Put(owner, sections) => {
                        let next = with_sections(&h.registry.peek(), &owner, sections);
                        match next {
                            Ok(registry) => {
                                let mut signal = h.registry;
                                signal.set(registry);
                            }
                            Err(why) => {
                                let _span = tracing::info_span!(
                                    "ext.sections",
                                    ext.id = %owner,
                                    ext.outcome = "refused",
                                    ext.why = %why,
                                )
                                .entered();
                            }
                        }
                    }
                    Change::Withdraw(owner) => h.remove_extension(&owner),
                }
            }
        }
    });
    sink
}

/// Extensions' settings values as the settings page above has them
/// ([`use_page_setting_values`]).
///
/// A host reads the page's values and its writes go back to the page,
/// whose store keeps them: a file on a desk, the browser's storage in the
/// web app (#495).
#[derive(Clone, Default)]
pub struct PageValues {
    now: std::sync::Arc<std::sync::Mutex<std::collections::BTreeMap<String, serde_json::Value>>>,
    writes: std::sync::Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>>,
}

impl std::fmt::Debug for PageValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageValues").finish_non_exhaustive()
    }
}

fn locked<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl crate::hostapi::SettingValues for PageValues {
    fn get(&self, key: &str) -> Option<serde_json::Value> {
        locked(&self.now).get(key).cloned()
    }

    fn set(&self, key: &str, value: serde_json::Value) -> Result<(), String> {
        locked(&self.now).insert(key.to_owned(), value.clone());
        locked(&self.writes).push((key.to_owned(), value));
        Ok(())
    }
}

/// A JSON value as a setting's.
fn setting_value(v: &serde_json::Value) -> Option<architect_settings::Value> {
    match v {
        serde_json::Value::Bool(b) => Some(architect_settings::Value::Bool(*b)),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map(architect_settings::Value::Int)
            .or_else(|| n.as_f64().map(architect_settings::Value::Float)),
        serde_json::Value::String(t) => Some(architect_settings::Value::Text(t.clone())),
        _ => None,
    }
}

fn setting_json(v: &architect_settings::Value) -> serde_json::Value {
    match v {
        architect_settings::Value::Bool(b) => serde_json::Value::Bool(*b),
        architect_settings::Value::Int(i) => serde_json::json!(i),
        architect_settings::Value::Float(f) => serde_json::json!(f),
        architect_settings::Value::Text(t) => serde_json::Value::String(t.clone()),
    }
}

/// The settings page above's values of extensions' settings, for a host
/// (`Host::with_setting_values`): kept in step with the page as it
/// changes, a host's writes set on the page.
#[must_use = "give it to `Host::with_setting_values`"]
pub fn use_page_setting_values() -> PageValues {
    let values = use_hook(PageValues::default);
    let Some(h) = try_use_settings() else {
        return values;
    };
    let copy = values.clone();
    use_effect(move || {
        let registry = h.registry.read();
        let now = h.values.read();
        let mut out = std::collections::BTreeMap::new();
        for section in registry.sections() {
            if !matches!(section.owner, architect_settings::Owner::Extension(_)) {
                continue;
            }
            for setting in section.settings() {
                if let Some(v) = now.get(&registry, &setting.key) {
                    out.insert(setting.key.clone(), setting_json(&v));
                }
            }
        }
        *locked(&copy.now) = out;
    });
    let writes = values.clone();
    use_future(move || {
        let writes = writes.clone();
        async move {
            loop {
                futures_timer::Delay::new(std::time::Duration::from_millis(150)).await;
                let queued: Vec<_> = std::mem::take(&mut *locked(&writes.writes));
                for (key, value) in queued {
                    if let Some(v) = setting_value(&value) {
                        h.set(&key, v);
                    }
                }
            }
        }
    });
    values
}
