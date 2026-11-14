//! The app's extensions, together (feature `host`).
//!
//! What is linked in,
//! installed and in development, which copy of each runs, the registries
//! to search and install from, the organization's policy, hot reload,
//! and what a project needs when it is opened or joined.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use semver::VersionReq;

use crate::catalog::{Candidate, Chosen, Origin, choose};
use crate::host::{Event, Host, LoadError};
use crate::index::{Hit, Listing};
use crate::install::{Install, InstallError, Installs, Source};
use crate::manifest::Kind;
use crate::package::Package;
use crate::project::{Plan, Policy, Requirements, plan};
use crate::watch::Watch;

/// A search hit and the registry it is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub source: String,
    pub hit: Hit,
}

struct Inner {
    host: Host,
    installs: Installs,
    sources: Mutex<Vec<Arc<dyn Source>>>,
    builtins: Mutex<Vec<Package>>,
    dev: Mutex<Vec<PathBuf>>,
    disabled: Mutex<BTreeSet<String>>,
    policy: Mutex<Option<Policy>>,
    watch: Mutex<Watch>,
    /// Where `EXT_CLONE` puts copies (`crate::clone`).
    dev_root: Mutex<Option<PathBuf>>,
    /// The last load that failed of each extension, until it loads or
    /// goes: what [`Extensions::problems`] reports.
    failed: Arc<Mutex<std::collections::BTreeMap<String, LoadError>>>,
    /// Development copies built from source (`crate::source`).
    builds: crate::source::Builds,
    /// The registries a person adds and the development switch
    /// (`crate::registries`), when the app keeps them.
    registries: Mutex<Option<crate::registries::Registries>>,
}

/// The topic of the `Changed` event a person installing or turning on an
/// extension is, its detail the extension's id.
///
/// Told once it runs, never at startup nor on a reload: what an app
/// offers the extension's tour on (`[[tours]]`).
pub const TURNED_ON: &str = "extensions.turned_on";

/// The app's extensions. Cheap to clone.
#[derive(Clone)]
pub struct Extensions {
    inner: Arc<Inner>,
}

