//! A distribution of Architect, as code (#491).
//!
//! A distribution is the app someone builds: its name, bundle id, icon
//! and config folder, and the extensions **compiled into it** (linked,
//! `architect_extensions::linked`), alone or as bundles. Everything else
//! stays installable at run time, as in every build. The Architect app
//! ([`architect`]) and the dev sandbox (`architect_app::sandbox`) are two;
//! a downstream build is another, a crate of its own:
//!
//! ```ignore
//! fn main() -> std::process::ExitCode {
//!     architect_desktop::run(
//!         architect_distribution::architect()
//!             .named("Studio")
//!             .bundle_id("app.fasttrackstudio.studio")
//!             .folder("studio")
//!             .linked(&setlists::ext::Setlists),
//!     )
//! }
//! ```
//!
//! One list drives every face of the build: the window's extensions
//! (`apps/ui/src/extensions.rs` `use_lab_extensions`), the headless node's
//! block renderers on share links and Sites ([`Distribution::blocks`]),
//! and the web bundle's `/app/distribution.json` ([`Distribution::describe`],
//! `cargo xtask dist`).
//!
//! The binary says which distribution it is once, first thing in `main`
//! ([`Distribution::install`]); [`current`] reads it back.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use architect_extensions::Manifest;
use architect_extensions::linked::Extension;
use architect_sites::{BlockRenderer, Blocks};
use serde_json::json;

/// A linked extension, as a distribution holds it.
pub type Linked = &'static dyn Extension;

/// A distribution's icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Icon {
    /// The app's icon as SVG text: the desktop entry's and the Dock's
    /// (`scripts/desktop/*.sh` render it), the web app's.
    pub svg: &'static str,
    /// The menu-bar item's SF Symbol, on a Mac.
    pub tray_symbol: &'static str,
    /// The tray item's freedesktop icon name, elsewhere.
    pub tray_name: &'static str,
}

impl Icon {
    /// Architect's own.
    pub const ARCHITECT: Self = Self {
        svg: include_str!("../../desktop/icons/architect.svg"),
        tray_symbol: "building.columns",
        tray_name: "accessories-text-editor",
    };

    /// `svg`, with the tray's default marks.
    #[must_use]
    pub const fn svg(svg: &'static str) -> Self {
        Self {
            svg,
            tray_symbol: Self::ARCHITECT.tray_symbol,
            tray_name: Self::ARCHITECT.tray_name,
        }
    }

    /// With the tray item's SF Symbol and freedesktop icon name.
    #[must_use]
    pub const fn tray(self, symbol: &'static str, name: &'static str) -> Self {
        Self {
            tray_symbol: symbol,
            tray_name: name,
            ..self
        }
    }
}

/// Extensions built to work together, compiled in as one (#367): each
/// stays its own extension, with its own id.
#[derive(Clone)]
pub struct Bundle {
    id: String,
    name: String,
    members: Vec<Linked>,
}

impl std::fmt::Debug for Bundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bundle")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("members", &ids(&self.members))
            .finish()
    }
}

impl Bundle {
    /// An empty bundle.
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            members: Vec::new(),
        }
    }

    /// With `extension` as a member.
    #[must_use]
    pub fn member(mut self, extension: Linked) -> Self {
        self.members.push(extension);
        self
    }

    /// Its id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Its name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its members, in order.
    #[must_use]
    pub fn members(&self) -> &[Linked] {
        &self.members
    }
}

/// An older folder a distribution takes over once
/// ([`Distribution::adopts`]).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Adopted {
    folder: String,
    env: String,
}

/// A distribution of Architect: who the app is and what is compiled in.
#[derive(Clone)]
pub struct Distribution {
    name: String,
    app: String,
    folder: String,
    bundle_id: String,
    version: String,
    icon: Icon,
    sandbox: bool,
    linked: Vec<Linked>,
    bundles: Vec<Bundle>,
    blocks: Vec<Arc<dyn BlockRenderer>>,
    adopts: Option<Adopted>,
}

