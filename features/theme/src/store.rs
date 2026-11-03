//! The cascade ([`Layers`]) and the one [`ThemeStore`] an app keeps: it
//! composes the layers, resolves the theme they choose, and tells its
//! subscribers once per real change (herdr's compose, set,
//! broadcast-if-changed).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use crate::model::{Appearance, Problem, Registry, Scheme, Source, Theme, ThemeChoice};
use crate::resolve::{Resolved, resolve_in};

/// A layer of the cascade, lowest first: each one's choice, appearance
/// and overrides win over those below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LayerKind {
    /// architect's: `FastTrackStudio` light and dark, by the system.
    Builtin,
    /// What loaded extensions contribute: their own tokens' defaults
    /// (`clicker.face`), under everything the app and the person say, and
    /// under the theme's own overrides too, so a theme file restyles an
    /// extension ([`ThemeStore::contribute`]).
    Extension,
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
    pub const ALL: [Self; 9] = [
        Self::Builtin,
        Self::Extension,
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
            Self::Extension => "extension",
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
    /// Tokens by name, over the theme's: literals or expressions
    /// (`crate::expr`), evaluated after every layer is in.
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
        let mut chosen_by = LayerKind::Builtin;
        let mut appearance = Appearance::System;
        let mut overrides = BTreeMap::new();
        let mut set_by = BTreeMap::new();
        for (kind, layer) in &self.0 {
            if let Some(c) = &layer.choice {
                choice = c.clone();
                chosen_by = *kind;
            }
            if let Some(a) = layer.appearance {
                appearance = a;
            }
            for (k, v) in &layer.overrides {
                overrides.insert(k.clone(), v.clone());
                set_by.insert(k.clone(), *kind);
            }
        }
        let scheme = appearance.scheme(system);
        let theme = choice.pick(scheme).to_owned();
        Composed {
            choice,
            chosen_by,
            appearance,
            scheme,
            theme,
            overrides,
            set_by,
        }
    }
}

/// The layers, composed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composed {
    pub choice: ThemeChoice,
    /// The layer whose choice it is.
    pub chosen_by: LayerKind,
    pub appearance: Appearance,
    /// The scheme the appearance asks for (a single choice's theme may
    /// be the other).
    pub scheme: Scheme,
    /// The theme it shows.
    pub theme: String,
    /// Every layer's overrides, the higher over the lower.
    pub overrides: BTreeMap<String, String>,
    /// The layer each override is from.
    pub set_by: BTreeMap<String, LayerKind>,
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

/// What one owner other than the app (an extension, by its id) adds to
/// the themes while it is there ([`ThemeStore::contribute`]): themes,
/// and its own tokens' defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contribution {
    /// Its themes ([`Source::Extension`] when they are an extension's
    /// files).
    pub themes: Vec<Theme>,
    /// The theme files that did not read: the theme's id and why. The
    /// owner's last good theme of that id stays.
    pub failed: Vec<(String, Vec<Problem>)>,
    /// Its tokens' defaults by name, literals or expressions
    /// (`"clicker.face": "mix($card, $brand, 0.12)"`): the
    /// [`LayerKind::Extension`] layer, under the app's and the person's.
    pub tokens: BTreeMap<String, String>,
}

impl Contribution {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.themes.is_empty() && self.failed.is_empty() && self.tokens.is_empty()
    }
}

struct State {
    /// The app's themes: built in, its folders, the ones it built.
    registry: Registry,
    /// Every owner's contribution, by owner.
    contributions: BTreeMap<String, Contribution>,
    /// `registry` and the contributed themes: what everything reads.
    full: Registry,
    /// The contributions' problems: files that did not read, an id or a
    /// token someone else has.
    contributed: Vec<Problem>,
    layers: Layers,
    system: Scheme,
    composed: Composed,
    current: Resolved,
    missing: Option<Problem>,
}

