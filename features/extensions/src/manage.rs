//! Installing with consent, previews, updates and pins (#181, #239 X5).
//!
//! - [`Extensions::preview`] fetches a listed version into a staging
//!   folder and reads it, without installing or running anything: its
//!   kind and what runs it, the capabilities it asks for (and which a
//!   person has not granted), what it adds to the interface, its hooks,
//!   its network hosts and folders, and where it comes from (a git
//!   repository at its ref, a registry's folder, files).
//! - [`Extensions::install_with`] installs one, asking for consent first
//!   when the app keeps it (`Host::with_grants`): what it asks past its
//!   grant is answered as [`Installed::NeedsConsent`] — the person (or an
//!   agent, after the person's yes) installs again with a [`Scope`] to
//!   grant it in.
//! - [`Extensions::update`] takes each installed extension (or one) to
//!   the newest version its pin allows, skipping what is unchanged, and
//!   stops at one whose capabilities grow.
//! - [`Extensions::pin`] keeps an install at the versions a range allows.
//! - [`Extensions::updates`] says which installs have a newer version
//!   their pin allows, with its changelog line and what it asks past what
//!   was granted, without installing anything (`EXT_UPDATE {check:
//!   true}`, the manager's "update available").
//! - [`Extensions::install_archive`] installs a signed `.archext` that came
//!   as a file or from a peer: the same signature check and consent as a
//!   registry's.

use semver::VersionReq;
use serde::Serialize;
use serde_json::{Value, json};

use crate::capabilities::Ask;
use crate::consent::Scope;
use crate::extensions::Extensions;
use crate::index::{Listing, Location};
use crate::install::InstallError;
use crate::manifest::{Capabilities, Kind, MANIFEST, Manifest};

/// What a version would be and do, read before it is installed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Preview {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    /// `data`, `luau`, `wasm`, `process`.
    pub kind: String,
    /// What runs it here, in words; and whether this app can.
    pub runner: String,
    pub runs_here: bool,
    /// The registry it is listed in, and where its files are: a git
    /// repository at a ref (`url@rev`), a registry folder, files.
    pub source: String,
    pub location: String,
    /// The ref a git package is pinned to, when it is one.
    pub git_ref: Option<String>,
    pub capabilities: Capabilities,
    /// Everything it asks for, one line each.
    pub asks: Vec<Ask>,
    /// What of that a person has not granted: empty when nothing needs a
    /// yes (or the app asks no one).
    pub needs_consent: Vec<Ask>,
    /// What it adds, one line each (`2 tabs: Board, Chart`).
    pub adds: Vec<String>,
    /// Its hooks: the event, and the action or script that runs.
    pub hooks: Vec<String>,
    /// Its actions: id, name, effect.
    pub actions: Vec<Value>,
    pub views: Vec<String>,
    pub settings: Vec<String>,
    /// Network hosts and folders it may reach.
    pub network: Vec<String>,
    pub files: Vec<String>,
    /// The version installed now, and its pin.
    pub installed: Option<String>,
    pub pinned: Option<String>,
    /// The target triples it has builds for (a process extension's
    /// archive); empty when it runs anywhere.
    pub platforms: Vec<String>,
    /// Why it cannot run here when it cannot: no build for this platform
    /// (`runs_here` is false then).
    pub cannot_run: Option<String>,
    /// The key its archive is signed by, when it is one.
    pub signed_by: Option<String>,
    /// What this version changed, as its registry says.
    pub changelog: String,
}

/// An installed extension with a newer version its pin allows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Available {
    pub id: String,
    pub name: String,
    /// The version installed, and the one an update would take.
    pub installed: String,
    pub newest: String,
    /// The newer version's changelog line (its registry's `changelog`).
    pub changelog: String,
    pub pin: Option<String>,
    /// A newer version past the pin, when there is one: not taken.
    pub past_pin: Option<String>,
    /// What the newer version asks that was not granted (or, when the app
    /// keeps no consent, that the installed one did not ask): applying it
    /// waits for a person's yes.
    pub asks_more: Vec<Ask>,
    /// Why it cannot run here, when the newer version cannot.
    pub cannot_run: Option<String>,
    /// It could not be read (fetched, verified): why.
    pub error: Option<String>,
}

