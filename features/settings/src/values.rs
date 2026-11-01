//! The values set: anything not set reads as its default; a value is
//! checked against its setting before it is kept.

use std::collections::{BTreeMap, BTreeSet};

use facet::Facet;

use crate::model::{Control, Scope, Setting, Value};
use crate::registry::Registry;

/// Why a value was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueError {
    UnknownKey(String),
    /// Not the kind the setting takes, or not one of its choices.
    Refused {
        key: String,
        why: String,
    },
    /// The person's organization has set it for everyone.
    Locked(String),
}

/// The values set, by key: what is kept (per scope, by the app).
#[derive(Debug, Clone, Default, PartialEq, Facet)]
pub struct Values {
    pub set: BTreeMap<String, Value>,
}

impl Values {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `key`'s value, its default when not set (or no longer valid).
    #[must_use]
    pub fn get(&self, registry: &Registry, key: &str) -> Option<Value> {
        let (_, setting) = registry.setting(key)?;
        Some(
            self.set
                .get(key)
                .filter(|v| check(setting, v).is_ok())
                .cloned()
                .unwrap_or_else(|| setting.default.clone()),
        )
    }

    #[must_use]
    pub fn get_bool(&self, registry: &Registry, key: &str) -> bool {
        self.get(registry, key)
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// Sets `key`, numbers kept inside their range. Setting the default
    /// clears it.
    ///
    /// # Errors
    ///
    /// [`ValueError`]: no such setting, or a value it does not take.
    // By value: a value is built where it is set (`Value::Bool(true)`).
    #[allow(clippy::needless_pass_by_value)]
    pub fn set(&mut self, registry: &Registry, key: &str, value: Value) -> Result<(), ValueError> {
        let (_, setting) = registry
            .setting(key)
            .ok_or_else(|| ValueError::UnknownKey(key.to_owned()))?;
        let value = check(setting, &value)?;
        if value == setting.default {
            self.set.remove(key);
        } else {
            self.set.insert(key.to_owned(), value);
        }
        Ok(())
    }

    /// Puts `key` back to its default.
    pub fn reset(&mut self, key: &str) {
        self.set.remove(key);
    }

    /// Whether `key` differs from its default.
    #[must_use]
    pub fn is_changed(&self, key: &str) -> bool {
        self.set.contains_key(key)
    }

    /// The settings of section `id` that differ from their defaults.
    #[must_use]
    pub fn changed_in<'r>(&self, registry: &'r Registry, id: &str) -> Vec<&'r Setting> {
        registry
            .section(id)
            .map(|s| s.settings().filter(|t| self.is_changed(&t.key)).collect())
            .unwrap_or_default()
    }

    /// Every setting of section `id` back to its default (Restore
    /// defaults). How many changed.
    pub fn restore_section(&mut self, registry: &Registry, id: &str) -> usize {
        let keys: Vec<String> = self
            .changed_in(registry, id)
            .into_iter()
            .map(|t| t.key.clone())
            .collect();
        for k in &keys {
            self.set.remove(k);
        }
        keys.len()
    }

    /// Serialised (facet JSON), for a store.
    ///
    /// # Errors
    ///
    /// The encoder's message.
    pub fn to_json(&self) -> Result<String, String> {
        facet_json::to_string(self).map_err(|e| e.to_string())
    }

    /// Read back. A value an extension no longer declares is kept (it may
    /// load again), and read as nothing until it does.
    ///
    /// # Errors
    ///
    /// The decoder's message.
    pub fn from_json(json: &str) -> Result<Self, String> {
        facet_json::from_str(json).map_err(|e| e.to_string())
    }
}

/// An organization's settings (`architect-orgs`): what an admin sets as
/// everyone's default, and which of those no member may change. Which
/// org's applies is the app's to say (the window's current org).
#[derive(Debug, Clone, Default, PartialEq, Facet)]
pub struct OrgSettings {
    /// A default in place of the one declared, by key.
    pub defaults: BTreeMap<String, Value>,
    /// Keys a member cannot change: they read the org's default (or the
    /// declared one).
    pub locked: BTreeSet<String>,
}