impl PartialEq for Extensions {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl std::fmt::Debug for Extensions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Extensions")
            .field("host", &self.inner.host)
            .field("installs", &self.inner.installs)
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Extensions {
    /// Extensions loaded by `host`, installed under `installs` (for the
    /// host's platform: [`Host::set_platform`] comes first).
    #[must_use]
    pub fn new(host: Host, installs: Installs) -> Self {
        let installs = installs.with_platform(host.platform());
        let failed = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
        {
            let failed = Arc::clone(&failed);
            host.on_event(move |event| {
                let mut failed = lock(&failed);
                match event {
                    Event::Failed { id, error } => {
                        failed.insert(id.clone(), error.clone());
                    }
                    Event::Loaded(change) => {
                        failed.remove(&change.id);
                    }
                    Event::Unloaded { id } => {
                        failed.remove(id);
                    }
                }
            });
        }
        Self {
            inner: Arc::new(Inner {
                host,
                installs,
                sources: Mutex::new(Vec::new()),
                builtins: Mutex::new(Vec::new()),
                dev: Mutex::new(Vec::new()),
                disabled: Mutex::new(BTreeSet::new()),
                policy: Mutex::new(None),
                watch: Mutex::new(Watch::new()),
                dev_root: Mutex::new(None),
                failed,
                builds: crate::source::Builds::default(),
                registries: Mutex::new(None),
            }),
        }
    }

    /// Searches and installs from `source` too.
    #[must_use]
    pub fn with_source(self, source: impl Source + 'static) -> Self {
        self.add_source(source);
        self
    }

    pub fn add_source(&self, source: impl Source + 'static) {
        lock(&self.inner.sources).push(Arc::new(source));
    }

    /// Searches and installs from `registries` too (the store's index and
    /// the ones a person adds, each trusting its own publishers' keys),
    /// and puts `EXT_REGISTRY_*` and `EXT_DEV` on the list
    /// ([`Self::install_actions`]); the development folders it keeps are
    /// loaded when development installs are on.
    #[must_use]
    pub fn with_registries(self, registries: crate::registries::Registries) -> Self {
        if registries.dev_installs() {
            lock(&self.inner.dev).extend(
                registries
                    .dev_folders()
                    .into_iter()
                    .filter(|d| d.join(crate::manifest::MANIFEST).is_file()),
            );
        }
        *lock(&self.inner.registries) = Some(registries);
        self
    }

    /// The registries the app keeps, when it keeps them.
    #[must_use]
    pub fn registries(&self) -> Option<crate::registries::Registries> {
        lock(&self.inner.registries).clone()
    }

    /// Every publisher key a registry trusts: what a `.archext` from a
    /// file is checked against, past [`Installs::trust`].
    #[must_use]
    pub fn publishers(&self) -> architect_extension_archive::Trust {
        self.sources()
            .iter()
            .fold(architect_extension_archive::Trust::none(), |t, s| {
                t.and(&s.trust())
            })
    }

    /// A first-party extension linked into the build: it runs unless a
    /// newer one is installed, or a development copy is.
    #[must_use]
    pub fn with_builtin(self, package: Package) -> Self {
        lock(&self.inner.builtins).push(package);
        self
    }

    /// An in-process extension linked into the build (`crate::linked`):
    /// its manifest a built-in package, its code run here while that copy
    /// is the one that runs. An installed or development copy of its id
    /// overrides it (and runs as its kind says: a process, its program
    /// built from the same crate).
    ///
    /// # Errors
    ///
    /// Its manifest is refused.
    #[cfg(feature = "linked")]
    pub fn with_linked(
        self,
        extension: &dyn crate::linked::Extension,
    ) -> Result<Self, crate::ManifestError> {
        let package = Package::embedded(extension.manifest(), extension.files())?;
        // Not for this platform (#578): listed, its `build` never run (the
        // crate still compiles here, stubbing what it cannot do).
        let platform = self.inner.host.platform();
        if crate::platform::runs_on(&package.manifest.platforms, &platform) {
            self.inner
                .host
                .link(&package.manifest, crate::linked::Builder::of(extension));
        }
        Ok(self.with_builtin(package))
    }

    /// Installs only what `policy` allows (the organization's), from now.
    pub fn set_policy(&self, policy: Option<Policy>) {
        *lock(&self.inner.policy) = policy;
    }

    #[must_use]
    pub fn host(&self) -> &Host {
        &self.inner.host
    }

    #[must_use]
    pub fn installs(&self) -> &Installs {
        &self.inner.installs
    }

    fn sources(&self) -> Vec<Arc<dyn Source>> {
        let mut sources = lock(&self.inner.sources).clone();
        if let Some(r) = self.registries() {
            sources.extend(r.sources());
        }
        sources
    }

    /// Brings every registry up to date: the ones that failed, and why.
    #[must_use]
    pub fn refresh(&self) -> Vec<(String, InstallError)> {
        self.sources()
            .iter()
            .filter_map(|s| s.refresh().err().map(|e| (s.name(), e)))
            .collect()
    }

    /// What `query` finds in every registry, best first.
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<Found> {
        let mut found: Vec<Found> = self
            .sources()
            .iter()
            .filter_map(|s| s.index().ok().map(|i| (s.name(), i)))
            .flat_map(|(source, index)| {
                index.search(query).into_iter().map(move |hit| Found {
                    source: source.clone(),
                    hit,
                })
            })
            .collect();
        found.sort_by(|a, b| b.hit.score.cmp(&a.hit.score));
        found
    }

    /// Every listing in every registry.
    #[must_use]
    pub fn offers(&self) -> Vec<Listing> {
        self.sources()
            .iter()
            .filter_map(|s| s.index().ok())
            .flat_map(|i| i.listings)
            .collect()
    }

    /// The listing `req` picks for `id` (the policy's say included), and
    /// its source.
    pub(crate) fn find_listed(
        &self,
        id: &str,
        req: &VersionReq,
    ) -> Result<(Arc<dyn Source>, Listing), InstallError> {
        self.find(id, req)
    }

    /// Every listing of `id`, each with its source's name.
    pub(crate) fn listed(&self, id: &str) -> Vec<(String, Listing)> {
        self.sources()
            .iter()
            .filter_map(|s| s.index().ok().map(|i| (s.name(), i)))
            .flat_map(|(source, index)| {
                index
                    .listings
                    .into_iter()
                    .filter(|l| l.id == id)
                    .map(move |l| (source.clone(), l))
            })
            .collect()
    }

    /// The newest listing of `id` that `req` takes and the policy
    /// allows, and the registry it is in.
    fn find(&self, id: &str, req: &VersionReq) -> Result<(Arc<dyn Source>, Listing), InstallError> {
        let policy = lock(&self.inner.policy).clone();
        let mut best: Option<(Arc<dyn Source>, Listing)> = None;
        let mut refused = None;
        for source in self.sources() {
            let Ok(index) = source.index() else { continue };
            for l in index
                .listings
                .into_iter()
                .filter(|l| l.id == id && req.matches(&l.version))
            {
                if let Some(Err(why)) = policy.as_ref().map(|p| p.allows(&l.id, &l.version)) {
                    refused = Some(why);
                    continue;
                }
                if best.as_ref().is_none_or(|(_, b)| b.version < l.version) {
                    best = Some((Arc::clone(&source), l));
                }
            }
        }
        best.ok_or_else(|| {
            refused.map_or_else(
                || InstallError::NotFound(format!("{id} {req}")),
                InstallError::NotAllowed,
            )
        })
    }

    /// Installs the newest `id` that `req` takes (and the policy allows),
    /// and loads it if it is the copy that runs.
    ///
    /// # Errors
    ///
    /// No registry has it, the policy refuses it, or it cannot be fetched.
    pub fn install(&self, id: &str, req: &VersionReq) -> Result<Install, InstallError> {
        let (source, listing) = self.find(id, req)?;
        let install = self.inner.installs.install(source.as_ref(), &listing)?;
        let _ = self.sync();
        self.turned_on(&install.id);
        Ok(install)
    }

    /// Removes `id`'s install and unloads it (the built-in, if there is
    /// one, comes back). False when it was not installed.
    ///
    /// # Errors
    ///
    /// It cannot be removed.
    pub fn uninstall(&self, id: &str) -> Result<bool, InstallError> {
        let removed = self.inner.installs.uninstall(id)?;
        let _ = self.sync();
        Ok(removed)
    }

    /// Runs the package in `dir` as a development copy (`architect ext
    /// dev`): it shadows every other copy of its id, and reloads when its
    /// files change.
    #[must_use = "what happened"]
    pub fn dev(&self, dir: impl Into<PathBuf>) -> Vec<Event> {
        let dir = dir.into();
        {
            let mut dev = lock(&self.inner.dev);
            if !dev.contains(&dir) {
                dev.push(dir);
            }
        }
        self.sync()
    }

    /// Ends the development copy in `dir` (and its builds, when it is
    /// built from source; its folder stays).
    #[must_use = "what happened"]
    pub fn end_dev(&self, dir: &Path) -> Vec<Event> {
        lock(&self.inner.dev).retain(|d| d != dir);
        self.inner.builds.forget(dir);
        self.sync()
    }

    /// Turns `id` off: unloaded, and left unloaded whatever copies of it
    /// there are, until [`Self::enable`].
    #[must_use = "what happened"]
    pub fn disable(&self, id: &str) -> Vec<Event> {
        lock(&self.inner.disabled).insert(id.to_owned());
        self.sync()
    }

    /// Turns `id` back on.
    #[must_use = "what happened"]
    pub fn enable(&self, id: &str) -> Vec<Event> {
        let was_off = lock(&self.inner.disabled).remove(id);
        let events = self.sync();
        if was_off {
            self.turned_on(id);
        }
        events
    }

    /// Tells the app's list ([`TURNED_ON`]) that a person installed or
    /// turned on `id` and it runs: where the app offers its tour once.
    pub(crate) fn turned_on(&self, id: &str) {
        if self.inner.host.get(id).is_some() {
            self.inner.host.list().changed(TURNED_ON, id);
        }
    }

    /// Whether `id` is turned off.
    #[must_use]
    pub fn is_disabled(&self, id: &str) -> bool {
        lock(&self.inner.disabled).contains(id)
    }

    /// Every copy there is of every extension, and why the ones that
    /// cannot be read cannot.
    fn candidates(&self) -> (Vec<Candidate<Package>>, Vec<Event>) {
        self.candidates_with(true)
    }

    /// [`Self::candidates`]; with `unbuilt`, also a development copy built
    /// from source whose first build is not good yet (shown, not run).
    fn candidates_with(&self, unbuilt: bool) -> (Vec<Candidate<Package>>, Vec<Event>) {
        let mut out = Vec::new();
        let mut failed = Vec::new();
        let mut add = |package: Package, origin: Origin| {
            out.push(Candidate {
                id: package.manifest.id.clone(),
                version: package.manifest.version.clone(),
                origin,
                package,
            });
        };
        for p in lock(&self.inner.builtins).iter() {
            add(p.clone(), Origin::Builtin);
        }
        for install in self
            .inner
            .installs
            .list()
            .into_iter()
            .filter(Install::is_extension)
        {
            match Package::read(&install.dir) {
                Ok(p) => add(p, Origin::Installed),
                Err(e) => failed.push(Event::Failed {
                    id: install.id,
                    error: LoadError::Package(e),
                }),
            }
        }
        for dir in lock(&self.inner.dev).iter() {
            if !unbuilt && self.inner.builds.pending(dir) {
                continue;
            }
            match Package::read(dir) {
                Ok(p) => add(p, Origin::Dev),
                Err(e) => failed.push(Event::Failed {
                    id: dir.display().to_string(),
                    error: LoadError::Package(e),
                }),
            }
        }
        (out, failed)
    }

    /// Development copies built from source (`crate::source`).
    pub(crate) fn builds(&self) -> &crate::source::Builds {
        &self.inner.builds
    }

    /// Where copies go (`crate::clone`).
    pub(crate) fn dev_root_slot(&self) -> std::sync::MutexGuard<'_, Option<PathBuf>> {
        lock(&self.inner.dev_root)
    }