impl std::fmt::Debug for Distribution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Distribution")
            .field("name", &self.name)
            .field("app", &self.app)
            .field("folder", &self.folder)
            .field("bundle_id", &self.bundle_id)
            .field("sandbox", &self.sandbox)
            .field("linked", &ids(&self.linked))
            .field("bundles", &self.bundles)
            .field("blocks", &self.blocks.len())
            .finish_non_exhaustive()
    }
}

fn ids(list: &[Linked]) -> Vec<String> {
    list.iter().map(|e| id_of(*e)).collect()
}

/// A linked extension's id, from its manifest; its manifest's first line
/// when it does not read (`Extensions::with_linked` refuses it then).
fn id_of(extension: Linked) -> String {
    Manifest::parse(extension.manifest())
        .map_or_else(|e| format!("<unreadable manifest: {e}>"), |m| m.id)
}

/// A name as a folder, an app or an id: lower case, runs of anything but
/// letters and digits as one `-`.
#[must_use]
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

impl Distribution {
    /// A distribution called `name`, with nothing compiled in.
    ///
    /// Its app name (the control socket's, `<app> actions …`) and its
    /// folder are `name`'s [`slug`], its bundle id
    /// `app.<slug>`; set them with [`app`](Self::app),
    /// [`folder`](Self::folder) and [`bundle_id`](Self::bundle_id). Its
    /// version is architect's.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        let slug = slug(&name);
        Self {
            app: slug.clone(),
            folder: slug.clone(),
            bundle_id: format!("app.{slug}"),
            name,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            icon: Icon::ARCHITECT,
            sandbox: false,
            linked: Vec::new(),
            bundles: Vec::new(),
            blocks: Vec::new(),
            adopts: None,
        }
    }

    /// Renamed, keeping what is compiled in: a build of the Architect app
    /// under a name of its own (`architect().named("Studio")`). Its app
    /// name, folder and bundle id follow the new name, as in
    /// [`new`](Self::new).
    #[must_use]
    pub fn named(self, name: impl Into<String>) -> Self {
        let fresh = Self::new(name);
        Self {
            name: fresh.name,
            app: fresh.app,
            folder: fresh.folder,
            bundle_id: fresh.bundle_id,
            adopts: None,
            ..self
        }
    }

    /// Its app name: its control socket's and its agents' guide's
    /// (`<app> actions list`), the binary it is best named.
    #[must_use]
    pub fn app(mut self, app: impl Into<String>) -> Self {
        self.app = app.into();
        self
    }

    /// Its folder's name under the platform's config dir: its settings,
    /// themes, layouts, extensions and the agents' guide.
    /// `$<FOLDER>_CONFIG_DIR` (`architect::settings::env_var`) moves it.
    #[must_use]
    pub fn folder(mut self, folder: impl Into<String>) -> Self {
        self.folder = folder.into();
        self
    }

    /// Its bundle identifier: the one instance, the login item, the Dock.
    #[must_use]
    pub fn bundle_id(mut self, id: impl Into<String>) -> Self {
        self.bundle_id = id.into();
        self
    }

    /// Its version, for the about and the web app (default: architect's).
    #[must_use]
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Its icons.
    #[must_use]
    pub const fn icon(mut self, icon: Icon) -> Self {
        self.icon = icon;
        self
    }

    /// On the sandbox's made-up data: its Inbox, Tasks and Schedule are
    /// sample lists and its first run asks nothing. Only the dev sandbox
    /// is.
    #[must_use]
    pub const fn sandbox(mut self) -> Self {
        self.sandbox = true;
        self
    }

    /// With `extension` compiled in: linked into the app as a built-in,
    /// the same extension (id, version) the store installs, which a
    /// development copy overrides (#272).
    #[must_use]
    pub fn linked(mut self, extension: Linked) -> Self {
        self.linked.push(extension);
        self
    }

    /// With every member of `bundle` compiled in.
    #[must_use]
    pub fn bundle(mut self, bundle: Bundle) -> Self {
        self.bundles.push(bundle);
        self
    }

    /// With `renderer` drawing the blocks it claims on the node's share
    /// links and Sites, before the linked extensions' embed pages: a Rust
    /// renderer such as Keyflow's engraver (`architect_sites::FnBlock`).
    #[must_use]
    pub fn block(mut self, renderer: impl BlockRenderer + 'static) -> Self {
        self.blocks.push(Arc::new(renderer));
        self
    }

    /// Takes over `folder` once (`$env` its override): when this
    /// distribution's own folder is empty, the old one is copied in
    /// ([`Distribution::adopt_folder`]), and `$env` is read as this one's
    /// override. The Architect app adopts the lab's.
    #[must_use]
    pub fn adopts(mut self, folder: impl Into<String>, env: impl Into<String>) -> Self {
        self.adopts = Some(Adopted {
            folder: folder.into(),
            env: env.into(),
        });
        self
    }

    /// Its name, for the window's title, the menus and the about.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its app name ([`app`](Self::app)).
    #[must_use]
    pub fn app_name(&self) -> &str {
        &self.app
    }

    /// Its folder's name ([`folder`](Self::folder)), also the app id its
    /// stores open with.
    #[must_use]
    pub fn folder_name(&self) -> &str {
        &self.folder
    }

    /// Its bundle identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.bundle_id
    }

    /// Its version.
    #[must_use]
    pub fn version_str(&self) -> &str {
        &self.version
    }

    /// Its icons.
    #[must_use]
    pub const fn icons(&self) -> Icon {
        self.icon
    }

    /// On made-up data ([`sandbox`](Self::sandbox)).
    #[must_use]
    pub const fn is_sandbox(&self) -> bool {
        self.sandbox
    }

    /// Its bundles.
    #[must_use]
    pub fn bundles(&self) -> &[Bundle] {
        &self.bundles
    }

    /// Every extension compiled in, in order: those linked alone, then
    /// each bundle's members; an id seen before is left out (the first
    /// wins).
    #[must_use]
    pub fn extensions(&self) -> Vec<Linked> {
        let mut seen = std::collections::BTreeSet::new();
        self.linked
            .iter()
            .chain(self.bundles.iter().flat_map(|b| b.members.iter()))
            .copied()
            .filter(|e| seen.insert(id_of(*e)))
            .collect()
    }

    /// Its folder: `<config dir>/<folder>`, or what `$<FOLDER>_CONFIG_DIR`
    /// names. `None` where the platform has no config dir.
    #[must_use]
    pub fn app_dir(&self) -> Option<PathBuf> {
        architect::settings::app_dir(&self.folder).ok()
    }

    /// [`app_dir`](Self::app_dir), or a folder of its name under the temp
    /// dir.
    #[must_use]
    pub fn app_dir_or_temp(&self) -> PathBuf {
        self.app_dir()
            .unwrap_or_else(|| std::env::temp_dir().join(&self.folder))
    }

    /// Says this is the distribution this process runs, once, before the
    /// app starts any thread; a later call is ignored and answers the
    /// first. A distribution that [adopts](Self::adopts) a folder takes
    /// its old override as its own here when only the old one is set.
    pub fn install(self) -> &'static Self {
        let mut fresh = false;
        let installed = INSTALLED.get_or_init(|| {
            fresh = true;
            self
        });
        if fresh && let Some(old) = &installed.adopts {
            let new = architect::settings::env_var(&installed.folder);
            let unset = std::env::var_os(&new).is_none_or(|v| v.is_empty());
            if let Some(value) = std::env::var_os(&old.env).filter(|v| !v.is_empty() && unset) {
                // SAFETY: `install` is the binaries' first act in `main`,
                // before any thread starts.
                unsafe { std::env::set_var(new, value) };
            }
        }
        installed
    }

    /// Copies the folder it [adopts](Self::adopts) into its own, once.
    ///
    /// When its folder is missing or empty: the old one's settings,
    /// themes, layouts and tour place carry over, and the old folder is
    /// left as it was. Nothing happens when either folder is overridden by
    /// its environment variable (a test, a session of the person's
    /// choosing). Returns the folder copied into.
    #[must_use]
    pub fn adopt_folder(&self) -> Option<PathBuf> {
        let old = self.adopts.as_ref()?;
        let overridden = |app: &str| {
            std::env::var_os(architect::settings::env_var(app)).is_some_and(|v| !v.is_empty())
        };
        if overridden(&self.folder) || overridden(&old.folder) {
            return None;
        }
        let from = architect::settings::app_dir(&old.folder).ok()?;
        let to = architect::settings::app_dir(&self.folder).ok()?;
        match copy_once(&from, &to) {
            Ok(true) => {
                tracing::info!(from = %from.display(), to = %to.display(), "app.config.migrated");
                Some(to)
            }
            Ok(false) => None,
            Err(e) => {
                tracing::warn!(from = %from.display(), to = %to.display(), "app.config.migrate_failed: {e}");
                None
            }
        }
    }

    /// The renderers the node draws a share's and a Site's blocks with
    /// (#488): its own ([`block`](Self::block)), then, with feature
    /// `pages`, each compiled-in extension's `[[embeds]] page` that claims
    /// a fence, as the app draws it. Their packages are written under
    /// `dir` (`<dir>/<id>/`) for the page runner to read.
    #[must_use]
    pub fn blocks(&self, dir: &Path) -> Blocks {
        let mut blocks = Blocks::new();
        for renderer in &self.blocks {
            blocks.add(Arc::clone(renderer));
        }
        #[cfg(feature = "pages")]
        for extension in self.extensions() {
            match page_blocks(extension, dir) {
                Ok(Some(renderer)) => blocks.add(Arc::new(renderer)),
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(%error, "a linked extension's embed pages did not load")
                }
            }
        }
        #[cfg(not(feature = "pages"))]
        let _ = dir;
        blocks
    }

    /// What it is, as the web bundle's `/app/distribution.json` and
    /// `<app> distribution` print it: its name, app, folder, bundle id,
    /// version, and every compiled-in extension (id, name, version,
    /// description, the bundle it came in).
    #[must_use]
    pub fn describe(&self) -> serde_json::Value {
        let bundle_of = |id: &str| {
            self.bundles
                .iter()
                .find(|b| b.members.iter().any(|m| id_of(*m) == id))
                .map(|b| b.id.clone())
        };
        let extensions: Vec<serde_json::Value> = self
            .extensions()
            .into_iter()
            .filter_map(|e| Manifest::parse(e.manifest()).ok())
            .map(|m| {
                json!({
                    "id": m.id,
                    "name": m.name,
                    "version": m.version.to_string(),
                    "description": m.description,
                    "bundle": bundle_of(&m.id),
                })
            })
            .collect();
        let bundles: Vec<serde_json::Value> = self
            .bundles
            .iter()
            .map(|b| json!({ "id": b.id, "name": b.name, "members": ids(&b.members) }))
            .collect();
        json!({
            "name": self.name,
            "app": self.app,
            "folder": self.folder,
            "bundle_id": self.bundle_id,
            "version": self.version,
            "sandbox": self.sandbox,
            "extensions": extensions,
            "bundles": bundles,
        })
    }
}