impl State {
    /// `full` and `contributed` again, from `registry` and the
    /// contributions; the contributions' tokens as the extension layer.
    fn merge(&mut self) -> Layers {
        let mut full = self.registry.clone();
        let mut problems = Vec::new();
        let mut tokens: BTreeMap<String, String> = BTreeMap::new();
        let mut token_owner: BTreeMap<String, String> = BTreeMap::new();
        for (owner, c) in &self.contributions {
            for (_, why) in &c.failed {
                problems.extend(why.iter().cloned());
            }
            for theme in &c.themes {
                if full.contains(&theme.id) {
                    problems.push(
                        Problem::new(
                            &theme.id,
                            &theme.source,
                            format!("{owner}: another theme has the id {:?}", theme.id),
                        )
                        .at("id"),
                    );
                } else {
                    full.insert(theme.clone());
                }
            }
            for (name, value) in &c.tokens {
                if let Some(first) = token_owner.get(name) {
                    problems.push(Problem {
                        theme: owner.clone(),
                        source: format!("extension {owner}"),
                        field: Some(format!("tokens.{name}")),
                        message: format!("{first} has this token already"),
                    });
                    continue;
                }
                token_owner.insert(name.clone(), owner.clone());
                tokens.insert(name.clone(), value.clone());
            }
        }
        self.full = full;
        self.contributed = problems;
        let mut layers = self.layers.clone();
        if !tokens.is_empty() {
            // What the app set on the extension layer itself wins.
            let mut layer = layers.get(LayerKind::Extension);
            for (name, value) in tokens {
                layer.overrides.entry(name).or_insert(value);
            }
            layers.set(LayerKind::Extension, layer);
        }
        layers
    }
}

struct Inner {
    state: Mutex<State>,
    /// `state.current`'s revision, readable without the lock.
    revision: AtomicU64,
    listeners: Mutex<(u64, Vec<(u64, Listener)>)>,
    reloader: Mutex<Option<Reloader>>,
}