    /// The copies there are that cannot be read, as failed loads.
    pub(crate) fn unreadable(&self) -> Vec<Event> {
        self.candidates().1
    }

    /// Each extension's last load that failed, until it loads or goes.
    pub(crate) fn failed_loads(&self) -> Vec<(String, LoadError)> {
        lock(&self.inner.failed)
            .iter()
            .map(|(id, e)| (id.clone(), e.clone()))
            .collect()
    }

    /// Which copy of each extension runs, and what it shadows: the
    /// extensions page.
    #[must_use]
    pub fn chosen(&self) -> Vec<Chosen<Package>> {
        choose(self.candidates().0)
    }

    /// Loads the copy of each extension that should run, reloads one
    /// whose copy changed, unloads one that has none: what happened.
    #[must_use = "what happened"]
    pub fn sync(&self) -> Vec<Event> {
        let host = &self.inner.host;
        let (candidates, mut events) = self.candidates_with(false);
        let disabled = lock(&self.inner.disabled).clone();
        let chosen: Vec<_> = choose(candidates)
            .into_iter()
            .filter(|c| !disabled.contains(&c.runs.id))
            .collect();
        for loaded in host.loaded() {
            if !chosen.iter().any(|c| c.runs.id == loaded.manifest.id) {
                host.unload(&loaded.manifest.id);
                events.push(Event::Unloaded {
                    id: loaded.manifest.id,
                });
            }
        }
        // What cannot run here and is no longer chosen is forgotten.
        for (m, _) in host.not_here() {
            if !chosen.iter().any(|c| c.runs.id == m.id) {
                host.forget_not_here(&m.id);
            }
        }
        // Providers before what requires them (`[requires]`).
        let chosen = by_requires(chosen);
        let not_here = host.not_here();
        for c in &chosen {
            let runs = &c.runs;
            let same = host.get(&runs.id).is_some_and(|l| {
                l.origin == runs.origin
                    && l.manifest == runs.package.manifest
                    && l.dir.as_deref() == runs.package.dir()
            });
            // Tried, and cannot run here (#578): listed, not retried.
            let stays_out = not_here.iter().any(|(m, _)| m == &runs.package.manifest);
            if same || stays_out {
                continue;
            }
            events.push(match host.load(&runs.package, runs.origin) {
                Ok(change) => Event::Loaded(change),
                Err(error) => Event::Failed {
                    id: runs.id.clone(),
                    error,
                },
            });
        }
        // What lost a service it requires goes, until a provider is back.
        events.extend(host.drop_unprovided());
        // What is loaded now is what the next poll compares against.
        let _ = lock(&self.inner.watch).changed(self.dirs());
        events
    }