impl OrgSettings {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `key` defaults to `value` in this org.
    #[must_use]
    pub fn default_to(mut self, key: impl Into<String>, value: Value) -> Self {
        self.defaults.insert(key.into(), value);
        self
    }

    /// No member may change `key`.
    #[must_use]
    pub fn lock(mut self, key: impl Into<String>) -> Self {
        self.locked.insert(key.into());
        self
    }

    #[must_use]
    pub fn is_locked(&self, key: &str) -> bool {
        self.locked.contains(key)
    }
}

/// Everything set, by where it lives.
///
/// This device's values, the person's (on every device), the open
/// project's, and the org's defaults and locks under them. A setting's [`Scope`] says which of the
/// three holds it; a value reads, in order, the org's lock, its own
/// scope's value, the org's default, the declared default.
#[derive(Debug, Clone, Default, PartialEq, Facet)]
pub struct ScopedValues {
    pub device: Values,
    pub synced: Values,
    pub project: Values,
    pub org: OrgSettings,
}

impl ScopedValues {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The values of `scope`.
    #[must_use]
    pub const fn of(&self, scope: Scope) -> &Values {
        match scope {
            Scope::Device => &self.device,
            Scope::Synced => &self.synced,
            Scope::Project => &self.project,
        }
    }

    const fn of_mut(&mut self, scope: Scope) -> &mut Values {
        match scope {
            Scope::Device => &mut self.device,
            Scope::Synced => &mut self.synced,
            Scope::Project => &mut self.project,
        }
    }

    /// Puts `values` in as `scope`'s (they came from its store: loaded,
    /// or changed on another device).
    pub fn replace(&mut self, scope: Scope, values: Values) {
        *self.of_mut(scope) = values;
    }

    /// `key`'s default: the org's, when it set one `key` takes, else the
    /// declared one.
    #[must_use]
    pub fn default_of(&self, registry: &Registry, key: &str) -> Option<Value> {
        let (_, setting) = registry.setting(key)?;
        Some(
            self.org
                .defaults
                .get(key)
                .and_then(|v| check(setting, v).ok())
                .unwrap_or_else(|| setting.default.clone()),
        )
    }

    /// `key`'s value: the org's when it is locked; else what its scope
    /// holds (if still valid); else its default.
    #[must_use]
    pub fn get(&self, registry: &Registry, key: &str) -> Option<Value> {
        let (_, setting) = registry.setting(key)?;
        if self.org.is_locked(key) {
            return self.default_of(registry, key);
        }
        self.of(setting.scope)
            .set
            .get(key)
            .filter(|v| check(setting, v).is_ok())
            .cloned()
            .or_else(|| self.default_of(registry, key))
    }