/// An app's themes: the registry, the layers, the system's scheme, and
/// the theme they resolve to.
///
/// One per app; clones share it. `Send` and `Sync`: the Action List's
/// handlers, a file watcher, a worker and the window all hold it. A
/// reader on any thread gets a whole [`Resolved`] ([`Self::current`], an
/// `Arc` taken under the lock), and [`Self::revision`] is an atomic a UI
/// thread polls per frame.
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
                    full: registry.clone(),
                    registry,
                    contributions: BTreeMap::new(),
                    contributed: Vec::new(),
                    layers,
                    system,
                    composed,
                    current,
                    missing,
                }),
                listeners: Mutex::new((0, Vec::new())),
                reloader: Mutex::new(None),
                revision: AtomicU64::new(0),
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

    /// The revision of the theme showing; lock-free, cheap enough to poll
    /// every frame. It changes after [`Self::current`] does.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.inner.revision.load(Ordering::Acquire)
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

    /// Every theme it knows: the app's and the contributed ones.
    #[must_use]
    pub fn registry(&self) -> Registry {
        self.lock().full.clone()
    }

    /// Every theme, by id.
    #[must_use]
    pub fn themes(&self) -> Vec<ThemeInfo> {
        let s = self.lock();
        let mut problems = s.full.problems();
        problems.extend(s.contributed.iter().cloned());
        s.full
            .iter()
            .map(|t: &Theme| ThemeInfo {
                id: t.id.clone(),
                label: t.label.clone(),
                scheme: s.full.scheme_of(&t.id),
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
    /// not there, and the token expressions that did not resolve (an
    /// unknown `$name`, a cycle), each drawn magenta.
    #[must_use]
    pub fn problems(&self) -> Vec<Problem> {
        let s = self.lock();
        let mut out = s.full.problems();
        out.extend(s.contributed.iter().cloned());
        out.extend(s.missing.clone());
        out.extend(s.current.problems().iter().cloned());
        out
    }

    /// Changes the store with `f`, then composes and resolves again; true
    /// (and the revision bumped, every listener told once) only when what
    /// shows changed.
    fn apply(&self, f: impl FnOnce(&mut State)) -> bool {
        let (changed, current) = {
            let mut s = self.lock();
            f(&mut s);
            let layers = s.merge();
            let (composed, resolved, missing) = evaluate(&s.full, &layers, s.system);
            s.missing = missing;
            let changed = composed != s.composed || !resolved.same_look(&s.current);
            if changed {
                let revision = s.current.revision().saturating_add(1);
                s.composed = composed;
                s.current = resolved.at_revision(revision);
                self.inner.revision.store(revision, Ordering::Release);
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
            .full
            .scheme_of(id)
            .ok_or_else(|| format!("no theme {id:?}"))?;
        let choice = self.composed().choice.with_pick(id, scheme);
        Ok(self.update(LayerKind::Pick, |l| l.choice = Some(choice)))
    }

    /// Shows `id` until [`Self::end_preview`], over every other layer.
    pub fn preview(&self, id: &str) -> Result<bool, String> {
        if !self.lock().full.contains(id) {
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

    /// Adds `theme`, or replaces the one of its id, while the app runs
    /// (an import, a document's variables: [`Theme::from_tokens`]). The
    /// theme showing is resolved again; true, and the revision bumped,
    /// only when what shows changed.
    pub fn insert_theme(&self, theme: Theme) -> bool {
        self.apply(|s| s.registry.insert(theme))
    }

    /// Removes theme `id` (a chosen one falls back to the built-in
    /// default, with the problem said).
    pub fn remove_theme(&self, id: &str) -> bool {
        self.apply(|s| s.registry.remove(id))
    }

    /// Puts `owner`'s contribution (an extension's themes and tokens,
    /// as it loads or reloads) in place of the one it had: a diff, so a
    /// theme or a token it no longer has goes, and the revision moves
    /// only when what shows changed. A file in [`Contribution::failed`]
    /// keeps the owner's last good theme of its id, and is a problem
    /// ([`Self::problems`]). A theme whose id the app (or an earlier
    /// owner) has, or a token another owner set, is left out, with the
    /// problem said.
    pub fn contribute(&self, owner: &str, contribution: Contribution) -> bool {
        self.apply(|s| {
            let mut next = contribution;
            if let Some(before) = s.contributions.get(owner) {
                for (id, _) in &next.failed {
                    if next.themes.iter().any(|t| &t.id == id) {
                        continue;
                    }
                    if let Some(good) = before.themes.iter().find(|t| &t.id == id) {
                        next.themes.push(good.clone());
                    }
                }
            }
            if next.is_empty() {
                s.contributions.remove(owner);
            } else {
                s.contributions.insert(owner.to_owned(), next);
            }
        })
    }

    /// Takes everything `owner` contributed away (an extension unloads):
    /// a theme of its that shows falls back to the built-in default, with
    /// the problem said.
    pub fn withdraw(&self, owner: &str) -> bool {
        self.apply(|s| {
            s.contributions.remove(owner);
        })
    }

    /// What `owner` contributes now.
    #[must_use]
    pub fn contribution(&self, owner: &str) -> Option<Contribution> {
        self.lock().contributions.get(owner).cloned()
    }

    /// How [`Self::reload`] re-reads the themes (`fs::ThemeDirs` gives one).
    pub fn set_reloader(&self, reloader: Reloader) {
        *lock(&self.inner.reloader) = Some(reloader);
    }

    /// Re-reads the themes with the reloader (the in-memory ones,
    /// [`Source::Memory`], are kept); false when there is none or nothing
    /// that shows changed.
    pub fn reload(&self) -> bool {
        let Some(reloader) = lock(&self.inner.reloader).clone() else {
            return false;
        };
        let previous = self.lock().registry.clone();
        let mut next = reloader(&previous);
        // What the app built in memory is not on disk: it stays.
        for theme in previous.iter().filter(|t| t.source == Source::Memory) {
            next.insert(theme.clone());
        }
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
    let chosen_by = if missing.is_some() {
        LayerKind::Builtin
    } else {
        composed.chosen_by
    };
    let resolved = resolve_in(
        &theme,
        registry,
        Some(chosen_by),
        &composed.overrides,
        &composed.set_by,
    );
    (composed, resolved, missing)
}
