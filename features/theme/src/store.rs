//! The cascade ([`Layers`]) and the one [`ThemeStore`] an app keeps: it
//! composes the layers, resolves the theme they choose, and tells its
//! subscribers once per real change (herdr's compose, set,
//! broadcast-if-changed).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use crate::model::{Appearance, Problem, Registry, Scheme, Source, Theme, ThemeChoice};
use crate::resolve::{Resolved, resolve};

/// A layer of the cascade, lowest first: each one's choice, appearance
/// and overrides win over those below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LayerKind {
    /// architect's: `FastTrackStudio` light and dark, by the system.
    Builtin,
    /// The app's default (Studio's own pair).
    App,
    /// The organization's default.
    Org,
    /// The person's setting, on every device.
    Person,
    /// This device's.
    Device,
    /// The workspace's or screenset's look.
    Workspace,
    /// An explicit pick: a picker, `THEME_SET`.
    Pick,
    /// A transient preview (hovering a theme card, `THEME_PREVIEW`).
    Preview,
}

impl LayerKind {
    pub const ALL: [Self; 8] = [
        Self::Builtin,
        Self::App,
        Self::Org,
        Self::Person,
        Self::Device,
        Self::Workspace,
        Self::Pick,
        Self::Preview,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::App => "app",
            Self::Org => "org",
            Self::Person => "person",
            Self::Device => "device",
            Self::Workspace => "workspace",
            Self::Pick => "pick",
            Self::Preview => "preview",
        }
    }

    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == name)
    }
}

/// What one layer says. Everything is optional: a layer that says
/// nothing leaves the one below it showing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layer {
    pub choice: Option<ThemeChoice>,
    pub appearance: Option<Appearance>,
    /// Tokens by name, over the theme's.
    pub overrides: BTreeMap<String, String>,
}

impl Layer {
    #[must_use]
    pub fn choice(mut self, choice: ThemeChoice) -> Self {
        self.choice = Some(choice);
        self
    }

    #[must_use]
    pub const fn appearance(mut self, appearance: Appearance) -> Self {
        self.appearance = Some(appearance);
        self
    }

    #[must_use]
    pub fn token(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.overrides.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.choice.is_none() && self.appearance.is_none() && self.overrides.is_empty()
    }
}

/// Every layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layers(BTreeMap<LayerKind, Layer>);

impl Default for Layers {
    /// The built-in layer alone: `FastTrackStudio`'s pair, by the system.
    fn default() -> Self {
        Self(BTreeMap::from([(
            LayerKind::Builtin,
            Layer::default()
                .choice(ThemeChoice::pair(
                    crate::builtin::DEFAULT_LIGHT,
                    crate::builtin::DEFAULT_DARK,
                ))
                .appearance(Appearance::System),
        )]))
    }
}

impl Layers {
    #[must_use]
    pub fn get(&self, kind: LayerKind) -> Layer {
        self.0.get(&kind).cloned().unwrap_or_default()
    }

    pub fn set(&mut self, kind: LayerKind, layer: Layer) {
        if layer.is_empty() {
            self.0.remove(&kind);
        } else {
            self.0.insert(kind, layer);
        }
    }

    /// The layers composed, with the system's scheme `system`.
    #[must_use]
    pub fn compose(&self, system: Scheme) -> Composed {
        let mut choice =
            ThemeChoice::pair(crate::builtin::DEFAULT_LIGHT, crate::builtin::DEFAULT_DARK);
        let mut appearance = Appearance::System;
        let mut overrides = BTreeMap::new();
        for layer in self.0.values() {
            if let Some(c) = &layer.choice {
                choice = c.clone();
            }
            if let Some(a) = layer.appearance {
                appearance = a;
            }
            overrides.extend(layer.overrides.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        let scheme = appearance.scheme(system);
        let theme = choice.pick(scheme).to_owned();
        Composed {
            choice,
            appearance,
            scheme,
            theme,
            overrides,
        }
    }
}

/// The layers, composed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composed {
    pub choice: ThemeChoice,
    pub appearance: Appearance,
    /// The scheme the appearance asks for (a single choice's theme may
    /// be the other).
    pub scheme: Scheme,
    /// The theme it shows.
    pub theme: String,
    /// Every layer's overrides, the higher over the lower.
    pub overrides: BTreeMap<String, String>,
}

/// One theme the store knows, as a picker or `THEME_LIST` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeInfo {
    pub id: String,
    pub label: String,
    pub scheme: Option<Scheme>,
    pub source: Source,
    pub problems: Vec<String>,
}

