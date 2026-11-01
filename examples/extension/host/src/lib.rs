//! The extension example's host: a small app that runs extensions, as a
//! real one does (`examples/extension/README.md`).
//!
//! Three things make an app an extension host, and they are all here:
//!
//! 1. **An Action List and a settings registry** for extensions to add to.
//!    [`extensions`] puts a [`Host`] over them, runs process extensions
//!    with a [`ProcessRunner`], and finds packages in a registry.
//! 2. **A poll for saves.** [`Extensions::reload_changed`] reloads every
//!    loaded extension whose files changed; the app calls it on a timer.
//! 3. **Somewhere to draw what extensions draw.** A view publishes its
//!    coordinate spaces and mounts the layers drawn over it; each loaded
//!    extension's [`Overlay`] runs as a live script
//!    (`architect_canvas::luau::Session`) on a runtime gated by the
//!    extension's capabilities ([`Overlay::runtime`]). A process
//!    extension's view mounts with `architect_extensions::remote::RemoteView`.
//!
//! `src/main.rs` is that app with a window; `tests/` drive this library
//! headless.

use std::path::{Path, PathBuf};

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{self, Host, SectionSink};
use architect_extensions::install::{Installs, Source};
use architect_extensions::manifest::Manifest;
use architect_extensions::process::{ProcessRunner, Processes};
use architect_scripting::luau::Runtime;

/// The kind of the host's one view, the timeline: what an overlay names
/// in its `view` to draw over it.
pub const TIMELINE: &str = "example.timeline";

/// The Luau extension in the example's registry
/// (`examples/extension/registry/hello`).
pub const HELLO: &str = "example.hello";

/// The process extension (`examples/extension/counter`).
pub const COUNTER: &str = "example.counter";

/// The counter's manifest, as its crate has it.
pub const COUNTER_MANIFEST: &str = include_str!("../../counter/extension.toml");

/// The counter's program: built by cargo beside the host's.
pub const COUNTER_PROGRAM: &str = "example-extension-counter";

/// The app's extensions, packages found in `registry`.
///
/// Loaded onto `list` and `sections`, process extensions supervised in
/// `processes`, installs kept under `installed`. The `EXT_*` actions (search, install,
/// remove, list) go on `list` too, so ⌘K and a model can manage them.
pub fn extensions(
    list: ActionList,
    sections: impl SectionSink + 'static,
    processes: Processes,
    installed: PathBuf,
    registry: impl Source + 'static,
) -> Extensions {
    let mut host = Host::new(list, sections);
    // Process extensions need a runner of their own (Luau and data ones
    // are built in). Without one, a process extension fails to load with
    // "no runner"; the rest of the app runs as it was.
    if let Ok(runner) = ProcessRunner::new() {
        host = host.with_runner(runner.with_processes(processes));
    }
    let extensions = Extensions::new(host, Installs::new(installed)).with_source(registry);
    // Only fails when another feature already took an `EXT_*` id.
    let _ = extensions.install_actions();
    extensions
}

/// Lays the counter out as a package in `dir`, its program `program`.
///
/// For a development copy (`Extensions::dev`): its manifest, and a link
/// to `program` under the name its `[entry]` gives. A published process extension ships its
/// built program inside the package instead.
///
/// # Errors
///
/// `dir` cannot be written, or the link made.
#[cfg(unix)]
pub fn counter_package(program: &Path, dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("extension.toml"), COUNTER_MANIFEST)?;
    let link = dir.join(COUNTER_PROGRAM);
    if link.symlink_metadata().is_ok() {
        std::fs::remove_file(&link)?;
    }
    std::os::unix::fs::symlink(program, &link)?;
    Ok(dir.to_path_buf())
}

/// One overlay of a loaded extension: its script, and the extension it
/// runs as.
#[derive(Clone, Debug, PartialEq)]
pub struct Overlay {
    /// Its extension's manifest: the capabilities its script runs within.
    pub manifest: Manifest,
    /// Its id among the extension's contributions (`ruler`).
    pub id: String,
    /// The kind of view it draws over (`example.timeline`).
    pub view: String,
    /// The script's text, as loaded.
    pub source: String,
}

impl Overlay {
    /// What names this copy of it: a reload that changes the script
    /// changes the key, which is what starts the script again.
    #[must_use]
    pub fn key(&self) -> String {
        format!(
            "{}/{}@{}#{:x}",
            self.manifest.id,
            self.id,
            self.manifest.version,
            fingerprint(&self.source)
        )
    }

    /// A runtime for its script over `list`, gated as its extension is:
    /// its own actions, and those its `[capabilities]` name.
    #[must_use]
    pub fn runtime(&self, list: ActionList) -> Runtime {
        Runtime::new(list).with_gate(host::gate(&self.manifest))
    }
}

/// Every overlay of the extensions loaded in `host`, over views of kind
/// `view`. One whose script cannot be read is left out (its extension
/// failed to load already, or the file went since).
#[must_use]
pub fn overlays(host: &Host, view: &str) -> Vec<Overlay> {
    host.loaded()
        .into_iter()
        .flat_map(|loaded| {
            let dir = loaded.dir.clone();
            loaded
                .manifest
                .overlays
                .iter()
                .filter(|o| o.view == view)
                .filter_map(|o| {
                    let source = std::fs::read_to_string(dir.as_ref()?.join(&o.script)).ok()?;
                    Some(Overlay {
                        manifest: loaded.manifest.clone(),
                        id: o.id.clone(),
                        view: o.view.clone(),
                        source,
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// A cheap hash of `text`: FNV-1a, enough to tell two saves apart.
fn fingerprint(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The example's registry folder, `examples/extension/registry`, in
/// `here` or a folder above it (`cargo run` from anywhere in the
/// repository).
#[must_use]
pub fn find_registry(here: &Path) -> Option<PathBuf> {
    here.ancestors()
        .map(|a| a.join("examples/extension/registry"))
        .find(|d| d.join("index.toml").is_file())
}