static INSTALLED: OnceLock<Distribution> = OnceLock::new();

/// The distribution this process runs, once it has said
/// ([`Distribution::install`]).
#[must_use]
pub fn current() -> Option<&'static Distribution> {
    INSTALLED.get()
}

/// The lab's folder before it became the Architect app: the Architect
/// app [adopts](Distribution::adopts) it.
pub const LAB_FOLDER: &str = "architect-lab";

/// The lab's override of its folder.
pub const LAB_CONFIG_ENV: &str = "ARCHITECT_LAB_CONFIG_DIR";

/// The Architect app, with architect's first-party extensions compiled in.
///
/// The person's vaults, opened or made on first run; compiled in (#368):
/// Projects, Tasks, the Time Tracker, the Calendar, Day Plans, Threads,
/// Notify and Files. Each runs alone; together a project's page shows its tasks and
/// their time, a task its timer, the calendar the tasks' days.
///
/// A downstream build starts here and renames it (`.named("Studio")`), or
/// starts from [`Distribution::new`] for none of them. The core the app
/// is made of (Bases, the theme picker, the Command Palette, Navigator,
/// the extensions manager and the settings page) is in every build.
#[must_use]
pub fn architect() -> Distribution {
    Distribution::new("Architect")
        .app("architect-desktop")
        .folder("architect")
        .bundle_id("app.fasttrackstudio.architect")
        .adopts(LAB_FOLDER, LAB_CONFIG_ENV)
        .bundle(project_management())
        .linked(&architect_calendar::Calendar)
        // Day Plans (#444): day templates, their rules, each day's plan,
        // drawn under the Calendar's events.
        .linked(&architect_dayplan::DayPlans)
        // Threads and Notify (#348): comments on anything; the bell.
        .linked(&architect_threads::Threads)
        .linked(&architect_notify::Notify)
        // Files (#347): projects and asset libraries, over FILES_*.
        .linked(&architect_files_extension::FilesExtension)
}