/// What an install did.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Installed {
    /// Installed (and loaded, if it could be).
    Done {
        id: String,
        version: String,
        loaded: bool,
        granted: Option<String>,
    },
    /// Not installed: a person must grant what it asks first.
    NeedsConsent { preview: Box<Preview> },
}

/// What an update did to each installed extension.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Updated {
    /// Taken to a newer version: id, from, to.
    pub updated: Vec<Value>,
    /// Already the newest its pin allows: id, version, pin.
    pub unchanged: Vec<Value>,
    /// Newer, but asking for more than was granted: not updated.
    pub needs_consent: Vec<Preview>,
    /// Could not be checked or fetched: id, why.
    pub failed: Vec<Value>,
}

fn location(l: &Location) -> (String, Option<String>) {
    match l {
        Location::Path(p) => (format!("registry folder {p}"), None),
        Location::Git { url, rev } => (format!("{url}@{rev}"), Some(rev.clone())),
        Location::Files(files) => (format!("{} file(s) fetched", files.len()), None),
        Location::Archive { url, .. } => (format!("signed archive {url}"), None),
    }
}

fn runner_words(m: &Manifest) -> String {
    match m.kind {
        Kind::Data => "nothing runs: settings, presets, keys and views as data".to_owned(),
        Kind::Luau => "Luau scripts in the app's sandbox, over its actions".to_owned(),
        Kind::Wasm => "a WASM component in the app's sandbox".to_owned(),
        Kind::Process => format!(
            "its own program ({}), a process on this device",
            if m.entry.path.is_empty() {
                "unnamed"
            } else {
                &m.entry.path
            }
        ),
    }
}

/// What a contribution adds, a line a kind.
fn adds(m: &Manifest) -> Vec<String> {
    let mut out = Vec::new();
    if !m.actions.is_empty() {
        let names: Vec<&str> = m.actions.iter().map(|a| a.name.as_str()).collect();
        out.push(format!(
            "{} action(s): {}",
            m.actions.len(),
            names.join(", ")
        ));
    }
    if !m.views.is_empty() {
        let names: Vec<&str> = m.views.iter().map(|v| v.title.as_str()).collect();
        out.push(format!("{} view(s): {}", m.views.len(), names.join(", ")));
    }
    if !m.sections.is_empty() {
        let names: Vec<&str> = m.sections.iter().map(|s| s.title.as_str()).collect();
        out.push(format!("settings: {}", names.join(", ")));
    }
    if !m.themes.is_empty() {
        out.push(format!("{} theme(s)", m.themes.len()));
    }
    let contributes = serde_json::to_value(&m.contributes).unwrap_or(Value::Null);
    for (field, label) in [
        ("tabs", "tab"),
        ("status", "status item"),
        ("menus", "menu entry"),
        ("context", "context-menu entry"),
        ("bar", "bar button"),
        ("openers", "opener"),
        ("doc_kinds", "document kind"),
        ("slots", "chrome slot"),
    ] {
        let Some(items) = contributes.get(field).and_then(Value::as_array) else {
            continue;
        };
        if items.is_empty() {
            continue;
        }
        let names: Vec<String> = items
            .iter()
            .map(|i| {
                ["title", "text", "name", "id", "slot", "kind"]
                    .iter()
                    .find_map(|k| i.get(*k).and_then(Value::as_str))
                    .unwrap_or("")
                    .to_owned()
            })
            .collect();
        out.push(format!("{} {label}(s): {}", items.len(), names.join(", ")));
    }
    out
}

impl Extensions {
    /// The listing `req` picks for `id`, and its source.
    fn pick(
        &self,
        id: &str,
        req: &VersionReq,
    ) -> Result<(std::sync::Arc<dyn crate::install::Source>, Listing), InstallError> {
        self.find_listed(id, req)
    }