    fn dirs(&self) -> Vec<PathBuf> {
        self.inner
            .host
            .loaded()
            .into_iter()
            .filter_map(|l| l.dir)
            // A crate built from source reloads when it builds.
            .filter(|d| !self.inner.builds.has(d))
            .collect()
    }

    /// Reloads every loaded extension whose files changed since the last
    /// poll, rebuilds each development copy built from source whose
    /// sources changed and swaps in what built (`crate::source`): what
    /// happened. One that fails keeps its last good copy.
    #[must_use = "what happened"]
    pub fn reload_changed(&self) -> Vec<Event> {
        let changed = lock(&self.inner.watch).changed(self.dirs());
        let host = &self.inner.host;
        let mut events = self.rebuild_changed();
        for dir in changed {
            let Some(loaded) = host
                .loaded()
                .into_iter()
                .find(|l| l.dir.as_ref() == Some(&dir))
            else {
                continue;
            };
            let id = loaded.manifest.id.clone();
            events.push(
                match Package::read(&dir)
                    .map_err(LoadError::Package)
                    .and_then(|p| {
                        if p.manifest.id == id {
                            host.load(&p, loaded.origin)
                        } else {
                            Err(LoadError::Start(format!(
                                "its id changed to {}; remove it and install that",
                                p.manifest.id
                            )))
                        }
                    }) {
                    Ok(change) => Event::Loaded(change),
                    Err(error) => Event::Failed { id, error },
                },
            );
        }
        events
    }