/// The Project Management bundle (#375): Projects, Tasks and the Time
/// Tracker.
#[must_use]
pub fn project_management() -> Bundle {
    Bundle::new("project-management", "Project Management")
        .member(&architect_projects::Projects)
        .member(&architect_tasks::Tasks)
        .member(&architect_time_tracker::TimeTracker)
}

/// `extension`'s embed pages that claim a fence, its package written
/// under `dir/<id>`; `None` when it has none.
#[cfg(feature = "pages")]
fn page_blocks(
    extension: Linked,
    dir: &Path,
) -> Result<Option<architect_sites::PageBlocks>, String> {
    let manifest = Manifest::parse(extension.manifest()).map_err(|e| e.to_string())?;
    let draws = manifest
        .contributes
        .embeds
        .iter()
        .any(|e| e.page.is_some() && !e.claims.fence.is_empty());
    if !draws {
        return Ok(None);
    }
    let at = dir.join(&manifest.id);
    let write = |rel: &str, text: &str| -> Result<(), String> {
        let path = at.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    };
    write("extension.toml", extension.manifest())?;
    for (rel, text) in extension.files() {
        write(rel, text)?;
    }
    architect_sites::PageBlocks::load(&at).map(Some)
}

/// Copies `from` into `to` when `from` is a folder and `to` is missing or
/// empty. Links are copied as links (an extension's development copy is
/// one); anything else that is neither a file nor a folder is skipped.
///
/// # Errors
///
/// A file cannot be read or written.
pub fn copy_once(from: &Path, to: &Path) -> std::io::Result<bool> {
    if !from.is_dir() {
        return Ok(false);
    }
    if to.is_dir() && std::fs::read_dir(to)?.next().is_some() {
        return Ok(false);
    }
    let mut todo = vec![(from.to_path_buf(), to.to_path_buf())];
    while let Some((src, dst)) = todo.pop() {
        std::fs::create_dir_all(&dst)?;
        for e in std::fs::read_dir(&src)? {
            let e = e?;
            let (path, into) = (e.path(), dst.join(e.file_name()));
            let kind = e.file_type()?;
            if kind.is_symlink() {
                #[cfg(unix)]
                std::os::unix::fs::symlink(std::fs::read_link(&path)?, &into)?;
            } else if kind.is_dir() {
                todo.push((path, into));
            } else if kind.is_file() {
                std::fs::copy(&path, &into)?;
            }
        }
    }
    Ok(true)
}