    /// Reads version `req` of `id` (the newest it allows) without
    /// installing it: what [`Preview`] says.
    ///
    /// # Errors
    ///
    /// No such listing, or it could not be fetched or read.
    pub fn preview(&self, id: &str, req: &VersionReq) -> Result<Preview, InstallError> {
        let (source, listing) = self.pick(id, req)?;
        let root = self.installs().root().to_path_buf();
        std::fs::create_dir_all(&root).map_err(|e| InstallError::Io(e.to_string()))?;
        let staging = root.join(format!(".preview-{}", crate::install::slug(id)));
        let _ = std::fs::remove_dir_all(&staging);
        let fetched = source.fetch(&listing, &staging);
        let mut signed_by = None;
        let read = fetched.and_then(|f| {
            let dir = match f {
                crate::install::Fetched::Copied => staging.clone(),
                crate::install::Fetched::Linked(dir) => dir,
                // Opened (its signature checked) but not written: its
                // manifest is read from the archive.
                crate::install::Fetched::Archive(bytes) => {
                    let v = self.installs().open_archive(&bytes, source.as_ref())?;
                    signed_by = Some(v.signer.clone());
                    let text = v.manifest().ok_or_else(|| {
                        InstallError::Mismatch(format!("the archive has no {MANIFEST}"))
                    })?;
                    return Manifest::parse(text).map_err(|e| InstallError::Mismatch(e.to_string()));
                }
            };
            let path = dir.join(MANIFEST);
            let text = std::fs::read_to_string(&path)
                .map_err(|e| InstallError::Io(format!("{}: {e}", path.display())))?;
            Manifest::parse(&text).map_err(|e| InstallError::Mismatch(e.to_string()))
        });
        let _ = std::fs::remove_dir_all(&staging);
        let manifest = read?;
        let mut preview = self.preview_of(&manifest, &listing, &source.name());
        preview.signed_by = signed_by;
        Ok(preview)
    }

    /// The preview of `manifest`, listed as `listing` in `source`.
    #[must_use]
    pub fn preview_of(&self, manifest: &Manifest, listing: &Listing, source: &str) -> Preview {
        let m = manifest;
        let (location, git_ref) = location(&listing.location);
        let caps = m.capabilities.clone();
        let needs_consent = self
            .host()
            .grants()
            .map_or_else(Vec::new, |g| g.needs(&m.id, &caps));
        let installed = self.installs().get(&m.id);
        let cannot_run = listing.no_build_for(crate::index::host_targets());
        Preview {
            id: m.id.clone(),
            name: m.name.clone(),
            version: m.version.to_string(),
            description: m.description.clone(),
            kind: m.kind.name().to_owned(),
            runner: runner_words(m),
            runs_here: self.host().runs(m.kind) && cannot_run.is_none(),
            source: source.to_owned(),
            location,
            git_ref,
            asks: caps.asks(),
            needs_consent,
            adds: adds(m),
            hooks: m
                .contributes
                .hooks
                .iter()
                .map(|h| {
                    format!(
                        "on {}: {}",
                        h.on,
                        h.action
                            .clone()
                            .or_else(|| h.script.clone())
                            .unwrap_or_default()
                    )
                })
                .collect(),
            actions: m
                .actions
                .iter()
                .map(|a| json!({ "id": a.id, "name": a.name, "effect": a.effect.name() }))
                .collect(),
            views: m.views.iter().map(|v| v.id.clone()).collect(),
            settings: m
                .sections
                .iter()
                .flat_map(|s| s.settings.iter().map(|x| x.key.clone()))
                .collect(),
            network: caps.hosts.clone(),
            files: caps.folders.clone(),
            capabilities: caps,
            installed: installed.as_ref().map(|i| i.version.to_string()),
            pinned: installed.and_then(|i| i.pin),
            platforms: listing.platforms.clone(),
            cannot_run,
            signed_by: None,
            changelog: listing.changelog.clone(),
        }
    }