    /// What opening (or joining) a project with `requirements` takes:
    /// checked against what is loaded, what the registries offer, the
    /// kinds this app runs and the organization's policy.
    #[must_use]
    pub fn open(&self, requirements: &Requirements) -> Plan {
        let here: Vec<_> = self
            .inner
            .host
            .loaded()
            .into_iter()
            .map(|l| (l.manifest.id, l.manifest.version))
            .collect();
        let policy = lock(&self.inner.policy).clone();
        let host = self.inner.host.clone();
        plan(
            requirements,
            &here,
            &self.offers(),
            move |kind| match kind {
                "data" => host.runs(Kind::Data),
                "luau" => host.runs(Kind::Luau),
                "wasm" => host.runs(Kind::Wasm),
                "process" => host.runs(Kind::Process),
                // A package of the app's own kinds (`ReaPack`'s): files.
                _ => true,
            },
            policy.as_ref(),
        )
    }

    /// Installs what `plan` says to (the person said yes, once, to all of
    /// it) and loads it: each install, or why it failed.
    #[must_use = "what happened"]
    pub fn accept(&self, plan: &Plan) -> Vec<Result<Install, (String, InstallError)>> {
        let sources = self.sources();
        let out = plan
            .install
            .iter()
            .map(|listing| {
                let source = sources
                    .iter()
                    .find(|s| s.index().is_ok_and(|i| i.listings.contains(listing)))
                    .ok_or_else(|| {
                        (
                            listing.id.clone(),
                            InstallError::NotFound(listing.id.clone()),
                        )
                    })?;
                self.inner
                    .installs
                    .install(source.as_ref(), listing)
                    .map_err(|e| (listing.id.clone(), e))
            })
            .collect();
        let _ = self.sync();
        out
    }
}

/// `chosen` with each extension after the ones providing what it requires
/// (`[requires]`), the order otherwise kept; a cycle or a missing provider
/// leaves the rest in their order (their loads say why).
fn by_requires(chosen: Vec<Chosen<Package>>) -> Vec<Chosen<Package>> {
    let provides =
        |c: &Chosen<Package>, service: &str| c.runs.package.manifest.provides.contains_key(service);
    let mut left = chosen;
    let mut out: Vec<Chosen<Package>> = Vec::with_capacity(left.len());
    while !left.is_empty() {
        let ready = left.iter().position(|c| {
            c.runs.package.manifest.requires.keys().all(|service| {
                !left
                    .iter()
                    .any(|o| o.runs.id != c.runs.id && provides(o, service))
            })
        });
        match ready {
            Some(i) => out.push(left.remove(i)),
            None => {
                out.append(&mut left);
            }
        }
    }
    out
}
