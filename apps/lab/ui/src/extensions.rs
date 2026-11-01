//! The lab's extensions: the metronome, a real one
//! (`apps/lab/extensions/metronome`), installed from the lab's registry
//! (`apps/lab/extensions`, found above the working directory, or at
//! `ARCHITECT_LAB_EXTENSIONS`) when the Workspace page's switch turns it
//! on, and turned off when it goes off. Loaded, its section is on the
//! settings page and its action in ⌘K; a save to its files reloads it.
//! With no registry to be found (a phone), the copy linked into the build
//! runs.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::Host;
use architect_extensions::install::{FolderRegistry, Installs};
use architect_extensions::ui::use_extension_sections;
use architect_extensions::{Package, VersionReq};
use dioxus::prelude::*;

const METRONOME: &str = "fts.metronome";

/// How often a loaded extension's files are checked for a save.
const POLL: Duration = Duration::from_millis(500);

/// The lab's registry folder.
fn registry() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("ARCHITECT_LAB_EXTENSIONS") {
        return Some(dir.into());
    }
    let here = std::env::current_dir().ok()?;
    here.ancestors()
        .map(|a| a.join("apps/lab/extensions"))
        .find(|d| d.join("index.toml").is_file())
}

/// The metronome, linked into the build.
fn builtin() -> Option<Package> {
    Package::embedded(
        include_str!("../../extensions/metronome/extension.toml"),
        &[(
            "count_in.luau",
            include_str!("../../extensions/metronome/count_in.luau"),
        )],
    )
    .ok()
}

/// Runs the lab's extensions while mounted, the metronome while
/// `metronome` is on.
#[component]
pub fn LabExtensions(metronome: bool) -> Element {
    let sections = use_extension_sections();
    let list = use_hook(|| Arc::new(ActionList::new()));
    architect_commands::use_action_list(Arc::clone(&list));
    let (extensions, from_registry) = use_hook(|| {
        let host = Host::new((*list).clone(), sections.clone());
        let root = architect::settings::app_dir("architect-lab")
            .unwrap_or_else(|_| std::env::temp_dir().join("architect-lab"));
        let mut extensions = Extensions::new(host, Installs::new(root.join("extensions")));
        let found = registry();
        match &found {
            Some(dir) => extensions = extensions.with_source(FolderRegistry::new(dir)),
            None => {
                if let Some(p) = builtin() {
                    extensions = extensions.with_builtin(p);
                }
            }
        }
        let _ = extensions.install_actions();
        // Off until the switch says otherwise.
        let _ = extensions.disable(METRONOME);
        (extensions, found.is_some())
    });
    let follow = extensions.clone();
    use_effect(use_reactive!(|metronome| {
        if metronome {
            if from_registry && follow.installs().get(METRONOME).is_none() {
                let _ = follow.install(METRONOME, &VersionReq::STAR);
            }
            let _ = follow.enable(METRONOME);
        } else {
            let _ = follow.disable(METRONOME);
        }
    }));
    use_future(move || {
        let extensions = extensions.clone();
        async move {
            loop {
                futures_timer::Delay::new(POLL).await;
                let _ = extensions.reload_changed();
            }
        }
    });
    rsx! {}
}