    /// Installs `id` (the newest `req` allows), first asking for consent
    /// when the app keeps it: what it asks past its grant comes back as
    /// [`Installed::NeedsConsent`] unless `grant` says where the person
    /// granted it. `by` is who said yes (kept with the grant). `pin`
    /// keeps it at `req` from then on.
    ///
    /// # Errors
    ///
    /// Not found, not allowed, or it could not be fetched or written.
    pub fn install_with(
        &self,
        id: &str,
        req: &VersionReq,
        grant: Option<Scope>,
        by: &str,
        pin: bool,
    ) -> Result<Installed, InstallError> {
        let mut granted = None;
        if let Some(grants) = self.host().grants() {
            let preview = self.preview(id, req)?;
            if let Some(why) = preview.cannot_run {
                return Err(InstallError::NotFound(format!("{id} {}: {why}", preview.version)));
            }
            if !preview.needs_consent.is_empty() || grant.is_some() {
                match grant {
                    None => {
                        return Ok(Installed::NeedsConsent {
                            preview: Box::new(preview),
                        });
                    }
                    Some(scope) => {
                        granted = Some(scope.describe());
                        grants.grant(
                            id,
                            &preview.version,
                            preview.capabilities.clone(),
                            scope,
                            by,
                        );
                    }
                }
            }
        }
        let install = self.install(id, req)?;
        if pin {
            let range = if *req == VersionReq::STAR {
                format!("={}", install.version)
            } else {
                req.to_string()
            };
            self.installs().set_pin(id, Some(&range))?;
        }
        Ok(Installed::Done {
            loaded: self.host().get(&install.id).is_some(),
            id: install.id,
            version: install.version.to_string(),
            granted,
        })
    }

    /// Takes installed `only` (every installed extension when `None`) to
    /// the newest version its pin allows: unchanged ones are skipped, and
    /// one asking for more than was granted waits for a person
    /// ([`Updated::needs_consent`]).
    #[must_use]
    pub fn update(&self, only: Option<&str>) -> Updated {
        let mut out = Updated::default();
        for install in self
            .installs()
            .list()
            .into_iter()
            .filter(crate::install::Install::is_extension)
            .filter(|i| only.is_none_or(|o| o == i.id))
        {
            let req = match install.pin.as_deref().map(VersionReq::parse) {
                None => VersionReq::STAR,
                Some(Ok(r)) => r,
                Some(Err(e)) => {
                    out.failed
                        .push(json!({ "id": install.id, "why": format!("its pin: {e}") }));
                    continue;
                }
            };
            let newest = match self.pick(&install.id, &req) {
                Ok((_, l)) => l,
                Err(e) => {
                    out.failed
                        .push(json!({ "id": install.id, "why": e.to_string() }));
                    continue;
                }
            };
            if newest.version <= install.version {
                out.unchanged.push(json!({
                    "id": install.id,
                    "version": install.version.to_string(),
                    "pin": install.pin,
                }));
                continue;
            }
            let exact =
                VersionReq::parse(&format!("={}", newest.version)).unwrap_or(VersionReq::STAR);
            match self.install_with(&install.id, &exact, None, "update", false) {
                Ok(Installed::Done { version, .. }) => out.updated.push(json!({
                    "id": install.id,
                    "from": install.version.to_string(),
                    "to": version,
                })),
                Ok(Installed::NeedsConsent { preview }) => out.needs_consent.push(*preview),
                Err(e) => out
                    .failed
                    .push(json!({ "id": install.id, "why": e.to_string() })),
            }
        }
        out
    }