/// Re-reads the themes: given the registry as it is, the new one (a
/// theme whose file broke is kept as it was: last good).
pub type Reloader = Arc<dyn Fn(&Registry) -> Registry + Send + Sync>;

type Listener = Arc<dyn Fn(&Resolved) + Send + Sync>;

struct State {
    registry: Registry,
    layers: Layers,
    system: Scheme,
    composed: Composed,
    current: Resolved,
    missing: Option<Problem>,
}

struct Inner {
    state: Mutex<State>,
    listeners: Mutex<(u64, Vec<(u64, Listener)>)>,
    reloader: Mutex<Option<Reloader>>,
}

/// An app's themes: the registry, the layers, the system's scheme, and
/// the theme they resolve to.
///
/// One per app; clones share it. `Send` and
/// `Sync`: the Action List's handlers, a file watcher and the window all
/// hold it.
#[derive(Clone)]
pub struct ThemeStore {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for ThemeStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.lock();
        f.debug_struct("ThemeStore")
            .field("theme", &s.composed.theme)
            .field("revision", &s.current.revision())
            .finish_non_exhaustive()
    }
}

impl PartialEq for ThemeStore {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Default for ThemeStore {
    /// architect's built-in themes, the built-in layer, a dark system.
    fn default() -> Self {
        Self::new(Registry::builtin())
    }
}

impl ThemeStore {
    /// A store over `registry`, with only the built-in layer, the system
    /// dark until the host says ([`Self::set_system`]).
    #[must_use]
    pub fn new(registry: Registry) -> Self {
        let layers = Layers::default();
        let system = Scheme::Dark;
        let (composed, current, missing) = evaluate(&registry, &layers, system);
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    registry,
                    layers,
                    system,
                    composed,
                    current,
                    missing,
                }),
                listeners: Mutex::new((0, Vec::new())),
                reloader: Mutex::new(None),
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        lock(&self.inner.state)
    }

    /// The theme showing.
    #[must_use]
    pub fn current(&self) -> Resolved {
        self.lock().current.clone()
    }

    #[must_use]
    pub fn revision(&self) -> u64 {
        self.lock().current.revision()
    }

    /// The layers, composed.
    #[must_use]
    pub fn composed(&self) -> Composed {
        self.lock().composed.clone()
    }

    #[must_use]
    pub fn layer(&self, kind: LayerKind) -> Layer {
        self.lock().layers.get(kind)
    }

    #[must_use]
    pub fn system(&self) -> Scheme {
        self.lock().system
    }

    #[must_use]
    pub fn registry(&self) -> Registry {
        self.lock().registry.clone()
    }

    /// Every theme, by id.
    #[must_use]
    pub fn themes(&self) -> Vec<ThemeInfo> {
        let s = self.lock();
        let problems = s.registry.problems();
        s.registry
            .iter()
            .map(|t: &Theme| ThemeInfo {
                id: t.id.clone(),
                label: t.label.clone(),
                scheme: s.registry.scheme_of(&t.id),
                source: t.source.clone(),
                problems: problems
                    .iter()
                    .filter(|p| p.theme == t.id)
                    .map(ToString::to_string)
                    .collect(),
            })
            .collect()
    }

    /// Everything wrong: theme files, parents, a chosen theme that is
    /// not there.
    #[must_use]
    pub fn problems(&self) -> Vec<Problem> {
        let s = self.lock();
        let mut out = s.registry.problems();
        out.extend(s.missing.clone());
        out
    }

    /// Changes the store with `f`, then composes and resolves again; true
    /// (and the revision bumped, every listener told once) only when what
    /// shows changed.
    fn apply(&self, f: impl FnOnce(&mut State)) -> bool {
        let (changed, current) = {
            let mut s = self.lock();
            f(&mut s);
            let (composed, resolved, missing) = evaluate(&s.registry, &s.layers, s.system);
            s.missing = missing;
            let changed = composed != s.composed || !resolved.same_look(&s.current);
            if changed {
                let revision = s.current.revision().saturating_add(1);
                s.composed = composed;
                s.current = resolved.at_revision(revision);
            }
            (changed, s.current.clone())
        };
        if changed {
            let listeners: Vec<Listener> = lock(&self.inner.listeners)
                .1
                .iter()
                .map(|(_, l)| l.clone())
                .collect();
            for listener in listeners {
                listener(&current);
            }
        }
        changed
    }

    /// Puts `layer` at `kind`; false when nothing that shows changed.
    pub fn set(&self, kind: LayerKind, layer: Layer) -> bool {
        self.apply(|s| s.layers.set(kind, layer))
    }

    /// Changes the layer at `kind` with `f`.
    pub fn update(&self, kind: LayerKind, f: impl FnOnce(&mut Layer)) -> bool {
        self.apply(|s| {
            let mut layer = s.layers.get(kind);
            f(&mut layer);
            s.layers.set(kind, layer);
        })
    }

    /// Picks theme `id` (the [`LayerKind::Pick`] layer): it replaces the
    /// half of the choice showing now that matches its own lightness.
    pub fn pick(&self, id: &str) -> Result<bool, String> {
        let scheme = self
            .lock()
            .registry
            .scheme_of(id)
            .ok_or_else(|| format!("no theme {id:?}"))?;
        let choice = self.composed().choice.with_pick(id, scheme);
        Ok(self.update(LayerKind::Pick, |l| l.choice = Some(choice)))
    }

    /// Shows `id` until [`Self::end_preview`], over every other layer.
    pub fn preview(&self, id: &str) -> Result<bool, String> {
        if !self.lock().registry.contains(id) {
            return Err(format!("no theme {id:?}"));
        }
        Ok(self.update(LayerKind::Preview, |l| {
            l.choice = Some(ThemeChoice::One(id.to_owned()));
        }))
    }

    pub fn end_preview(&self) -> bool {
        self.set(LayerKind::Preview, Layer::default())
    }

    /// Token `name` set to `value` in layer `kind`.
    pub fn set_token(&self, kind: LayerKind, name: &str, value: &str) -> bool {
        self.update(kind, |l| {
            l.overrides.insert(name.to_owned(), value.to_owned());
        })
    }

    /// Token `name` (every one when `None`) back to what is under layer
    /// `kind`.
    pub fn reset_token(&self, kind: LayerKind, name: Option<&str>) -> bool {
        self.update(kind, |l| match name {
            Some(n) => {
                l.overrides.remove(n);
            }
            None => l.overrides.clear(),
        })
    }

    /// The OS's light or dark, for [`Appearance::System`]. The host calls
    /// it at start and whenever the OS changes (a Blitz window's
    /// `WindowEvent::ThemeChanged`, a browser's `prefers-color-scheme`).
    pub fn set_system(&self, scheme: Scheme) -> bool {
        self.apply(|s| s.system = scheme)
    }

    /// Replaces the themes (the theme showing is resolved again: a file
    /// may have new colours under the same id).
    pub fn set_registry(&self, registry: Registry) -> bool {
        self.apply(|s| s.registry = registry)
    }

    /// How [`Self::reload`] re-reads the themes (`fs::ThemeDirs` gives one).
    pub fn set_reloader(&self, reloader: Reloader) {
        *lock(&self.inner.reloader) = Some(reloader);
    }

    /// Re-reads the themes with the reloader; false when there is none or
    /// nothing that shows changed.
    pub fn reload(&self) -> bool {
        let Some(reloader) = lock(&self.inner.reloader).clone() else {
            return false;
        };
        let next = reloader(&self.registry());
        self.set_registry(next)
    }

    /// Calls `listener` with the theme after every change, until the
    /// [`Subscription`] is dropped. It runs on the thread that made the
    /// change.
    #[must_use]
    pub fn subscribe(&self, listener: impl Fn(&Resolved) + Send + Sync + 'static) -> Subscription {
        let id = {
            let mut l = lock(&self.inner.listeners);
            let id = l.0;
            l.0 = l.0.saturating_add(1);
            l.1.push((id, Arc::new(listener)));
            id
        };
        Subscription {
            store: Arc::downgrade(&self.inner),
            id,
        }
    }
}

/// A [`ThemeStore::subscribe`] listener, removed when this drops.
#[must_use]
pub struct Subscription {
    store: Weak<Inner>,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(inner) = self.store.upgrade() {
            lock(&inner.listeners).1.retain(|(id, _)| *id != self.id);
        }
    }
}

/// The layers composed and the theme they choose resolved; a chosen
/// theme that is not there falls back to the built-in default for the
/// scheme, with the problem said.
fn evaluate(
    registry: &Registry,
    layers: &Layers,
    system: Scheme,
) -> (Composed, Resolved, Option<Problem>) {
    let composed = layers.compose(system);
    let (theme, missing) = match registry.get(&composed.theme) {
        Some(theme) => (theme.clone(), None),
        None => (
            crate::builtin::fallback(composed.scheme),
            Some(
                Problem::new(
                    &composed.theme,
                    &Source::Builtin,
                    "chosen, but there is no such theme",
                )
                .at("choice"),
            ),
        ),
    };
    let resolved = resolve(&theme, registry).with_overrides(&composed.overrides);
    (composed, resolved, missing)
}