    /// Sets `key` in its scope, numbers kept inside their range; setting
    /// its default clears it. The scope it went to.
    ///
    /// # Errors
    ///
    /// [`ValueError`]: no such setting, a value it does not take, or the
    /// org has locked it.
    // By value, as `Values::set`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn set(
        &mut self,
        registry: &Registry,
        key: &str,
        value: Value,
    ) -> Result<Scope, ValueError> {
        let (_, setting) = registry
            .setting(key)
            .ok_or_else(|| ValueError::UnknownKey(key.to_owned()))?;
        if self.org.is_locked(key) {
            return Err(ValueError::Locked(key.to_owned()));
        }
        let value = check(setting, &value)?;
        let scope = setting.scope;
        if Some(&value) == self.default_of(registry, key).as_ref() {
            self.of_mut(scope).set.remove(key);
        } else {
            self.of_mut(scope).set.insert(key.to_owned(), value);
        }
        Ok(scope)
    }

    /// Puts `key` back to its default; the scope it was in.
    pub fn reset(&mut self, registry: &Registry, key: &str) -> Option<Scope> {
        let (_, setting) = registry.setting(key)?;
        self.of_mut(setting.scope)
            .set
            .remove(key)
            .map(|_| setting.scope)
    }

    /// Whether `key` differs from its default (a locked one never does:
    /// the member did not choose it).
    #[must_use]
    pub fn is_changed(&self, registry: &Registry, key: &str) -> bool {
        !self.org.is_locked(key)
            && registry
                .setting(key)
                .is_some_and(|(_, s)| self.of(s.scope).set.contains_key(key))
    }

    #[must_use]
    pub fn is_locked(&self, key: &str) -> bool {
        self.org.is_locked(key)
    }

    /// The settings of section `id` that differ from their defaults.
    #[must_use]
    pub fn changed_in<'r>(&self, registry: &'r Registry, id: &str) -> Vec<&'r Setting> {
        registry
            .section(id)
            .map(|s| {
                s.settings()
                    .filter(|t| self.is_changed(registry, &t.key))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every setting of section `id` back to its default (Restore
    /// defaults): the scopes that changed, each once.
    pub fn restore_section(&mut self, registry: &Registry, id: &str) -> Vec<Scope> {
        let keys: Vec<String> = self
            .changed_in(registry, id)
            .into_iter()
            .map(|t| t.key.clone())
            .collect();
        let mut scopes = Vec::new();
        for k in &keys {
            if let Some(scope) = self.reset(registry, k) {
                if !scopes.contains(&scope) {
                    scopes.push(scope);
                }
            }
        }
        scopes
    }
}

/// `value` as `setting` keeps it: the right kind, one of its choices, a
/// number inside its range (snapped to its step).
fn check(setting: &Setting, value: &Value) -> Result<Value, ValueError> {
    let refused = |why: &str| ValueError::Refused {
        key: setting.key.clone(),
        why: why.to_owned(),
    };
    match (&setting.control, value) {
        (Control::Switch, Value::Bool(_))
        | (Control::Text { .. } | Control::Shortcut, Value::Text(_)) => Ok(value.clone()),
        (Control::Select(c) | Control::Segmented(c) | Control::Cards(c), v) => c
            .iter()
            .any(|choice| &choice.value == v)
            .then(|| v.clone())
            .ok_or_else(|| refused("not one of its choices")),
        (Control::Slider { min, max, step, .. }, Value::Float(f)) => {
            Ok(Value::Float(snap_f(*f, *min, *max, *step)))
        }
        (Control::Slider { min, max, step, .. }, Value::Int(i)) => {
            #[allow(clippy::cast_precision_loss, clippy::as_conversions)]
            let f = *i as f64;
            Ok(Value::Float(snap_f(f, *min, *max, *step)))
        }
        (Control::Stepper { min, max, step, .. }, Value::Int(i)) => {
            Ok(Value::Int(snap_i(*i, *min, *max, *step)))
        }
        _ => Err(refused("not the kind of value it takes")),
    }
}

fn snap_f(v: f64, min: f64, max: f64, step: f64) -> f64 {
    let v = v.clamp(min, max);
    if step > 0.0 {
        ((v - min) / step)
            .round()
            .mul_add(step, min)
            .clamp(min, max)
    } else {
        v
    }
}

fn snap_i(v: i64, min: i64, max: i64, step: i64) -> i64 {
    let v = v.clamp(min, max);
    if step > 0 {
        let off = v.saturating_sub(min);
        let n = off.checked_div(step).unwrap_or(0);
        let rest = off.checked_rem(step).unwrap_or(0);
        let n = if rest.saturating_mul(2) >= step {
            n.saturating_add(1)
        } else {
            n
        };
        min.saturating_add(n.saturating_mul(step)).clamp(min, max)
    } else {
        v
    }
}