    /// Which installs (`only`, or every one) have a newer version their
    /// pin allows: its changelog line, a newer one past the pin, and what
    /// it asks past what was granted. Nothing is installed; each newer
    /// version is fetched and read (an archive's signature checked) for
    /// what it asks.
    #[must_use]
    pub fn updates(&self, only: Option<&str>) -> Vec<Available> {
        let mut out = Vec::new();
        for install in self
            .installs()
            .list()
            .into_iter()
            .filter(crate::install::Install::is_extension)
            .filter(|i| only.is_none_or(|o| o == i.id))
        {
            let req = install
                .pin
                .as_deref()
                .and_then(|p| VersionReq::parse(p).ok())
                .unwrap_or(VersionReq::STAR);
            let Ok((_, newest)) = self.pick(&install.id, &req) else {
                continue;
            };
            let past_pin = self
                .pick(&install.id, &VersionReq::STAR)
                .ok()
                .map(|(_, l)| l.version)
                .filter(|v| *v > newest.version && install.pin.is_some())
                .map(|v| v.to_string());
            if newest.version <= install.version {
                if let Some(past) = past_pin {
                    out.push(Available {
                        id: install.id.clone(),
                        name: newest.name.clone(),
                        installed: install.version.to_string(),
                        newest: install.version.to_string(),
                        changelog: String::new(),
                        pin: install.pin.clone(),
                        past_pin: Some(past),
                        asks_more: Vec::new(),
                        cannot_run: None,
                        error: None,
                    });
                }
                continue;
            }
            let exact =
                VersionReq::parse(&format!("={}", newest.version)).unwrap_or(VersionReq::STAR);
            let (asks_more, cannot_run, error) = match self.preview(&install.id, &exact) {
                Ok(p) => {
                    let more = if self.host().grants().is_some() {
                        p.needs_consent
                    } else {
                        let before = self
                            .host()
                            .get(&install.id)
                            .map(|l| l.manifest.capabilities.asks())
                            .unwrap_or_default();
                        p.asks.into_iter().filter(|a| !before.contains(a)).collect()
                    };
                    (more, p.cannot_run, None)
                }
                Err(e) => (Vec::new(), None, Some(e.to_string())),
            };
            out.push(Available {
                id: install.id.clone(),
                name: newest.name.clone(),
                installed: install.version.to_string(),
                newest: newest.version.to_string(),
                changelog: newest.changelog.clone(),
                pin: install.pin.clone(),
                past_pin,
                asks_more,
                cannot_run,
                error,
            });
        }
        out
    }

    /// Installs a signed `.archext` that came as a file or from a peer
    /// (`from` says which), as [`Self::install_with`] does a registry's:
    /// its signature checked against the trusted keys, then consent for
    /// what it asks past its grant (unless `grant` says where the person
    /// granted it), then loaded.
    ///
    /// # Errors
    ///
    /// The archive is refused (unsigned, untrusted key, tampered, no build
    /// for this platform), or it cannot be written.
    pub fn install_archive(
        &self,
        bytes: Vec<u8>,
        from: &str,
        grant: Option<Scope>,
        by: &str,
    ) -> Result<Installed, InstallError> {
        let (v, manifest, listing) = self.installs().read_archive(&bytes, from)?;
        let mut preview = self.preview_of(&manifest, &listing, from);
        preview.signed_by = Some(v.signer.clone());
        if let Some(why) = preview.cannot_run {
            return Err(InstallError::NotFound(format!(
                "{} {}: {why}",
                preview.id, preview.version
            )));
        }
        let mut granted = None;
        if let Some(grants) = self.host().grants() {
            if !preview.needs_consent.is_empty() || grant.is_some() {
                match grant {
                    None => {
                        return Ok(Installed::NeedsConsent {
                            preview: Box::new(preview),
                        });
                    }
                    Some(scope) => {
                        granted = Some(scope.describe());
                        grants.grant(
                            &preview.id,
                            &preview.version,
                            preview.capabilities.clone(),
                            scope,
                            by,
                        );
                    }
                }
            }
        }
        let install = self.installs().install_archive(bytes, from)?;
        let _ = self.sync();
        Ok(Installed::Done {
            loaded: self.host().get(&install.id).is_some(),
            id: install.id,
            version: install.version.to_string(),
            granted,
        })
    }

    /// Keeps installed `id` at `versions` (a semver range; its version
    /// now, exactly, when `None`), or lets it update again (`unpin`).
    ///
    /// # Errors
    ///
    /// It is not installed, or the range does not read.
    pub fn pin(
        &self,
        id: &str,
        versions: Option<&str>,
        unpin: bool,
    ) -> Result<Option<String>, InstallError> {
        if unpin {
            self.installs().set_pin(id, None)?;
            return Ok(None);
        }
        let range = if let Some(v) = versions {
            v.to_owned()
        } else {
            let installed = self
                .installs()
                .get(id)
                .ok_or_else(|| InstallError::NotFound(format!("{id} is not installed")))?;
            format!("={}", installed.version)
        };
        Ok(self.installs().set_pin(id, Some(&range))?.pin)
    }

    /// Every version of `id` the registries list, newest first, each with
    /// the registry it is in.
    #[must_use]
    pub fn versions(&self, id: &str) -> Vec<(String, Listing)> {
        let mut out = self.listed(id);
        out.sort_by(|a, b| b.1.version.cmp(&a.1.version));
        out
    }
}
